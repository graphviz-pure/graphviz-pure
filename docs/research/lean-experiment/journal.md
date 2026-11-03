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
`countIndexed_eq_reference`: on a well-formed block graph the indexed counter never takes an early
return and returns the reference's `i32` total, final `order` array and half set. Also
`countWithPolicy_eq_reference` (the dispatch), `query_count_eq_hits` (each query's `u64` count
is the reference's natural-number count), and `countIndexed_wrapped` (the `i32` is the exact count
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
`CGraph::edges`. They are now hypotheses of the theorem and mentioned in
`docs/perf/circo-crossings.md`.

Extraction verdict: not attempted. The shared Lean install has an `aeneas` directory but no `bin/aeneas`
binary, so the theorem is over a hand-written model; the PR lists every difference ("Model vs
Rust" in the file header). The risk that remains is that the model and the Rust drift apart, which
extraction would remove.
