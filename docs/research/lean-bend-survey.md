# Can Bend2 and Lean give us provably faster layout algorithms?

Survey written 2026-11-03. Not a plan; an assessment of two questions:

1. Does Bend2 compile to Rust, and if not, how should it relate to this port?
2. Where in the codebase could a Lean proof justify doing less work, and what has the literature already proved?

## 1. Bend2 and Rust

### What Bend2 is

Bend2 (github.com/bendlang/bend, released 2026-09-17) is a from-scratch rewrite. It shares no code or
semantics with Bend 1 or HVM. It is a dependently typed language with Python-like syntax whose type
checker doubles as a proof checker. Laws go in `LAWS.bend`, proofs in `PROOF.bend`, and `bend PROOF.bend`
acts as a commit gate. The kernel of its `--verdict` mode is formalised in Lean (`bend2/bendtt.lean`);
the day-to-day checker, the parser and the compiler are not proven, and the compiler is described by
its author as 99% AI-written and not yet audited.

### Compile targets

The README is explicit: targets are C, Metal, CUDA and JavaScript; Lua, Luau and Python are planned.
There is no Rust target and none is planned. Three further facts matter more than the missing target:

- **The C output is a whole program, not a library.** Issue #813 asked for a native library target
  callable from Rust or C. The maintainer declined: the C output has process-global state, a library
  target would be "a new lane", and the team is keeping the compiler small. It now sits in
  `WONTFIX.txt` under SOON, planned but unscheduled. So a proven Bend core cannot be linked into
  the Rust binary today, and a C-side link would be `unsafe` FFI, which the project's safe-Rust rule
  forbids anyway.
- **Proofs stop at Bend's semantics.** The README says laws are proven about Bend's semantics and the
  backends can still differ, giving F32 NaN as the example. Foreign effects (C bodies) are outside the
  proofs. Nothing Bend proves transfers to Rust code we write.
- **Numeric types are Nat, U32 and F32.** There is no f64. Every layout kernel in this repo is f64
  and must reproduce C Graphviz's IEEE results bit for bit (`docs/roadmap/post-1.0.md`, decision 1).
  Bend2 cannot even state our correctness condition, let alone prove it.

### Bend to C to Rust, or Bend to Rust?

Neither. The generated C is the output of an interaction-net style compiler, not hand-structured
algorithmic C, so a line-by-line port would yield unidiomatic Rust with none of the proof carried over.
Writing a Bend-to-Rust backend would be a compiler project against a moving, unaudited frontend, and
the result would still have no f64 and no proof that survives translation.

The useful thing to borrow from Bend2 is the workflow, not the tool: a human-written law file, a
machine-checked proof that must pass before a change merges. This repo already does the informal
version of that with its proof documents and `*-check` Cargo features (see §2.1). Lean 4 is the
right place to make those proofs formal, because:

- Mathlib has the graph, order and finite-sum theory we need; Bend's `Base` has almost no Nat lemmas
  and its author has said "we need a mathlib".
- Lean has two active IEEE 754 libraries (FloatSpec, a port of Flocq, and FloatLib), so small
  rounding lemmas about f64 are statable.
- Rust can be extracted into Lean with Aeneas (via Charon) or Hax, so a proof can be about the
  actual Rust kernel for pure, integer-only functions. A 2026 experience report (arXiv 2605.30106)
  did exactly this for production Rust with AI provers closing the obligations, and lists the
  obstacles: toolchain drift, extraction limits, missing lemmas.

### Recommendation on question 1

Do not put Bend2 in the code path. Keep the Rust port as the only implementation. Use Lean 4 to prove
"fast path = C-mirror path" statements, starting with the four proofs this repo has already written
in prose, and keep the existing `*-check` features as the runtime oracle for anything not yet
formalised. Revisit Bend2 only if it ships a library target and f64.

## 2. Where proofs could buy speed in this codebase

### The constraint that shapes everything

Output must be byte-identical to C Graphviz 16.1.0. A "derived law" therefore cannot be "a better
approximation" or "a different local optimum". It has to be an exact equality: the fast path computes
the same integers, or the same IEEE doubles via the same rounding sequence, as the mirror of C.
That rules out reassociated sums, factorised Laplacians, grids or Barnes–Hut for fdp, and any change
to tie-breaking. It also makes Lean unusually well suited: equational proofs over integers and
finite sums are Mathlib's home ground, and the few float lemmas we need are small.

