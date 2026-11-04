# What Aeneas extracts from graphviz-pure

Measured 2026-11-03 for #727 with Aeneas `ab34bd28` (Lean backend on `leanprover/lean4:v4.35.0`),
Charon `7b462d91` (`nightly-2026-09-17`), on `crates/graphviz-pure-layout` at the commit of this
file. Raw logs stay on the shared machine under `/jobs/work/gvp-lean/logs/`.

## The answer

- **The whole crate does not extract.** Charon accepts it. Aeneas fails on 1,853 function
  signatures (mostly closures and borrowed graph types), drops 87 bodies, then stops with a fatal
  internal error before writing any file.
- **Per-function extraction works.** `charon cargo --start-from <path>` translates only the named
  functions and what they call. That is how `scripts/proofs/extract.sh` runs, and it is the route
  for #728–#735.
- **Integer kernels extract and compile today**: circo's Fenwick update and prefix query, its
  indexed crossing count `count_indexed` and the reference scan it equals (`CircoCount`, after the
  refactor of #745, below), and twopi's BFS kernel `leaf_bfs` (the search inside `set_n_steps_to_leaf`, after the #766 zero-copy split) and `set_subtree_size`. All are
  committed under `proofs/GvpProofs/Extracted/` and rest on no axioms beyond Lean's three.
- **`count_indexed` extracts only in single-loop pieces** (#745). Before the refactor it was
  rejected (a `?` inside a nested loop), and `count_all_crossings_reference` needed `Iterator::zip`.
  See "The #745 refactor" for what was changed and the rule it follows.
- **Floats block every float kernel**, in two places: Aeneas rejects float arithmetic and float
  literals, and the Lean library does not define `F64`. Draft PR AeneasVerif/aeneas#1073 is the
  upstream fix. Until then, floats can only pass through untouched; #733 and #734 need a Lean model.

## Does Charon accept the crate?

Yes, with no change to the source. `charon cargo --preset=aeneas` in `crates/graphviz-pure-layout`
built the crate and its two dependencies in 73 s at 4 jobs and wrote a 696 MB LLBC file for the
whole crate. A filtered run (`--start-from`) rebuilds only the layout crate and takes 18–25 s.
Charon prints one warning: without the `miri` component, it falls back to rustc's default sysroot.
That costs only the bodies of standard-library functions, which extraction keeps opaque anyway.

## Does Aeneas accept the whole crate?

No. Aeneas ran for 88 s on 4 CPUs, then died with `Fatal error: exception
Aeneas.Errors.CFailure(_)` while translating the `Iterator` trait declaration. It wrote no file.
Before that it logged these errors:

| Count | Aeneas error | Typical source |
|---:|---|---|
| 1,814 | `Internal error: please file an issue` (function signature) | closures (`{closure}::FnOnce`), types with lifetimes such as `Dot<'_>`, `LGraph<'_>` |
| 29 | `Returns inside of nested loops are not supported yet` | ortho, pack, ns |
| 23 | `Early returns inside of loops are not supported yet` | `count_indexed`, sparse matrix checks |
| 18 | `Continue to outer loops are not supported yet` | labelled `continue` |
| 17 | `Breaks to outer loops are not supported yet` | `pack/mod.rs`, `common/ns.rs` |
| 17 | `Unimplemented` | `String` and `Cell` internals |
| 13 | `found an occurrence of a lifetime constraint` | borrowed structs |
| 14 | `Arrow types are not supported yet`, `arrow types with locally quantified regions` | function pointers and closures stored in structs (`ortho/maze.rs`) |

The 1,814 signature errors cluster in `dotsplines.rs` (134), `fdpgen/layout.rs` (126),
`neatoinit.rs` (98), `spring_electrical.rs` (84) and `mincross.rs` (82). Per-module extraction
(`--start-from crate::circogen::blockpath`) pulls in those same types through `Circ` and `LGraph`.
So a kernel gets through when it is a free function over slices, integers and plain structs.

## circo: `blockpath.rs`, lines 339–547

| Function | Aeneas | Lean | Blocker | Smallest fix |
|---|---|---|---|---|
State at the first measurement (#727); the last column says how each was resolved.

| Function | Aeneas | Lean | Blocker | Resolution |
|---|---|---|---|---|
| `index_update` (Fenwick add) | accepted, 0.6 s | compiles, no extra axioms | see "Aeneas bugs" (`?` on `get_mut`) | #745 wrote `tree.get_mut(i)` as a `match`; now in `CircoCount` |
| `index_prefix` (Fenwick prefix sum) | accepted, 0.5 s | compiles, no extra axioms | none | in `CircoCount` |
| `wrap_crossings`, `add_crossings` | accepted | compiles | none | in `CircoCount` |
| `count_indexed` | **rejected** | — | `Early returns inside of loops are not supported yet` (the `?` inside `for &e in edges { for half in … }`), and `zip(1..)` | #745: helpers holding one loop each, an explicit order counter; now in `CircoCount` |
| `count_all_crossings`, `count_checked`, `count_with_policy` | accepted | does not compile: they call `count_indexed`, whose body was `sorry` | `count_indexed` | not extracted by #745 (they take `&mut Circ`, which holds `f64`, `String` and `BTreeMap` fields); the kernels they call are |
| `count_all_crossings_reference` | accepted, but 430–850 s (four nested loops) | does not compile: `` `zip` is not a field of structure `core.iter.traits.iterator.Iterator` `` | `list.iter().zip(1..)`: the Lean library's `Iterator` model has no `zip` | #745: the same explicit counter, and three helpers; now 10 s in all and in `CircoCount` |

## The #745 refactor: what Aeneas rejected, and the rule that fixed it

Measured 2026-11-03 with the same Aeneas and Charon. `CircoCount` is extracted with
`--start-from crate::circogen::blockpath::count_indexed --start-from
crate::circogen::blockpath::count_all_crossings_reference`; Charon plus Aeneas take 30 s cold and
10 s warm, the reference scan included (it took 430–850 s as four nested loops in one function).

| What the old code did | Aeneas | The fix (same integers, same order, same wrapping) |
|---|---|---|
| `?` inside `for &e in edges { for half in … { … ? … } }` | `Early returns inside of loops are not supported yet` | one helper per loop (`tree_step`, `exclude_edges`, `query_edges`, `restore_edges`, `open_edges`), each returning `Option`; the node loop calls them with `?`, so no `?` is more than one loop deep |
| a first loop with `return None` followed by the node loop with `?`, in one function | the same error, on the whole of `count_indexed`, even after the nested `?` were gone | the first loop is its own function, `reset_orders`. Observed, not bisected further: **keep one loop with an early exit per function** (the two-loop function was rejected until this loop moved out; every helper with a single such loop extracts) |
| `list.iter().zip(1..)` | accepted, but the Lean does not compile (`zip` missing from `Iterator`) | `for &n in list { occurrence = occurrence.wrapping_add(1); … }` |
| `c.arena.order[e]` through `c: &mut Circ` | `Circ` holds `f64`, `String`, `BTreeMap`; the signature would pull all of them in | the kernels take `order: &mut [i32]`, `head: &[usize]`, `tail: &[usize]`; `count_with_policy` splits the borrow of `c.arena` |
| `*c.arena.order.get_mut(e)? = 0`, `*open.get_mut(i)? &= !mask` (`?` on `Option<&mut T>`) | accepted, but Aeneas calls `Try::branch` with a backward function that its own template lacks (bug 1 below) | `match order.get_mut(e) { Some(slot) => *slot = 0, None => return None }`. Clippy's `question_mark` flags that, so each such function carries `#[allow(clippy::question_mark)]` and a comment |
| `open.iter().enumerate()` inside the reference scan | accepted (the library models `enumerate`) | none; kept as it was |

Standard-library functions this extraction needed a hand model for (`CircoCount/FunsExternal.lean`):
`u64::trailing_zeros`, `usize::wrapping_neg`, `<&u64 as BitAnd>::bitand`, `Range::clone`, and the
two halves of `?` on an `Option` (`Try::branch`, `FromResidual::from_residual`, here without a
backward function, because no `?` is left on a mutable borrow). `Slice::fill`, `Slice::get`,
`get_mut`, slice and range iterators needed none.

What stays in the extraction that is not the release build: Charon compiles the dev profile, so
`cfg!(debug_assertions)` is true. `add_crossings` therefore carries its `massert` and *fails* past
`i32::MAX` where the shipped binary wraps, and the reference's `crossings += 1` is a checked add.
Theorems over the extraction are about runs without overflow (`FenwickExtracted.lean`). `#[cfg(test)]`
and `circo-check` blocks are absent from the extraction. The `MUTATION` const generic is a `U8`
argument (`0#u8` in production).

The extracted Fenwick code evaluates as expected in Lean: on a four-slot tree, inserting keys 1
and 2 gives `index_prefix 2 = 2` and `index_prefix 1 = 1`. The circo types do appear in the
generated files for the functions that take `Circ`, and `Circ`'s `BTreeMap` field becomes an
opaque type in a `TypesExternal` template.

## twopi: `circle.rs`

| Function | Aeneas | Lean | Notes |
|---|---|---|---|
| `set_n_steps_to_leaf` (iterative DFS with an explicit stack; the version before #729) | accepted, 2 s total | compiles, no extra axioms | `Vec::clear`, `Vec::pop`, `last_mut` and `Range::clone` modelled by hand. **Gone since #766**: #736 replaced this code by a BFS and left the extraction of the old code in the tree |
| `set_n_steps_to_leaf` (multi-source BFS, as shipped by #729) | **internal error** in `translate_fun_sigs` (`iterator.rs` 42:0, the `Iterator` trait declaration), no file written | — | the function takes `&LGraph` and walks `g.all_edges(sg, n)`, an `impl Iterator` built from closures and `chain`; Charon accepts it, Aeneas dies on the signatures it pulls in. The extraction starts from `leaf_bfs` instead; see "The #766 zero-copy kernel" |
| `leaf_bfs` and its helpers (the same BFS over borrowed slices of `LGraph`, #766) | accepted, 2 s, with `LEdge` and `edge_ends` opaque | compiles | hand models: `Try::branch`, `from_residual`, `Vec::clear`, `<&Vec as IntoIterator>::into_iter`, and the type `LEdge` with `edge_ends` (`TypesExternal.lean`, `FunsExternal.lean`) |
| `set_subtree_size` (walk to the root per leaf; the version before #729) | accepted | compiles, no extra axioms | gone since #766 |
| `set_subtree_size` (one reverse sweep, as shipped by #729) | rejected as written: `Can't end abstraction … as it is set as non-endable` on the closure in `parent.and_then(\|p\| r.d.get_mut(p))`, and a warning that the library has no model of `Iterator::rev` | compiles after the fix | #766: a countdown `while` loop and `if let … && let …` instead of `.rev()` and the closure; same integers, same order, no allocation |
| `set_child_subtree_spans`, `set_subtree_spans`, positions | not tried in the crate | — | float arithmetic and `PI`: see below |

### The #766 zero-copy kernel

`extract.sh --check` failed on `TwopiCircle` on master (Aeneas internal error, no file written).
The cause was not a toolchain change: the committed extraction was of the code *before* #729. #736
squashed the BFS rewrite into the same commit and left the older `Funs.lean` (it still had
`LeafFrame` and the DFS stack) in the tree. Re-extracting at #736's own commit fails the same way.

A first attempt (PR #771) made the BFS extractable by copying every component's adjacency into three
scratch vectors. Review rejected it: +8 to +15 allocations per layout, and extra instructions for
components without leaves. The rule for the second attempt was that shipped Rust may change only if
allocations are exactly +0 on every ratchet case and instructions do not rise; otherwise it stays as
on master and `--check` is made honest with a marker file. The rule was met, so no marker file exists.

`leaf_bfs(out, inn, edges, member, nodes, d, queue)` is the BFS over borrowed slices of `LGraph`'s
own `out`, `inn` and `edges`, `Sub`'s `member` and `nodes`, `RData`'s node vector and the existing
queue. It visits, for each node, the out-edges whose head is in the subgraph, then the in-edges
whose tail is in it and that are not loops: `LGraph::all_edges`'s order, so the queue and every
distance are the same. Nothing is copied. Every function in it is `#[inline(always)]`.
`set_n_steps_to_leaf` is a wrapper that passes the borrows, and the extraction starts from
`leaf_bfs`, so `LGraph` stays out of the module. Each rejection on the way, in the order met:

| Shape | Aeneas | Fix |
|---|---|---|
| `is_some_and(\|node\| …)` and `.and_then(\|node\| …)` in the seed and distance steps | `Could not match the contexts` (closures) | plain `if let` and `?` |
| `for &n in nodes { if let Some(node) = d.get_mut(n) && … { queue.push(n) } }` in a function that also takes `d: &mut [RNode]` | `Could not match the contexts`, then `Internal error` for the `match` form | the seed loop alone in `seed_leaves(d: &[RNode], …)`: with a shared `d` it extracts |
| `while let Some(&n) = queue.get(head)` | `Found a case of unsupported nested borrows` on the whole loop | `while head < queue.len() { let Some(&n) = queue.get(head) else { break }; … }` |
| both neighbour loops inside the `while`, `edges: &[LEdge]` as a parameter | `unsupported nested borrows` / `Could not match the contexts`, at every shape tried: `for` or `while`, the body inline or in a helper, `&[usize]` or `&Vec<usize>`, `out.get(n)` inside or outside the callee, a bool-returning `relax_leaf`, an unconditional write. Probes isolated it: the same loop with `&[bool]` or `&[RNode]` extracts; adding an unused `edges: &[LEdge]` parameter alone makes Aeneas reject it | `LEdge` is the type with `Option<TextLabel>`, `Vec<Bezier>`, ports and floats. Declared opaque (`--opaque crate::common::types::LEdge`), with `edge_ends(e: &LEdge) -> (usize, usize)` (also `--opaque`) as the only way the kernel reads it. The generated `TypesExternal.lean` is hand-written as a two-field structure, `edge_ends` as `(e.tail, e.head)` |
| `for i in (0..len).rev()` in `set_subtree_size` | no model of `Iterator::rev` | `let mut i = len; while i > 0 { i = i.saturating_sub(1); … }` |
| `parent.and_then(\|p\| r.d.get_mut(p))` | `Can't end abstraction … non-endable` | `if let Some(p) = parent && let Some(p) = r.d.get_mut(p)` |

The cost of the `LEdge` model: a theorem over this extraction is about a graph whose edges are
`(tail, head)` pairs and trusts `edge_ends` to be that. The Rust function it models is one line.
The adjacency membership filters, the loop exclusion and the order are in the extracted code itself.

Measured with the ratchet (`scripts/bench/ratchet.py`; allocations exact, instruction counts from
Callgrind) on this branch against master's `circle.rs`: allocations +0 on all 19 cases, peak heap
+0.00%. Instructions on the three twopi cases, 5 to 8 runs of each binary in the ratchet's own
invocation: `fx_arrows` 549,576,223 (master, mean) against 549,575,584, `twopi_tree_5000` 410,596,000
against 410,494,000 (-0.026%, every run), `twopi_pairs_1000` median 623.0M against 619.3M. The pairs
case varies by about 0.5% between runs of the *same* binary (hash seeds), so one ratchet run cannot
resolve it; eight runs of each binary put the new one lower, not higher.

`RNode` carries `span` and `theta` as `f64`. They extract as `F64` fields, which needed
`GvpProofs/Extracted/Floats.lean` (below), and the two integer kernels never read them.

## What blocks floats, exactly

Aeneas rejects every float operation inside a function body. On the shapes of twopi's span code:

| Rust | Aeneas error |
|---|---|
| `span / subtree_size as f64` | `Invalid inputs for binop` |
| `a * a + b * b` | `Invalid inputs for binop` |
| `const UNSET: f64 = 10.00;`, `2.0 * PI` | `Improperly typed constant value` |
| `a != UNSET` (a float comparison, no arithmetic) | accepted, as Lean `!=` |
| `a.mul_add(b, c)`, `x.sqrt()` | accepted as calls to unmodelled functions (template `axiom`s) |

A rejected body becomes `sorry`, so it cannot be used. Separately, the Lean library at this commit
defines no `F64`. Aeneas prints the type name (`ExtractBase.ml`, `float_name`), and Lean then
rejects any structure with an `f64` field (``universe level metavariables``). In a function
signature it is worse: Lean silently binds `F64` as an implicit variable (AeneasVerif/aeneas#828,
reported 2026-11-03). We handle this in two ways:

- `proofs/lakefile.toml` sets `autoImplicit = false` for `GvpProofs`, so an undefined name is an
  error, never an implicit variable.
- `proofs/GvpProofs/Extracted/Floats.lean` defines `Aeneas.Std.F64 := Float` (IEEE binary64) and
  `F32 := Float32`, and `extract.sh` imports it in every generated file that mentions them. Lean's
  kernel cannot reason about `Float` values, so floats can be carried through but not reasoned about.

The upstream fix is draft PR AeneasVerif/aeneas#1073 ("Initial draft of basic float support for
lean backend", open since 2026-05, last updated 2026-08-31). It lowers `f32`/`f64` to Lean's
`Float32`/`Float` and keeps most operations `opaque`. It would remove the binop and constant
errors, but it would still not give IEEE semantics to prove rounding sequences with. #733 and
#734 need FloatSpec or an explicit IEEE model in either case.

## The other features on the issue's list

- **Closures**: rejected at the signature (`Internal error`, `Arrow types are not supported yet`),
  which is most of the whole-crate failure. Kernels that take a closure need it turned into a
  plain function or an enum.
- **Nested-loop `break`, `continue` and `return`**: rejected (`Breaks to outer loops`, `Continue
  to outer loops`, `Returns inside of nested loops`, `Early returns inside of loops`). One loop
  deep is fine, `?` included.
- **`HashMap`, `BTreeMap`**: they appear only as opaque types (`axiom` in `TypesExternal`). A
  function that calls their methods needs a hand-written model of each method. Neither measured
  kernel touches one.
- **`Vec` and slice methods**: the library models `len`, `push`, indexing, `get`, `get_mut`,
  `iter`, `insert` and `fill`. It lacks `pop`, `clear`, `last_mut`, `div_ceil`, `trailing_zeros`
  and `wrapping_neg`. Each missing one lands in the extraction's `FunsExternal.lean`, which is
  hand-written and committed.
- **Iterator adapters**: `zip` is declared in the library but missing from its `Iterator` model, so
  code using it does not compile; ranges and slice iterators work.
- **Trait objects**: not exercised by these kernels; the whole-crate run failed before any
  `dyn` showed up separately.

## Aeneas bugs found on the way

1. **`?` on `Option<&mut T>`.** In `index_update`, `tree.get_mut(i)?` generates a call that expects
   `Try::branch` to return a backward function. `index_prefix`'s `tree.get(i)?` and Aeneas's own
   template declare `branch` without one. No single model satisfies both, so the two Fenwick
   functions were first extracted as separate modules, each with its own `branch`. #745 sidesteps
   the bug in the Rust (`match` instead of `?` on `get_mut`), which lets all of them share one
   module; the two separate modules (`CircoFenwickQuery`, `CircoFenwickUpdate`) are gone.
2. **`--monomorphize-mut` panics** in Charon (``used `substitute` on an item coming from a
   trait``) on a file holding just the three Fenwick functions, so that workaround is unavailable.
3. **`Iterator::zip`** is in the library's function list but not in its `Iterator` structure.
   Aeneas warns, and the generated Lean fails to compile.
4. **`F64` undefined** (AeneasVerif/aeneas#828), above.

None has been reported upstream from this project yet: that is an outward-facing step, and it is
the owner's call.

## Cost

| Step | Wall clock |
|---|---|
| Charon on the whole crate (cold, with dependencies) | 73 s |
| `extract.sh` on 1–2 integer functions (Charon + Aeneas) | 18–25 s |
| Aeneas on `count_all_crossings_reference` alone, as one function of four nested loops | 430–850 s |
| `extract.sh` on `CircoCount` (`count_indexed`, the reference and their eleven helpers, after #745) | 30 s cold, 10 s warm |
| Lake build of an extracted module (oleans for Mathlib and Aeneas downloaded) | 2–3 s |
| `extract.sh --check` on the three committed extractions (first measurement) | 64 s |

## What this means for the other tickets

| Ticket | Extraction route |
|---|---|
| #728 circo Fenwick = bitset | proved over a model (#742); #745 extracted the shipped functions and restated the Fenwick lemmas over them (`FenwickExtracted.lean`); #770 proved the full simulation over the extraction (`FenwickExtractedMain.lean`) |
| #729 twopi BFS and postorder sizes | both current functions extracted; extract the new BFS the same way |
| #728 circo Fenwick = bitset | proved over a model (#742); #745 extracted the shipped functions and restated the Fenwick lemmas over them (`FenwickExtracted.lean`); the full simulation over the extraction is the next step (`proofs/wip/FenwickExtractedMain.lean`) |
| #729 twopi BFS and postorder sizes | extracted (#766): `leaf_bfs` and `set_subtree_size`, over borrowed slices, with `LEdge` modelled as `(tail, head)` |
| #730 circo incremental delta | write it with one-loop-deep `?`, one early-exit loop per function and no `zip`, then extract |
| #731 dot NS leave-edge index | not measured; `ns.rs` has 57 signature errors at crate level, so try `--start-from` on a free function over slices |
| #732 prose proofs | model only, by design |
| #733, #734 float lemmas | model only until #1073 lands, and probably after |
| #735 triangle adjacency, `canreach` | not measured; integer code, so per-function extraction is likely |
