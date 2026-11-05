# Limits and cancellation API

Design for decision D8's follow-up ([docs/roadmap/1.0.md](1.0.md#decisions), #529, owner's
decision of 2026-11-05 in #712): a library user can cap the time, memory and graph size of a
call into `graphviz-pure` or `graphviz-pure-raster`, and can cancel a running layout or render.

**Short version.** A `Limits` value says what to stop at. `limits.run(|| ...)` applies it to
every facade and raster call inside the closure, on the calling thread. When a limit is
reached, the call returns a `GraphvizError` of a new kind, `LimitExceeded` or `Cancelled`.
Without `run`, nothing changes: no limit applies, and every output stays byte-identical to C.

## The API

All of it is additive. `GraphvizErrorKind` is already `#[non_exhaustive]`, and `GraphvizError`
is opaque, so a new kind and a new accessor are minor changes (docs/versioning.md).

```rust
use graphviz_pure::{CancelToken, Limits};
use std::time::{Duration, Instant};

let cancel = CancelToken::new();          // Clone + Send + Sync; cancel() from anywhere
let limits = Limits::new()                // every limit off
    .max_input_bytes(1 << 20)             // DOT source length
    .max_nodes(10_000)                    // per graph, counted while parsing
    .max_edges(50_000)
    .deadline(Instant::now() + Duration::from_secs(5))
    .max_steps(5_000_000)                 // deterministic work budget
    .max_allocation(512 << 20)            // largest input-sized buffer, in bytes
    .heap_limit(2 << 30, my_heap_in_use)  // total heap, read from the caller's allocator
    .cancel_token(cancel.clone())
    .cancel_when(|| client_went_away());  // polled; needs no second thread

let svg = limits.run(|| graphviz_pure::render_dot(src, "dot", "svg"));
match svg {
    Err(e) if e.kind() == graphviz_pure::GraphvizErrorKind::LimitExceeded => {
        // e.limit() == Some(graphviz_pure::Limit::Nodes), and so on
    }
    Err(e) if e.kind() == graphviz_pure::GraphvizErrorKind::Cancelled => {}
    _ => {}
}
```

New public items, all re-exported by the facade (and `Limits`, `Limit`, `CancelToken` by the
raster crate, so a raster-only user needs nothing else):

| item | what it is |
| :--- | :--- |
| `Limits` | `#[non_exhaustive]`, `Clone + Debug + Default`, builder methods above, `run`. |
| `Limits::run<R>(&self, f: impl FnOnce() -> R) -> R` | Applies the limits to the calls `f` makes on this thread. Nested `run`s combine: the stricter value of each limit applies. |
| `CancelToken` | An `Arc<AtomicBool>`: `new`, `cancel`, `is_cancelled`. |
| `Limit` | `#[non_exhaustive]` enum: `InputBytes`, `Nodes`, `Edges`, `Deadline`, `Steps`, `Allocation`, `Heap`. |
| `GraphvizErrorKind::LimitExceeded`, `::Cancelled` | New kinds. `Display`: `Limit exceeded: <detail>` and `Cancelled`. |
| `GraphvizError::limit() -> Option<Limit>` | Which limit, for `LimitExceeded`. |

Why a scope and not a parameter. The facade has twelve entry points and the raster crate one.
A `_with_limits` twin of each doubles the API, and `LayoutOptions` reaches only layout, not
parsing or rendering. A scope covers every call, including future ones, with no new
signatures. The port already keeps per-call state this way (`diag::capture`), and rayon's
`ThreadPool::install` is the same shape.

## What each limit means

