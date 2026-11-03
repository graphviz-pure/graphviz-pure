# Journal

Append-only. One dated entry per event worth remembering for the write-up: decisions, dead ends,
timings, surprises. Delegates append under their ticket number; the coordinator appends under
"coord". Newest at the bottom. No personal names, hostnames or home paths.

## 2026-11-03 coord: why not Bend2

The owner asked whether Bend2 could compile to Rust and whether Lean could prove faster layout
algorithms. Bend2 targets C, Metal, CUDA and JavaScript only. Its C output is a whole program with
process-global state; a library target was declined in its issue #813 (planned, unscheduled). It
has Nat, U32 and F32 and no f64, so it cannot state our correctness condition. Its proofs are about
Bend's semantics and stop at the C boundary. Decision: Lean 4 directly, no Bend in the code path.
Full reasoning in `../lean-bend-survey.md`.

## 2026-11-03 coord: the owner's call

Owner: "I like the Lean 4 angle. Let's not wait until later. Let's do the Rust to Lean extraction
now. I want to test these for their true speedup." Also: make it a milestone, document it as an
experiment, keep a blow-by-blow for a blog post. Milestone 10 created, label `area:proofs`,
tickets #727–#735.

## 2026-11-03 coord: toolchain facts found while setting up

- Aeneas's Lean backend pins `leanprover/lean4:v4.35.0`; Mathlib has a tag of the same name
  (commit 56a00585). Pinned both.
