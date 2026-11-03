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

These are Lean's IEEE 754 binary64 and binary32 types, the same choice as #1073. What the alias
does and does not preserve of Rust's `f64`:

* Lean's `Float` has a logical model (`Init.Data.Float.Model`) with defined arithmetic and both
  signed zeros, but a single canonical NaN. Rust's `f64` distinguishes NaN sign and payload bits.
  So a theorem over `F64` is a statement about values up to NaN identity: it cannot distinguish
  two NaNs, and it must not be read as a claim about Rust's NaN bit patterns.
* Aeneas rejects float arithmetic and float literals in function bodies
  (`docs/research/lean-experiment/extraction.md`), so today every extracted function only carries
  floats through unchanged (struct fields it does not touch). For those functions the alias is
  exact: a value that is not computed on cannot lose a bit. Any future theorem about a function
  that computes on floats needs a bit-exact model (for example `BitVec 64` with an IEEE
  interpretation, or FloatSpec), not this alias.
* If Aeneas later defines `Aeneas.Std.F64`, this file stops compiling: delete it and the import.
-/

namespace Aeneas.Std

/-- Rust's `f64`. -/
abbrev F64 := Float

/-- Rust's `f32`. -/
abbrev F32 := Float32

end Aeneas.Std
