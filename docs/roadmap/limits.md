# Limits and cancellation API

Design for decision D8's follow-up ([docs/roadmap/1.0.md](1.0.md#decisions), #529, owner's
decision of 2026-11-05 in #712): a library user can cap the time, memory and graph size of a
call into `graphviz-pure` or `graphviz-pure-raster`, and can cancel a running layout or render.

**Short version.** A `Limits` value says what to stop at; the checks are cooperative. `limits.run(|| ...)` applies it to
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
    .heap_limit(2 << 30, || my_heap_in_use()) // Fn() -> usize + Send + Sync + 'static
    .cancel_token(cancel.clone())
    .cancel_when(|| client_went_away());  // Fn() -> bool + Send + Sync + 'static; polled

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
| `Limits` | A struct with private fields (so it can grow), `Clone + Debug + Default + Send + Sync`, the builder methods above (each takes and returns `self`), and `run`. |
| `Limits::run<R>(&self, f: impl FnOnce() -> R) -> R` | Applies the limits to the calls `f` makes on this thread. The time, work, heap and cancel limits cover the whole `run`; the size and allocation limits apply to each call on its own. Nested `run`s combine: the stricter value of each limit applies. |
| `CancelToken` | An `Arc<AtomicBool>`: `new`, `cancel`, `is_cancelled`. |
| `Limit` | `#[non_exhaustive]` enum: `InputBytes`, `Nodes`, `Edges`, `Deadline`, `Steps`, `Allocation`, `Heap`. |
| `GraphvizErrorKind::LimitExceeded`, `::Cancelled` | New kinds. `Display`: `Limit exceeded: <detail>` and `Cancelled: the call was cancelled`. |
| `GraphvizError::limit() -> Option<Limit>` | Which limit, for `LimitExceeded`. |

Why a scope and not a parameter. The facade has twelve entry points and the raster crate one.
A `_with_limits` twin of each doubles the API, and `LayoutOptions` reaches only layout, not
parsing or rendering. A scope covers every call, including future ones, with no new
signatures. The port already keeps per-call state this way (`diag::capture`), and rayon's
`ThreadPool::install` is the same shape.

## What each limit means

- **Input bytes, nodes, edges.** The input's length is checked when the call starts. Nodes
  and edges are counted while parsing, per graph: the edge limit at each edge (one statement
  such as `{a0 ... a2999} -> {b0 ... b2999}` asks for nine million), the node limit at the
  next statement, so one long node list is read before it stops (the input-bytes limit bounds
  it). They count what the DOT declares, not what an engine adds (dot's virtual nodes); the
  steps, deadline, allocation and heap limits cover those.
- **Deadline.** Read from `std::time::Instant` at checkpoints. `wasm32-unknown-unknown` has no
  clock (`Instant::now` panics there), so a caller on it cannot make an `Instant`; it uses
  `cancel_when` with its own clock.
- **Steps.** Each checkpoint counts its units of work: one for a fine-grained loop (a pivot),
  the elements visited for a coarse one (a stress iteration counts n² for n nodes). Two runs of
  the same version on the same input and budget stop at the same point, so a test or a fuzzer
  can pin the outcome. The count is not stable across releases: a release that moves a
  checkpoint changes it.
- **Allocation.** The largest single buffer whose size the input chooses, at four sites:
  polygon vertices (`peripheries`, `sides`, #591), neato's dense model (Kamada-Kawai's two
  `n × n` arrays, SGD's pair terms, stress majorization's packed Laplacians), dot's node and
  edge arenas, and the raster pixmap, after C's cairo scaling (#675, #716). Checked before
  the allocation. dot's arenas are checked, with the doubling `Vec` does, before the two
  phases that grow them in bulk: a virtual-node chain (a large `minlen`) and position's
  auxiliary graph (a slack node per edge). Any other growth of them past the limit stops the
  call after that one reallocation. Past it the current call
  stops: every later checkpoint in that call reports it, and the call returns its error. The
  next call in the `run` is not stopped by it. Other buffers (sfdp's matrices, the output
  string) are not covered. The port's own vertex budget (`MAX_VERTICES`) is checked first:
  `peripheries=2147483647` fails with C's out-of-memory message, an error of kind
  `Allocation` whose `limit()` is `None`, whatever `max_allocation` is.
- **Heap.** Safe Rust cannot read the process heap without a global allocator, and a library
  must not install one. The caller passes a closure (`Fn() -> usize + Send + Sync + 'static`)
  that reads its own counting allocator; checkpoints compare it with the limit. It is read
  only at checkpoints, so one allocation larger than the room left goes through (a node of
  10⁸ peripheries allocates 3.2 GB of vertices in one array, and the heap limit stops the call
  after it). Use the allocation limit for the input-sized buffers and an OS limit (`ulimit
  -v`, a cgroup) for the rest.
- **Cancel.** `CancelToken::cancel` from another thread, or `cancel_when`, a closure polled at
  checkpoints, for a single-threaded caller (an event loop, WebAssembly).

## How it stops a run

Checkpoints sit in these loops and phase boundaries:

- the parser's statement loop and each edge of an edge statement;
- the start of every layout and every render;
- network simplex: the tight-tree merge and the pivots;
- dot: each phase's start, each component's and cluster's mincross, mincross steps (each rank
  counts its nodes), transposes, virtual-node chains, the auxiliary graph of position, spline
  routing per edge group, and the ortho router per edge;