- Aeneas float support: PRs #321/#332 (2024) added "support" for float literals in Charon; issue
  #828 (2026-03) still reports "unsupported floats"; draft PR #1073 (2026-05, updated 2026-08)
  adds basic float support to the Lean backend. Expectation: extraction works for integer kernels
  only; float lemmas (#733, #734) will be stated against FloatSpec or FloatLib over a model.
- Mathlib's oleans are several GB and worktrees sit on an 84%-full root disk. `scripts/proofs/lake.sh`
  symlinks `proofs/.lake` into one shared directory under `/jobs/work/gvp-lean/` and serialises
  runs with a lock.
- Four tickets overlap open perf PRs (#703 edits `ns.rs`, #706 edits `shortest.rs` and
  `limit_boxes`, #726 edits `stress.rs`/`conjgrad.rs`). Recorded as GitHub blocked-by links on
  #731, #733, #735, #734; their delegates stack on those branches rather than wait.
- The Rust-to-Lean experience report (arXiv 2605.30106) warns of Lean toolchain drift between
  Aeneas, Mathlib and the AI provers. We pin everything to one Lean version and accept an older
  Mathlib.

## 2026-11-03 #734: neato `orthog1f` no-op gate: measured, rarely fires, no speedup

**Question.** `stress_majorization_kd_mkernel` computes `lap2 · coords[k]` for the stress value.
`conjugate_gradient_mkernel` then applies `orthog1f` to the same vector and computes `A · x` again
for the initial residual. If `orthog1f` is a bitwise no-op, that second product is the first one.
Skipping it behind an O(n) exact test saves one O(n²) mat-vec per dimension per outer iteration.
The ticket said: measure first, and stop if the gate fires on under 10% of iterations.

**Method.** A scratch counter, not committed, in a release build of the CLI. Before each per-dimension
CG call it clones `coords[k]`, runs `orthog1f`, and compares bit patterns (so `-0.0` and `+0.0`
count as different). The counter also tracked "equal as floats but not bitwise": it was 0 on every
input. In the non-pinned path CG receives `coords[k]` itself, and in the pinned path a copy of it,
and nothing writes it between the stress mat-vec and the CG call. So "CG's vector equals the one
the stress mat-vec used" holds on every call, and the only open condition is the no-op test.
Inputs: the neato rows of `scripts/bench/sets/full.tsv`, plus `rnd2000` (`tests/sfdp/gen_rnd2000.gv`
with its `layout=sfdp` line removed, because that attribute overrides `-Kneato`). A "call" is one
CG call, so two per outer iteration in 2-D.

| input | n | outer iterations | CG calls | no-op calls | rate |
|---|---:|---:|---:|---:|---:|
| neato/wild_gv16_graphs_badvoro | 1,235 | 156 | 312 | 1 | 0.3% |
| neato/hw3_vpsc_random200 | 200 | 121 | 242 | 4 | 1.7% |
| neato/hw2_random_120 | 120 | 200 | 400 | 7 | 1.8% |
| neato/fx_NaN | 76 | 190 | 380 | 18 | 4.7% |
| neato/wild_gv16_graphs_b69 (main solve) | 71 | 99 | 198 | 10 | 5.1% |
| synth/neato_sparse_250 | 250 | 151 | 302 | 8 | 2.6% |
| synth/neato_sparse_500 | 500 | 180 | 360 | 5 | 1.4% |
| synth/neato_sparse_1000 | 1,000 | 200 | 400 | 0 | 0% |
| synth/neato_sparse_2000 | 2,000 | 200 | 400 | 1 | 0.25% |
| rnd2000 under neato | 2,000 | 200 | 400 | 1 | 0.25% |

Over those ten solves the rate is 55 of 3,394 CG calls, 1.6%. Where the gate fires it is mostly in
graphs of under 100 nodes, where the mat-vec costs microseconds. The same run also solved four
tiny component sub-layouts of `wild_gv16_graphs_b69` (n = 22, 3, 2, 3). Counting those, 379 of
4,034 calls (9.4%) are no-ops, but 316 of the 379 come from the n = 2 and n = 3 components, whose
mat-vecs are a handful of flops. Weighted by mat-vec cost (n²), the saved work is under 0.5% of
the stress mat-vecs, and those are a fraction of each iteration (the CG loop, which has no
reusable product, dominates).

**Why it rarely fires.** The test needs the f32 sum of all n coordinates to divide to exactly
zero. After the first iteration `x` is the output of `x += alpha p`, so its f32 sum is a rounding
residue, not zero, and the chance of landing on exactly 0 falls as n grows (not derived, only observed). The
table shows it: a few percent at n of about 70 to 250, and about zero at n of 1,000 and above.

**Decision.** Under the ticket's 10% gate: no Rust change, no Lean file. The Lean statement
("`orthog1f` leaves every element unchanged, so `A · orthog1f x = A · x`") would be a one-line
`congr`, and it would justify a gate that does not pay. A docs-only PR records the numbers.
The counter patch stays out of the repository.

**Cost.** About 0.5 h of Rust reading and counter code, about 2 h of waiting for build slots and
for the rnd2000 run (87 s), and 0 h Lean. Lean caught nothing, since no proof was attempted. Dead
ends: running `gen_rnd2000.gv` with `-Kneato` first ran sfdp, because the file sets `layout=sfdp`
(0.4 s, no counter output); removing that line fixed it. Extraction verdict: not attempted.

**What this says for the experiment.** A measurement can retire a proof ticket. The gate looked free
and exact on paper, and it costs nothing to state, but its hit rate is a property of f32 rounding,
and only a run showed it. Worth a line in the write-up: measure the gate before formalising it.

## 2026-11-03 #728: circo Fenwick count = bitset count

Result: proved, no `sorry`. `proofs/GvpProofs/Circo/Fenwick.lean`, 1,285 lines (about 260
definitions and docs, about 1,030 proof), builds with `scripts/proofs/lake.sh build`. Main theorem
`countIndexed_eq_reference`: on a well-formed block graph of at most `i32::MAX` list nodes, the
indexed counter takes none of the semantic early returns the model keeps (bounds-check `?` returns
are dropped from the model, see below) and returns the reference's `i32` total, final `order` array
and half set. Also `countWithPolicy_eq_reference` (the dispatch, without its two
reference-selecting clauses), `query_count_eq_hits` (each query's `u64` count is the
reference's natural-number count), and `countIndexed_wrapped` (the `i32` is the exact count
modulo 2^32).

Time (wall clock of the agent session): about 0.7 h, all Lean. No Rust changed; reading the Rust and
the prose took about 0.1 h of that. The previous attempt at this ticket stopped without writing
anything because the Fenwick-versus-set tracking looked hard; modelling the passes as a state
machine and proving a per-occurrence simulation made it routine.

How it went:
- Definitions first, mirroring the Rust one function per function. They compiled on the first try.
- Fenwick lemmas (about 130 lines). The only non-routine step was the update lemma: slot `j` is
  touched by an update from `i` iff `j - lowbit j < i <= j`. Proved by strong induction on `i`
  splitting on the parities of `i` and `j`, with `lowbit (2a) = 2 * lowbit a` and
  `lowbit (2a+1) = 1`; `omega` closes every case. The bit tricks on `usize`
  (`i & i.wrapping_neg()`, `i & (i - 1)`) were the one place Lean needed a SAT call: three
  64-bit identities by `bv_decide`. Those add three `bv_decide` axioms to the theorem's axiom
  list (`propext`, `Classical.choice`, `Quot.sound` otherwise). Replacing them by kernel proofs
  is possible and was not done.
- Invariant: after `k` occurrences, every set half has an edge order in `1..k`, belongs to a
  block edge, and the tree is the Fenwick tree of the order histogram of the set. One occurrence is
  four loops (exclude, query, restore-and-close, open); each is proved against a closed form, and
  the reference's first loop is proved equal to "count against the unchanged set, then erase",
  because it only erases halves the predicate excludes. The "repeated node occurrences" worry of
  the first attempt never needed a case split: erasing an absent key is a no-op in both
  counters, and reopening is blocked because an erased edge keeps a positive order.
- Dead ends: none that cost time. The `#eval` of the indexed model on a small graph does not
  terminate in the interpreter (closures of `Function.update` pile up); the reference and the
  natural-number count evaluate (23 crossings on the test graph). Not investigated further; the
  theorem makes the evaluation redundant.

What Lean caught: nothing wrong with the code. About the prose: its argument holds, but it relies
on two facts it states only for production graphs or not at all, and the proof needed both:
(1) a node's adjacency slice has no repeated edge id (the first loop would take a unit out of the
tree twice), (2) every set half belongs to a block edge whose incident-ness to the current node is
exactly "head or tail equals the node" (the prose says "exactly the halves whose head or tail
equals the current node" without saying the set is restricted to block edges). Both hold for
`CGraph::edges` for (1); (2) is a consequence of every recount clearing the bitset and only slice
edges setting bits. They are now hypotheses of the theorem and mentioned in
`docs/perf/circo-crossings.md`.

Review (a second model, reading the Rust against the Lean statement by statement) confirmed both
hypotheses hold in production and found no mismatch on the indexed path, and two limits of the
claim: the theorems do not carry the `list.len() <= i32::MAX` bound under which "indexed =
reference" is true (beyond it the Rust reference's `i32` orders wrap and Rust never runs the index),
and the model performs tree updates before, not after, the bit and order writes, which only matters
on failure paths the theorem excludes. Both are now stated in the file header.

Extraction verdict: not attempted. The shared Lean install has an `aeneas` directory but no `bin/aeneas`
binary, so the theorem is over a hand-written model; the PR lists every difference ("Model vs
Rust" in the file header). The risk that remains is that the model and the Rust drift apart, which
extraction would remove. The review above is the first measurement of that risk: two documentation
gaps, zero semantic ones, found in about an hour of a second model's time.
## 2026-11-03 #729: twopi multi-source BFS and reverse sweep

Rust change: `set_n_steps_to_leaf` (a depth-first relaxation from every leaf) became one
multi-source BFS in `find_center_node`; `set_subtree_size` (a walk to the root from every childless
node) became one pass over the discovery order of `set_n_steps_to_center` backwards. The old code
stays in the test module as the oracle, next to the recursion it was itself checked against.

- **Time.** About 0.3 h on Rust and tests, about 0.4 h on Lean, plus 27 min waiting for a cargo
  slot on a loaded machine. Most of the Lean time went into choosing the invariants; each lemma
  then compiled in one to three tries. Measurements are in the PR.
- **Proof size.** `proofs/GvpProofs/Twopi/LeafDist.lean`: 859 lines (about 715 of definitions and
  proof, the rest the header and comments), 31 theorems. No `sorry`; the headline theorems depend on
  `propext`, `Classical.choice` and `Quot.sound` only. Build: 3.5 s on a warm Mathlib cache.
- **Extraction verdict.** Not tried: the Aeneas binary did not exist on this machine when the Lean
  step began, so the theorems are over a Lean model. `LeafDist.lean` lists the seven differences
  from the Rust (finite type for indices, `ℕ` for `u64`, adjacency lists as input, the old
  iterative stack replaced by the recursion it mirrors, fuel, `parent`/`order` as inputs).
- **What Lean caught.** Nothing wrong in the code. It did sharpen three claims:
  1. The issue says "tree". The relaxation fixpoint equals the distance to the nearest leaf on every
     connected component, with cycles, parallel edges and loops. The theorem is stated for that
     (`SimpleGraph.dist` of `fromRel` of the neighbour lists), not for trees.
  2. The BFS loop is correct for **any** pop order, because a node is pushed every time its label
     drops. The proof never uses FIFO; BFS order only makes each node pushed once (a performance
     fact, not a correctness one). The `prev` skip of the old relaxation is sound for a reason
     that is not obvious in the C: the child's entry label is two above `prev`'s, so skipping
     the edge back can never leave a pair more than one apart. The proof has to carry that as a
     precondition of every call (`val prev + 2 ≤ ns`).
  3. The size sweep and the per-leaf walks agree without assuming that `n_children == 0` means
     "no node has this parent". Both just count with the same `n_children == 0` test. What the
     sweep does need is that every parent comes before its children in `order`; that is a property
     of `set_n_steps_to_center` that the proof takes as a hypothesis and the random test checks
     (it asserts the position order on every case).
- **Dead ends.**
  1. First plan for the old relaxation: a stack machine with an invariant per frame ("the newest
     frame of `x` has `ns = val x + 1`", frames sorted by value). Abandoned before writing code:
     the invariant needs an ordering fact between old and new frames of the same node that nothing
     else uses. The recursion with a Hoare-style contract (`go_pre`) was shorter: lowered labels
     keep the property "no pair more than one out of line unless it already was", proved by induction on
     fuel with the stale `ns` as a parameter.
  2. Stating the sweep by induction on the reversed list needed a "done" set; induction on the
     list in discovery order, read through `foldr`, needed none (`sweep_form`).
  3. Lean 4.35's `if_pos`/`if_neg` are deprecated in favour of `ite_eq_left`/`ite_eq_right`, which
     reads backwards; the compiler's own hint was correct, only confusing.
- **Panic sites.** The BFS and the sweep use `get` and saturating arithmetic where the old code did;
  `circle.rs` goes from 56 index / 8 arith sites to 50 / 6.

## 2026-11-03 #732: NS cut-set and mincross-scan prose proofs

Formalised the two prose proofs behind dot's largest shipped speedups, over Lean models of the
shipped code. No Rust change. Both files build with no `sorry` and only the standard axioms
(`propext`, `Classical.choice`, `Quot.sound`).

- `proofs/GvpProofs/Dot/MincrossScans.lean`, 1,773 lines, 113 definitions and theorems.
  `transpose_same`: when `build_scan` accepts, C's `transpose` loop and the memo scan make the same
  reads, with the same counts and exchange decisions in the same order. They stop in the same
  round and leave the same orders, `candidate` and `valid`, and, after `publish_scans`, the same
  rank buffers and `ND_order`. It is stated per round, because the loop need not end (#682).
- `proofs/GvpProofs/Ns/CutSets.lean`, 2,482 lines, 171 definitions and theorems.
  `Net.Valid.enterEdge_eq`: on a valid tree with (I1) for `e`, the cut-set `enter_edge(e)` with its
  fallback returns what C's stack-based `enter_edge(e)` returns once its loop ends, ties
  included. "Valid" means the `lim`/`low` numbering is current, which in production holds at the
  first pivot and after every renumbering, not at later pivots (see the review note below). Around it: the interval characterisation of subtrees, the candidate set as a
  root-free cut, Lemma 1 over the pruned stack machine, `scanned_before` as C's scan order, and
  §4.2's algebra. That algebra is a cut basis, the pivot's new basis, the four cut-set bullets,
  `cutvalue(t) −= σ(t)·cutvalue(e)`, the slack shift, and (I1) for the new tree however it is
  stored. A tiny concrete instance per file shows the hypotheses can hold.

**Hours.** About 1.1 h wall clock: 0.3 h reading the pages and the Rust and designing the
models, 0.3 h on the mincross file, 0.5 h on the network-simplex file. Rust: 0 h. A single-file
check takes 5 to 30 s; the full `lake.sh build` takes 11 s on a warm cache.

**What the prose left out or got imprecise.** Nothing in either page is wrong. These are the
places the formalisation had to add a step or correct a word:

1. mincross §3.2, (M) after an exchange: `forget_pairs` walks v's and w's edges to find the
   pairs (x, y) of rank r ± 1 that have edges to v and w. The counts it protects are read from
   x's and y's edge lists. So the argument needs every edge listed at both ends. The page never
   says so; the model needs it as `View.Sym`.
2. mincross §3.1, (E): the proof uses (W) and the proven permutation at every exchange. The page
   checks them once, before the `transpose`, and does not say that exchanges keep them.
   `exchange_inv` proves they do; this was the one step the prose left out entirely.
3. mincross §3.2: "`exchange(v, w)` swaps two adjacent orders". The scan does not swap. It writes
   the absolute places `base + i + 1` and `base + i`, which equal C's swap only on a proven
   permutation (`stepS_sim`).
4. mincross §3.3, deferred `ND_order`: `publish_scans` writes only moved ranks. So the claim needs
   `ND_order` to equal the compact order on entry for every node outside a moved window. The
   page does not state this precondition; the model assumes it in `buildScan`.
5. mincross (W): the code's test (`windows_hold_their_orders`) equals the page's wording only on
   a proven permutation (`holds_iff_w`), which `build_scan` checks first.
6. NS §4.1: "the lower index in that node's list comes first (precomputed: these lists don't
   change)". The code computes nothing in advance: `first_of` walks the list. This is wrong about
   cost, not about correctness.
7. NS §4.1: the cut sets take the minimum in their storage order, using `scanned_before` only
   between equal slacks. That equals C's first minimum only if `scanned_before` is a transitive,
   total order. It is, since it is position in one list, but the page does not say it is needed
   (`foldl_cutStep_spec`).
8. NS §2 and §4.1: the proof's representation of C's scan order as a list needs each candidate
   to appear once in that list; `nodup_scanList` proves it from the tree (one parent per node,
   disjoint sibling subtrees, duplicate-free edge lists). The selection rule itself (strict
   replacement, `ns.rs` `enter_edge`) would still pick the first minimum with repeats; the page
   does not mention the duplicate-freeness the list view relies on.
9. NS Lemma 1 uses, without saying so, that `Enter` is set whenever `Slack ≤ 0`. That holds
   because `Slack` starts at `INT_MAX`; pruning would be unsafe from any start of `Slack ≤ 0`
   (`Ok`).
10. NS §4.2, Ranks: "an edge's slack changes exactly when one of its ends is in S" should read
    "only when exactly one of its ends is in S".
11. NS §4.2, Cut sets: "a fundamental cycle has no coefficient ±2" is asserted, and the toggle
    rule rests on it for the *new* tree. The proof had to show that `T − e + f` is again a
    spanning tree (it needs σ(e) = +1) and that cut potentials take two values
    (`CutBasis.pivot`, `pot_two`, `coef_range`).
12. NS §2, the child test, and the in/out choice: C picks the search by `lim(tail) < lim(head)`,
    the cut sets by `par(tail) == e`. They agree on a valid tree (`tail_child_iff`), which the page
    takes for granted. The model also had to state outright that every tree edge is the parent
    edge of exactly one end, and that `ND_tree_in`/`ND_tree_out` hold exactly the tree edges.

**What the formalisation forced into the open.** It used a root-free definition of a cut's sides
(joined by tree edges other than `e`), which turns "does not depend on the rooting" into a
definition. It replaced the incidence algebra of §4.2 with cut potentials. In that form, (I1)'s
"equivalently: the fundamental cycle contains t" and the cut-value update are each a few lines of
linear algebra. Scan order is one recursive list, `scanList`, and "comes before" is a sublist
`[c, w] <+ scanList`. With that, the climb to the LCA in `scanned_before` became an induction on
two down-paths.

**Dead ends.** None at the level of the proof. Lean fixes: `if_pos`/`if_neg` are deprecated in
this Mathlib (`ite_eq_left`/`ite_eq_right`); `to` is a keyword; `rw` across two cut-basis types
that are defeq but not syntactically equal fails, and `exact` is needed instead.

**Not covered yet.** NS: that C's two `treeupdate` walks and the cut sets' LCA climb visit exactly
the edges with σ(t) ≠ 0 (the cut-value theorem is stated per edge); the re-hanging of parent
pointers (I3); that `rerank` reaches exactly the subtree under its start; §5–§8. Mincross: §2
(`reorder_settled`), and §2.1's claim that an unconstrained view's `left2right` is false.

**Review (a second model, Rust against the Lean statements).** Two applicability gaps and two
overclaims, no error in the proofs:
- The production cut-set path does not renumber `lim`/`low` after a pivot (`cut.rs` re-hangs `par`;
  its `dfs_range` hook is a no-op). Reproduced through `rank2`: after one pivot a child had a
  larger `lim` than its parent. So `Valid` holds at the first pivot and after each renumbering,
  and `enterEdge_eq` is not yet a theorem about the later production pivots. Filed as #750. This is
  the experiment's first case of a review narrowing a proof's claimed scope: the proof was right,
  its applicability statement was wrong, and `ns-check` stays as the oracle for what it misses.
- The model's fallback runs C's search on the unchanged `Net`; Rust's fallback reconstructs ranks
  and renumbers first (§5, excluded). "Fallback included" was an overclaim; reworded.
- Mincross ports are `ℤ` in the model and `f64` in Rust; NaN ports cannot be embedded. Filed as
  #751; the theorem now states the no-NaN invariant it needs.
- Finding 8 above overstated what the selection rule needs; corrected in place.

**Extraction verdict.** Not attempted: the Aeneas binary was not installed when the Lean work
started. Both theorems are stated over models that follow the Rust functions' shape. Each file's
header has a "Model vs Rust" section. The `ns-check` and `mincross-check` features stay. This
proof retires neither of them: they also check the parts not formalised here, and they check the
shipped code, not the models.
## 2026-11-03 #727: Aeneas and Charon installed, extract.sh, CI job

Time: about one hour of wall clock from the first download to the committed extractions. No Rust
changed. Lean written by hand: 168 lines (three `FunsExternal.lean` files and `Floats.lean`).
Lean generated: 564 lines across three extractions.

Install, each step timed on the shared machine at nice 15 and 4 jobs:

| Step | Time |
|---|---|
| opam 2.6.1 static binary (checksum checked) | 1 s |
| GMP 6.3.0 from source, static, into the install dir (no system headers, no root install) | 35 s |
| `opam init` | 23 s |
| OCaml 5.3.0 switch | 173 s |
| Aeneas's OCaml dependencies | 79 s |
| `nightly-2026-09-17`, minimal profile + rustc-dev, llvm-tools, rust-src, rustfmt (1.6 GB) | 21 s |
| Charon, Rust half | 249 s |
| Charon, OCaml half | 1 s |
| Aeneas | 113 s |
| Aeneas's Lean library through Lake (a prebuilt release archive was downloaded) | 15 s |

Aeneas at `ab34bd28` still pins `leanprover/lean4:v4.35.0`, so no older commit was needed.

Dead ends and surprises, in order:
- `opam init --no-setup -n` fails because `-n` is the short form of `--no-setup`.
- The first Charon build failed three ways at once: `cargo fmt` missing from the minimal
  nightly, two dune processes started by `make -j4` fighting over dune's lock, and
  `dllzarith.so: No such file or directory` because `CAML_LD_LIBRARY_PATH` from `opam env` was
  not set. `make setup-charon` would also have run Charon's full test suite, which needs every
  target in Charon's `rust-toolchain`. Built with `make build-charon-rust` and `dune build`.
- For about 40 minutes the shared build slots refused to start anything: `/cache` was below its
  20 GB floor because of an unrelated cache. Single-file `charon rustc` probes need no slot, so
  per-function answers came from verbatim copies of the kernels first, then were confirmed on
  the real crate once space came back.
- Aeneas starts one OCaml domain per CPU; `extract.sh` passes `-sequential`.
- Aeneas needed 430–850 s for one function, `count_all_crossings_reference`, with four nested
  loops. Everything else took seconds.
- Editing `extract.sh` while a queued run was executing it broke that run (bash reads scripts
  as it goes).

What Lean caught: the generated Lean for structures with an `f64` field did not compile, because
Aeneas names `F64` without defining it. A function signature with `f64` would have compiled with
`F64` bound as an implicit variable, and every theorem over it would have been about the wrong
thing. The same bug had been filed upstream earlier the same day (AeneasVerif/aeneas#828). Fixed
here with `autoImplicit = false` for the whole library and an explicit `F64 := Float`. This is the
experiment's first finding: a silent mistranslation that only the type checker caught.

Extraction verdict: per-function, yes; whole crate, no (fatal internal error after 1,853 rejected
signatures). Committed: circo's Fenwick update and prefix query, twopi's `set_n_steps_to_leaf` and
`set_subtree_size`, each depending on no axioms beyond Lean's three. Not extractable without a
refactor: `count_indexed` (`?` inside a nested loop) and the reference scan (`zip`); filed as #745.
Floats: rejected in bodies (`Invalid inputs for binop`, `Improperly typed constant value`) until
draft PR AeneasVerif/aeneas#1073 or later. Two more upstream bugs: `?` on `Option<&mut T>` emits a
`Try::branch` shape that contradicts its own template, so the Fenwick update and query are
extracted separately; and Charon's `--monomorphize-mut` panics. Details in `extraction.md`.
## 2026-11-03 #730: circo incremental crossing delta

Rust change in `circogen/blockpath.rs`, proof in `proofs/GvpProofs/Circo/CrossingDelta.lean`
(601 lines, no `sorry`). Time: about 1.3 h on the Rust side (derivation, code, tests, measuring)
and about 1 h on Lean, overlapped. About 35 minutes of the wall clock went to waiting for the
shared build slots, which refuse to start below 20 GB free on the cache disk.

**What the issue assumed, and what is true.** The issue says the delta of a move depends on the
moved node's edges alone. It does not. `count_all_crossings` is not a chord-crossing count: on a
permutation it counts every pair of edges `(a,b)`, `(a',b')` with `a < a' < b` and `b' != b`
(nested chords included), because the C code erases the opposite edge half at the closing end and
so never closes anything. The pairs a move changes include every edge that starts between the old
and the new slot, which is a range sum over the whole list. The delta needs prefix sums of the
list without the moved node, plus the sorted ranks of the node's neighbours. Per tentative move
that is `O(deg)`; the prefix arrays are built once per node, `O(n + m)`.

**Derivation.** I found the closed form `sum fd*span - sum C(d,2)` by hand from the C loop, then
checked it against a Python port of the reference on 600 random graphs and 29,004 tentative moves
before writing any Rust or Lean. Dead ends on the way: (1) a pair-by-pair delta, which needs, for
each neighbour, the in-edges of the neighbour's own neighbours, `O(sum of neighbour degrees)`;
(2) composing the move from adjacent swaps, `O(n * deg)`; (3) a delta over the full prefix arrays
with `x` still in them, which needs a second pass over the neighbour pairs. Cutting `x` out first
makes the arrays independent of where `x` sits, so one build serves all `2 * deg` tentative moves.

**What Lean did.** Model, not extraction: when I reached this step `/jobs/work/gvp-lean/aeneas/bin/aeneas` did not
exist and `scripts/proofs/extract.sh` was not on the base branch. The model is the list as a position function (positions `2 rank + 1`
for the other nodes and `2 c` for the moved one, so the move needs no renumbering), the block as
a symmetric irreflexive relation. Proved: `crossings_formula` (the pair count equals the degree
formula), `crossings_move` (moving `x` to cut `c` gives `Kc + phi c`, with `phi` written as the
Rust loop), `fast_eq_recount` (the wrapped `i32` sum of the old total and the delta is the wrapped
total of the moved list; `toI32` is the `i32` representative, `wrap_foldl` covers the recount
adding its queries one at a time). Lean caught no bug in the Rust or in the Python derivation. It
forced three hypotheses to be explicit, which became `Delta::new`'s eligibility test: no loops,
symmetric adjacency, positions distinct. Parallel edges are outside the model; the Rust declines
them. Not proved: that C's operational loop on a permutation equals the pair count (a
state-machine proof over edge halves, about 200 more lines by estimate; covered by the
every-move oracle and the randomized tests instead), that `insert_nodelist` is the cut
`rank(n) + j`, and that the loops in `Delta::prepare` build `FP`/`DP`.

**Extraction verdict.** Not attempted. The binary appeared as I finished, but #747's report
names closures, iterator adapters (`enumerate`, `partition_point`) and `?` inside loops as
Aeneas blockers, and `Delta::phi` and `Delta::prepare` use all three. Extracting them would first
need a rewrite into loop-per-helper free functions over slices, as #745 does for `count_indexed`;
that is the follow-up that would retire the "model" gap.

**Oracle kept.** `count_all_crossings` stays; under `cfg(test)` and `circo-check` every
tentative move is recounted and asserted equal to the delta. Production lists of 16 or fewer
keep the recount scan. The delta applies to all 37 non-trivial blocks of the 282 circo test
inputs; none was declined.

**Ratchet surprise.** First version added 19 allocations on the circo case (the ratchet allows
none). Fix: `list_copy` is allocated only when a move is recounted, and `Delta::new` shares one
vector for membership and rank. Net: one allocation fewer than before.
## 2026-11-03 #733: limit_boxes retry takes only the new samples, with a binary64 lemma

Ticket: when `routesplines_` doubles `delta` and runs `limitBoxes` again, reuse the even samples and
take only the odd ones; prove the reuse bit-exact over an IEEE 754 model. First float lemma of the
experiment. Branch `perf/733-limit-boxes-reuse`, stacked on #706 (which rewrites `limit_boxes`).

**Time.** About 1 h on Rust (read the loop, the change, a unit test and a check-mode assertion;
plus waiting, the first release build queued 28 min for a build slot on a loaded machine), about
2.5 h on Lean (library setup 15 min, the float lemmas 1.5 h, the list and fold bookkeeping 40 min),
30 min on measurement and docs. The Lean file is 650 lines, 466 of them statements and proofs once comments and blank lines are
left out (the module comment is the "Model against Rust" section).

**Step 1: how is `t` computed?** The ticket warned that an accumulated `t` makes the lemma false.
It is `si / num_div` with `si += 1.0` from `0.0`: `si` accumulates, but a sum of whole numbers
below 2^53 is exact, so `si` is exactly the integer `k`; `t` is a quotient of two exact integers,
not an accumulated one. `num_div = delta * boxn` is the exact whole number `10 * 2^j * boxn`. So
the reuse is valid. Measured nothing here; read C and Rust, then proved it.

**Library choice.** FloatLib (`lean-dojo/FloatLib`, commit 8c88715) builds against our pin: its
`lean-toolchain` is `v4.35.0` and its manifest pins the same Mathlib commit (56a00585), so the
dependency adds no second Mathlib. Added as one `[[require]]` in `proofs/lakefile.toml`, with the
revision pinned; `lake update FloatLib` took 13 s, the first build of the 2,394 jobs it needs took
2 min 17 s on top of the Mathlib cache, and the package directory is 831 MB in the shared lake
directory. Nothing broke. FloatSpec (`Beneficial-AI-Foundation/FloatSpec`, the Flocq port) was not
tried: it is on `v4.34.0-rc2` with a Mathlib `v4.34.0-rc2` and a `cslib` dependency, so it cannot
share our Mathlib; bumping or duplicating would be its own ticket.

**What FloatLib gave.** A bit-exact `Model binary64` (`Model.div`, `Model.add`, `Model.mul`), with
`toReal_div_eq_roundAt_of_abs_div_le_posMaxFinite` (an operation is the real result rounded once,
finite under a bound), `roundAt` (Flocq's rounding), `roundDyadic_of_representable` (a whole number
below 2^53 is a binary64 value) and `eq_of_toReal_eq_of_nonzero` (equal reals of two nonzero finite
words are the same word). That was nearly all of it. Friction: the namespaces are long and the
names are in `FloatLib.Floats.Formats.BinaryInterchange.Model`; there is no `Model.ofNat`, so
`n as f64` is `roundDyadic` of a dyadic and has to be shown exact each time; one lemma I wanted
(`2^52 <= posMaxFinite`) is `private`, and I re-proved a weaker one; `linarith` does not see
through `2^971` (a `gcongr` step does); and nothing says a positive quotient of whole numbers does
not round to zero, so that took a monotonicity argument about `2^-53`. I did not find a bug in
FloatLib.

**What Lean caught.**
- The premise was too narrow. The ticket said power-of-two scaling is exact, so `fl(2k)/fl(2N) =
  fl(k)/fl(N)`. The lemma needs no scaling: a binary64 quotient is the real quotient rounded once
  and `2k/2N = k/N` as reals. It holds for any common factor of whole numbers below 2^53, not only
  2. It is shorter than the scaling proof would have been.
- `k = 0` is a separate case, and the first sample of every segment (`si = 0`) is it. The word
  equality rests on `eq_of_toReal_eq_of_nonzero`, which needs a nonzero result, so `0.0 / x` is
  handled by the sign rule (`+0.0` over a positive denominator), and for `k > 0` a positive
  quotient of whole numbers has to be shown not to underflow to zero.
- Writing the "folding a sample twice changes nothing" step as a lemma over `fmin`/`fmax` forced
  the question of ties: `fmin(-0.0, +0.0)` may return either, and the choice is the code
  generator's (x86 `minsd` returns its second operand, wasm `f64.min` returns `-0.0`). With a
  tie that keeps the current limit the lemma is clean; with the real `f64::min` the retry could
  flip the sign of a zero limit. The Rust now takes the odd-only retry only when no box limit
  is a zero (`may_skip_even_samples`). That guard came from the proof attempt, not from a failing
  test. The guard's own correctness is argued in prose in `docs/perf/limit-boxes-reuse.md`, not
  in Lean.
- The design the ticket described ("keep the even samples from the previous pass") needs no storage:
  C does not reset the boxes between passes, so they already hold every even sample. The change
  keeps nothing and computes less.

**Dead ends.**
- Looked for a Flocq scaling lemma for `roundAt` (`roundAt (2 * x) = 2 * roundAt x`); there is
  none in FloatLib, and it turned out unnecessary.
- First draft of the fold lemma used Mathlib's `min` on a linear order; that excludes NaN and says
  nothing about ties. Replaced by an abstract "absorbing step" (`keep`, `stable`, `covers`) with
  `<` only, which covers binary64's `<` with NaNs.
- Deleted the working Lean file with a mistyped `rm` while tidying, and rebuilt it from the
  scratch pieces in 15 minutes. The file now lives in the PR from the first commit on.

**Extraction.** Aeneas and Charon are installed under the shared Lean directory. A two-function
crate (`si / num_div` on `f64`, and an integer loop counting the odd samples) went through
`charon cargo --preset=aeneas` and `aeneas -backend lean`: the integer loop extracts to a Lean
loop body with checked arithmetic; the `f64` division gives "Invalid inputs for binop" and its
body is `sorry`. So the float lemma is over a hand model of the loop (`passVals`, `oddRetryVals`),
and the integer shape of the odd loop could be extracted separately; I did not do that, as the
lemma that matters is the float one. No extracted file is committed.

**Runtime result.** Retries are rare. In 9,652 spline routes over the corpus 96 passes were
retries, in three dot inputs. None of the twopi/circo/fdp `splines=true` rows the ticket named
ever retries, so for them the change is neutral (+0.006 % instructions on twopi `ngk10_4`, the
cost of the loop bounds). Where it does retry the gain is real but small in absolute terms:
b57 -14.6 % instructions, b51 -4.9 %, b106 -0.3 %. The ticket's measurement target was wrong; the
proof and the saving stand regardless. Details in `docs/perf/limit-boxes-reuse.md`.
## 2026-11-03 #735: pathplan triangle adjacency and dot `canreach`

Stacked on #706 (`perf/704-pathplan-shortcuts`). Extraction verdict: not tried. The Aeneas binary
was not installed yet when the Lean step began (`/jobs/work/gvp-lean/aeneas` held a source checkout
and no `bin/`), so both statements are over Lean models and the PR has a "Model vs Rust" section.

**Hours.** About 0.7 h of Rust (reading, design, `reach.rs` and its tests, gates), about 1.3 h of
Lean (`TriAdjacency.lean` 221 lines, `CanReach.lean` 207 lines, no `sorry`; about ten failed compiles
in all), and about 0.6 h waiting: every `cargo-slot.sh` build refused to start because `/cache`
had 11 GB free against its 20 GB guard (`MIN_FREE_CACHE_GB`), with the biggest item there a Go
build cache that is not this project's. The owner chose to lower the guard to 8 GB for this task's
builds, with every other cap kept (slot queue, nice 15, cgroup, `ulimit -v`) and `SCCACHE_DIR` on
`/jobs`, and said not to delete the Go cache.

**What the ticket got wrong.** (a) The O(T^2 * 9) adjacency loop was already gone from the shipped
path: #706's `clip::triangulate` links triangles by remembering which clip made each side, and
the all-pairs loop survives only as `triangulate_c`, the oracle under `cfg(test)` and
`pathplan-check`. So there was no Rust to change for (a). `TriAdjacency.lean` therefore proves the
edge-keyed pairing the ticket describes against the loop the oracle runs, which justifies the
oracle's meaning and any later replacement of it; it does not describe `clip::triangulate`, whose
agreement with the oracle stays differential-tested (4,000 x 3 random polygons in `clip.rs`).
(b) "Memoise reachability across the pairs" is wrong as written: `make_lr_constraints` adds
auxiliary edges between queries, so a cached "no" can turn into "yes". Writing the statement in
Lean forced the hypothesis `Extends` (edges are only appended) and the shape `resume_exact`: a
cached set stays exact once closed under the new edges. A cache that kept answers without
resuming would have been wrong, and nothing in the corpus would have shown it (see below).

**What Lean caught.** `AtMostTwo` is a real premise, not a formality: with three triangles on one
undirected edge the all-pairs loop leaves asymmetric links (the last pair written wins) where the
keyed map links none (a `decide` example in `TriAdjacency.lean`). Simple polygons satisfy it; the random "wild" closed
walks in `clip.rs` need not, which is why those tests compare against the oracle rather than
assume it. For `canreach` Lean caught nothing in the code; it caught the cache design above
before the code existed.

**Result.** No speedup, and none expected: across the 590 dot-layout inputs of `tests/fixtures`
and `tests/wild`, `canreach` is called zero times (it runs only for flat edges that carry a label);
on three generated graphs of 200, 800 and 2,000 labelled flat edges it is called 110, 438 and
1,162 times and finds 152, 626 and 1,638 nodes in all, 2.0 s either way at 2,000. Callgrind on
`unix.gv` -Tsvg: 17,373,478 instructions on #706's tip, 17,373,075 here. The change is kept because
the proof and the check mode now exist; the survey's "small absolute time" was right.

**Dead ends.** `induction stk` on the stack instead of the function's own induction principle
(`dfs.induct`) lost the "already seen" case; `Fintype` in `termination_by` needs the instance
named in the signature, a `variable` is not enough; a `def` that matches on a list must be
`show`n to its unfolded form before `simp` sees it; `if_pos` and friends are deprecated in this
Mathlib. A first plan to cache only positive answers was dropped: it is correct but saves nothing
the resume does not.

**Not proved.** The per-node `scanned` counters, the early exit when the target joins the set, and
the memory cap are in `reach.rs` and not in the Lean model; the tests in `reach.rs` and the
`position-check` feature (an answer from scratch beside every answer) stand for them. The
`position-check` run is the runtime check this proof does not retire.

## 2026-11-03 coord: first wave wrap-up

Nine tickets, nine delegates, nine adversarial reviews, one day. What landed on master by 17:10 UTC:
the Lake project and Aeneas extraction (#736, #747), four proofs of shipped code (#742, #746), three
speedups with proofs (#744 twopi, #753 circo, #749 spline routing, plus #743 with none), and one
measured negative (#741). #754 (NS index) is reviewed and waits on the branch it stacks on.

**What the proofs bought.** Three kinds of result, in order of value:
1. *Speed with an equivalence argument too fiddly for prose.* The circo delta (#730) is the headline:
   the ticket's premise (a delta from the moved node's edges alone) was wrong, the derivation that
   replaced it needs prefix sums over the whole list, and the proof forced three eligibility
   hypotheses that became the fallback test. 86% fewer instructions on the ratchet case, up to 188×
   wall time against 1.0, output byte-identical on all 282 circo inputs.
2. *Finding gaps in arguments the code already relied on.* No shipped code was wrong. Every
   formalisation surfaced hypotheses the prose used silently: duplicate-free adjacency slices and
   block-only halves (#728), parent-before-child order (#729), twelve imprecisions across the NS and
   mincross pages (#732), the no-NaN limit invariant and the `boxn` bound (#733).
3. *Stopping bad ideas before code.* Writing the `canreach` statement (#735) showed a plain memo of
   answers would be wrong because edges are appended between queries; measuring the stress gate
   (#734) showed it fires on 1.6% of calls and is not worth stating.

**What the reviews bought.** Every PR was reviewed by a model from a different family (GPT-6.1 Sol)
with the Rust and the Lean side by side. Lean's kernel checks the proof; the review checks the one
thing it cannot: that the model is the code and the hypotheses hold where the code is called. The
reviews found: a theorem whose `Valid` hypothesis is false on production state after the first NS
pivot (#746, reproduced through `rank2`; now #750), an unenforced memory cap (#743, 64 MiB against a
32 MiB cap), a vacuous freshness check and a cgroup escape in the extraction script (#747), the
`i32::MAX` domain of the Fenwick theorem (#742), a wrong `Float` contract (#747), and several
overclaims in headers and journal entries. Zero false alarms across nine reviews. Each finding was
fixed on the PR before merge, by the coordinator, in under an hour.

**What extraction bought, and did not.** Aeneas accepts single functions, not the crate; five
integer kernels are extracted and type-check. No theorem in this wave is stated over extracted code
yet: every proof is over a model with a "Model vs Rust" list, because the functions proved about use
floats, closures, `?` in loops, or iterator adapters that Aeneas rejects (#745 is the first refactor
to change that). The honest status is that extraction is plumbing in place, not a result.

**Costs.** Lean time per ticket ranged from 0.7 h (#728, after a first attempt that wrote nothing) to
most of a session (#732). The machine lost about 40 minutes for every delegate to a full `/cache`
disk unrelated to this work. Three CI runs were cancelled externally mid-run and re-run. The journal
conflicts every time two PRs land, because every PR appends to this file; keeping both sides has
been the right resolution each time.

**For the write-up.** The sentence to lead with: under a byte-identical output rule, a speedup is an
equation, and a theorem prover is the right tool for equations. The sentence to follow it with: the
prover never found a bug in the code, and it found a gap in every argument.
## 2026-11-03 #731: dot NS `leave_edge` index

Stacked on #703 (`perf/702-ns-computed-cuts`), which edits the same file; the skeleton of #727 was
cherry-picked so `proofs/` exists. 

**What was built.** `NegSlots` (`common/ns/leave.rs`): the tree slots with a negative cut value as a
64-ary tree of bit words, edited at the three places that change a tree edge's cut value
(`treeupdate`, the cut-set path's cycle loop, `exchange_tree_edges`). `leave_edge` visits only
members from the cursor. `proofs/GvpProofs/Ns/LeaveEdge.lean` proves it returns C's slot and leaves
C's cursor.

**Extraction verdict.** Statement over a Lean model, not over extracted Rust. When the Lean step
began `/jobs/work/gvp-lean/aeneas/bin/aeneas` did not exist; it appeared while the Rust was being
measured. A probe in a scratch directory (Charon `--start-from crate::common::ns::leave`, then
Aeneas, 3 s) gave: the 64-ary tree (`NegSlots::build`, `set`, `next_from`, `insert_bit`, `word`)
extracts with bodies; `members` (the walk over the index) does not ("Could not match the contexts":
a `?` inside a `while`), and neither does the reference walk `ns::leave_edge` that it pulls in
("Breaks to outer loops are not supported yet": a `return` inside nested control flow). Charon
rejects `--start-from` on an inherent-impl method (`{impl T}::m` is only accepted as a whole-path
prefix), so a method is extracted by naming its module. Nothing was committed: the extracted files
import `Aeneas`, which `proofs/lakefile.toml` does not require until #727's extraction PR lands. The
follow-up that would close the model gap: rewrite `members` and the reference walk without `?` and
early `return` in loops, extract the module, and prove `next_from` against `NextSpec` (the one
piece of the bridge that is tested but not proved). The theorem is over an abstract ordered set
(`Finset` plus a successor query), so it does not change when that proof arrives.

**What Lean caught.** Nothing wrong in the Rust: the index was written from the C before the proof,
and the proof went through on the first full attempt. What the proof forced into the open, and the
prose argument would have let slide:
- the equality is false unless the cursor satisfies `s ≤ L`: slots past the end read as arbitrary
  values in the wrap loop. That is an invariant of C's loop, not a precondition of the function, and
  it took `leaveRef_cursor_le` to carry it across pivots (`run_idx_eq_run_ref`);
- the cursor after a scan that finds fewer than `Search_size` negatives is `j`, but `L` (not 0) when
  `j` was 0, because the first loop runs to the end and the wrap loop is skipped; and the cursor
  stays *on* the slot of the last counted negative, so the next call counts it again. Both are in
  the Rust and in the model; a "restore the start" model is wrong on exactly those calls;
- the `ns-check` oracle compares the chosen edge and the cursor, but the index could also be wrong
  in a way that a given run never exercises (a missed edit site on a path the test graphs do not
  take). The invariant `Inv` and the edit lemma `inv_set` turn that into a closed list of edit
  sites, which the oracle checks on every slot at every pivot instead of only on the chosen ones.

**Dead ends / friction.**
- `Finset.min'` is not in `Mathlib.Data.Finset.Basic` (import `Mathlib.Data.Finset.Max`).
- A first `scanRange_skip` without the hypothesis `i ≤ q` is false (natural subtraction), found when
  the zero case of the induction could not be closed.
- The shared build machine's cargo guard refused every build for over half an hour (the `/cache`
  disk was below the 20 GB the guard wants, because of an unrelated 63 GB Go build cache). With the
  owner's leave the Rust builds ran with `MIN_FREE_CACHE_GB=8` and `SCCACHE_DIR` on `/jobs`; the
  Lean steps were unaffected (`lake.sh` has no such guard) and the stall cost no Lean time.

**Measured, and the answer the ticket did not expect.** The ticket assumed the walk is most of what
is left of a pivot after #653/#702. It is not. A profile of `dot_dag_20000` on #703's head puts
`rank2_with` (where the walk is inlined) at 3% self time in all, with no samples on the walk's own
source lines; instructions fall 1.2% with the index, wall time by under 2%. The remaining network
simplex cost there is `CutStore::move_cut_sets`, `compact` and `push` (about 11% together). The walk
matters on graphs whose cut values go non-negative early and stay so (long parallel chains): −8.7%
instructions on `dot_par_10_500`, −5.7% on `par_20_500`, −5% wall on the first. Output is
byte-identical on all of them and on the whole test suite. Allocation counts fall on every ratchet
case that runs network simplex (the index adds one; sizing `Tree_edge` once removes about log₂ n
reallocations per run).

**Cost.** Roughly 0.7 h on Lean (model, five lemmas, the run theorem, worked cases: first full
compile was the whole file; 404 lines of which 282 are code, 28 declarations, standard axioms
only), 1.0 h on Rust (index, three edit sites, check-mode oracle, differential and randomised unit
tests, two clippy rounds), 1.2 h measuring and profiling (ratchet before and after, perf, interleaved
timings), and about 0.6 h stalled on the machine's disk guard and a server restart.

## 2026-11-03 #751: mincross scans, NaN ports

Lean and docs only; no Rust change (the one Rust edit, a temporary probe, was reverted).

**The gap.** `MincrossScans.lean` (#732) compared ports as `ℤ`; Rust compares `f64` with `>`. The
header handed the difference to the caller ("no port is NaN", not checked, not proved).

**Route.** The ticket offered (a) show NaN ports are unreachable and assert it, or (b) make the
port comparison abstract. Took (b). For (a), a probe in the two places that copy `port.p.x` into the
crossing store (`mincross.rs`, then fuzzing) showed the premise "ports come from finite node
geometry" is false: `width=inf` on a polygon gives ports of `±inf`, because `strtod` accepts `inf`
and `nan` and `late_double` only clamps from below. NaN did not appear in about 500 hostile graphs
(`width`, `height`, `skew`, `distortion`, `orientation`, `margin`, `fontsize`, `penwidth` set to
`inf`, `nan`, `1e308`, over seven shapes, both rank directions, compass and record ports, loops and
back edges). From the outputs, `nan` sizes fall back to the label size and record sizes saturate; I did not trace why in the source.
That is evidence, not a proof; proving it would be an audit of the whole `graphviz-pure-geom` crate,
and (b) makes it unnecessary.

**What was built.** `View` and `EdgeEnd` take a port type `P`; `View.portGt : P → P → Bool` is
Rust's `e1.port_x > e2.port_x`. `crosses` and `crossLoop` take the comparison; `crosses_congr`,
`crossLoop_congr` follow. About 100 lines added to a 1,776-line file, and every existing theorem
compiled unchanged apart from the extra argument. A "Ports" section models IEEE `>` (`F64.gt`: false
when either side is NaN) and instantiates `transpose_same` for it (`transpose_same_f64`) and for
Lean's `Float` (`transpose_same_float`), with the lemmas for what a NaN breaks.

**What the generic version revealed.** The proof needed *nothing* of the comparison: no
irreflexivity, asymmetry, transitivity or totality, and not even that it is a function of the
values' order. Port comparisons occur only inside `crosses`, and the only lemma about `crosses`
(`crosses_congr`) rewrites its order half; the port half is identical on both sides because C's loop
and the scan evaluate the same edges. So the ℤ model's stated route (embed the ports
order-preservingly) was a stronger argument than the theorem needed. What a NaN does break, shown in
`F64`: totality (`NaN` is neither above, below nor equal), and transitivity of "not greater"
(`a ≯ NaN`, `NaN ≯ c`, `a > c`). Irreflexivity and asymmetry survive. A future proof that sorts ports
or reads `≯` as `≤` (e.g. one about `reorder`, whose own NaN condition is the *median*, not the port)
would need the no-NaN hypothesis after all.

**Dead ends / friction.** Lean's `Float` is opaque to the kernel, so nothing about NaN can be proved
there; the NaN facts live in the `F64` model and `Float` only gets the instantiation. Build: the
whole generalisation compiled on the second run (the first failed only on the worked example's
missing field).

**Cost.** About 0.5 h on Rust (probe, fuzz script, reverting), 0.4 h on Lean, 0.2 h on docs. Extraction
verdict: not applicable (statement over a Lean model, as in #732).
## 2026-11-03 #750: cut-set `enter_edge` at production pivots, whose `lim`/`low` are stale

`Net.Valid.enterEdge_eq` (#732) assumed a current `lim`/`low` numbering. Production renumbers only
when the pivots begin and on a fallback, so the theorem covered the first pivot and no later one.
`proofs/GvpProofs/Ns/CutSetsStale.lean` (1,343 lines, 95 definitions and theorems, standard axioms
only) closes that. No Rust change.

**Route.** Route (a) of the ticket: a renumbered reference tree, not a weaker `Valid`. Route (b)
would have meant re-proving Lemma 1, the candidate set and the tie order on parent pointers alone.
Those proofs in `CutSets.lean` all go through `lim`/`low` intervals. Route (a) reuses all of them
unchanged and adds four steps:
1. The cut-set search reads `par`, the lists, the cut set and the slacks, never the numbering.
   In the model the only numbering it read was the climbs' fuel (`lim(v) + 1`). That fuel is now
   a parameter, and past the tree's depth it does not matter (`climb_fuel`).
2. C's `enter_edge` returns the same edge on every valid numbering of one tree (`cEnter_renum`).
   This is §5's Lemma 2 for `enter_edge`, proved as "every test C's loop makes is a question
   about the tree". The step functions of the two runs are equal as functions, so no fuel bound
   is needed. With 1 and 2: the production `enter_edge(e)` is C's on any valid numbering of the
   stored tree (`enterEdgeP_eq_every`). Its fallback is C's search after `hand_over`'s
   renumbering.
3. `Tree` is `Valid` without the numbering. A finite spanning tree always has a valid numbering
   (`Tree.exists_valid`, by strong induction on subtree size, with the children glued left to
   right). Acyclicity comes from the parent chain reaching the root, which has no parent.
4. (I3), which `CutSets.lean` had listed as not formalised: the pointers Rust's re-hang loop
   leaves form a spanning tree from the same root (`Valid.rehang_tree`). A spanning tree's
   parent pointers are fixed by its edges and root (`Tree.par_unique`), so they are the ones C's
   `dfs_range` assigns. `pivot_step` combines this with `pivot_against`: from the tree before a
   pivot alone, the tree after it has a valid numbering and the §4.2 cuts. So the hypothesis
   that `pivot_against` assumed is now proved.

**What the stale numbering breaks, and what it does not.** It breaks C's search on the stored
state: in `Example750` (three nodes, the issue's shape: after one pivot node 1 hangs below node 2
with the larger `lim`), C's search on the stale numbering tests the wrong end as the child. It
searches the parent side of the cut and returns no edge. The answer is `a`, which the cut-set path
and C's search on a valid numbering both return. In a pivot that would be `rank2`'s "no entering
edge" error. Production never runs C's search on that state: every route into C's functions
(`enter_edge`'s and `update`'s `Fallback`, `LR_balance`'s two hand-overs) goes through
`hand_over`, which renumbers from the root first. So there is no finding about the fallback
design: the renumbering is needed, and it is there. It does not break the cut-set path: its tie
climbs, `first_tied`, `lca` and `prepare` test children as `par(c) == e`. In the Rust, the reads of
`lim`/`low` on that path are `tree_path` (from `worth_it` and the build, when the pivots begin), the
computed build (also at the beginning), check mode, and a `debug_assert` that runs only once the
cut sets are gone. The stale values are copied back to dot's nodes after `rank`, and nothing reads
them there (§6 of the page).

**The oracle.** `ns-check` cannot see this class of bug. Check mode runs C's `dfs_range` after
every pivot (`check::dfs_range`), so the cut-set path always runs on a current numbering there. A
cut-set path that wrongly read `lim`/`low` would pass it. The runtime guards for stale numbering
are the shipped-mode tests that turn the hooks off and compare final results against C
(`a_fallback_renumbers_the_tree_it_hands_to_c`, `lr_balance_hands_over_when_its_walks_pass_the_allowance`).
The new theorem retires neither check. It covers, as a model, the stale case that check mode
cannot reach. Follow-up #765: make check mode compare the cut-set search on the stale production
state against C's search on a separately renumbered copy.

**What Lean caught.** Nothing wrong in the Rust. Three things the prose glossed over:
- the climbs' fuel in the #732 model was `lim(v) + 1`. On a stale state that is a number of the
  old tree and can be too small. It is a modelling artefact, not a Rust bug: Rust climbs until it
  reaches `v`, within its `TIE_CLIMBS` budget. But the old model, read on a production state, read
  `lim`/`low` after all;
- the existence of a valid numbering needs the graph to be finite (`Fintype V`). Without it, a
  spanning tree can be infinite, and no numbering bounded by `lim(root)` exists;
- §4.2's (I3) paragraph argues that the re-hang assigns what `dfs_range` assigns. The proof here
  does not compare with `dfs_range` at all: both sides are spanning trees of `T − e + f` from the
  same root, and that fixes `par`. That is a shorter argument than the page's, and it needs
  nothing about `dfs_range`'s pruning.

**Dead ends / friction.**
- `simp only` turning `(N.renum ..).lim` into `ma` inside an `if` gave a term that was not
  type-correct, because its `Decidable` instance still mentioned the old form. `by_cases`, then
  `simp` with the case's fact, avoids it.
- `rcases ... with ⟨rfl, rfl⟩` on `t = f` substituted `f` away, and a later `hT' f` no longer
  elaborated. Fixed with a named equation and `rw`.
- A first `sub_disjoint` proof got tangled. It became four lines once `inSub_last` had a version
  with no `Valid` argument.
- `Valid` re-proves `kids` lemmas through the numbering (`kid_ne` via `kid_lim`). For `Tree`, they
  needed numbering-free proofs (`kid_ne` from `par_excl`: no parent edge is a loop).

**Extraction verdict.** Not attempted: `cut.rs` is the same module whose shape #732 judged out of
reach (pool, stamps, `Result` early returns in loops), and this ticket is Lean and docs only.

**Hours.** About 0.6 h reading the issue, the brief, the page and the Rust; 1.7 h on Lean (the
core theorem compiled in five rounds, the `Tree` part and the existence of a numbering in two, the
re-hang in four, `par_unique` in two; a single-file check takes about 5 s); 0.4 h on the docs.
Rust: 0 h.
## 2026-11-03 #745: circo `count_indexed` and the reference scan extract; first theorem over shipped Rust

Result: extracted. `proofs/GvpProofs/Extracted/CircoCount/` holds `count_indexed`, `count_all_crossings_reference`
and their helpers (`scripts/proofs/extract.sh`, 1,151 lines of `Funs.lean`, 68 hand-written lines of
`FunsExternal.lean`, no `axiom`). `proofs/GvpProofs/Circo/FenwickExtracted.lean` (398 lines, about 90
of them the header; no `sorry`) restates the Fenwick lemmas and the `i32` accumulator over those
generated definitions. The full simulation (`count_indexed` = reference over the extraction) is *not*
proved: it is stated, with one placeholder proof, in `proofs/wip/FenwickExtractedMain.lean`, outside
the `GvpProofs` glob, with the list of lemmas still owed.

**Hours.** Rust and extraction about 0.45 h (the refactor, three extraction runs, one clippy round,
oracle tests, the ratchet before and after); Lean about 0.45 h (a scratch file per step; the first
theorem over generated code took most of it); docs and measurement about 0.2 h. Counting wall-clock
from the first read of the issue; a person new to Aeneas would not match it.

**What Aeneas rejected, and the refactor that fixed it** (details and the table are in
`extraction.md`, "The #745 refactor"):
- `?` in a nested loop (the issue's diagnosis) was right and one helper per edge loop fixed it.
- It was not enough. With the helpers in place Aeneas still said `Early returns inside of loops`
  about `count_indexed`'s whole body, until the first loop (`for &e in adjacency { … return None }`)
  also became a function (`reset_orders`). I did not bisect the rule further: one early-exit loop
  per function works, two in one function did not.
- `zip(1..)` became a wrapping counter. `?` on `get_mut` became a `match`, which also removes the
  second `Try::branch` model (so the two Fenwick modules of #727 are replaced by `CircoCount`);
  clippy's `question_mark` objects, hence four local `#[allow]`s with a comment.
- The kernels now take `order: &mut [i32]`, `head`, `tail` instead of `&mut Circ`, so the
  signatures drag in no `f64`, `String` or `BTreeMap`. `count_with_policy` is not extracted.
- The reference scan took 430–850 s to extract as one function of four nested loops. As four small
  functions the whole extraction is 10 s. The nested loops were the cost, not the arithmetic.

**Did any Rust behaviour change?** No, with one debug-build exception. Same integers, same order,
same wrapping, and `circo-check` stays and passes on the whole workspace test run. The exception:
`zip(1..)` panicked in a debug build once the list reached `i32::MAX` elements (the range steps
before yielding, so the panic is at exactly `i32::MAX`, which the indexed path's `<=` guard admits);
`wrapping_add` does not panic there. Release builds wrap identically in both. Instructions on `circo_sparse_400`: 120,944,119 before, 120,964,565 after (+0.017%); on
`badvoro`, +0.02%; allocations unchanged (ratchet case `wild_gv16_directed_sdh`: 25,864,074 →
25,864,687 instructions, 16,175 allocations both). Output is byte-identical on those inputs.

**How the extracted statements differ from the model's** (the header of `FenwickExtracted.lean`
has the same list):
- Every function is `Result`. The Fenwick operations are shown to be `ok` on any slice of length `N`;
  `add_crossings` is not: Charon builds the dev profile, so `cfg!(debug_assertions)` is true in the
  extraction and it *fails* past `i32::MAX` where the release build wraps. So the extracted totals
  are statements about runs where no addition overflows. The model proved the wrapped total; the
  extraction cannot say "wrapped" for the reference, because `crossings += 1` is a checked add there.
- The model hard-wired 64-bit `usize`. The extraction's `usize` is `System.Platform.numBits`
  (32 or 64), so each theorem takes `Is64`. The wasm build is 32-bit; nothing is claimed for it.
  This is a hypothesis the model made silently and the extraction made visible.
- Slices instead of functions `ℕ → _`: the statements carry `tree.length = N` and read through `tz`,
  and equate the result with the model only on slots `1..N-1` (the model's function agrees with the
  slice everywhere else, which no statement reads).
- Mutable borrows come back as results: `index_update` returns `(Option Unit × Slice U64)`.
- The model's fuel counters became `loop` fixpoints and `loop.spec_decr_nat` with a measure.
- Axioms: the same as `Fenwick.lean`, `propext`, `Classical.choice`, `Quot.sound` and the three
  `bv_decide` axioms behind `lowbit_*` and `prevIdx_eq` (`add_crossings_*` use only the standard three).

**What Lean caught.** Nothing wrong in the Rust. In the theorems, it forced the pointer-width
assumption above into the open (a gap in #728's statement, now explicit). Beyond that, one design
choice: the update refinement (`index_update_refines`) holds for *any* slice of length `N`, not only a
valid Fenwick tree (the loop invariant is "slot `j` plus the touch bit at the current index equals
slot `j` plus the touch bit at the first index"), so the next layer can use it without carrying
`Valid`; the prefix refinement does need `Valid`, because it is the tree's contents that make the
sum meaningful.

**Dead ends / friction.**
- `simp only [bind_tc_ok]`, `ok_bind` and `Result.bind` do not reduce `let x ← ok v; k x` in a
  goal; plain `simp` does (and also unfolds `lowbit`/`nextIdx` into `Usize.size` arithmetic, which
  is then matched back to the model's by `rw [e1]; exact nextIdx_eq …`).
- `show WP.spec (ok …) _` would not unify the postcondition; `simp` then `rw [WP.spec_ok]` did.
- `subst` on `j = ↑i` eliminated `j` and broke the next lines; state the facts about `↑i`.
- `UScalar.size_def` is unused for `Usize`/`U64` once `numBits_def` is given (linter noise;
  the file turns off `unusedSimpArgs`).
- `scripts/proofs/extract.sh --check` fails on `TwopiCircle` with an Aeneas internal error
  (`translate_fun_sigs`), and does so on a clean checkout of master as well, so it is not from this
  change; filed as #766. `CircoCount` is current under `--check`.

**Extraction verdict.** Extracted, with a refactor, and the Fenwick data structure plus the `i32`
accumulator are now proved over the shipped definitions. The remaining distance to the full theorem
is one refinement lemma per helper (list in the wip file); the slice-iterator loops and the
`trailing_zeros` bit walk are the two kinds of step not yet exercised on generated code.

## 2026-11-03 #770: circo indexed = reference, proved over the extracted Rust

Result: proved. `proofs/GvpProofs/Circo/FenwickExtractedMain.lean` (2,608 lines, 75 of them the
header; no `sorry`) proves `count_indexed_eq_reference` over the Aeneas-extracted `count_indexed` and
`count_all_crossings_reference` (`Extracted/CircoCount/`): if the reference returns
`ok (c, order', open')`, the indexed counter with `MUTATION = 0` returns `ok (some c, order', open',
tree')`, and `c` is the exact crossing count. Axioms: the standard three and the three `bv_decide`
axioms `Fenwick.lean` already used. The wip file is deleted. This is the experiment's first
end-to-end theorem about shipped code rather than a model of it.

**Hours.** Lean about 0.8 h, docs about 0.1 h, Rust none. Wall-clock from the first read of the
issue; most of it was compile-and-fix cycles of 8 s each on one file.

**Route.** Refinement, as the issue intended: one lemma per extracted helper saying "this helper, on
a concrete state related to a model state, returns `ok` with the model step's result"
(`tree_step` → `idxHalves`, `exclude_edges` → `foldlM idxExclude`, `restore_edges` →
`foldlM idxRestoreClose`, `open_edges` → `foldlM idxOpen`, `reference_open` → `foldl refOpen`,
`reference_close` → `refClose_fold`'s closed form), and the model's simulation reused at node
granularity: each node step calls `Fenwick.lean`'s `idxOccurrence_step` for the next model state and
its invariant. The model's top theorem was not reusable as is (next point), so the two node loops
were re-inducted over the extraction; the per-node and per-loop model lemmas were used unchanged.

**What the extracted statement needed that the model did not.**
- *The exact count.* The model proved the wrapped `i32` total (`BitVec 32`). The extraction is the
  checked build, where `add_crossings` fails past `i32::MAX`, so the indexed side must know each
  query's count before adding it. The model's top theorem never exposes that; its lemma
  `query_count_eq_hits` does, so the node step re-derives the post-exclusion facts (the set `T`, the
  `Valid` tree) as `idxOccurrence_step` does internally, and `query_refines` tracks the total in `ℤ`.
- *Partial correctness for the reference.* The hypothesis is that the reference returned `ok`, so the
  reference lemmas are "if `ok y` then …" (`PC`): every failing read and the checked `crossings += 1`
  are vacuous, and no reference read needed a bound. The reference's `i32` result equals the exact
  count, which bounds it by `i32::MAX`; that bound is what the indexed side's assertion needs. So
  "the reference did not overflow" implies "the indexed counter does not either", and is the only
  overflow hypothesis.
- *Words and the bit walk.* The model's `Finset` of halves became `u64` words: an abstraction
  `bitsOf`, word set/clear lemmas, and for the reference scan `trailing_zeros` = lowest set bit and
  `bits & (bits - 1)` clears it, then a word-by-word count. This was the largest new part (about 600
  lines with the scan). The model's ascending `Finset.sort` turned out not to matter: only the count
  is used.
- *The frame of `arena.order`.* To conclude the two final order slices are *equal*, the relation
  says the non-adjacency slots keep the caller's values; the model's `ℕ → ℕ` never had to.
- *Slice lengths*: `tail.len() = E`, `2E ≤ 64 · open.len()`, offset ranges inside `adjacency`,
  `order.len() = E`; and `Is64`. The wip statement's `head.len() = E` is unused (the indexed path
  never reads `head`) and was dropped.

**What Lean caught.** Nothing wrong in the Rust. One hypothesis in the target statement was
unnecessary (`head.len() = E`), and one expected hypothesis turned out to be derivable (the indexed
accumulator's non-overflow, from the reference's). No model lemma was false or had to change.

**Dead ends / friction.**
- `Result` is an interaction tree, not an inductive: `cases` gives `ret`/`vis`/`div`, and `ok`
  injectivity is `Result.ok.injEq`. The "plain `simp` reduces `let x ← ok v` and `simp only` does
  not" note under #745 has a cause: generated code binds with `Std.bind` (lemma `bind_ok`), while
  library definitions such as `Slice.index_mut_usize` use the typeclass bind (`bind_tc_ok`).
  `let (a, b) ← …` is Aeneas's `uncurry`; `simp` needs `uncurry_apply_pair`.
- `simp only [h]` after `rw [loop]` also rewrote inside the remaining loop's body, so the loop was
  no longer the named function and `rfl` failed. Fix: one `*_loop_unfold` lemma per generated loop,
  then rewrite only the body applied to the current state. After that, plain induction on the
  remaining slice length was simpler than `loop.spec_decr_nat` with a body metavariable (a `have`
  elaborated the step proof before the body was known).
- `set e := l[i]` does not reach occurrences introduced by later unfolding; `obtain ⟨e, he⟩ : ∃ e,
  l[i] = e` and rewriting with `he` does.
- `if_pos`/`if_neg` are deprecated in this Mathlib (`ite_eq_left`/`ite_eq_right`).

**What this says about model-based proofs.** The model was right, and its per-node simulation was
reusable unchanged, so the hours spent on it were not wasted. But its *statement* was not the one
the extraction has: it proved the property in the release build's arithmetic (wrapping, which is
what shipped move decisions read), while the extraction is in the checked build's arithmetic
(Charon builds with `debug_assertions`), so the top theorem could not be transferred and the
wrapped regime above `i32::MAX` stays with the model proof.
The pieces that transferred were those that state semantic content (`query_count_eq_hits`: "this
count is that number of halves"), not an equality of two model runs. And the parts the model
abstracted (word packing, the bit walk, the frame of untouched slots) were a quarter of the new
proof. A model proof is a good first draft of the invariant; it is not yet a proof of the code.
