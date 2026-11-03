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