- **Input bytes, nodes, edges.** Checked while parsing, so a large input stops before it is
  built. Nodes and edges count what the DOT declares, per graph, not the nodes an engine adds
  (dot's virtual nodes). The steps, deadline and heap limits cover those.
- **Deadline.** Read from `std::time::Instant` at checkpoints. `wasm32-unknown-unknown` has no
  clock (`Instant::now` panics there), so a caller on it cannot make an `Instant`; it uses
  `cancel_when` with its own clock.
- **Steps.** Each checkpoint counts one step. Two runs of the same version on the same input
  and budget stop at the same point, so a test or a fuzzer can pin the outcome. The count is not
  stable across releases: a release that moves a checkpoint changes it.
- **Allocation.** The largest single buffer whose size the input chooses: polygon vertices
  (`peripheries`, `sides`; today's `MAX_VERTICES` budget, #591), neato's and sfdp's dense
  `n × n` matrices, and the raster pixmap (#675, #716). Checked before the allocation.
- **Heap.** Safe Rust cannot read the process heap without a global allocator, and a library
  must not install one. The caller passes `fn() -> usize` that reads its own counting allocator;
  checkpoints compare it with the limit. Without such an allocator, use the allocation limit
  and an OS limit (`ulimit -v`, a cgroup).
- **Cancel.** `CancelToken::cancel` from another thread, or `cancel_when`, a closure polled at
  checkpoints, for a single-threaded caller (an event loop, WebAssembly).

## How it stops a run

Checkpoints sit in the long loops: network simplex pivots, mincross passes and transposes,
dot's position and spline phases, neato's stress and majorization iterations, fdp's and sfdp's
iterations and levels, circo, twopi, osage and patchwork's per-component loops, and the
renderers' per-node and per-edge emission and the raster draw. A checkpoint is
`limits::tick()`: one thread-local countdown and a branch. With no `run` in force the countdown
starts at `u64::MAX` and the slow path never runs. With limits, the slow path runs every 1,024
steps (every step for a step budget smaller than that): it reads the token, the callback, the
clock and the heap probe.

When a limit is reached the thread's state records it, and from then on every checkpoint
reports it at once. The loop that saw it leaves the way it leaves at `maxiter` (a network
simplex pivot loop stops pivoting; a stress loop stops iterating), or returns the error where
the function already returns `Result`. The remaining phases then hit their own checkpoints and
stop too. The facade function checks the state when the call ends and returns the limit's
error, whatever the pipeline returned. So a limited call returns either the complete output,
byte-identical to an unlimited call, or the error; never a partial drawing.

- `render_dot_all_partial` and `render_svg_all_partial` return the graphs finished before the
  limit and the limit's error, as they do for a syntax error.
- After `layout_graph` fails on a limit, the graph's layout is unspecified: lay it out again or
  drop it.
- Diagnostics collected before the stop stay on the error.
- Worker threads of the `parallel` feature do not see the scope. Checkpoints sit outside the
  parallel sections, on the calling thread.

No thread is started, no signal is used and no `unsafe` is needed: the state is a
`thread_local!` of `Cell`s, the token an `AtomicBool`.

## Byte identity and cost

Limits are off by default, and a checkpoint never changes a float or a decision, only whether
the loop continues. An unlimited run takes the same branches as today. The fidelity ratchets
stay as they are. Each implementation PR runs the perf ratchet (instructions must not rise more
than 2%, allocations not at all); checkpoints go in outer loops, not in per-element inner loops,
to keep that.

The command line gets no flags: C has none, and its output and exit status stay C's. The
JavaScript API (`@graphviz-pure/wasm`) is a follow-up issue once the Rust API has shipped.

## Pull requests

1. This document.
2. Core and facade: `Limits`, `CancelToken`, `Limit`, the two error kinds, `run`, the checkpoint
   machinery, the input-size limits in the parser, the heap probe. Tests for each limit with a
   checkpoint in network simplex as the first loop.
3. dot: mincross, position, splines and the rest of dot's loops.
4. neato, fdp, sfdp, circo, twopi, osage and patchwork.
5. Rendering and raster: emission checkpoints, the allocation limit at the polygon, matrix and
   pixmap sites.

Each runs `scripts/check-semver.sh pr`, the fidelity tests and the perf ratchet.
