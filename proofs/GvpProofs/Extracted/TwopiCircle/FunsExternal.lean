-- Hand-written models of the functions that the extracted twopi code calls and Aeneas's library
-- does not define (Aeneas lists them in FunsExternal_Template.lean). Not generated:
-- scripts/proofs/extract.sh leaves this file alone and warns when the list changes.
-- Each definition follows the Rust standard library's documented behaviour. They sit in the
-- extraction's namespace, so another extraction may model the same function without a clash.
module
public import Aeneas
public import GvpProofs.Extracted.TwopiCircle.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.unusedVariables false

namespace GvpProofs.Extracted.TwopiCircle

/-- `<Option<T> as Try>::branch`, the first half of `?`: `Some(x)` continues with `x`, `None`
breaks with `None`. Every `?` here is on an owned or shared value, so there is no backward
function (the `get_mut`s are `if let`s). -/
@[rust_fun
  "core::option::{core::ops::try_trait::Try<core::option::Option<@T>>}::branch"]
def core.option.Option.Insts.CoreOpsTry_traitTry.branch {T : Type _} :
    Option T → Result (core.ops.control_flow.ControlFlow (Option Never) T)
  | some x => ok (.Continue x)
  | none => ok (.Break none)

/-- `<Option<T> as FromResidual<Option<!>>>::from_residual`, the second half of `?`: `None`. -/
@[rust_fun
  "core::option::{core::ops::try_trait::FromResidual<core::option::Option<@T>, core::option::Option<!>>}::from_residual"]
def core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual
    (T : Type _) (_ : Option Never) : Result (Option T) :=
  ok none

/-- `Vec::<T>::clear`: removes every element. -/
@[rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::clear"]
def alloc.vec.Vec.clear
    {T : Type _} (A : Type _) (v : alloc.vec.Vec T) : Result (alloc.vec.Vec T) :=
  ok (alloc.vec.Vec.new T)

/-- `<&Vec<T> as IntoIterator>::into_iter`: `v.iter()`, an iterator over the elements in order. -/
@[rust_fun
  "alloc::vec::{core::iter::traits::collect::IntoIterator<&'a alloc::vec::Vec<@T>, &'a @T, core::slice::iter::Iter<'a, @T>>}::into_iter"]
def SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter
    {T : Type _} (A : Type _) (v : alloc.vec.Vec T) : Result (core.slice.iter.Iter T) :=
  core.slice.Slice.iter (alloc.vec.Vec.deref v)

/-- `edge_ends` in `circle.rs`, which the extraction treats as opaque: `(e.tail, e.head)`. -/
def twopigen.circle.edge_ends (e : common.types.LEdge) : Result (Std.Usize × Std.Usize) :=
  ok (e.tail, e.head)

end GvpProofs.Extracted.TwopiCircle
