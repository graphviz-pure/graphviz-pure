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

/-- `<Range<Idx> as Clone>::clone`, derived: clones `start`, then `end`. -/
@[rust_fun
  "core::ops::range::{core::clone::Clone<core::ops::range::Range<@Idx>>}::clone"]
def core.ops.range.Range.Insts.CoreCloneClone.clone
    {Idx : Type _} (cloneCloneInst : core.clone.Clone Idx) (r : core.ops.range.Range Idx) :
    Result (core.ops.range.Range Idx) := do
  let start ← cloneCloneInst.clone r.start
  let «end» ← cloneCloneInst.clone r.«end»
  ok { start, «end» }

/-- `<Option<T> as PartialEq>::eq`, derived: equal variants, then equal payloads. -/
@[rust_fun
  "core::option::{core::cmp::PartialEq<core::option::Option<@T>, core::option::Option<@T>>}::eq"]
def core.option.Option.Insts.CoreCmpPartialEqOption.eq
    {T : Type _} (cmpPartialEqInst : core.cmp.PartialEq T T) :
    Option T → Option T → Result Bool
  | some a, some b => cmpPartialEqInst.eq a b
  | none, none => ok true
  | _, _ => ok false

/-- `<[T]>::last_mut`: the last element, if any, and the slice with it written back. -/
@[rust_fun "core::slice::{[@T]}::last_mut"]
def core.slice.Slice.last_mut
    {T : Type _} (s : Slice T) : Result ((Option T) × (Option T → Slice T)) :=
  let i := s.length - 1
  ok (s.val.getLast?, fun x => .from (s.val.set_opt i x) (by have := s.property; simp))

/-- `Vec::<T>::pop`: removes and returns the last element. -/
@[rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::pop"]
def alloc.vec.Vec.pop
    {T : Type _} (A : Type _) (v : alloc.vec.Vec T) : Result ((Option T) × (alloc.vec.Vec T)) :=
  ok (v.val.getLast?, alloc.vec.Vec.from v.val.dropLast (by have := v.property; simp; omega))

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

end GvpProofs.Extracted.TwopiCircle