### 2.1 Proofs already written in prose, with runtime check modes

These are the cheapest wins. Statement and invariant exist; only the formalisation is missing.

| Proof doc | Code | Check feature | Statement to formalise |
|---|---|---|---|
| `docs/perf/ns-cut-sets.md` | `common/ns/cut.rs` | `ns-check` | C's `enter_edge(e)` candidates are exactly the cut-set members crossing against `e`; pivot choice is identical |
| `docs/perf/mincross-scans.md` | `dotgen/mincross/scan.rs` | `mincross-check` | Invariants (W), (F), (E), (M); settled-pair memo gives the same exchange decisions |
| `docs/perf/circo-crossings.md` | `circogen/blockpath.rs:339-545` | `circo-check` | Fenwick count equals the bitset count (an inversion count) |
| `docs/perf/spline-cubic-634.md` | `geom/pathplan/route.rs:252-296` | randomised test | Rounded Horner bounds are sound rejection gates under monotone IEEE rounding |

The circo one is a direct instance of inversion counting; Isabelle's AFP `List_Inversions` and a
Why3-verified Fenwick tree already exist as templates. The mincross and NS ones are pure integer
combinatorics on a spanning tree with postorder intervals; Mathlib's `SimpleGraph` and `Finset.sum`
cover the vocabulary.

### 2.2 Integer-only speedups with clean equivalence statements (not yet implemented)

| Site | Current cost | Proposed | Lemma |
|---|---|---|---|
| `common/ns.rs:401` `leave_edge` | linear scan of tree edges per pivot, stops after 30 negatives | index of negative-cut-value slots | index reproduces the cyclic scan order and the `SEARCHSIZE` cutoff |
| `circogen/blockpath.rs:624` recount after each tentative move | O(E log E) full recount per move | incremental delta for the moved node | wrapping i32 addition is a group mod 2^32, so old + delta = recount |
| `twopigen/circle.rs:61` `set_n_steps_to_leaf` | relaxation from every leaf, O(n·depth) | multi-source BFS | fixpoint equals `SimpleGraph.dist` to the nearest leaf (Mathlib has `dist`, `edist`, triangle inequality) |
| `twopigen/circle.rs:219` `set_subtree_size` | leaf-to-root walk per leaf | one postorder pass | same integers by structural induction on the tree |
| `geom/pathplan/shortest.rs` triangle adjacency | O(T²) all-pairs | edge-keyed map | each diagonal is shared by at most two triangles |
| `dotgen/position.rs:159` `canreach` | fresh DFS and HashSet per pair | memoised reachability | reachability is transitive; memo returns the same boolean |

The dot NS gain is constant-factor by design: the simplex must choose the same edges in the same
order, so no proof can make it asymptotically faster without changing output.

### 2.3 IEEE lemmas that let work be skipped

| Site | Lemma | Saves |
|---|---|---|
| `common/routespl.rs:82,370` `limit_boxes` retry loop | after `delta` doubles, `t = 2k/2N` equals `k/N` bit for bit (power-of-two scaling is exact) and min/max is idempotent | half the samples per retry |
| `neatogen/stress.rs:478` + `conjgrad.rs:28` | if `orthog1f` is a bitwise no-op (f32 mean exactly 0), the stress mat-vec equals CG's first `A·x` | one O(n²) product per dimension per iteration, gated by an O(n) check |
| `core/src/math/portable/pow` | `pow(x, 2) = x*x` exactly (already used); `y = -1`, `y = 0.5` cases with stated exceptions | pow calls in sfdp repulsion |
| `docs/perf/wasm-fma.md` software FMA | Boldo–Melquiond round-to-odd (proved in Coq) | T1b exactness argument for wasm |
| `neatogen/robust.rs` `orient2d`/`incircle` | filter error bound plus exact fallback gives the exact sign | nothing new, but Melquiond–Pion 2007 certified exactly this filter in Coq and is the model to port |

