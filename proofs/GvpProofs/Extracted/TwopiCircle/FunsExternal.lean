-- Hand-written models of the standard-library functions that the extracted twopi code calls and
-- Aeneas's library does not define (Aeneas lists them in FunsExternal_Template.lean). Not
-- generated: scripts/proofs/extract.sh leaves this file alone and warns when the list changes.
-- Each definition follows the Rust standard library's documented behaviour, in Aeneas's encoding
-- of mutable borrows (a value plus a "backward" function that writes it back, as in
-- `core.slice.index.Usize.get_mut`). They sit in the extraction's namespace, so another
-- extraction may model the same function without a name clash.
module
public import Aeneas
public import GvpProofs.Extracted.TwopiCircle.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.unusedVariables false

namespace GvpProofs.Extracted.TwopiCircle

/-- `Vec::<T>::clear`: removes every element. -/
@[rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::clear"]
def alloc.vec.Vec.clear
    {T : Type _} (A : Type _) (v : alloc.vec.Vec T) : Result (alloc.vec.Vec T) :=
  ok (alloc.vec.Vec.new T)

end GvpProofs.Extracted.TwopiCircle