- neato: stress majorization, Kamada-Kawai, SGD, shortest paths, and overlap removal (prism's
  tries and scaling, voronoi, scale);
- fdp: the force iterations and overlap removal; sfdp: the spring-electrical iterations and the
  stress smoothing; circo: crossing reduction;
- the spline routing that neato, fdp, circo, twopi, osage and patchwork share, per node;
- the SVG-family emitter, per node, per edge and per periphery; the raster crate before the
  pixmap is allocated and after the draw, and `render_pdf` when it starts, before the
  conversion and after it.

twopi, osage and patchwork have no checkpoint of their own: their layouts are linear or
`n log n` past the shared phases (packing, spline routing, rendering), which do check. The
check modes (`mincross-check`, `ns-check`, ...) replay some loops to compare them with C's;
checkpoints that end a loop sit at the entries of those loops, not inside the compared parts.

A checkpoint is
`limits::tick()` or `limits::tick_by(units)`: one thread-local countdown and a branch. With no
`run` in force the countdown starts at `u64::MAX` and the slow path never runs. With limits, the
slow path runs every 1,024 units, or sooner when the work budget is about to run out: it reads
the tokens, the callbacks, the clock and the heap probe.

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

No thread is started, no signal is used and no `unsafe` is needed: the state is a
`thread_local!` of `Cell`s, the token an `AtomicBool`.

### The checks are cooperative

A call stops at its next checkpoint. Between checkpoints it runs on: a coarse loop's single
iteration, a phase with none (resvg's draw is one call), or one large allocation. Measured on
the series' head (an optimized test build counting allocations with dhat) with a 200 ms
deadline, dot on a 3,000-node, 4,500-edge digraph stopped at
0.20 s, fdp on badvoro at 0.33 s and neato `overlap=false` on 3,000 nodes 0.4 s after a
cancel; a 50 ms deadline on a `size="1000,1000!"` PNG stopped at 0.5 s. That is not a bound.
Services still need a worker or process limit (SECURITY.md).

### Threads, re-entry and async

- The limits live in thread-local state. A thread that the `run` closure starts does not see
  them, and neither do the worker threads of the `parallel` feature; the checkpoints are on
  the calling thread.
- A `cancel_when` or heap-probe closure runs inside the call, on its thread. It may call back
  into the library: the nested call runs under the same limits and counts its work against
  them, and a check inside the closure reports the state as it is without running the
  closures again, so it cannot recurse.
- `run` takes a synchronous closure. An `async` caller cannot hold a `run` across an
  `.await`; call `run` inside the synchronous part, around the library calls (or on a
  blocking worker thread).

## Byte identity and cost

Limits are off by default, and a checkpoint never changes a float or a decision, only whether
the loop continues. Measured over the series against master 08553b47, the default path costs at
most +0.18% instructions on the perf-ratchet cases, with the same allocations. The twopi
cases move by up to about 1% either way from a hash seed (#794) and are left out of that
figure. An unlimited run takes the same branches as today. The fidelity ratchets
stay as they are. Each implementation PR runs the perf ratchet (instructions must not rise more
than 2%, allocations not at all); checkpoints go in outer loops, not in per-element inner loops,
to keep that.

The command line gets no flags: C has none, and its output and exit status stay C's. The
JavaScript API (`@graphviz-pure/wasm`) is a follow-up issue once the Rust API has shipped.

## Pull requests

1. This document (#787).
2. Core and facade (#792): `Limits`, `CancelToken`, `Limit`, the two error kinds, `run`, the
   checkpoint machinery, the input-size limits in the parser, the heap probe, and network
   simplex's pivot loop.
3. dot (#796): the tight-tree merge, mincross, virtual-node chains, phase boundaries,
   spline routing and the ortho router.
4. neato, fdp, sfdp and circo, overlap removal, and the spline routing the other engines share
   (#797).
5. Rendering, raster and PDF, and the allocation limit at the polygon, neato-model, dot-arena
   and pixmap sites (#799).

The series merges as one unit, so `max_allocation` never ships without its sites.

Each runs `scripts/check-semver.sh pr`, the fidelity tests and the perf ratchet.
