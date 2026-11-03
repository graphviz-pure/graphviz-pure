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
- **Integer kernels extract and compile today**: circo's Fenwick update and prefix query, and twopi's
  `set_n_steps_to_leaf` and `set_subtree_size`. All four are committed under
  `proofs/GvpProofs/Extracted/` and rest on no axioms beyond Lean's three.
- **`count_indexed` does not extract**: it has a `?` inside a nested loop. `count_all_crossings`
  calls it, so that fails as well. The refactor is filed as #745; the theorem of #728 needs it.
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
| `index_update` (Fenwick add) | accepted, 0.6 s | compiles, no extra axioms | see "Aeneas bugs" (`?` on `get_mut`) | none: extracted on its own as `CircoFenwickUpdate` |
| `index_prefix` (Fenwick prefix sum) | accepted, 0.5 s | compiles, no extra axioms | none | none: `CircoFenwickQuery` |
| `wrap_crossings`, `add_crossings` | accepted | compiles | none | none |
| `count_indexed` | **rejected** | — | `Early returns inside of loops are not supported yet` (the `?` inside `for &e in edges { for half in … }`), and `zip(1..)` | #745: helpers holding one loop each, an explicit order counter |
| `count_all_crossings`, `count_checked`, `count_with_policy` | accepted | does not compile: they call `count_indexed`, whose body is `sorry` | `count_indexed` | #745 |
| `count_all_crossings_reference` | accepted, but 430–850 s (four nested loops) | does not compile: `` `zip` is not a field of structure `core.iter.traits.iterator.Iterator` `` | `list.iter().zip(1..)`: the Lean library's `Iterator` model has no `zip` | #745: the same explicit counter |

The extracted Fenwick code evaluates as expected in Lean: on a four-slot tree, inserting keys 1
and 2 gives `index_prefix 2 = 2` and `index_prefix 1 = 1`. The circo types do appear in the
generated files for the functions that take `Circ`, and `Circ`'s `BTreeMap` field becomes an
opaque type in a `TypesExternal` template.

## twopi: `circle.rs`

| Function | Aeneas | Lean | Notes |
|---|---|---|---|
| `set_n_steps_to_leaf` (iterative DFS with an explicit stack) | accepted, 2 s total | compiles, no extra axioms | `Vec::clear`, `Vec::pop`, `last_mut` and `Range::clone` modelled by hand |
| `set_subtree_size` (walk to the root per leaf) | accepted | compiles, no extra axioms | `RNode.subtree_size + 1` is a checked add in Lean (it fails on overflow), as in a debug build |
| `set_child_subtree_spans`, `set_subtree_spans`, positions | not tried in the crate | — | float arithmetic and `PI`: see below |

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
  `iter` and `insert`. It lacks `pop`, `clear`, `last_mut`, `fill`, `div_ceil`, `trailing_zeros`
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
   functions are extracted as separate modules, each with its own `branch`.
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
| Aeneas on `count_all_crossings_reference` alone | 430–850 s |
| Lake build of an extracted module (oleans for Mathlib and Aeneas downloaded) | 2–3 s |
| `extract.sh --check` on the three committed extractions | 64 s |

## What this means for the other tickets

| Ticket | Extraction route |
|---|---|
| #728 circo Fenwick = bitset | Fenwick primitives extracted now; the full `count_indexed` theorem waits for #745 |
| #729 twopi BFS and postorder sizes | both current functions extracted; extract the new BFS the same way |
| #730 circo incremental delta | write it with one-loop-deep `?` and no `zip`, then extract |
| #731 dot NS leave-edge index | not measured; `ns.rs` has 57 signature errors at crate level, so try `--start-from` on a free function over slices |
| #732 prose proofs | model only, by design |
| #733, #734 float lemmas | model only until #1073 lands, and probably after |
| #735 triangle adjacency, `canreach` | not measured; integer code, so per-function extraction is likely |
