-- Hand-written models of the standard-library functions that the extracted counting code calls
-- and Aeneas's library does not define (Aeneas lists them in FunsExternal_Template.lean). Not
-- generated: scripts/proofs/extract.sh leaves this file alone and warns when the list changes.
-- They sit in the extraction's namespace, so another extraction may model the same function.
module
public import Aeneas
public import GvpProofs.Extracted.CircoCount.Types
@[expose] public section
open Aeneas Aeneas.Std Result ControlFlow Error
set_option linter.dupNamespace false
set_option linter.unusedVariables false

namespace GvpProofs.Extracted.CircoCount

/-- `u64::trailing_zeros`: the index of the lowest set bit, or 64 for zero. -/
@[rust_fun "core::num::{u64}::trailing_zeros"]
def core.num.U64.trailing_zeros (x : Std.U64) : Result Std.U32 :=
  ok (UScalar.ofNatCore
    (((List.range 64).find? (fun k => x.val.testBit k)).getD 64) (by
      have : ((List.range 64).find? (fun k => x.val.testBit k)).getD 64 ≤ 64 := by
        cases h : (List.range 64).find? (fun k => x.val.testBit k) with
        | none => simp
        | some k =>
          have := List.mem_of_find?_eq_some h
          simp at this ⊢; omega
      have h32 : (64 : Nat) < 2 ^ UScalarTy.U32.numBits := by decide
      exact lt_of_le_of_lt this h32))

/-- `usize::wrapping_neg`: `0.wrapping_sub(self)`. -/
@[rust_fun "core::num::{usize}::wrapping_neg"]
def core.num.Usize.wrapping_neg (x : Std.Usize) : Result Std.Usize :=
  ok (core.num.Usize.wrapping_sub 0#usize x)

/-- `<&u64 as BitAnd<u64>>::bitand`: `*self & rhs`. -/
@[rust_fun
  "core::ops::bit::{core::ops::bit::BitAnd<&'0 u64, u64, u64>}::bitand"]
def Shared0U64.Insts.CoreOpsBitBitAndU64U64.bitand (x y : Std.U64) : Result Std.U64 :=
  ok (x &&& y)

/-- `<Range<Idx> as Clone>::clone`, derived: clones `start`, then `end`. -/
@[rust_fun
  "core::ops::range::{core::clone::Clone<core::ops::range::Range<@Idx>>}::clone"]
def core.ops.range.Range.Insts.CoreCloneClone.clone
    {Idx : Type _} (cloneCloneInst : core.clone.Clone Idx) (r : core.ops.range.Range Idx) :
    Result (core.ops.range.Range Idx) := do
  let start ← cloneCloneInst.clone r.start
  let «end» ← cloneCloneInst.clone r.«end»
  ok { start, «end» }

/-- `<Option<T> as Try>::branch`, the first half of `?`: `Some(x)` continues with `x`, `None`
breaks with `None`. Every `?` in this extraction is on an owned or shared value (the `get_mut`
calls are `match`es, see `docs/research/lean-experiment/extraction.md`), so `branch` has no
backward function here, unlike in the older `CircoFenwickUpdate` extraction. -/
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

end GvpProofs.Extracted.CircoCount
