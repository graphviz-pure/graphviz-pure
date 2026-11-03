module
public import Aeneas
@[expose] public section

/-!
# Stand-ins for Rust's `f32` and `f64` in extracted files

Aeneas at the pinned commit prints `f64` as `F64` but defines no `F64` (AeneasVerif/aeneas#828;
draft PR #1073 adds float support by lowering to Lean's `Float`). Without a definition, Lean
either rejects the generated structure or, in a function signature, silently binds `F64` as an
implicit variable. `scripts/proofs/extract.sh` adds `public import GvpProofs.Extracted.Floats` to
every generated file that mentions `F32` or `F64`.

These are Lean's IEEE 754 binary64 and binary32 types, the same choice as #1073. Lean's kernel
knows nothing about their arithmetic, so a theorem may carry a float through, never reason about
its value. Aeneas also rejects float arithmetic and float literals in function bodies
(`docs/research/lean-experiment/extraction.md`). If Aeneas later defines `Aeneas.Std.F64`, this
file stops compiling: delete it and the import then.
-/

namespace Aeneas.Std

/-- Rust's `f64`. -/
abbrev F64 := Float

/-- Rust's `f32`. -/
abbrev F32 := Float32

end Aeneas.Std