FloatSpec (Flocq port, still has `sorry`s) or FloatLib (certified executable formats, Sept 2026) are
the Lean libraries to build on. Lean's built-in `Float` has no lemmas by design.

### 2.4 Ruled out by the byte-identical rule

Spatial grid or Barnes–Hut for fdp (`mlimit = ∞` means far pairs contribute nonzero force; and
pairs are applied twice by design), factorising the fixed `lap2` in stress majorization, Fenwick in
`rcross` (correct but measured as not worth it), SIMD or reassociated reductions, any change to
convergence tolerances or iteration caps. These are the "float-order" class decision 1 forbids. If
that policy is ever relaxed for a non-default mode, convergence proofs for stress majorization and
force-directed layout exist on paper (sufficient conditions via eigenvalues) but have never been
mechanised.

## 3. Literature

### Mechanised graph and optimisation algorithms

- **Minimum-cost flow in Isabelle/HOL.** Abdulaziz and Ammer, ITP 2024 (arXiv 2602.03701):
  executable, verified capacity-scaling algorithms including Orlin's, in a reusable graph-algorithm
  library built on Lammich and Sefidgar's network-flow refinement work. Closest existing relative of
  dot's network simplex (which is the dual simplex on a spanning tree). No verified network simplex
  exists in any prover.
- **Simplex LP in Isabelle/HOL.** Parsert (arXiv 2403.19639): optimising simplex with soundness and
  termination, code-generated solver. Coq has a simplex formalisation too (Boulmé, Maréchal).
- **Shortest paths.** Mathlib has `SimpleGraph.dist`/`edist` and the path lemmas; a VU Amsterdam
  thesis has a Lean 4 graph library with Dijkstra, Kruskal and topological sort. In 2026 a team of
  Claude agents produced C-HD, a Lean-verified SSSP algorithm whose theorem bundles the program, its
  exactness and its cost bound (vals.ai). This is the clearest precedent for "prove a faster algorithm
  in Lean", but the constants are large and nothing was benchmarked.
- **APSP.** Alman and Vassilevska Williams (arXiv 2610.06783, Oct 2026) give O(n^2.9995) deterministic
  APSP, discovered by an Anthropic model. Asymptotic only, irrelevant to neato in practice: neato's
  unweighted APSP is already n BFS runs, which is optimal for sparse graphs.
- **Agentic algorithm design with Lean bounds.** arXiv 2610.35595: an LLM pipeline that proposes
  an algorithm, formalises it, and accepts it only once a runtime bound is proved. Same shape as what
  this repo would do, applied to evolutionary algorithms.

### Mechanised computational geometry

- **Convex hulls in Coq**: Pichardie and Bertot; Brun, Dufourd and Magaud (hypermaps, total
  correctness). In Isabelle: Meikle and Fleuriot.
- **Delaunay triangulation in Coq**: Dufourd and Bertot, edge flips with a well-founded termination
  argument. Relevant to `neatogen/delaunay.rs` and `voronoi.rs`.
- **Certified floating-point filters**: Melquiond and Pion 2007, the orientation-2 predicate filter,
  formally verified. This is the direct template for `neatogen/robust.rs`.
- **Round-to-odd**: Boldo and Melquiond, Coq. Already cited in `docs/perf/wasm-fma.md`.

### Graph drawing specifically

Null result. No search surfaced any mechanised proof about layout, crossing minimisation, rank
assignment or spline routing in Lean, Coq or Isabelle. Convergence results for force-directed
methods and stress majorization exist only on paper. The one adjacent formal object is inversion
counting (AFP `List_Inversions`), which is exactly what two-layer crossing counts are.

### Rust-to-Lean tooling

Aeneas (Charon → LLBC → pure Lean), Hax (Rust → Lean/F*/Rocq), and the 2026 experience report
(arXiv 2605.30106) that ran production Rust through them with AI provers. Verus, Creusot and Kani
prove in their own ecosystems, not Lean; Kani is bounded. For this repo, Aeneas on small pure
integer kernels (Fenwick count, NS cut-set membership, twopi BFS) is the realistic entry point.
Float kernels would be specified in Lean against a FloatSpec model rather than extracted.

## 4. Suggested order of work, if pursued

