-- Hand-written models of the standard-library functions that the extracted Fenwick code calls
-- and Aeneas's library does not define (Aeneas lists them in FunsExternal_Template.lean). Not
-- generated: scripts/proofs/extract.sh leaves this file alone and warns when the list changes.
-- They sit in the extraction's namespace, so another extraction may model the same function.
module
public import Aeneas
public import GvpProofs.Extracted.CircoFenwickUpdate.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.unusedVariables false

namespace GvpProofs.Extracted.CircoFenwickUpdate

/-- `usize::wrapping_neg`: `0.wrapping_sub(self)`. -/
@[rust_fun "core::num::{usize}::wrapping_neg"]
def core.num.Usize.wrapping_neg (x : Std.Usize) : Result Std.Usize :=
  ok (core.num.Usize.wrapping_sub 0#usize x)

/-- `<Option<T> as Try>::branch`, the first half of `?`: `Some(x)` continues with `x`, `None`
breaks with `None`. Here `T` is a mutable borrow (`tree.get_mut(i)?`), so Aeneas's generated code
expects the borrow's backward function as a second component, which the signature in Aeneas's own
template lacks (the two disagree; see docs/research/lean-experiment/extraction.md). The backward
function returns the possibly updated payload. -/
@[rust_fun
  "core::option::{core::ops::try_trait::Try<core::option::Option<@T>>}::branch"]
def core.option.Option.Insts.CoreOpsTry_traitTry.branch {T : Type _} (o : Option T) :
    Result (core.ops.control_flow.ControlFlow (Option Never) T ×
      (core.ops.control_flow.ControlFlow (Option Never) T → Option T)) :=
  match o with
  | some x =>
    ok (.Continue x, fun
      | .Continue y => some y
      | .Break _ => some x)
  | none => ok (.Break none, fun _ => none)

/-- `<Option<T> as FromResidual<Option<!>>>::from_residual`, the second half of `?`: `None`. -/
@[rust_fun
  "core::option::{core::ops::try_trait::FromResidual<core::option::Option<@T>, core::option::Option<!>>}::from_residual"]
def core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual
    (T : Type _) (_ : Option Never) : Result (Option T) :=
  ok none

end GvpProofs.Extracted.CircoFenwickUpdate