1. Formalise the circo Fenwick equivalence (inversion count) in Lean. Smallest, pure integer, has a
   check mode and existing templates.
2. Formalise twopi's two leaf-distance passes and implement the BFS replacement. Uses Mathlib's
   graph distance directly; a real O(n·depth) to O(n+m) saving on deep trees.
3. Formalise the NS cut-set candidate statement (`ns-cut-sets.md` §2 and §4) and the mincross
   invariants. Larger, but these guard the two biggest dot speedups already shipped.
4. Prove the `limit_boxes` and `pow` IEEE lemmas against FloatSpec. Small statements, moderate
   library friction.
5. Only then consider Aeneas extraction of the Rust kernels so proofs are about the shipped code.

Each step keeps the `*-check` feature as the runtime oracle until its Lean proof is sorry-free.

## 5. Actionable conclusions

### Decisions

1. **Bend2: no.** Not a code path, not a prototyping tool, not a proof tool. No Rust target, no library
   output, no f64, proofs do not transfer. Re-evaluate only if it ships a library target and f64.
2. **Lean 4: yes, in a `proofs/` directory with a Lake project, Mathlib as the one dependency.**
   Proof targets are "fast path = C-mirror path" equalities. The `*-check` Cargo features stay as the
   runtime oracle until each proof is sorry-free, then become optional.
3. **Rust extraction (Aeneas): later.** Only after two or three proofs exist and have paid off.

### Tickets, in order

| # | Ticket | Kind | Expected gain | Effort |
|---|---|---|---|---|
| 1 | twopi: multi-source BFS for `set_n_steps_to_leaf` and one postorder pass for `set_subtree_size`, with a Lean proof that both equal the current fixpoints | new speedup + proof | O(n·depth) to O(n+m); twopi tree_20000 spends 28% in this path | small; Mathlib `SimpleGraph.dist` covers it |
| 2 | circo: incremental crossing delta per tentative move instead of a full recount, proof that wrapped old+delta equals the recount | new speedup + proof | removes the 68–83% self-time hot spot after Fenwick; circo scales n^2.9 today | medium; pure i32 arithmetic |
| 3 | dot NS: index of negative-cut-value tree slots for `leave_edge`, proof it yields the same edge under the cyclic scan and `SEARCHSIZE = 30` | new speedup + proof | constant-factor on the remaining NS pivot cost in dot_dag_5000/20000 | medium |
| 4 | Formalise the circo Fenwick equivalence (inversion count) | proof of shipped code | none directly; retires `circo-check` and is the template for 2 | small |
| 5 | Formalise NS cut-set candidate membership and mincross invariants (W), (F), (E), (M) | proof of shipped code | protects the two largest dot speedups already merged | large |
| 6 | routespl `limit_boxes`: reuse even samples after `delta` doubles, Lean lemma on exact power-of-two scaling and min/max idempotence against FloatSpec | new speedup + float proof | halves resampling on each of up to 15 retries in spline routing | medium; first FloatSpec dependency |
| 7 | stress majorization: O(n) exact-zero-mean test to skip one O(n²) mat-vec per dimension per iteration | new speedup, runtime-gated | up to one of three mat-vecs per iteration on neato when the gate fires; measure hit rate first | small to measure, small to ship |
| 8 | Pathplan: edge-keyed triangle adjacency and memoised `canreach` in dot position | new speedup, trivial proof | removes two O(T²) and O(pairs·n) scans; small absolute time | small |

### Not worth doing

- Fenwick in dot `rcross` (already measured: pairs are too thin).
- Any grid, Barnes–Hut, Laplacian factorisation or reassociated reduction while byte-identical output
  is policy.
- Porting AlphaEvolve or the Alman–Vassilevska Williams APSP: asymptotic only, no practical gain on
  neato's sparse BFS-based APSP.

### What a first PR looks like

Ticket 1. Branch `perf/<issue>-twopi-bfs`. Add `proofs/lakefile.lean`, `proofs/Twopi/LeafDist.lean`
proving `fixpoint = dist to nearest leaf` over `SimpleGraph`, implement the BFS, keep the existing
reference test as the runtime check, benchmark tree_20000 against v1.0.0 per `docs/perf/`.
