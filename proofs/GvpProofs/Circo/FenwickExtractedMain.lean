import Mathlib
import GvpProofs.Circo.FenwickExtracted

/-!
# circo: `count_indexed` = `count_all_crossings_reference`, over the *extracted* Rust

Subject: `crates/graphviz-pure-layout/src/circogen/blockpath.rs`, as Aeneas extracts it into
`GvpProofs/Extracted/CircoCount/Funs.lean`. `Fenwick.lean` proves the simulation on a hand-written
Lean model; `FenwickExtracted.lean` proves the Fenwick and accumulator lemmas over the generated
definitions. This file proves the end-to-end theorem over the generated definitions themselves:
every function named in `count_indexed_eq_reference` is one Aeneas wrote from the shipped Rust.

## Theorem

`count_indexed_eq_reference`: if the extracted `count_all_crossings_reference` returns
`ok (c, order', open')`, then on a well-formed block graph the extracted `count_indexed` (with the
production `MUTATION = 0`) returns `ok (some c, order', open', tree')`, and `c` is the model's exact
crossing count `natCrossings` (as an integer, not reduced modulo `2 ^ 32`). Axioms: `propext`,
`Classical.choice`, `Quot.sound` and the three `bv_decide` axioms of `Fenwick.lean`.

## Route: refine each extracted helper to the model step, reuse the model's simulation

One refinement lemma per extracted helper, on a concrete state related to a model state:

| extracted | model | lemma |
|---|---|---|
| `tree_step` (range loop over the two halves) | `idxHalves` | `tree_step_refines` |
| `exclude_edges` | `foldlM idxExclude` | `exclude_refines` |
| `query_edges` | `query_count_eq_hits`, per edge | `query_refines` |
| `restore_edges` | `foldlM idxRestoreClose` | `restore_refines` |
| `open_edges` | `foldlM idxOpen` | `open_refines` |
| `reset_orders`, reference `loop0` | `initState` | `reset_orders_spec`, `ref_reset_PC` |
| `reference_scan` (word loop, bit walk) | `hits` | `scan_PC`, `scan_hits` |
| `reference_close` | `refClose_fold` | `ref_close_PC` |
| `reference_open` | `foldl refOpen` | `ref_open_PC` |

The node loops (`idx_occ`/`idx_run`, `ref_occ_PC`/`ref_run_PC`) then follow one model run: at
each node they call `Fenwick.lean`'s `idxOccurrence_step` for the next model state and its
invariant `Inv`, and `exclude_fold`, `query_fold`, `refClose_fold` for the stages. The model's
top-level `countIndexed_eq_reference` is not reused as is, because its `i32` total wraps and says
nothing about the size of each query's count; the checked build needs that size (below), so the
node-level induction is redone with an exact count, and the model's per-node theorem does the rest.

## Relations (concrete state ↔ model state)

* `ORel`: `arena.order` holds `ord e` (as `i32`) on every adjacency edge, and the caller's original
  value everywhere else (the frame condition makes the two final order slices *equal*, `ORel_inj`).
* `bitsOf`: the set halves of the `open` words (`bitsOf_inj`: same length and same bits, same slice).
* `TRel`: the tree slice agrees with the model's tree on slots `1..N-1`; `indexUpdate_congr` and
  `indexPrefix_congr` show the model's Fenwick operations only read those slots.
* `IRel` (indexed side) is the three together; `RRel` (reference side) drops the tree.

## What the extracted statement needed that the model did not

* **Partial correctness for the reference.** Its reads are `index_usize` (fail when out of range)
  and its `crossings += 1` is a checked add. The reference lemmas are `PC x P` ("if `x = ok y`
  then `P y`"), so failures are vacuous and none of the reference's reads needs a bound.
* **Overflow is derived, not assumed.** `add_crossings` fails past `i32::MAX` in the extraction
  (`debug_assertions`). The reference's result is an `i32` equal to the exact count, so the count is
  at most `i32::MAX`, and `query_refines` uses exactly that bound for the indexed side's assertion.
  This needs the *exact* per-query count (`query_count_eq_hits`), which is why the total is tracked
  in `ℕ`/`ℤ` and not as the model's `BitVec 32`.
* **The bitset as words.** The model's `Finset` became `u64` words: `bitAt`, word updates
  (`bitsOf_set_bit`, `bitsOf_clear_bit`), and the reference's bit walk (`trailing_zeros` is the lowest
  set bit, `bits & (bits - 1)` clears it: `tz_spec`, `clear_lowest`). The model's ascending
  `Finset.sort` order turned out to be irrelevant: only the count is used.
* **Slice lengths.** `tail.length = E`, `2 E ≤ 64 · open.length`, every offset range inside
  `adjacency`, `order.length = E`; the model's arrays were total functions. The wip statement's
  `head.length = E` is not needed (the indexed path never reads `head`; the reference's reads are
  covered by partial correctness) and was dropped.
* **`Is64`** for every `usize` bit identity, as in `FenwickExtracted.lean`.
* **Loop plumbing.** One unfolding lemma per generated `loop` (`*_loop_unfold`), then induction on
  the remaining slice length; the iterator models of Aeneas's library (`IteratorSliceIter.next`,
  `IteratorRange.next`, `IteratorEnumerate.next`, `Slice.fill`) are used through their definitions.
-/

set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.style.longLine false
set_option linter.unnecessarySeqFocus false
set_option linter.unusedTactic false
set_option linter.style.multiGoal false
set_option linter.flexible false

open Aeneas Aeneas.Std Result ControlFlow
open GvpProofs.Extracted.CircoCount GvpProofs.Circo.Fenwick GvpProofs.Circo.FenwickExtracted

namespace GvpProofs.Circo.FenwickExtractedMain

/-! ## Partial correctness -/

/-- `x` is `ok y` only for a `y` with `P y` (a failure or divergence satisfies it vacuously). -/
def PC {α : Type} (x : Result α) (P : α → Prop) : Prop := ∀ y, x = ok y → P y

theorem PC_ok {α : Type} {x : α} {P : α → Prop} (h : P x) : PC (ok x) P := by
  intro y hy; simp at hy; subst hy; exact h

theorem PC_fail {α : Type} {e : Error} {P : α → Prop} : PC (fail e) P := by
  intro y hy; simp at hy

theorem PC_bind {α β : Type} {x : Result α} {f : α → Result β} {P : α → Prop} {Q : β → Prop}
    (hx : PC x P) (hf : ∀ a, P a → PC (f a) Q) : PC (x >>= f) Q := by
  intro y hy
  cases hx' : x with
  | ret a => rw [hx'] at hy; exact hf a (hx a hx') y (by simpa using hy)
  | vis i k => rw [hx'] at hy; simp [bind_vis] at hy
  | div => rw [hx'] at hy; simp at hy

theorem PC_mono {α : Type} {x : Result α} {P Q : α → Prop} (h : PC x P) (hPQ : ∀ a, P a → Q a) :
    PC x Q := fun y hy => hPQ y (h y hy)

theorem PC_loop {α β : Type} (measure : α → ℕ) (inv : α → Prop) (post : β → Prop)
    (body : α → Result (ControlFlow α β))
    (hBody : ∀ x, inv x → PC (body x) (fun r => match r with
      | .done y => post y
      | .cont x' => inv x' ∧ measure x' < measure x))
    (x : α) (hx : inv x) : PC (loop body x) post := by
  induction hm : measure x using Nat.strong_induction_on generalizing x with
  | _ m ih =>
    intro y hy
    rw [loop] at hy
    cases hb : body x with
    | ret r =>
      rw [hb] at hy
      have hr := hBody x hx r hb
      cases r with
      | done z => simp at hy; subst hy; exact hr
      | cont x' => exact ih (measure x') (hm ▸ hr.2) x' hr.1 rfl y (by simpa using hy)
    | vis i k => rw [hb] at hy; simp [bind_vis] at hy
    | div => rw [hb] at hy; simp at hy

/-! ## Abstraction maps -/

/-- Bit `h % 64` of word `h / 64` of the bitset (`false` past its end). -/
def bitAt (o : Slice U64) (h : ℕ) : Bool :=
  match o.val[h / 64]? with
  | some w => w.val.testBit (h % 64)
  | none => false

/-- The set halves of the `open` bitset. -/
def bitsOf (o : Slice U64) : Finset ℕ :=
  (Finset.range (64 * o.length)).filter (fun h => bitAt o h = true)

theorem mem_bitsOf (o : Slice U64) (h : ℕ) : h ∈ bitsOf o ↔ bitAt o h = true := by
  unfold bitsOf
  rw [Finset.mem_filter, Finset.mem_range]
  constructor
  · exact fun hh => hh.2
  · intro hh
    refine ⟨?_, hh⟩
    unfold bitAt at hh
    cases hw : o.val[h / 64]? with
    | none => rw [hw] at hh; cases hh
    | some w =>
      have := (List.getElem?_eq_some_iff.mp hw).1
      have : o.length = o.val.length := rfl
      omega

/-- Two bitsets of the same length with the same set bits are equal. -/
theorem bitsOf_inj {a b : Slice U64} (hl : a.length = b.length) (hb : bitsOf a = bitsOf b) :
    a = b := by
  apply Slice.ext
  apply List.ext_getElem?
  intro w
  have hla : a.length = a.val.length := rfl
  have hlb : b.length = b.val.length := rfl
  cases ha : a.val[w]? with
  | none =>
    have : a.val.length ≤ w := List.getElem?_eq_none_iff.mp ha
    exact (List.getElem?_eq_none_iff.mpr (by omega)).symm
  | some x =>
    have hwa := (List.getElem?_eq_some_iff.mp ha).1
    obtain ⟨y, hy⟩ : ∃ y, b.val[w]? = some y := ⟨_, List.getElem?_eq_getElem (by omega)⟩
    rw [hy]
    congr 1
    have hk : ∀ k, x.val.testBit k = y.val.testBit k := by
      intro k
      by_cases hk : k < 64
      · have e1 : (64 * w + k) / 64 = w := by omega
        have e2 : (64 * w + k) % 64 = k := by omega
        have := congrArg (fun S => 64 * w + k ∈ S) hb
        simp only [mem_bitsOf, bitAt, e1, e2, ha, hy] at this
        simpa using this
      · have hx := x.hBounds
        have hy' := y.hBounds
        simp [UScalarTy.numBits] at hx hy'
        have h64 : (18446744073709551616 : ℕ) ≤ 2 ^ k := by
          rw [show (18446744073709551616 : ℕ) = 2 ^ 64 by norm_num]
          exact Nat.pow_le_pow_right (by norm_num) (by omega)
        rw [Nat.testBit_eq_false_of_lt (by omega), Nat.testBit_eq_false_of_lt (by omega)]
    have := Nat.eq_of_testBit_eq hk
    exact UScalar.eq_equiv x y |>.mpr this

/-- `arena.order` relates to the model's `ord`: on the adjacency edges it holds `ord e` (as an
`i32`), and every other slot is the caller's original value. -/
structure ORel (order0 : Slice I32) (adj : List ℕ) (ord : ℕ → ℕ) (order : Slice I32) : Prop where
  inAdj : ∀ e ∈ adj, ∃ x, order.val[e]? = some x ∧ x.val = (ord e : ℤ)
  outAdj : ∀ e, e ∉ adj → order.val[e]? = order0.val[e]?

theorem ORel_inj {order0 : Slice I32} {adj : List ℕ} {ord : ℕ → ℕ} {a b : Slice I32}
    (ha : ORel order0 adj ord a) (hb : ORel order0 adj ord b) : a = b := by
  apply Slice.ext
  apply List.ext_getElem?
  intro e
  by_cases he : e ∈ adj
  · obtain ⟨x, hx, hxv⟩ := ha.inAdj e he
    obtain ⟨y, hy, hyv⟩ := hb.inAdj e he
    rw [hx, hy]
    congr 1
    exact IScalar.eq_equiv x y |>.mpr (by rw [hxv, hyv])
  · rw [ha.outAdj e he, hb.outAdj e he]

/-- The tree slice agrees with the model's tree on the slots `1..N-1` that `Valid` reads. -/
def TRel (N : ℕ) (tree : Slice U64) (t : ℕ → ZMod (2 ^ 64)) : Prop :=
  tree.length = N ∧ ∀ j, 0 < j → j < N → tz tree j = t j

/-! ## The model's Fenwick operations only read slots `1..N-1` -/

theorem indexUpdate_congr {N : ℕ} (hN : N < 2 ^ 62) (d : ZMod (2 ^ 64)) {t t' : ℕ → ZMod (2 ^ 64)}
    (ht : ∀ j, 0 < j → j < N → t j = t' j) (i : ℕ) :
    match indexUpdate N d t i, indexUpdate N d t' i with
    | none, none => True
    | some u, some u' => ∀ j, 0 < j → j < N → u j = u' j
    | _, _ => False := by
  unfold indexUpdate
  by_cases hi : i = 0 ∨ N ≤ i
  · simp [hi]
  · simp only [hi, ↓reduceIte]
    intro j hj0 hj
    rw [updateGo_eq N hN d N i t (by omega) (by omega) j hj0 hj,
      updateGo_eq N hN d N i t' (by omega) (by omega) j hj0 hj, ht j hj0 hj]

theorem prefixGo_congr {t t' : ℕ → ZMod (2 ^ 64)} {N : ℕ} (hN : N < 2 ^ 62)
    (ht : ∀ j, 0 < j → j < N → t j = t' j) :
    ∀ (fuel i : ℕ) (sum : ZMod (2 ^ 64)), i < N → prefixGo t fuel i sum = prefixGo t' fuel i sum
  | 0, i, sum, _ => rfl
  | fuel + 1, i, sum, hi => by
    simp only [prefixGo]
    by_cases h0 : 0 < i
    · simp only [h0, ↓reduceIte]
      have hp := prevIdx_eq i (by omega)
      have hle := lowbit_le i
      rw [ht i h0 hi, prefixGo_congr hN ht fuel (prevIdx i) _ (by omega)]
    · simp [h0]

theorem indexPrefix_congr {t t' : ℕ → ZMod (2 ^ 64)} {N : ℕ} (hN : N < 2 ^ 62)
    (ht : ∀ j, 0 < j → j < N → t j = t' j) (i : ℕ) :
    indexPrefix N t i = indexPrefix N t' i := by
  unfold indexPrefix
  by_cases hi : N ≤ i
  · simp [hi]
  · simp only [hi, ↓reduceIte, prefixGo_congr hN ht i i 0 (by omega)]

/-! ## Slice-iterator loops -/

theorem next_some {T : Type} (sl : Slice T) (i : ℕ) (hi : i < sl.val.length) :
    core.slice.iter.IteratorSliceIter.next (⟨sl, i⟩ : core.slice.iter.Iter T) =
      ok (some sl.val[i], ⟨sl, i + 1⟩) := by
  simp [core.slice.iter.IteratorSliceIter.next, hi]
  rfl

theorem next_none {T : Type} (sl : Slice T) (i : ℕ) (hi : ¬ i < sl.val.length) :
    core.slice.iter.IteratorSliceIter.next (⟨sl, i⟩ : core.slice.iter.Iter T) =
      ok (none, ⟨sl, i⟩) := by
  simp [core.slice.iter.IteratorSliceIter.next, hi]

/-- A `for x in slice` loop, total correctness: the invariant is indexed by the position. -/
theorem slice_loop_spec {T S R : Type} (sl : Slice T)
    (body : core.slice.iter.Iter T × S → Result (ControlFlow (core.slice.iter.Iter T × S) R))
    (inv : ℕ → S → Prop) (post : R → Prop)
    (hstep : ∀ i s, i ≤ sl.val.length → inv i s → body (⟨sl, i⟩, s) ⦃ r => match r with
        | .done y => post y
        | .cont x' => x'.1 = ⟨sl, i + 1⟩ ∧ i < sl.val.length ∧ inv (i + 1) x'.2 ⦄)
    (i0 : ℕ) (hi0 : i0 ≤ sl.val.length) (s0 : S) (h0 : inv i0 s0) :
    loop body (⟨sl, i0⟩, s0) ⦃ post ⦄ := by
  apply loop.spec_decr_nat (fun x => sl.val.length - x.1.i)
    (fun x => x.1.slice = sl ∧ x.1.i ≤ sl.val.length ∧ inv x.1.i x.2) post
  · rintro ⟨⟨sl', i⟩, s⟩ ⟨hsl, hi, hinv⟩
    dsimp only at hsl hi hinv
    subst hsl
    apply WP.spec_mono (hstep i s hi hinv)
    intro r hr
    cases r with
    | done y => exact hr
    | cont x' =>
      obtain ⟨h1, h2, h3⟩ := hr
      obtain ⟨it, s'⟩ := x'
      dsimp only at h1 h3 ⊢
      subst h1
      refine ⟨⟨rfl, ?_, h3⟩, ?_⟩ <;> dsimp only <;> omega
  · exact ⟨rfl, hi0, h0⟩

/-- A `for x in slice` loop, partial correctness. -/
theorem slice_loop_PC {T S R : Type} (sl : Slice T)
    (body : core.slice.iter.Iter T × S → Result (ControlFlow (core.slice.iter.Iter T × S) R))
    (inv : ℕ → S → Prop) (post : R → Prop)
    (hstep : ∀ i s, i ≤ sl.val.length → inv i s → PC (body (⟨sl, i⟩, s)) (fun r => match r with
        | .done y => post y
        | .cont x' => x'.1 = ⟨sl, i + 1⟩ ∧ i < sl.val.length ∧ inv (i + 1) x'.2))
    (i0 : ℕ) (hi0 : i0 ≤ sl.val.length) (s0 : S) (h0 : inv i0 s0) :
    PC (loop body (⟨sl, i0⟩, s0)) post := by
  apply PC_loop (fun x => sl.val.length - x.1.i)
    (fun x => x.1.slice = sl ∧ x.1.i ≤ sl.val.length ∧ inv x.1.i x.2) post
  · rintro ⟨⟨sl', i⟩, s⟩ ⟨hsl, hi, hinv⟩
    dsimp only at hsl hi hinv
    subst hsl
    refine PC_mono (hstep i s hi hinv) ?_
    intro r hr
    cases r with
    | done y => exact hr
    | cont x' =>
      obtain ⟨h1, h2, h3⟩ := hr
      obtain ⟨it, s'⟩ := x'
      dsimp only at h1 h3 ⊢
      subst h1
      refine ⟨⟨rfl, ?_, h3⟩, ?_⟩ <;> dsimp only <;> omega
  · exact ⟨rfl, hi0, h0⟩

/-! ## The reset prologue -/

/-- The orders after resetting the adjacency edges to `0`. -/
def resetSpec (order : Slice I32) (adj : List ℕ) (e : ℕ) : Option I32 :=
  if e ∈ adj then (order.val[e]?).map (fun _ => 0#i32) else order.val[e]?

theorem take_succ' {α : Type} (l : List α) (i : ℕ) (hi : i < l.length) :
    l.take (i + 1) = l.take i ++ [l[i]] := by
  rw [List.take_add_one, List.getElem?_eq_getElem hi]
  rfl

theorem into_iter_eq {T : Type} (s : Slice T) :
    SharedSlice.Insts.CoreIterTraitsCollectIntoIteratorSharedIter.into_iter s = ok ⟨s, 0⟩ := rfl

theorem reset_orders_spec (order : Slice I32) (adjacency : Slice Usize)
    (hadj : ∀ x ∈ adjacency.val, x.val < order.val.length) :
    ∃ order', circogen.blockpath.reset_orders order adjacency = ok (some (), order') ∧
      ∀ e, order'.val[e]? = resetSpec order (adjacency.val.map (·.val)) e := by
  unfold circogen.blockpath.reset_orders
  rw [into_iter_eq, bind_ok]
  suffices this : WP.spec (circogen.blockpath.reset_orders_loop ⟨adjacency, 0⟩ order) (fun r => ∃ order' : Slice I32, r = (some (), order') ∧
      ∀ e, order'.val[e]? = resetSpec order (adjacency.val.map (·.val)) e) by
    obtain ⟨r, hr, order', rfl, h⟩ := spec_ex this
    exact ⟨order', hr, h⟩
  unfold circogen.blockpath.reset_orders_loop
  refine slice_loop_spec adjacency _
    (fun i o => ∀ e, o.val[e]? = resetSpec order ((adjacency.val.map (·.val)).take i) e)
    _
    (by
      intro i o hi hinv
      dsimp only
      unfold circogen.blockpath.reset_orders_loop.body
      by_cases hlt : i < adjacency.val.length
      · rw [next_some _ _ hlt]
        have hel := hadj _ (List.getElem_mem hlt)
        have hlo : (adjacency.val[i]).val < o.val.length := by
          have := hinv (adjacency.val[i]).val
          unfold resetSpec at this
          by_contra hc
          rw [List.getElem?_eq_none_iff.mpr (by omega)] at this
          split_ifs at this <;> simp_all [List.getElem?_eq_getElem hel]
        obtain ⟨x, hx⟩ : ∃ x, o.val[(adjacency.val[i]).val]? = some x :=
          ⟨_, List.getElem?_eq_getElem hlo⟩
        simp [core.slice.Slice.get_mut, hx]
        refine ⟨hlt, fun e => ?_⟩
        have hset : (o.set_opt adjacency.val[i] (some 0#i32)).val =
            o.val.set_opt (adjacency.val[i]).val (some 0#i32) := by simp [Slice.set_opt]
        rw [getElem?_set_opt_some, take_succ' _ _ (by simpa using hlt)]
        unfold resetSpec
        by_cases he : e = (adjacency.val[i]).val
        · subst he
          simp [hlo, List.getElem?_eq_getElem hel]
        · simp only [he, false_and, ↓reduceIte, hinv e]
          unfold resetSpec
          simp [he]
      · rw [next_none _ _ hlt]
        simp only [bind_ok, uncurry_apply_pair]
        rw [WP.spec_ok]
        refine ⟨o, rfl, fun e => ?_⟩
        rw [hinv e, List.take_of_length_le (by simp; omega)])
    0 (by omega) order (by intro e; simp [resetSpec])

theorem ref_reset_PC (order : Slice I32) (adjacency : Slice Usize) :
    PC (circogen.blockpath.count_all_crossings_reference_loop0 ⟨adjacency, 0⟩ order)
      (fun order' => ∀ e, order'.val[e]? = resetSpec order (adjacency.val.map (·.val)) e) := by
  unfold circogen.blockpath.count_all_crossings_reference_loop0
  refine slice_loop_PC adjacency _
    (fun i o => ∀ e, o.val[e]? = resetSpec order ((adjacency.val.map (·.val)).take i) e)
    _ ?_ 0 (by omega) order (by intro e; simp [resetSpec])
  intro i o hi hinv
  dsimp only
  unfold circogen.blockpath.count_all_crossings_reference_loop0.body
  by_cases hlt : i < adjacency.val.length
  · rw [next_some _ _ hlt]
    simp only [bind_ok, uncurry_apply_pair]
    unfold Slice.update
    split
    · rw [bind_fail]; exact PC_fail
    · next x hx =>
      have hlo : (adjacency.val[i]).val < o.val.length := (List.getElem?_eq_some_iff.mp hx).1
      simp only [bind_ok]
      apply PC_ok
      refine ⟨rfl, hlt, fun e => ?_⟩
      rw [Slice.from_val, List.getElem?_set, take_succ' _ _ (by simpa using hlt)]
      unfold resetSpec
      by_cases he : (adjacency.val[i]).val = e
      · subst he
        have hel : (adjacency.val[i]).val < order.val.length := by
          have := hinv (adjacency.val[i]).val
          unfold resetSpec at this
          rw [hx] at this
          by_contra hc
          rw [List.getElem?_eq_none_iff.mpr (by omega)] at this
          split_ifs at this <;> simp_all
        simp [hlo, List.getElem?_eq_getElem hel]
      · simp only [he, ↓reduceIte, hinv e]
        unfold resetSpec
        simp [Ne.symm he]
  · rw [next_none _ _ hlt]
    simp only [bind_ok, uncurry_apply_pair]
    apply PC_ok
    intro e
    rw [hinv e, List.take_of_length_le (by simp; omega)]

/-! ## Bits and halves -/

theorem u64_lt (w : U64) : w.val < 2 ^ 64 := by
  have := w.hBounds; simpa [UScalarTy.numBits] using this

theorem mask_spec (x : Usize) (hx : x.val < 64) :
    ∃ m : U64, (1#u64 <<< x : Result U64) = ok m ∧ m.val = 2 ^ x.val := by
  refine ⟨⟨(1#u64).bv.shiftLeft x.val⟩, ?_, ?_⟩
  · show UScalar.shiftLeft_UScalar 1#u64 x = _
    simp [UScalar.shiftLeft_UScalar, UScalar.shiftLeft, UScalarTy.numBits, hx]
  · show ((1#64 : BitVec 64) <<< x.val).toNat = _
    rw [BitVec.toNat_shiftLeft]
    simp only [BitVec.toNat_ofNat, Nat.one_mod_two_pow (by norm_num : 0 < 64), Nat.one_shiftLeft]
    exact Nat.mod_eq_of_lt (Nat.pow_lt_pow_right (by norm_num) hx)

theorem wmask_val (x : Usize) (hx : x.val < 64) :
    (core.num.U64.wrapping_shl 1#u64 (UScalar.cast .U32 x)).val = 2 ^ x.val := by
  have hc : (UScalar.cast UScalarTy.U32 x).val = x.val := by
    show (x.bv.zeroExtend 32).toNat = x.val
    rw [BitVec.zeroExtend_eq_setWidth, BitVec.toNat_setWidth]
    exact Nat.mod_eq_of_lt (by have : x.bv.toNat = x.val := rfl; omega)
  show ((1#64 : BitVec 64).shiftLeft ((UScalar.cast .U32 x).val % 64)).toNat = _
  rw [hc, Nat.mod_eq_of_lt hx]
  show ((1#64 : BitVec 64) <<< x.val).toNat = _
  rw [BitVec.toNat_shiftLeft]
  simp only [BitVec.toNat_ofNat, Nat.one_mod_two_pow (by norm_num : 0 < 64), Nat.one_shiftLeft]
  exact Nat.mod_eq_of_lt (Nat.pow_lt_pow_right (by norm_num) hx)

theorem testBit_and_mask_ne (w : ℕ) (k : ℕ) : (w &&& 2 ^ k ≠ 0) ↔ w.testBit k = true := by
  rw [Nat.and_two_pow]
  cases w.testBit k <;> simp

theorem testBit_or_mask (w k j : ℕ) : (w ||| 2 ^ k).testBit j = (w.testBit j || decide (k = j)) := by
  rw [Nat.testBit_or, Nat.testBit_two_pow]

theorem testBit_and_not_mask (w : U64) (m : U64) (k j : ℕ) (hm : m.val = 2 ^ k) (hj : j < 64) :
    (w &&& ~~~m).val.testBit j = (w.val.testBit j && !decide (k = j)) := by
  rw [UScalar.val_and, Nat.testBit_and]
  congr 1
  show (~~~m.bv).toNat.testBit j = _
  rw [BitVec.testBit_toNat, BitVec.getLsbD_not]
  simp only [hj, decide_true, Bool.true_and]
  rw [← BitVec.testBit_toNat]
  show (! m.val.testBit j) = _
  rw [hm, Nat.testBit_two_pow]

theorem testBit_ge64 (w : U64) (j : ℕ) (hj : 64 ≤ j) : w.val.testBit j = false :=
  Nat.testBit_eq_false_of_lt (lt_of_lt_of_le (u64_lt w) (Nat.pow_le_pow_right (by norm_num) hj))

/-- Writing word `wi` changes only the bits of that word. -/
theorem bitAt_set (o o' : Slice U64) (wi : ℕ) (hw : wi < o.val.length) (w' : U64)
    (ho' : o'.val = o.val.set wi w') (h : ℕ) :
    bitAt o' h = if h / 64 = wi then w'.val.testBit (h % 64) else bitAt o h := by
  unfold bitAt
  rw [ho', List.getElem?_set]
  by_cases hh : h / 64 = wi
  · subst hh; simp [hw]
  · simp [Ne.symm hh, hh]

theorem length_set_eq (o o' : Slice U64) (wi : ℕ) (w' : U64) (ho' : o'.val = o.val.set wi w') :
    o'.length = o.length := by
  show o'.val.length = o.val.length
  rw [ho', List.length_set]

/-- Setting bit `h0`. -/
theorem bitsOf_set_bit (o o' : Slice U64) (h0 : ℕ) (w m : U64) (hw : o.val[h0 / 64]? = some w)
    (hm : m.val = 2 ^ (h0 % 64)) (ho' : o'.val = o.val.set (h0 / 64) (w ||| m)) :
    bitsOf o' = insert h0 (bitsOf o) := by
  have hwl := (List.getElem?_eq_some_iff.mp hw).1
  ext h
  rw [Finset.mem_insert, mem_bitsOf, mem_bitsOf, bitAt_set o o' _ hwl _ ho']
  by_cases hh : h / 64 = h0 / 64
  · simp only [hh, ↓reduceIte, UScalar.val_or, hm, testBit_or_mask]
    have : bitAt o h = w.val.testBit (h % 64) := by unfold bitAt; rw [hh, hw]
    rw [this]
    have : h0 % 64 = h % 64 ↔ h = h0 := by omega
    rw [Bool.or_eq_true, decide_eq_true_iff, this]
    tauto
  · simp only [hh, ↓reduceIte]
    have : h ≠ h0 := fun e => hh (by rw [e])
    simp [this]

/-- Clearing bit `h0`. -/
theorem bitsOf_clear_bit (o o' : Slice U64) (h0 : ℕ) (w m : U64) (hw : o.val[h0 / 64]? = some w)
    (hm : m.val = 2 ^ (h0 % 64)) (ho' : o'.val = o.val.set (h0 / 64) (w &&& ~~~m)) :
    bitsOf o' = (bitsOf o).erase h0 := by
  have hwl := (List.getElem?_eq_some_iff.mp hw).1
  ext h
  rw [Finset.mem_erase, mem_bitsOf, mem_bitsOf, bitAt_set o o' _ hwl _ ho']
  by_cases hh : h / 64 = h0 / 64
  · simp only [hh, ↓reduceIte, testBit_and_not_mask w m _ _ hm (Nat.mod_lt _ (by norm_num))]
    have : bitAt o h = w.val.testBit (h % 64) := by unfold bitAt; rw [hh, hw]
    rw [this]
    have : h0 % 64 = h % 64 ↔ h = h0 := by omega
    rw [Bool.and_eq_true, Bool.not_eq_true', decide_eq_false_iff_not, this]
    tauto
  · simp only [hh, ↓reduceIte]
    have : h ≠ h0 := fun e => hh (by rw [e])
    simp [this]

theorem bitAt_word (o : Slice U64) (h : ℕ) (w : U64) (hw : o.val[h / 64]? = some w) :
    bitAt o h = w.val.testBit (h % 64) := by
  unfold bitAt; rw [hw]

theorem set_opt_some_val {α : Type} (l : List α) (i : ℕ) (v : α) : l.set_opt i (some v) = l.set i v := by
  apply List.ext_getElem?
  intro j
  rw [getElem?_set_opt_some, List.getElem?_set]
  by_cases h : j = i
  · subst h; by_cases hl : j < l.length <;> simp [hl]
  · simp [h, Ne.symm h]

theorem slice_set_opt_val {α : Type} (s : Slice α) (i : Usize) (v : α) :
    (s.set_opt i (some v)).val = s.val.set i.val v := by
  simp [Slice.set_opt, set_opt_some_val]

/-- `(e << 1) | b` on a 64-bit `usize`, the indexed counter's form. -/
theorem half_idx (h : Is64) (e : Usize) (he : e.val < 2 ^ 62) (b : Bool) :
    (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from b).val =
      2 * e.val + b.toNat := by
  have hsh : (core.num.Usize.wrapping_shl e 1#u32).val = 2 * e.val := by
    show (e.bv.shiftLeft ((1#u32).val % UScalarTy.Usize.numBits)).toNat = _
    have h1 : (1#u32).val % UScalarTy.Usize.numBits = 1 := by
      simp [UScalarTy.numBits, h]
    rw [h1]
    show (e.bv <<< 1).toNat = _
    rw [BitVec.toNat_shiftLeft]
    have hw : UScalarTy.Usize.numBits = 64 := by simp [UScalarTy.numBits, h]
    have : e.bv.toNat = e.val := rfl
    rw [this, Nat.shiftLeft_eq, h]
    rw [Nat.mod_eq_of_lt (by omega)]
    ring
  rw [UScalar.val_or, hsh]
  have hb : (core.convert.num.FromUsizeBool.from b).val = b.toNat := by
    cases b <;> simp [core.convert.num.FromUsizeBool.from]
  rw [hb]
  cases b
  · simp
  · show 2 * e.val ||| 1 = 2 * e.val + 1
    have := Nat.two_pow_add_eq_or_of_lt (i := 1) (b := 1) (by norm_num) e.val
    simpa using this.symm

/-! ## Small rewriting lemmas for the extracted calls -/

theorem get_usize {T : Type} (s : Slice T) (i : Usize) :
    core.slice.Slice.get (core.slice.index.SliceIndexUsizeSlice T) s i = ok s.val[i.val]? := rfl

theorem get_mut_usize {T : Type} (s : Slice T) (i : Usize) :
    core.slice.Slice.get_mut (core.slice.index.SliceIndexUsizeSlice T) s i =
      ok (s.val[i.val]?, s.set_opt i) := rfl

theorem branch_some {T : Type} (x : T) :
    core.option.Option.Insts.CoreOpsTry_traitTry.branch (some x) = ok (.Continue x) := rfl

theorem branch_none {T : Type} :
    core.option.Option.Insts.CoreOpsTry_traitTry.branch (none : Option T) = ok (.Break none) := rfl

theorem from_residual_eq (T : Type) (r : Option Never) :
    core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual T r = ok none := rfl

theorem bitand_eq (x y : U64) : Shared0U64.Insts.CoreOpsBitBitAndU64U64.bitand x y = ok (x &&& y) := rfl

theorem lift_eq {α : Type} (x : α) : lift x = ok x := rfl

theorem div64 (x : Usize) : ∃ q : Usize, x / 64#usize = ok q ∧ q.val = x.val / 64 := by
  obtain ⟨z, hz, hv⟩ := UScalar.div_spec x (y := 64#usize) (by simp)
  exact ⟨z, hz, by simpa using hv⟩

theorem mod64 (x : Usize) : ∃ q : Usize, x % 64#usize = ok q ∧ q.val = x.val % 64 := by
  obtain ⟨z, hz, hv⟩ := spec_ex (UScalar.rem_spec x (y := 64#usize) (by simp))
  exact ⟨z, hz, by simpa using hv⟩

theorem range_next_some (a b : Usize) (hab : a.val < b.val) :
    ∃ a' : Usize, core.iter.range.IteratorRange.next core.iter.range.StepUsize ⟨a, b⟩ =
      ok (some a, ⟨a', b⟩) ∧ a'.val = a.val + 1 := by
  obtain ⟨⟨o, r'⟩, hr, ho, hs, he⟩ := spec_ex
    (core.iter.range.IteratorRange.next_Usize_some_spec ⟨a, b⟩ (by exact hab))
  simp only at ho hs he
  refine ⟨r'.start, ?_, hs⟩
  rw [hr, ho]
  cases r'
  dsimp only at he ⊢
  rw [he]

theorem range_next_none (a b : Usize) (hab : ¬ a.val < b.val) :
    core.iter.range.IteratorRange.next core.iter.range.StepUsize ⟨a, b⟩ = ok (none, ⟨a, b⟩) := by
  obtain ⟨⟨o, r'⟩, hr, ho, hs⟩ := spec_ex
    (core.iter.range.IteratorRange.next_Usize_none_spec ⟨a, b⟩ (by simp only; omega))
  rw [hr, ho, hs]

theorem hcast_usize (h : Is64) (x : I32) (hx : 0 ≤ x.val) :
    (IScalar.hcast .Usize x).val = x.val.toNat := by
  rw [IScalar.hcast_val_eq]
  have hb := x.hBounds
  simp [IScalarTy.numBits, UScalarTy.numBits, h] at hb ⊢
  rw [Int.emod_eq_of_lt hx (by omega)]

/-! ## `tree_step` refines `idxHalves` -/

theorem tree_step_loop_unfold (it : core.ops.range.Range Usize) (order : Slice I32)
    (op tree : Slice U64) (e : Usize) (insert : Bool) :
    circogen.blockpath.tree_step_loop 0#u8 it order op tree e insert =
      Std.bind (circogen.blockpath.tree_step_loop.body 0#u8 order op e insert it tree) (fun r =>
        match r with
        | .cont (it', t') => circogen.blockpath.tree_step_loop 0#u8 it' order op t' e insert
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.tree_step_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b⟩⟩ | y <;> rfl

theorem tree_step_loop_half (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (op : Slice U64)
    (tree : Slice U64) (e : Usize) (insert : Bool) (s : IdxState) (x : I32)
    (hbits : bitsOf op = s.bits) (ht : TRel N tree s.tree)
    (hord : order.val[e.val]? = some x) (hxv : x.val = (s.ord e.val : ℤ))
    (half fin : Usize) (hlt : half.val < fin.val) (hw : half.val < 64 * op.length)
    (s1 : IdxState) (hs1 : idxHalf N (delta insert) e.val s half.val = some s1) :
    ∃ (st' : Usize) (tree' : Slice U64), st'.val = half.val + 1 ∧ TRel N tree' s1.tree ∧
      s1.toRefState = s.toRefState ∧
      circogen.blockpath.tree_step_loop 0#u8 ⟨half, fin⟩ order op tree e insert =
        circogen.blockpath.tree_step_loop 0#u8 ⟨st', fin⟩ order op tree' e insert := by
  obtain ⟨st', hnext, hst'⟩ := range_next_some half fin hlt
  obtain ⟨q, hq, hqv⟩ := div64 half
  obtain ⟨r, hr, hrv⟩ := mod64 half
  have hrl : r.val < 64 := by omega
  obtain ⟨m, hm, hmv⟩ := mask_spec r hrl
  have hql : q.val < op.val.length := by
    have : op.length = op.val.length := rfl
    rw [hqv]; omega
  obtain ⟨w, hw'⟩ : ∃ w, op.val[q.val]? = some w := ⟨_, List.getElem?_eq_getElem hql⟩
  have hbit : (w &&& m != 0#u64) = true ↔ half.val ∈ s.bits := by
    rw [← hbits, mem_bitsOf, bitAt_word op _ w (by rw [← hqv]; exact hw'), bne_iff_ne, ne_eq,
      UScalar.eq_equiv, UScalar.val_and, hmv, hrv]
    have := testBit_and_mask_ne w.val (half.val % 64)
    simpa using this
  by_cases hb : half.val ∈ s.bits
  · have hx0 : 0 ≤ x.val := by rw [hxv]; omega
    have hi4 := hcast_usize h x hx0
    -- the model's update
    have hs1' := hs1
    unfold idxHalf at hs1'
    simp only [hb, ↓reduceIte] at hs1'
    obtain ⟨t, ht1, rfl⟩ : ∃ t, indexUpdate N (delta insert) s.tree (s.ord e.val) = some t ∧
        s1 = { s with tree := t } := by
      cases hu : indexUpdate N (delta insert) s.tree (s.ord e.val) with
      | none => rw [hu] at hs1'; simp at hs1'
      | some t => rw [hu] at hs1'; exact ⟨t, rfl, by simpa using hs1'.symm⟩
    obtain ⟨r', tree', hupd, hl', hmatch⟩ := index_update_refines h tree N hN ht.1
      (IScalar.hcast .Usize x) insert
    have hiv : (IScalar.hcast .Usize x).val = s.ord e.val := by rw [hi4, hxv]; simp
    rw [hiv] at hmatch
    have hc := indexUpdate_congr hN (delta insert) (t := tz tree) (t' := s.tree) ht.2 (s.ord e.val)
    rw [ht1] at hc
    cases hu : indexUpdate N (delta insert) (tz tree) (s.ord e.val) with
    | none => rw [hu] at hc; exact absurd hc (by simp)
    | some t' =>
      rw [hu] at hc hmatch
      obtain ⟨rfl, hj⟩ := hmatch
      refine ⟨st', tree', hst', ⟨hl', fun j hj0 hjN => ?_⟩, rfl, ?_⟩
      · rw [hj j hj0 hjN]; exact hc j hj0 hjN
      · have hbody : circogen.blockpath.tree_step_loop.body 0#u8 order op e insert ⟨half, fin⟩ tree =
            ok (cont (⟨st', fin⟩, tree')) := by
          simp only [circogen.blockpath.tree_step_loop.body, hnext, bind_ok, uncurry_apply_pair]
          simp only [show (0#u8 != 1#u8) = true from rfl, ↓reduceIte, hq, bind_ok, get_usize, hw',
            branch_some, hr, hm, bitand_eq, hbit.mpr hb, hord, lift_eq, hupd, uncurry_apply_pair]
        rw [tree_step_loop_unfold, hbody, bind_ok]
  · have : s1 = s := by
      unfold idxHalf at hs1; simp only [hb, ↓reduceIte] at hs1; simpa using hs1.symm
    subst this
    refine ⟨st', tree, hst', ht, rfl, ?_⟩
    have hb' : (w &&& m != 0#u64) = false := by
      cases hh : (w &&& m != 0#u64)
      · rfl
      · exact absurd (hbit.mp hh) hb
    have hbody : circogen.blockpath.tree_step_loop.body 0#u8 order op e insert ⟨half, fin⟩ tree =
        ok (cont (⟨st', fin⟩, tree)) := by
      simp only [circogen.blockpath.tree_step_loop.body, hnext, bind_ok, uncurry_apply_pair]
      simp only [show (0#u8 != 1#u8) = true from rfl, ↓reduceIte, hq, bind_ok, get_usize, hw',
        branch_some, hr, hm, bitand_eq, hb', Bool.false_eq_true]
    rw [tree_step_loop_unfold, hbody, bind_ok]

theorem tree_step_loop_end (order : Slice I32) (op : Slice U64) (tree : Slice U64) (e : Usize)
    (insert : Bool) (half fin : Usize) (hge : ¬ half.val < fin.val) :
    circogen.blockpath.tree_step_loop 0#u8 ⟨half, fin⟩ order op tree e insert = ok (some (), tree) := by
  rw [tree_step_loop_unfold]
  simp only [circogen.blockpath.tree_step_loop.body, range_next_none half fin hge, bind_ok,
    uncurry_apply_pair]

theorem shl1_val (h : Is64) (e : Usize) (he : e.val < 2 ^ 62) :
    (core.num.Usize.wrapping_shl e 1#u32).val = 2 * e.val := by
  have := half_idx h e he false
  rw [UScalar.val_or] at this
  simpa [core.convert.num.FromUsizeBool.from] using this

theorem tree_step_refines (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (op : Slice U64)
    (tree : Slice U64) (e : Usize) (insert : Bool) (s : IdxState) (x : I32)
    (he : e.val < 2 ^ 62) (hW : 2 * e.val + 2 ≤ 64 * op.length)
    (hbits : bitsOf op = s.bits) (ht : TRel N tree s.tree)
    (hord : order.val[e.val]? = some x) (hxv : x.val = (s.ord e.val : ℤ))
    (s' : IdxState) (hs : idxHalves N (delta insert) e.val s = some s') :
    ∃ tree', circogen.blockpath.tree_step 0#u8 order op tree e insert = ok (some (), tree') ∧
      TRel N tree' s'.tree ∧ s'.toRefState = s.toRefState := by
  rw [idxHalves_eq] at hs
  obtain ⟨s1, hs1, hs2⟩ := Option.bind_eq_some_iff.mp hs
  have hsh := shl1_val h e he
  obtain ⟨b, hb⟩ : ∃ b : Usize, core.num.Usize.wrapping_add (core.num.Usize.wrapping_shl e 1#u32) 2#usize = b := ⟨_, rfl⟩
  have hbv : b.val = 2 * e.val + 2 := by
    rw [← hb]
    simp [hsh, Usize.size, UScalar.size_def, Usize.numBits_def, UScalarTy.numBits, h]
    omega
  unfold circogen.blockpath.tree_step
  simp only [lift_eq, bind_ok, hb]
  obtain ⟨st1, tr1, hst1, ht1, hc1, heq1⟩ := tree_step_loop_half h hN order op tree e insert s x
    hbits ht hord hxv (core.num.Usize.wrapping_shl e 1#u32) b (by omega) (by omega) s1
    (by rw [hsh]; exact hs1)
  have ho1 : s1.ord = s.ord := toRef_ord hc1
  have hb1 : s1.bits = s.bits := toRef_bits hc1
  obtain ⟨st2, tr2, hst2, ht2, hc2, heq2⟩ := tree_step_loop_half h hN order op tr1 e insert s1 x
    (by rw [hb1]; exact hbits) ht1 hord (by rw [ho1]; exact hxv) st1 b (by omega) (by omega) s'
    (by rw [hst1, hsh]; exact hs2)
  refine ⟨tr2, ?_, ht2, by rw [hc2, hc1]⟩
  rw [heq1, heq2, tree_step_loop_end _ _ _ _ _ _ _ (by omega)]

/-! ## `exclude_edges` refines `foldlM idxExclude` -/

theorem foldlM_drop_succ {α β : Type} (f : β → α → Option β) (L : List α) (i : ℕ) (hi : i < L.length)
    (a b : β) (h : (L.drop i).foldlM f a = some b) :
    ∃ a', f a L[i] = some a' ∧ (L.drop (i + 1)).foldlM f a' = some b := by
  rw [List.drop_eq_getElem_cons hi, List.foldlM_cons] at h
  obtain ⟨a', h1, h2⟩ := Option.bind_eq_some_iff.mp h
  exact ⟨a', h1, h2⟩

theorem foldlM_drop_end {α β : Type} (f : β → α → Option β) (L : List α) (i : ℕ) (hi : ¬ i < L.length)
    (a b : β) (h : (L.drop i).foldlM f a = some b) : a = b := by
  rw [List.drop_eq_nil_of_le (by omega)] at h
  simpa using h

theorem exclude_edges_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (op tree : Slice U64) :
    circogen.blockpath.exclude_edges_loop 0#u8 it order op tree =
      Std.bind (circogen.blockpath.exclude_edges_loop.body 0#u8 order op it tree) (fun r =>
        match r with
        | .cont (it', t') => circogen.blockpath.exclude_edges_loop 0#u8 it' order op t'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.exclude_edges_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b⟩⟩ | y <;> rfl

/-- What every edge of a node's slice satisfies on the indexed path: an id below `2 ^ 62` whose
two halves fit the bitset, and an order slot holding the model's order. -/
def EdgeOk (order : Slice I32) (op : Slice U64) (ord : ℕ → ℕ) (e : Usize) : Prop :=
  e.val < 2 ^ 62 ∧ 2 * e.val + 2 ≤ 64 * op.length ∧
    ∃ x : I32, order.val[e.val]? = some x ∧ x.val = (ord e.val : ℤ)

theorem exclude_refines (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (op : Slice U64)
    (edges : Slice Usize) :
    ∀ (i : ℕ) (tree : Slice U64) (s s' : IdxState), i ≤ edges.val.length →
      bitsOf op = s.bits → TRel N tree s.tree → (∀ e ∈ edges.val, EdgeOk order op s.ord e) →
      ((edges.val.map (·.val)).drop i).foldlM (idxExclude N) s = some s' →
      ∃ tree', circogen.blockpath.exclude_edges_loop 0#u8 ⟨edges, i⟩ order op tree =
        ok (some (), tree') ∧ TRel N tree' s'.tree ∧ s'.toRefState = s.toRefState := by
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro tree s s' hi hb ht hok hf
    have hge : ¬ i < edges.val.length := by omega
    have := foldlM_drop_end _ _ i (by simpa using hge) s s' hf
    subst this
    refine ⟨tree, ?_, ht, rfl⟩
    rw [exclude_edges_loop_unfold]
    simp only [circogen.blockpath.exclude_edges_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
  | succ k ih =>
    intro tree s s' hi hb ht hok hf
    have hlt : i < edges.val.length := by omega
    obtain ⟨s1, h1, h2⟩ := foldlM_drop_succ _ _ i (by simpa using hlt) s s' hf
    simp only [List.getElem_map] at h1
    obtain ⟨he62, hW, x, hx, hxv⟩ := hok _ (List.getElem_mem hlt)
    obtain ⟨tree1, ht1, htr1, hc1⟩ := tree_step_refines h hN order op tree edges.val[i] false s x
      he62 hW hb ht hx hxv s1 h1
    have ho1 := toRef_ord hc1
    have hb1 := toRef_bits hc1
    obtain ⟨tree2, ht2, htr2, hc2⟩ := ih (i + 1) (by omega) tree1 s1 s' (by omega) (by rw [hb1]; exact hb)
      htr1 (by rw [ho1]; exact hok) h2
    refine ⟨tree2, ?_, htr2, by rw [hc2, hc1]⟩
    rw [exclude_edges_loop_unfold]
    simp only [circogen.blockpath.exclude_edges_loop.body, next_some _ _ hlt, bind_ok,
      uncurry_apply_pair, ht1, branch_some]
    exact ht2

/-! ## `restore_edges` refines `foldlM (idxRestoreClose g n)` -/

/-- The tail slot of an edge holds the graph's tail. -/
def TailOk (tail : Slice Usize) (g : Graph) (e : Usize) : Prop :=
  ∃ t : Usize, tail.val[e.val]? = some t ∧ g.tail e.val = t.val

theorem halfOf_val (h : Is64) (g : Graph) (n e t : Usize) (he : e.val < 2 ^ 62)
    (ht : g.tail e.val = t.val) :
    (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from (t != n)).val =
      halfOf g n.val e.val := by
  rw [half_idx h e he, halfOf, ht]
  by_cases htn : t = n
  · subst htn; simp
  · have : t.val ≠ n.val := fun hh => htn (UScalar.eq_equiv t n |>.mpr hh)
    simp [htn, this]

theorem i32_pos_iff (x : I32) (k : ℕ) (hx : x.val = (k : ℤ)) : x > 0#i32 ↔ 0 < k := by
  rw [gt_iff_lt, IScalar.lt_equiv, hx]
  simp

theorem restore_edges_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (tail : Slice Usize) (op tree : Slice U64) (n : Usize) :
    circogen.blockpath.restore_edges_loop 0#u8 it order tail op tree n =
      Std.bind (circogen.blockpath.restore_edges_loop.body 0#u8 order tail n it op tree) (fun r =>
        match r with
        | .cont (it', o', t') => circogen.blockpath.restore_edges_loop 0#u8 it' order tail o' t' n
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.restore_edges_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c⟩⟩ | y <;> rfl

theorem u8_ne : (0#u8 != 1#u8) = true ∧ (0#u8 != 3#u8) = true ∧ (0#u8 != 4#u8) = true ∧
    ((0#u8 = 2#u8) ↔ False) := by
  refine ⟨rfl, rfl, rfl, ?_⟩
  simp [UScalar.eq_equiv]

theorem restore_step (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (tail : Slice Usize)
    (n : Usize) (g : Graph) (edges : Slice Usize) (i : ℕ) (hlt : i < edges.val.length)
    (op tree : Slice U64) (s s2 : IdxState) (hb : bitsOf op = s.bits) (ht : TRel N tree s.tree)
    (hok : EdgeOk order op s.ord edges.val[i]) (htl : TailOk tail g edges.val[i])
    (hs : idxRestoreClose g n.val N s edges.val[i].val = some s2) :
    ∃ op' tree', circogen.blockpath.restore_edges_loop.body 0#u8 order tail n ⟨edges, i⟩ op tree =
      ok (cont (⟨edges, i + 1⟩, op', tree')) ∧ op'.length = op.length ∧ bitsOf op' = s2.bits ∧
      TRel N tree' s2.tree ∧ s2.ord = s.ord := by
  obtain ⟨e, he_def⟩ : ∃ e, edges.val[i] = e := ⟨_, rfl⟩
  rw [he_def] at hok htl hs
  obtain ⟨he62, hW, x, hx, hxv⟩ := hok
  obtain ⟨t, htv, htg⟩ := htl
  unfold idxRestoreClose at hs
  obtain ⟨s1, hs1, hs2⟩ := Option.bind_eq_some_iff.mp hs
  obtain ⟨tree1, ht1, htr1, hc1⟩ := tree_step_refines h hN order op tree e true s x he62 hW hb ht hx hxv
    s1 hs1
  have ho1 := toRef_ord hc1
  have hb1 := toRef_bits hc1
  have hx0 : 0 ≤ x.val := by rw [hxv]; omega
  have hhalf := halfOf_val h g n e t he62 htg
  have hhl : halfOf g n.val e.val < 64 * op.length := by
    unfold halfOf; split_ifs <;> omega
  by_cases h0 : 0 < s.ord e.val
  · have hx0' : x > 0#i32 := (i32_pos_iff x _ hxv).mpr h0
    have h0' : 0 < s1.ord e.val := by rw [ho1]; exact h0
    simp only [h0', ↓reduceIte] at hs2
    obtain ⟨q, hq, hqv⟩ := div64 (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from (t != n))
    obtain ⟨r, hr, hrv⟩ := mod64 (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hhalf] at hqv hrv
    have hrl : r.val < 64 := by omega
    obtain ⟨m, hm, hmv⟩ := mask_spec r hrl
    have hql : q.val < op.val.length := by
      have : op.length = op.val.length := rfl
      rw [hqv]; omega
    obtain ⟨w, hw⟩ : ∃ w, op.val[q.val]? = some w := ⟨_, List.getElem?_eq_getElem hql⟩
    have hbit : (w &&& m != 0#u64) = true ↔ halfOf g n.val e.val ∈ s1.bits := by
      rw [hb1, ← hb, mem_bitsOf, bitAt_word op _ w (by rw [← hqv]; exact hw), bne_iff_ne, ne_eq,
        UScalar.eq_equiv, UScalar.val_and, hmv, hrv]
      have := testBit_and_mask_ne w.val (halfOf g n.val e.val % 64)
      simpa using this
    have hcommon : circogen.blockpath.restore_edges_loop.body 0#u8 order tail n ⟨edges, i⟩ op tree =
        if (w &&& m != 0#u64) = true then
          Std.bind (circogen.blockpath.index_update tree1 (IScalar.hcast .Usize x) false) (fun p =>
            Std.bind (core.option.Option.Insts.CoreOpsTry_traitTry.branch p.1) (fun cf5 =>
              match cf5 with
              | core.ops.control_flow.ControlFlow.Continue _ =>
                ok (cont (⟨edges, i + 1⟩, op.set_opt q (some (w &&& ~~~m)), p.2))
              | core.ops.control_flow.ControlFlow.Break residual => Std.bind
                (core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionNever.from_residual Unit residual)
                (fun o7 => ok (done (o7, op.set_opt q (some (w &&& ~~~m)), p.2)))))
        else ok (cont (⟨edges, i + 1⟩, op, tree1)) := by
      simp only [circogen.blockpath.restore_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
        uncurry_apply_pair, ht1, branch_some, get_usize, hx, hx0', ↓reduceIte, lift_eq, htv,
        u8_ne, decide_false, Bool.xor_false, hr, hm, hq, hw, bitand_eq, get_mut_usize]
      split_ifs <;> rfl
    by_cases hbs : halfOf g n.val e.val ∈ s1.bits
    · simp only [hbs, ↓reduceIte] at hs2
      obtain ⟨tt, htt, rfl⟩ : ∃ tt, indexUpdate N (-1) s1.tree (s1.ord e.val) = some tt ∧
          s2 = { s1 with bits := s1.bits.erase (halfOf g n.val e.val), tree := tt } := by
        cases hu : indexUpdate N (-1) s1.tree (s1.ord e.val) with
        | none => rw [hu] at hs2; simp at hs2
        | some tt => rw [hu] at hs2; exact ⟨tt, rfl, by simpa using hs2.symm⟩
      obtain ⟨r', tree2, hupd, hl', hmatch⟩ := index_update_refines h tree1 N hN htr1.1
        (IScalar.hcast .Usize x) false
      have hiv : (IScalar.hcast .Usize x).val = s1.ord e.val := by
        rw [hcast_usize h x hx0, hxv, ho1]; simp
      rw [hiv] at hmatch
      have hc := indexUpdate_congr hN (delta false) (t := tz tree1) (t' := s1.tree) htr1.2 (s1.ord e.val)
      rw [show delta false = -1 from rfl, htt] at hc
      rw [show delta false = -1 from rfl] at hmatch
      cases hu : indexUpdate N (-1) (tz tree1) (s1.ord e.val) with
      | none => rw [hu] at hc; exact absurd hc (by simp)
      | some t' =>
        rw [hu] at hc hmatch
        obtain ⟨rfl, hj⟩ := hmatch
        refine ⟨op.set_opt q (some (w &&& ~~~m)), tree2, ?_, ?_, ?_, ⟨hl', fun j hj0 hjN => ?_⟩, ?_⟩
        · rw [hcommon, ite_eq_left (hbit.mpr hbs), hupd]
          simp only [bind_ok, branch_some]
        · exact length_set_eq op _ q.val _ (slice_set_opt_val op q _)
        · show bitsOf _ = s1.bits.erase _
          rw [hb1, ← hb]
          exact bitsOf_clear_bit op _ _ w m (by rw [← hqv]; exact hw) (by rw [hmv, hrv])
            (by rw [slice_set_opt_val, hqv])
        · rw [hj j hj0 hjN]; exact hc j hj0 hjN
        · exact ho1
    · simp only [hbs, ↓reduceIte] at hs2
      cases hs2
      refine ⟨op, tree1, ?_, rfl, by rw [hb1, hb], htr1, ho1⟩
      have hbf : ¬ (w &&& m != 0#u64) = true := fun hh => hbs (hbit.mp hh)
      rw [hcommon, ite_eq_right hbf]
  · have hx0' : ¬ x > 0#i32 := fun hh => h0 ((i32_pos_iff x _ hxv).mp hh)
    have h0' : ¬ 0 < s1.ord e.val := by rw [ho1]; exact h0
    simp only [h0', ↓reduceIte] at hs2
    cases hs2
    refine ⟨op, tree1, ?_, rfl, by rw [hb1, hb], htr1, ho1⟩
    simp only [circogen.blockpath.restore_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
      uncurry_apply_pair, ht1, branch_some, get_usize, hx, hx0', ↓reduceIte]

theorem restore_refines (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (tail : Slice Usize)
    (n : Usize) (g : Graph) (edges : Slice Usize) :
    ∀ (i : ℕ) (op tree : Slice U64) (s s' : IdxState), i ≤ edges.val.length →
      bitsOf op = s.bits → TRel N tree s.tree →
      (∀ e ∈ edges.val, EdgeOk order op s.ord e ∧ TailOk tail g e) →
      ((edges.val.map (·.val)).drop i).foldlM (idxRestoreClose g n.val N) s = some s' →
      ∃ op' tree', circogen.blockpath.restore_edges_loop 0#u8 ⟨edges, i⟩ order tail op tree n =
        ok (some (), op', tree') ∧ op'.length = op.length ∧ bitsOf op' = s'.bits ∧
        TRel N tree' s'.tree ∧ s'.ord = s.ord := by
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro op tree s s' hi hb ht hok hf
    have hge : ¬ i < edges.val.length := by omega
    have := foldlM_drop_end _ _ i (by simpa using hge) s s' hf
    subst this
    refine ⟨op, tree, ?_, rfl, hb, ht, rfl⟩
    rw [restore_edges_loop_unfold]
    simp only [circogen.blockpath.restore_edges_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
  | succ k ih =>
    intro op tree s s' hi hb ht hok hf
    have hlt : i < edges.val.length := by omega
    obtain ⟨s1, h1, h2⟩ := foldlM_drop_succ _ _ i (by simpa using hlt) s s' hf
    simp only [List.getElem_map] at h1
    have hoki := hok _ (List.getElem_mem hlt)
    obtain ⟨op1, tree1, hbody, hl1, hb1, ht1, ho1⟩ := restore_step h hN order tail n g edges i hlt op
      tree s s1 hb ht hoki.1 hoki.2 h1
    obtain ⟨op2, tree2, hl2, hl2', hb2, ht2, ho2⟩ := ih (i + 1) (by omega) op1 tree1 s1 s' (by omega)
      hb1 ht1 (fun e he => by
        obtain ⟨⟨a, b, c⟩, d⟩ := hok e he
        exact ⟨⟨a, by rw [hl1]; exact b, by rw [ho1]; exact c⟩, d⟩) h2
    refine ⟨op2, tree2, ?_, by rw [hl2', hl1], hb2, ht2, by rw [ho2, ho1]⟩
    rw [restore_edges_loop_unfold, hbody, bind_ok]
    exact hl2

/-! ## `open_edges` refines `foldlM (idxOpen g n order)` -/

theorem open_edges_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (tail : Slice Usize) (op tree : Slice U64) (n : Usize) (occ : I32) :
    circogen.blockpath.open_edges_loop it order tail op tree n occ =
      Std.bind (circogen.blockpath.open_edges_loop.body tail n occ it order op tree) (fun r =>
        match r with
        | .cont (it', o', op', t') => circogen.blockpath.open_edges_loop it' o' tail op' t' n occ
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.open_edges_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c, d⟩⟩ | y <;> rfl

theorem ORel_update (order0 : Slice I32) (adj : List ℕ) (ord : ℕ → ℕ) (order : Slice I32)
    (hor : ORel order0 adj ord order) (e : Usize) (he : e.val ∈ adj) (occ : I32) (k : ℕ)
    (hocc : occ.val = (k : ℤ)) (hlt : e.val < order.val.length) :
    ORel order0 adj (Function.update ord e.val k) (order.set_opt e (some occ)) := by
  have hv : (order.set_opt e (some occ)).val = order.val.set e.val occ := by
    simp [Slice.set_opt, set_opt_some_val]
  refine ⟨fun e' he' => ?_, fun e' he' => ?_⟩
  · rw [hv, List.getElem?_set]
    by_cases hee : e.val = e'
    · subst hee; simp [hlt, hocc]
    · simp only [hee, ↓reduceIte, Function.update_of_ne (Ne.symm hee)]
      exact hor.inAdj e' he'
  · rw [hv, List.getElem?_set]
    have hee : e.val ≠ e' := fun hh => he' (hh ▸ he)
    simp only [hee, ↓reduceIte]
    exact hor.outAdj e' he'

theorem open_step (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order0 : Slice I32) (adj : List ℕ)
    (order : Slice I32) (tail : Slice Usize) (n : Usize) (g : Graph) (edges : Slice Usize) (i : ℕ)
    (hlt : i < edges.val.length) (op tree : Slice U64) (s s2 : IdxState) (occ : I32) (k : ℕ)
    (hocc : occ.val = (k : ℤ)) (hor : ORel order0 adj s.ord order) (hb : bitsOf op = s.bits)
    (ht : TRel N tree s.tree) (hadj : edges.val[i].val ∈ adj) (he62 : edges.val[i].val < 2 ^ 62)
    (hW : 2 * edges.val[i].val + 2 ≤ 64 * op.length) (htl : TailOk tail g edges.val[i])
    (hs : idxOpen g n.val N k s edges.val[i].val = some s2) :
    ∃ order' op' tree', circogen.blockpath.open_edges_loop.body tail n occ ⟨edges, i⟩ order op tree =
      ok (cont (⟨edges, i + 1⟩, order', op', tree')) ∧ ORel order0 adj s2.ord order' ∧
      op'.length = op.length ∧ bitsOf op' = s2.bits ∧ TRel N tree' s2.tree := by
  obtain ⟨e, he_def⟩ : ∃ e, edges.val[i] = e := ⟨_, rfl⟩
  rw [he_def] at hadj he62 hW htl hs
  obtain ⟨x, hx, hxv⟩ := hor.inAdj e.val hadj
  obtain ⟨t, htv, htg⟩ := htl
  have hxl : e.val < order.val.length := (List.getElem?_eq_some_iff.mp hx).1
  unfold idxOpen at hs
  by_cases h0 : s.ord e.val = 0
  · have hx0 : x = 0#i32 := by
      rw [IScalar.eq_equiv, hxv, h0]; simp
    simp only [h0, ↓reduceIte] at hs
    obtain ⟨tt, htt, rfl⟩ : ∃ tt, indexUpdate N 1 s.tree k = some tt ∧
        s2 = { s with ord := Function.update s.ord e.val k,
                      bits := insert (halfOf g n.val e.val) s.bits, tree := tt } := by
      cases hu : indexUpdate N 1 s.tree k with
      | none => rw [hu] at hs; simp at hs
      | some tt => rw [hu] at hs; exact ⟨tt, rfl, by simpa using hs.symm⟩
    have hhalf := halfOf_val h g n e t he62 htg
    have hhl : halfOf g n.val e.val < 64 * op.length := by
      unfold halfOf; split_ifs <;> omega
    obtain ⟨q, hq, hqv⟩ := div64 (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from (t != n))
    obtain ⟨r, hr, hrv⟩ := mod64 (core.num.Usize.wrapping_shl e 1#u32 ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hhalf] at hqv hrv
    have hrl : r.val < 64 := by omega
    obtain ⟨m, hm, hmv⟩ := mask_spec r hrl
    have hql : q.val < op.val.length := by
      have : op.length = op.val.length := rfl
      rw [hqv]; omega
    obtain ⟨w, hw⟩ : ∃ w, op.val[q.val]? = some w := ⟨_, List.getElem?_eq_getElem hql⟩
    have hk0 : 0 ≤ occ.val := by rw [hocc]; omega
    obtain ⟨r', tree2, hupd, hl', hmatch⟩ := index_update_refines h tree N hN ht.1
      (IScalar.hcast .Usize occ) true
    have hiv : (IScalar.hcast .Usize occ).val = k := by rw [hcast_usize h occ hk0, hocc]; simp
    rw [hiv] at hmatch
    have hc := indexUpdate_congr hN (delta true) (t := tz tree) (t' := s.tree) ht.2 k
    rw [show delta true = 1 from rfl, htt] at hc
    rw [show delta true = 1 from rfl] at hmatch
    cases hu : indexUpdate N 1 (tz tree) k with
    | none => rw [hu] at hc; exact absurd hc (by simp)
    | some t' =>
      rw [hu] at hc hmatch
      obtain ⟨rfl, hj⟩ := hmatch
      refine ⟨order.set_opt e (some occ), op.set_opt q (some (w ||| m)), tree2, ?_, ?_, ?_, ?_,
        ⟨hl', fun j hj0 hjN => ?_⟩⟩
      · simp only [circogen.blockpath.open_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
          uncurry_apply_pair, get_usize, hx, branch_some, hx0, ↓reduceIte, get_mut_usize, lift_eq,
          htv, hq, hw, hr, hm, hupd]
      · exact ORel_update order0 adj s.ord order hor e hadj occ k hocc hxl
      · exact length_set_eq op _ q.val _ (slice_set_opt_val op q _)
      · show bitsOf _ = insert _ s.bits
        rw [← hb]
        exact bitsOf_set_bit op _ _ w m (by rw [← hqv]; exact hw) (by rw [hmv, hrv])
          (by rw [slice_set_opt_val, hqv])
      · rw [hj j hj0 hjN]; exact hc j hj0 hjN
  · have hx0 : ¬ x = 0#i32 := by
      rw [IScalar.eq_equiv, hxv]; simpa using h0
    simp only [h0, ↓reduceIte] at hs
    cases hs
    refine ⟨order, op, tree, ?_, hor, rfl, hb, ht⟩
    simp only [circogen.blockpath.open_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
      uncurry_apply_pair, get_usize, hx, branch_some, hx0, ↓reduceIte]

theorem open_refines (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order0 : Slice I32) (adj : List ℕ)
    (tail : Slice Usize) (n : Usize) (g : Graph) (edges : Slice Usize) (occ : I32) (k : ℕ)
    (hocc : occ.val = (k : ℤ)) :
    ∀ (i : ℕ) (order : Slice I32) (op tree : Slice U64) (s s' : IdxState), i ≤ edges.val.length →
      ORel order0 adj s.ord order → bitsOf op = s.bits → TRel N tree s.tree →
      (∀ e ∈ edges.val, e.val ∈ adj ∧ e.val < 2 ^ 62 ∧ 2 * e.val + 2 ≤ 64 * op.length ∧
        TailOk tail g e) →
      ((edges.val.map (·.val)).drop i).foldlM (idxOpen g n.val N k) s = some s' →
      ∃ order' op' tree', circogen.blockpath.open_edges_loop ⟨edges, i⟩ order tail op tree n occ =
        ok (some (), order', op', tree') ∧ ORel order0 adj s'.ord order' ∧
        op'.length = op.length ∧ bitsOf op' = s'.bits ∧ TRel N tree' s'.tree := by
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro order op tree s s' hi hor hb ht hok hf
    have hge : ¬ i < edges.val.length := by omega
    have := foldlM_drop_end _ _ i (by simpa using hge) s s' hf
    subst this
    refine ⟨order, op, tree, ?_, hor, rfl, hb, ht⟩
    rw [open_edges_loop_unfold]
    simp only [circogen.blockpath.open_edges_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
  | succ k' ih =>
    intro order op tree s s' hi hor hb ht hok hf
    have hlt : i < edges.val.length := by omega
    obtain ⟨s1, h1, h2⟩ := foldlM_drop_succ _ _ i (by simpa using hlt) s s' hf
    simp only [List.getElem_map] at h1
    obtain ⟨ha, he62, hW, htl⟩ := hok _ (List.getElem_mem hlt)
    obtain ⟨order1, op1, tree1, hbody, hor1, hl1, hb1, ht1⟩ := open_step h hN order0 adj order tail n
      g edges i hlt op tree s s1 occ k hocc hor hb ht ha he62 hW htl h1
    obtain ⟨order2, op2, tree2, hl2, hor2, hl2', hb2, ht2⟩ := ih (i + 1) (by omega) order1 op1 tree1
      s1 s' (by omega) hor1 hb1 ht1 (fun e he => by
        obtain ⟨a, b, c, d⟩ := hok e he
        exact ⟨a, b, by rw [hl1]; exact c, d⟩) h2
    refine ⟨order2, op2, tree2, ?_, hor2, by rw [hl2', hl1], hb2, ht2⟩
    rw [open_edges_loop_unfold, hbody, bind_ok]
    exact hl2

/-! ## `query_edges`: the exact count, under the checked-build `i32` ceiling -/

theorem bmod_small (z : ℤ) (h0 : 0 ≤ z) (h1 : z ≤ 2147483647) : Int.bmod z (2 ^ 32) = z := by
  rw [Int.bmod_def]
  have : z % (2 ^ 32 : ℕ) = z := Int.emod_eq_of_lt h0 (by norm_num; omega)
  rw [this]
  simp only [Nat.cast_pow, Nat.cast_ofNat]
  split_ifs with hh
  · rfl
  · exfalso; apply hh; norm_num; omega

theorem add_crossings_val (value : I32) (count : U64) (h0 : 0 ≤ value.val)
    (hc : value.val + count.val ≤ 2147483647) :
    ∃ v', circogen.blockpath.add_crossings value count = ok v' ∧ v'.val = value.val + count.val := by
  have e1 : (I32.rMax % 18446744073709551616).toNat = 2147483647 := by
    simp [I32.rMax]
  have e2 : (value.val % 18446744073709551616).toNat = value.val.toNat := by
    have := value.hBounds
    simp at this
    omega
  have e3 : U64.size = 2 ^ 64 := by simp [U64.size, UScalar.size_def, U64.numBits_def, UScalarTy.numBits]
  have hcond : (count.val ≤ (2147483647 + (18446744073709551616 - value.val.toNat)) % 18446744073709551616) := by
    omega
  refine ⟨core.num.I32.wrapping_add value (UScalar.hcast IScalarTy.I32 count), ?_, ?_⟩
  · unfold circogen.blockpath.add_crossings circogen.blockpath.wrap_crossings
    simp [lift, core.num.I32.MAX, e1, e2, e3, massert, hcond]
  · rw [core.num.I32.wrapping_add_val_eq, UScalar.hcast_val_eq]
    have hcv : Int.bmod (count.val : ℤ) (2 ^ IScalarTy.I32.numBits) = count.val := by
      rw [show IScalarTy.I32.numBits = 32 from rfl]
      exact bmod_small _ (by omega) (by omega)
    rw [hcv]
    exact bmod_small _ (by omega) hc

theorem checked_sub_eq (a b : U64) (hba : b.val ≤ a.val) :
    ∃ z : U64, U64.checked_sub a b = some z ∧ z.val = a.val - b.val := by
  have := U64.checked_sub_bv_spec a b
  cases hz : U64.checked_sub a b with
  | none => rw [hz] at this; simp at this; omega
  | some z => rw [hz] at this; exact ⟨z, rfl, this.2.1⟩

theorem query_edges_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (tree : Slice U64) (total : U64) (c : I32) :
    circogen.blockpath.query_edges_loop it order tree total c =
      Std.bind (circogen.blockpath.query_edges_loop.body order tree total it c) (fun r =>
        match r with
        | .cont (it', c') => circogen.blockpath.query_edges_loop it' order tree total c'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.query_edges_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b⟩⟩ | y <;> rfl

/-- Running total over the edges `L` with an order: `Σ cnt e`. -/
def qsum (ord : ℕ → ℕ) (cnt : ℕ → ℕ) (L : List ℕ) : ℕ :=
  ((L.filter (fun e => 0 < ord e)).map cnt).sum

theorem qsum_cons (ord cnt : ℕ → ℕ) (e : ℕ) (L : List ℕ) :
    qsum ord cnt (e :: L) = (if 0 < ord e then cnt e else 0) + qsum ord cnt L := by
  unfold qsum
  by_cases h : 0 < ord e <;> simp [h, List.filter_cons]

theorem query_refines (h : Is64) {N : ℕ} (hN : N < 2 ^ 62) (order : Slice I32) (tree : Slice U64)
    (s : IdxState) (a : ℕ → ZMod (2 ^ 64)) (hv : Valid N s.tree a) (ht : TRel N tree s.tree)
    (total : U64) (cnt : ℕ → ℕ) (edges : Slice Usize)
    (hord : ∀ e ∈ edges.val, ∃ x : I32, order.val[e.val]? = some x ∧ x.val = (s.ord e.val : ℤ))
    (hcnt : ∀ e ∈ edges.val, 0 < s.ord e.val → ∃ p, indexPrefix N s.tree (s.ord e.val) = some p ∧
      checkedSub (total.val : ZMod (2 ^ 64)) p = some (cnt e.val)) :
    ∀ (i : ℕ) (c : I32), i ≤ edges.val.length → 0 ≤ c.val →
      c.val + qsum s.ord cnt ((edges.val.map (·.val)).drop i) ≤ 2147483647 →
      ∃ c', circogen.blockpath.query_edges_loop ⟨edges, i⟩ order tree total c = ok (some c') ∧
        c'.val = c.val + qsum s.ord cnt ((edges.val.map (·.val)).drop i) := by
  have hvt : Valid N (tz tree) a := fun j hj0 hj => by rw [ht.2 j hj0 hj]; exact hv j hj0 hj
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro c hi hc0 hcb
    have hge : ¬ i < edges.val.length := by omega
    refine ⟨c, ?_, ?_⟩
    · rw [query_edges_loop_unfold]
      simp only [circogen.blockpath.query_edges_loop.body, next_none _ _ hge, bind_ok,
        uncurry_apply_pair]
    · rw [List.drop_eq_nil_of_le (by simp; omega)]; simp [qsum]
  | succ k ih =>
    intro c hi hc0 hcb
    have hlt : i < edges.val.length := by omega
    have hdrop : (edges.val.map (·.val)).drop i = edges.val[i].val :: (edges.val.map (·.val)).drop (i + 1) := by
      rw [List.drop_eq_getElem_cons (by simpa using hlt)]; simp
    rw [hdrop, qsum_cons] at hcb ⊢
    obtain ⟨e, he_def⟩ : ∃ e, edges.val[i] = e := ⟨_, rfl⟩
    have hmem : e ∈ edges.val := he_def ▸ List.getElem_mem hlt
    rw [he_def] at hcb ⊢
    obtain ⟨x, hx, hxv⟩ := hord e hmem
    by_cases h0 : 0 < s.ord e.val
    · have hx0 : x > 0#i32 := (i32_pos_iff x _ hxv).mpr h0
      obtain ⟨p, hp, hsub⟩ := hcnt e hmem h0
      have hx0' : 0 ≤ x.val := by rw [hxv]; omega
      have hiv : (IScalar.hcast .Usize x).val = s.ord e.val := by
        rw [hcast_usize h x hx0', hxv]; simp
      obtain ⟨r, hr, hrm⟩ := index_prefix_refines h tree N hN ht.1 a hvt (IScalar.hcast .Usize x)
      rw [hiv, indexPrefix_congr hN ht.2, hp] at hrm
      obtain ⟨v1, rfl, hv1⟩ : ∃ v1 : U64, r = some v1 ∧ (v1.val : ZMod (2 ^ 64)) = p := by
        cases r with
        | none => simp at hrm
        | some v1 => exact ⟨v1, rfl, by simpa using hrm⟩
      unfold checkedSub at hsub
      have hvl : ((v1.val : ZMod (2 ^ 64))).val = v1.val := by
        rw [ZMod.val_natCast]; exact Nat.mod_eq_of_lt (u64_lt v1)
      have htl : ((total.val : ZMod (2 ^ 64))).val = total.val := by
        rw [ZMod.val_natCast]; exact Nat.mod_eq_of_lt (u64_lt total)
      rw [← hv1, hvl, htl] at hsub
      split_ifs at hsub with hle
      obtain ⟨z, hz, hzv⟩ := checked_sub_eq total v1 hle
      have hzc : z.val = cnt e.val := by simp at hsub; omega
      simp only [h0, ↓reduceIte] at hcb ⊢
      obtain ⟨c1, hc1, hc1v⟩ := add_crossings_val c z hc0 (by rw [hzc]; omega)
      obtain ⟨c2, hl2, hc2⟩ := ih (i + 1) (by omega) c1 (by omega) (by rw [hc1v, hzc]; omega)
        (by rw [hc1v, hzc]; push_cast at hcb ⊢; omega)
      refine ⟨c2, ?_, by rw [hc2, hc1v, hzc]; push_cast; ring⟩
      rw [query_edges_loop_unfold]
      simp only [circogen.blockpath.query_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
        uncurry_apply_pair, get_usize, hx, branch_some, hx0, ↓reduceIte, lift_eq, hr, hz, hc1]
      exact hl2
    · have hx0 : ¬ x > 0#i32 := fun hh => h0 ((i32_pos_iff x _ hxv).mp hh)
      simp only [h0, ↓reduceIte, zero_add] at hcb ⊢
      obtain ⟨c2, hl2, hc2⟩ := ih (i + 1) (by omega) c (by omega) hc0 (by simpa using hcb)
      refine ⟨c2, ?_, by rw [hc2]⟩
      rw [query_edges_loop_unfold]
      simp only [circogen.blockpath.query_edges_loop.body, next_some _ _ hlt, he_def, bind_ok,
        uncurry_apply_pair, get_usize, hx, branch_some, hx0, ↓reduceIte]
      exact hl2

/-! ## The bit walk of the reference scan: `trailing_zeros` and `bits &= bits - 1` -/

theorem lowest_bit (x : ℕ) (hx : x ≠ 0) :
    ∃ k, x.testBit k = true ∧ ∀ j < k, x.testBit j = false := by
  have hex := Nat.exists_testBit_of_ne_zero hx
  classical
  refine ⟨Nat.find hex, Nat.find_spec hex, fun j hj => ?_⟩
  have := Nat.find_min hex hj
  simpa using this

theorem tz_spec (x : U64) (k : ℕ) (hk : x.val.testBit k = true)
    (hlow : ∀ j < k, x.val.testBit j = false) :
    k < 64 ∧ ∃ z : U32, core.num.U64.trailing_zeros x = ok z ∧ z.val = k := by
  have hk64 : k < 64 := by
    by_contra hc
    rw [testBit_ge64 x k (by omega)] at hk
    cases hk
  refine ⟨hk64, _, rfl, ?_⟩
  have hf : (List.range 64).find? (fun k => x.val.testBit k) = some k := by
    rw [List.find?_eq_some_iff_getElem]
    refine ⟨hk, k, by simpa using hk64, by simp, fun j hj => ?_⟩
    simp only [List.getElem_range]
    simp [hlow j hj]
  simp only [hf, Option.getD_some]
  rw [UScalar.ofNatCore_val_eq]

theorem testBit_and_pred (x k : ℕ) (hk : x.testBit k = true) (hlow : ∀ j < k, x.testBit j = false)
    (j : ℕ) : (x &&& (x - 1)).testBit j = (x.testBit j && decide (j ≠ k)) := by
  have hmod : x % 2 ^ k = 0 := by
    apply Nat.eq_of_testBit_eq
    intro j
    rw [Nat.testBit_mod_two_pow, Nat.zero_testBit]
    by_cases hj : j < k
    · simp [hj, hlow j hj]
    · simp [hj]
  obtain ⟨q, hq⟩ : 2 ^ k ∣ x := Nat.dvd_of_mod_eq_zero hmod
  have hq1 : q.testBit 0 = true := by
    have := hk
    rw [hq, Nat.testBit_two_pow_mul] at this
    simpa using this
  have hqodd : q % 2 = 1 := by
    have := Nat.testBit_zero q
    rw [hq1] at this
    simp at this
    omega
  obtain ⟨m, rfl⟩ : ∃ m, q = 2 * m + 1 := ⟨q / 2, by omega⟩
  have hx : x = 2 ^ (k + 1) * m + 2 ^ k := by rw [hq, pow_succ]; ring
  have hp : 0 < 2 ^ k := Nat.two_pow_pos k
  have hx1 : x - 1 = 2 ^ (k + 1) * m + (2 ^ k - 1) := by omega
  have hlt1 : 2 ^ k < 2 ^ (k + 1) := Nat.pow_lt_pow_right (by norm_num) (by omega)
  rw [Nat.testBit_and, hx1, hx, Nat.testBit_two_pow_mul_add _ hlt1,
    Nat.testBit_two_pow_mul_add _ (by omega : 2 ^ k - 1 < 2 ^ (k + 1))]
  by_cases hj : j < k + 1
  · simp only [hj, ↓reduceIte, Nat.testBit_two_pow, Nat.testBit_two_pow_sub_one]
    by_cases hjk : j = k
    · subst hjk; simp
    · simp [hjk, Ne.symm hjk]
  · simp only [hj, ↓reduceIte]
    have : j ≠ k := by omega
    simp [this]

theorem clear_lowest (x : U64) (k : ℕ) (hk : x.val.testBit k = true)
    (hlow : ∀ j < k, x.val.testBit j = false) (j : ℕ) :
    (x &&& core.num.U64.wrapping_sub x 1#u64).val.testBit j = (x.val.testBit j && decide (j ≠ k)) ∧
      (x &&& core.num.U64.wrapping_sub x 1#u64).val < x.val := by
  have hx0 : x.val ≠ 0 := by intro h0; rw [h0] at hk; simp at hk
  have hsub : (core.num.U64.wrapping_sub x 1#u64).val = x.val - 1 := by
    rw [core.num.U64.wrapping_sub_val_eq]
    have e : UScalar.size .U64 = 2 ^ 64 := by simp [UScalar.size_def, UScalarTy.numBits]
    rw [e]
    have := u64_lt x
    have h1 : (1#u64).val = 1 := rfl
    rw [h1, show x.val + (2 ^ 64 - 1) = (x.val - 1) + 2 ^ 64 by omega, Nat.add_mod_right,
      Nat.mod_eq_of_lt (by omega)]
  rw [UScalar.val_and, hsub]
  refine ⟨testBit_and_pred x.val k hk hlow j, ?_⟩
  have := Nat.and_le_right (n := x.val) (m := x.val - 1)
  omega

/-! ## `reference_scan` counts the set halves that satisfy its test -/

/-- The scan's test on half `h`, read off the slices: `order[ep] > edge_order`, `head[ep] != n`,
`tail[ep] != n` with `ep = h / 2` (absent slots read as `0`; the scan fails on them). -/
abbrev scanPred (order : Slice I32) (head tail : Slice Usize) (th : I32) (n : Usize) (h : ℕ) : Prop :=
  th.val < ((order.val[h / 2]?).map (·.val)).getD 0 ∧
    ((head.val[h / 2]?).map (·.val)).getD 0 ≠ n.val ∧ ((tail.val[h / 2]?).map (·.val)).getD 0 ≠ n.val

theorem shl_lit (h : Is64) (x : Usize) (k : ℕ) (hk : k < 64) (y : I32) (hy : y.val = (k : ℤ)) :
    ∃ v : Usize, (x <<< y : Result Usize) = ok v ∧ v.val = (x.val * 2 ^ k) % 2 ^ 64 := by
  refine ⟨⟨x.bv.shiftLeft k⟩, ?_, ?_⟩
  · show UScalar.shiftLeft_IScalar x y = _
    have hy0 : y.val ≥ 0 := by rw [hy]; omega
    have hyt : IScalar.toNat y = k := by simp [IScalar.toNat, hy]
    have hk' : k < UScalarTy.Usize.numBits := by simp [UScalarTy.numBits, h]; omega
    unfold UScalar.shiftLeft_IScalar UScalar.shiftLeft
    rw [ite_eq_left hy0, hyt, ite_eq_left hk']
  · show (x.bv <<< k).toNat = _
    rw [BitVec.toNat_shiftLeft, Nat.shiftLeft_eq]
    have e1 : x.bv.toNat = x.val := rfl
    have e2 : (2 : ℕ) ^ System.Platform.numBits = 2 ^ 64 := by rw [h]
    rw [e1, e2]

theorem i32_add_one {c z : I32} (hz : c + 1#i32 = ok z) : z.val = c.val + 1 := by
  have h := IScalar.add_equiv c 1#i32
  rw [hz] at h
  simp only [Result.match.ok] at h
  rw [h.2.1]; rfl

theorem bne_usize (a b : Usize) : (a != b) = true ↔ a.val ≠ b.val := by
  rw [bne_iff_ne, ne_eq, UScalar.eq_equiv]

/-- One step of the bit walk: the half it visits and whether it counts. -/
theorem scan_step_PC (order : Slice I32) (head tail : Slice Usize) (th : I32)
    (n : Usize) (half : Usize) (c : I32) {β : Type} (K : I32 → Result β) (Q : β → Prop)
    (hK : ∀ c' : I32, c'.val = c.val + (if scanPred order head tail th n half.val then 1 else 0) →
      PC (K c') Q) :
    PC (Std.bind (half / 2#usize) (fun ep => Std.bind (Slice.index_usize order ep) (fun i3 =>
      Std.bind (if i3 > th then Std.bind (Slice.index_usize head ep) (fun i4 =>
        if (i4 != n) = true then Std.bind (Slice.index_usize tail ep) (fun i5 =>
          if (i5 != n) = true then c + 1#i32 else ok c) else ok c) else ok c) K))) Q := by
  obtain ⟨ep, hep, hepv⟩ := UScalar.div_spec half (y := 2#usize) (by simp)
  rw [hep, bind_ok]
  have hepv' : ep.val = half.val / 2 := by simpa using hepv
  unfold Slice.index_usize
  simp only [Slice.getElem?_Usize_eq, hepv']
  split
  · rw [bind_fail]; exact PC_fail
  · next x ho =>
    rw [bind_ok]
    by_cases hx : x > th
    · simp only [hx, ↓reduceIte]
      split
      · rw [bind_fail, bind_fail]; exact PC_fail
      · next hd hh =>
        rw [bind_ok]
        by_cases hhd : (hd != n) = true
        · simp only [hhd, ↓reduceIte]
          split
          · rw [bind_fail, bind_fail]; exact PC_fail
          · next tl ht =>
            rw [bind_ok]
            by_cases htl : (tl != n) = true
            · simp only [htl, ↓reduceIte]
              have hp : scanPred order head tail th n half.val := by
                refine ⟨?_, ?_, ?_⟩
                · simp only [ho, Option.map_some, Option.getD_some]
                  exact (IScalar.lt_equiv _ _).mp hx
                · simpa [hh] using (bne_usize hd n).mp hhd
                · simpa [ht] using (bne_usize tl n).mp htl
              intro y hy
              cases hz : c + 1#i32 with
              | ret z =>
                rw [hz, bind_ok] at hy
                exact hK z (by rw [i32_add_one hz]; simp [hp]) y hy
              | vis i k => rw [hz] at hy; simp [bind_vis] at hy
              | div => rw [hz] at hy; simp at hy
            · simp only [htl, Bool.false_eq_true, ↓reduceIte, bind_ok]
              have : ¬ scanPred order head tail th n half.val := by
                rintro ⟨_, _, h3⟩
                apply htl
                rw [bne_usize]
                simpa [ht] using h3
              exact hK c (by simp [this])
        · simp only [hhd, Bool.false_eq_true, ↓reduceIte, bind_ok]
          have : ¬ scanPred order head tail th n half.val := by
            rintro ⟨_, h2, _⟩
            apply hhd
            rw [bne_usize]
            simpa [hh] using h2
          exact hK c (by simp [this])
    · simp only [hx, ↓reduceIte, bind_ok]
      have : ¬ scanPred order head tail th n half.val := by
        rintro ⟨h1, _, _⟩
        apply hx
        simp only [ho, Option.map_some, Option.getD_some] at h1
        exact (IScalar.lt_equiv _ _).mpr h1
      exact hK c (by simp [this])

theorem scan_inner_PC (h : Is64) (order : Slice I32) (head tail : Slice Usize) (th : I32)
    (n wi : Usize) (w : U64) (hw : ∀ k, w.val.testBit k = true → 64 * wi.val + k < 2 ^ 63)
    (c0 : I32) :
    PC (circogen.blockpath.reference_scan_loop0_loop0 order head tail th n c0 wi w)
      (fun c => c.val = c0.val + ((Finset.range 64).filter (fun k => w.val.testBit k = true ∧
        scanPred order head tail th n (64 * wi.val + k))).card) := by
  unfold circogen.blockpath.reference_scan_loop0_loop0
  refine PC_loop (fun x => x.2.val)
    (fun x => (∀ k, x.2.val.testBit k = true → w.val.testBit k = true) ∧
      x.1.val = c0.val + ((Finset.range 64).filter (fun k => w.val.testBit k = true ∧
        x.2.val.testBit k = false ∧ scanPred order head tail th n (64 * wi.val + k))).card)
    _ _ ?_ (c0, w) ⟨fun k hk => hk, by
      rw [Finset.filter_false_of_mem (fun k _ hk => by rw [hk.1] at hk; simp at hk)]; simp⟩
  rintro ⟨c, bits⟩ ⟨hsub, hc⟩
  dsimp only at hsub hc ⊢
  unfold circogen.blockpath.reference_scan_loop0_loop0.body
  by_cases hb : (bits != 0#u64) = true
  · simp only [hb, ↓reduceIte]
    have hb0 : bits.val ≠ 0 := by
      intro h0; apply (bne_iff_ne.mp hb); exact (UScalar.eq_equiv _ _).mpr (by simpa using h0)
    obtain ⟨k, hk, hlow⟩ := lowest_bit bits.val hb0
    obtain ⟨hk64, z, hz, hzv⟩ := tz_spec bits k hk hlow
    have hwk := hsub k hk
    have hlt := hw k hwk
    obtain ⟨v, hv, hvv⟩ := shl_lit h wi 6 (by norm_num) 6#i32 (by rfl)
    have hvv' : v.val = 64 * wi.val := by rw [hvv, Nat.mod_eq_of_lt (by omega)]; ring
    simp only [hv, bind_ok, hz, lift_eq]
    have hcast : (UScalar.cast .Usize z).val = k := by
      show (z.bv.zeroExtend _).toNat = k
      rw [BitVec.zeroExtend_eq_setWidth, BitVec.toNat_setWidth]
      have : z.bv.toNat = z.val := rfl
      rw [this, hzv]
      exact Nat.mod_eq_of_lt (by
        have : UScalarTy.Usize.numBits = 64 := by simp [UScalarTy.numBits, h]
        rw [this]; omega)
    have hhalf : (v ||| UScalar.cast .Usize z).val = 64 * wi.val + k := by
      rw [UScalar.val_or, hvv', hcast]
      have := Nat.two_pow_add_eq_or_of_lt (i := 6) (b := k) (by norm_num; omega) wi.val
      simpa using this.symm
    refine scan_step_PC order head tail th n _ c _ _ ?_
    intro c' hc'
    apply PC_ok
    have hcl := fun j => (clear_lowest bits k hk hlow j).1
    have hdec := (clear_lowest bits k hk hlow 0).2
    dsimp only
    refine ⟨⟨fun j hj => ?_, ?_⟩, hdec⟩
    · rw [hcl] at hj
      simp only [Bool.and_eq_true] at hj
      exact hsub j hj.1
    · rw [hc', hc, hhalf, add_assoc]
      congr 1
      have hset : (Finset.range 64).filter (fun j => w.val.testBit j = true ∧
          (bits &&& core.num.U64.wrapping_sub bits 1#u64).val.testBit j = false ∧
          scanPred order head tail th n (64 * wi.val + j)) =
          if scanPred order head tail th n (64 * wi.val + k) then
            insert k ((Finset.range 64).filter (fun j => w.val.testBit j = true ∧
              bits.val.testBit j = false ∧ scanPred order head tail th n (64 * wi.val + j)))
          else (Finset.range 64).filter (fun j => w.val.testBit j = true ∧
              bits.val.testBit j = false ∧ scanPred order head tail th n (64 * wi.val + j)) := by
        ext j
        simp only [Finset.mem_filter, Finset.mem_range, hcl]
        by_cases hjk : j = k
        · subst hjk
          split_ifs with hp
          · simpa [hk, hk64, hwk] using hp
          · simpa [hk, hk64, hwk] using hp
        · split_ifs with hp <;> simp [hjk]
      rw [hset]
      have hknot : k ∉ (Finset.range 64).filter (fun j => w.val.testBit j = true ∧
          bits.val.testBit j = false ∧ scanPred order head tail th n (64 * wi.val + j)) := by
        simp [hk]
      split_ifs with hp
      · rw [Finset.card_insert_of_notMem hknot]; push_cast; ring
      · simp
  · simp only [hb, Bool.false_eq_true, ↓reduceIte]
    apply PC_ok
    have hb0 : bits = 0#u64 := (UScalar.eq_equiv _ _).mpr (by simpa using hb)
    dsimp only
    rw [hc]
    congr 3
    apply Finset.filter_congr
    intro j _
    have : bits.val.testBit j = false := by rw [hb0]; simp
    simp [this]

theorem enum_next_some (op : Slice U64) (i : ℕ) (hi : i < op.val.length) (cnt : Usize) :
    core.iter.adapters.enumerate.IteratorEnumerate.next
      (core.iter.traits.iterator.IteratorSliceIter U64) ⟨⟨op, i⟩, cnt⟩ =
      Std.bind (cnt + 1#usize) (fun c' => ok (some (cnt, op.val[i]), ⟨⟨op, i + 1⟩, c'⟩)) := by
  simp only [core.iter.adapters.enumerate.IteratorEnumerate.next,
    core.iter.traits.iterator.IteratorSliceIter, next_some _ _ hi, bind_ok, uncurry_apply_pair]

theorem enum_next_none (op : Slice U64) (i : ℕ) (hi : ¬ i < op.val.length) (cnt : Usize) :
    core.iter.adapters.enumerate.IteratorEnumerate.next
      (core.iter.traits.iterator.IteratorSliceIter U64) ⟨⟨op, i⟩, cnt⟩ = ok (none, ⟨⟨op, i⟩, cnt⟩) := by
  simp only [core.iter.adapters.enumerate.IteratorEnumerate.next,
    core.iter.traits.iterator.IteratorSliceIter, next_none _ _ hi, bind_ok, uncurry_apply_pair]

theorem scan_loop0_unfold (it : core.iter.adapters.enumerate.Enumerate (core.slice.iter.Iter U64))
    (order : Slice I32) (head tail : Slice Usize) (th : I32) (n : Usize) (c : I32) :
    circogen.blockpath.reference_scan_loop0 it order head tail th n c =
      Std.bind (circogen.blockpath.reference_scan_loop0.body order head tail th n it c) (fun r =>
        match r with
        | .cont (it', c') => circogen.blockpath.reference_scan_loop0 it' order head tail th n c'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.reference_scan_loop0; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b⟩⟩ | y <;> rfl

theorem usize_add_one {c z : Usize} (hz : c + 1#usize = ok z) : z.val = c.val + 1 := by
  have h := UScalar.add_equiv c 1#usize
  rw [hz] at h
  simp only [Result.match.ok] at h
  rw [h.2.1]; rfl

theorem card_word_split (op : Slice U64) (P : ℕ → Prop) [DecidablePred P] (i : ℕ)
    (hi : i < op.val.length) :
    ((bitsOf op).filter (fun h => h < 64 * (i + 1) ∧ P h)).card =
      ((bitsOf op).filter (fun h => h < 64 * i ∧ P h)).card +
      ((Finset.range 64).filter (fun k => op.val[i].val.testBit k = true ∧ P (64 * i + k))).card := by
  have hw : op.val[i / 1]? = some op.val[i] := by simp [List.getElem?_eq_getElem hi]
  have hmem : ∀ k, k < 64 → (64 * i + k ∈ bitsOf op ↔ op.val[i].val.testBit k = true) := by
    intro k hk
    rw [mem_bitsOf, bitAt_word op _ op.val[i] (by
      rw [show (64 * i + k) / 64 = i by omega]; exact List.getElem?_eq_getElem hi)]
    rw [show (64 * i + k) % 64 = k by omega]
  have hsplit : (bitsOf op).filter (fun h => h < 64 * (i + 1) ∧ P h) =
      (bitsOf op).filter (fun h => h < 64 * i ∧ P h) ∪
      ((Finset.range 64).filter (fun k => op.val[i].val.testBit k = true ∧ P (64 * i + k))).image
        (fun k => 64 * i + k) := by
    ext h
    simp only [Finset.mem_union, Finset.mem_filter, Finset.mem_image, Finset.mem_range]
    constructor
    · rintro ⟨hx, hlt, hp⟩
      by_cases hh : h < 64 * i
      · exact Or.inl ⟨hx, hh, hp⟩
      · right
        refine ⟨h - 64 * i, ⟨by omega, ?_, ?_⟩, by omega⟩
        · rw [← hmem _ (by omega), show 64 * i + (h - 64 * i) = h by omega]; exact hx
        · rw [show 64 * i + (h - 64 * i) = h by omega]; exact hp
    · rintro (⟨hx, hlt, hp⟩ | ⟨k, ⟨hk, hb, hp⟩, rfl⟩)
      · exact ⟨hx, by omega, hp⟩
      · exact ⟨(hmem k hk).mpr hb, by omega, hp⟩
  rw [hsplit, Finset.card_union_of_disjoint, Finset.card_image_of_injective _ (fun a b hab => by
    simpa using hab)]
  rw [Finset.disjoint_left]
  rintro h hx hy
  simp only [Finset.mem_filter, Finset.mem_image, Finset.mem_range] at hx hy
  obtain ⟨k, _, rfl⟩ := hy
  omega

theorem bitsOf_lt (op : Slice U64) (h : ℕ) (hh : h ∈ bitsOf op) : h < 64 * op.length := by
  unfold bitsOf at hh
  exact Finset.mem_range.mp (Finset.mem_filter.mp hh).1

theorem scan_outer_PC (h : Is64) (order : Slice I32) (head tail : Slice Usize) (th : I32) (n : Usize)
    (op : Slice U64) (hop : ∀ x ∈ bitsOf op, x < 2 ^ 63) (c0 : I32) :
    ∀ (i : ℕ) (cnt : Usize) (c : I32), i ≤ op.val.length → cnt.val = i →
      c.val = c0.val + ((bitsOf op).filter (fun x => x < 64 * i ∧
        scanPred order head tail th n x)).card →
      PC (circogen.blockpath.reference_scan_loop0 ⟨⟨op, i⟩, cnt⟩ order head tail th n c)
        (fun c' => c'.val = c0.val + ((bitsOf op).filter (scanPred order head tail th n)).card) := by
  intro i
  induction hk : op.val.length - i generalizing i with
  | zero =>
    intro cnt c hi hcnt hc
    have hge : ¬ i < op.val.length := by omega
    rw [scan_loop0_unfold]
    simp only [circogen.blockpath.reference_scan_loop0.body, enum_next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
    apply PC_ok
    rw [hc]
    congr 3
    apply Finset.filter_congr
    intro x hx
    have := bitsOf_lt op x hx
    have : op.length = op.val.length := rfl
    constructor
    · exact fun hh => hh.2
    · exact fun hh => ⟨by omega, hh⟩
  | succ k ih =>
    intro cnt c hi hcnt hc
    have hlt : i < op.val.length := by omega
    rw [scan_loop0_unfold]
    simp only [circogen.blockpath.reference_scan_loop0.body, enum_next_some _ _ hlt]
    intro y hy
    cases hz : cnt + 1#usize with
    | vis _ _ => rw [hz] at hy; simp [bind_vis] at hy
    | div => rw [hz] at hy; simp at hy
    | ret cnt' =>
      rw [hz] at hy
      simp only [bind_ok, uncurry_apply_pair] at hy
      have hcnt' := usize_add_one hz
      have hinner := scan_inner_PC h order head tail th n cnt op.val[i] (by
        intro k hk
        have hm : 64 * i + k ∈ bitsOf op := by
          rw [mem_bitsOf, bitAt_word op _ op.val[i] (by
            rw [show (64 * i + k) / 64 = i by
              have : k < 64 := by
                by_contra hc; rw [testBit_ge64 _ k (by omega)] at hk; cases hk
              omega]; exact List.getElem?_eq_getElem hlt)]
          have : k < 64 := by
            by_contra hc; rw [testBit_ge64 _ k (by omega)] at hk; cases hk
          rw [show (64 * i + k) % 64 = k by omega]; exact hk
        rw [hcnt]; exact hop _ hm) c
      cases hz2 : circogen.blockpath.reference_scan_loop0_loop0 order head tail th n c cnt op.val[i] with
      | vis _ _ => rw [hz2] at hy; simp [bind_vis] at hy
      | div => rw [hz2] at hy; simp at hy
      | ret c1 =>
        rw [hz2, bind_ok, bind_ok] at hy
        have hc1 := hinner c1 hz2
        refine ih (i + 1) (by omega) cnt' c1 (by omega) (by rw [hcnt', hcnt]) ?_ y hy
        rw [hc1, hc, card_word_split op _ i hlt, hcnt]
        push_cast
        ring

theorem scan_PC (h : Is64) (order : Slice I32) (head tail : Slice Usize) (op : Slice U64) (th : I32)
    (n : Usize) (c : I32) (hop : ∀ x ∈ bitsOf op, x < 2 ^ 63) :
    PC (circogen.blockpath.reference_scan order head tail op th n c)
      (fun c' => c'.val = c.val + ((bitsOf op).filter (scanPred order head tail th n)).card) := by
  unfold circogen.blockpath.reference_scan
  simp only [core.slice.Slice.iter, bind_ok, core.iter.traits.iterator.Iterator.enumerate.trait_default,
    core.iter.traits.iterator.Iterator.enumerate.default]
  exact scan_outer_PC h order head tail th n op hop c 0 0#usize c (by omega) rfl (by simp)

/-! ## The block graph read off the slices -/

/-- The adjacency slice of node `n`: `adjacency[offsets[n]]`. -/
def adjOf (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (n : ℕ) :
    List ℕ :=
  match offsets.val[n]? with
  | some r => ((adjacency.val.drop r.start.val).take (r.«end».val - r.start.val)).map (·.val)
  | none => []

/-- The block graph the counters read, from the `head` and `tail` slices. -/
def graphOf (head tail : Slice Usize) (offsets : Slice (core.ops.range.Range Usize))
    (adjacency : Slice Usize) : Graph where
  head e := (head.val[e]?.map (·.val)).getD 0
  tail e := (tail.val[e]?.map (·.val)).getD 0
  adj := adjOf offsets adjacency

/-- The scan's count is the model's `hits`. -/
theorem scan_hits (order0 order : Slice I32) (head tail : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (adj : List ℕ)
    (ord : ℕ → ℕ) (hor : ORel order0 adj ord order) (X : Finset ℕ) (hX : ∀ x ∈ X, x / 2 ∈ adj)
    (th : I32) (k : ℕ) (hth : th.val = (k : ℤ)) (n : Usize) :
    (X.filter (scanPred order head tail th n)).card =
      hits (graphOf head tail offsets adjacency) ord X k n.val := by
  unfold hits
  congr 1
  apply Finset.filter_congr
  intro x hx
  obtain ⟨y, hy, hyv⟩ := hor.inAdj _ (hX x hx)
  simp only [scanPred, hy, Option.map_some, Option.getD_some, hyv, hth, graphOf]
  constructor
  · rintro ⟨h1, h2, h3⟩; exact ⟨by exact_mod_cast h1, h2, h3⟩
  · rintro ⟨h1, h2, h3⟩; exact ⟨by exact_mod_cast h1, h2, h3⟩

/-! ## `reference_close`: the closed form of the closing pass -/

theorem hits_sdiff (g : Graph) (ord : ℕ → ℕ) (X Y : Finset ℕ) (th n : ℕ)
    (hY : ∀ x ∈ Y, g.head (x / 2) = n ∨ g.tail (x / 2) = n) :
    hits g ord (X \ Y) th n = hits g ord X th n := by
  unfold hits
  congr 1
  ext x
  simp only [Finset.mem_filter, Finset.mem_sdiff]
  constructor
  · rintro ⟨⟨h1, _⟩, h2⟩; exact ⟨h1, h2⟩
  · rintro ⟨h1, h2⟩
    refine ⟨⟨h1, fun hy => ?_⟩, h2⟩
    rcases hY x hy with hh | hh
    · exact h2.2.1 hh
    · exact h2.2.2 hh

theorem erased_append (g : Graph) (n : ℕ) (ord : ℕ → ℕ) (L : List ℕ) (e : ℕ) :
    erased g n ord (L ++ [e]) = if 0 < ord e then insert (halfOf g n e) (erased g n ord L)
      else erased g n ord L := by
  unfold erased
  by_cases h : 0 < ord e
  · ext x
    simp [h, List.filter_append, or_comm]
  · simp [h, List.filter_append]

theorem ref_close_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (head tail : Slice Usize) (op : Slice U64) (n : Usize) (c : I32) :
    circogen.blockpath.reference_close_loop it order head tail op n c =
      Std.bind (circogen.blockpath.reference_close_loop.body order head tail n it op c) (fun r =>
        match r with
        | .cont (it', o', c') => circogen.blockpath.reference_close_loop it' order head tail o' n c'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.reference_close_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c⟩⟩ | y <;> rfl

theorem index_usize_ok {α : Type} {s : Slice α} {i : Usize} {x : α} (h : Slice.index_usize s i = ok x) :
    s.val[i.val]? = some x := by
  unfold Slice.index_usize at h
  simp only [Slice.getElem?_Usize_eq] at h
  split at h
  · simp at h
  · next y hy => simp at h; subst h; exact hy

theorem update_ok {α : Type} {s s' : Slice α} {i : Usize} {x : α} (h : Slice.update s i x = ok s') :
    s'.val = s.val.set i.val x := by
  unfold Slice.update at h
  split at h
  · simp at h
  · simp at h; subst h; simp

/-- `(e << 1) | b`, the reference's form (`<<` on `usize`, checked shift amount). -/
theorem half_ref (h : Is64) (g : Graph) (n e t : Usize) (he : e.val < 2 ^ 62)
    (ht : g.tail e.val = t.val) (v : Usize) (hv : (e <<< 1#i32 : Result Usize) = ok v) :
    (v ||| core.convert.num.FromUsizeBool.from (t != n)).val = halfOf g n.val e.val := by
  obtain ⟨v', hv', hvv⟩ := shl_lit h e 1 (by norm_num) 1#i32 (by rfl)
  rw [hv] at hv'
  simp only [Result.ok.injEq] at hv'
  subst hv'
  have hvv' : v.val = 2 * e.val := by rw [hvv, Nat.mod_eq_of_lt (by omega)]; ring
  have hb : (core.convert.num.FromUsizeBool.from (t != n)).val = if t != n then 1 else 0 := by
    cases (t != n) <;> simp [core.convert.num.FromUsizeBool.from]
  rw [UScalar.val_or, hvv', hb, halfOf, ht]
  by_cases htn : t = n
  · subst htn; simp
  · have : t.val ≠ n.val := fun hh => htn (UScalar.eq_equiv t n |>.mpr hh)
    simp only [htn, bne_iff_ne, ne_eq, not_false_eq_true, ↓reduceIte, this, decide_true]
    have := Nat.two_pow_add_eq_or_of_lt (i := 1) (b := 1) (by norm_num) e.val
    simpa using this.symm

theorem PC_bind' {α β : Type} {x : Result α} {f : α → Result β} {Q : β → Prop}
    (h : ∀ a, x = ok a → PC (f a) Q) : PC (Std.bind x f) Q := by
  intro y hy
  cases hx : x with
  | ret a => rw [hx, bind_ok] at hy; exact h a hx y hy
  | vis i k => rw [hx] at hy; simp [bind_vis] at hy
  | div => rw [hx] at hy; simp at hy

theorem ref_close_step (h : Is64) (order0 order : Slice I32) (head tail : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (adj : List ℕ)
    (ord : ℕ → ℕ) (hor : ORel order0 adj ord order) (n : Usize) (edges : Slice Usize) (i : ℕ)
    (hlt : i < edges.val.length) (hadj : edges.val[i].val ∈ adj) (he62 : edges.val[i].val < 2 ^ 62)
    (op : Slice U64) (c : I32) (hop : ∀ x ∈ bitsOf op, x / 2 ∈ adj ∧ x < 2 ^ 63) :
    PC (circogen.blockpath.reference_close_loop.body order head tail n ⟨edges, i⟩ op c)
      (fun r => match r with
        | .cont (it', op', c') => it' = ⟨edges, i + 1⟩ ∧
            c'.val = c.val + (if 0 < ord edges.val[i].val then
              hits (graphOf head tail offsets adjacency) ord (bitsOf op) (ord edges.val[i].val) n.val
              else 0) ∧
            bitsOf op' = (if 0 < ord edges.val[i].val then
              (bitsOf op).erase (halfOf (graphOf head tail offsets adjacency) n.val edges.val[i].val)
              else bitsOf op) ∧ op'.length = op.length
        | .done _ => False) := by
  obtain ⟨e, he_def⟩ : ∃ e, edges.val[i] = e := ⟨_, rfl⟩
  rw [he_def] at hadj he62 ⊢
  obtain ⟨x, hx, hxv⟩ := hor.inAdj e.val hadj
  simp only [circogen.blockpath.reference_close_loop.body, next_some _ _ hlt, he_def, bind_ok,
    uncurry_apply_pair, lift_eq]
  refine PC_bind' (fun x' hx' => ?_)
  have := index_usize_ok hx'
  rw [hx] at this
  simp only [Option.some.injEq] at this
  subst this
  by_cases h0 : 0 < ord e.val
  · have hx0 : x > 0#i32 := (i32_pos_iff x _ hxv).mpr h0
    simp only [hx0, ↓reduceIte, h0]
    refine PC_bind' (fun c1 hc1 => ?_)
    have hc1v := scan_PC h order head tail op x n c (fun y hy => (hop y hy).2) c1 hc1
    rw [scan_hits order0 order head tail offsets adjacency adj ord hor (bitsOf op)
      (fun y hy => (hop y hy).1) x (ord e.val) hxv n] at hc1v
    refine PC_bind' (fun v hv => ?_)
    refine PC_bind' (fun t ht => ?_)
    have htv := index_usize_ok ht
    have htg : (graphOf head tail offsets adjacency).tail e.val = t.val := by
      simp [graphOf, htv]
    have hhalf := half_ref h (graphOf head tail offsets adjacency) n e t he62 htg v hv
    refine PC_bind' (fun i3 hi3 => ?_)
    obtain ⟨q3, hq3, hq3v⟩ := mod64 (v ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hq3] at hi3; simp only [Result.ok.injEq] at hi3; subst hi3
    refine PC_bind' (fun i7 hi7 => ?_)
    obtain ⟨q7, hq7, hq7v⟩ := div64 (v ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hq7] at hi7; simp only [Result.ok.injEq] at hi7; subst hi7
    rw [hhalf] at hq3v hq7v
    refine PC_bind' (fun w hw => ?_)
    have hwv := index_usize_ok hw
    refine PC_bind' (fun s hs => ?_)
    have hsv := update_ok hs
    apply PC_ok
    refine ⟨rfl, hc1v, ?_, ?_⟩
    · have hq3l : q3.val < 64 := by rw [hq3v]; omega
      exact bitsOf_clear_bit op s _ w (core.num.U64.wrapping_shl 1#u64 (UScalar.cast .U32 q3))
        (by rw [← hq7v]; exact hwv) (by rw [wmask_val _ hq3l, hq3v]) (by rw [hsv, hq7v])
    · show s.val.length = op.val.length
      rw [hsv, List.length_set]
  · have hx0 : ¬ x > 0#i32 := fun hh => h0 ((i32_pos_iff x _ hxv).mp hh)
    simp only [hx0, ↓reduceIte, h0]
    apply PC_ok
    exact ⟨rfl, by simp, rfl, rfl⟩

theorem ref_close_PC (h : Is64) (order0 order : Slice I32) (head tail : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (adj : List ℕ)
    (ord : ℕ → ℕ) (hor : ORel order0 adj ord order) (n : Usize) (edges : Slice Usize)
    (hinc : ∀ e ∈ edges.val, e.val ∈ adj ∧ e.val < 2 ^ 62 ∧
      ((graphOf head tail offsets adjacency).head e.val = n.val ∨
        (graphOf head tail offsets adjacency).tail e.val = n.val))
    (X0 : Finset ℕ) (hX0 : ∀ x ∈ X0, x / 2 ∈ adj ∧ x < 2 ^ 63) :
    ∀ (i : ℕ) (op : Slice U64) (c : I32), i ≤ edges.val.length →
      bitsOf op = X0 \ erased (graphOf head tail offsets adjacency) n.val ord
        ((edges.val.map (·.val)).take i) →
      PC (circogen.blockpath.reference_close_loop ⟨edges, i⟩ order head tail op n c)
        (fun res => res.1.val = c.val + qsum ord
            (fun e => hits (graphOf head tail offsets adjacency) ord X0 (ord e) n.val)
            ((edges.val.map (·.val)).drop i) ∧
          bitsOf res.2 = X0 \ erased (graphOf head tail offsets adjacency) n.val ord
            (edges.val.map (·.val)) ∧ res.2.length = op.length) := by
  set g := graphOf head tail offsets adjacency with hg
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro op c hi hb
    have hge : ¬ i < edges.val.length := by omega
    rw [ref_close_loop_unfold]
    simp only [circogen.blockpath.reference_close_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
    apply PC_ok
    refine ⟨?_, ?_, rfl⟩
    · rw [List.drop_eq_nil_of_le (by simp; omega)]; simp [qsum]
    · rw [hb, List.take_of_length_le (by simp; omega)]
  | succ k ih =>
    intro op c hi hb
    have hlt : i < edges.val.length := by omega
    have hmem := hinc _ (List.getElem_mem hlt)
    have hop : ∀ x ∈ bitsOf op, x / 2 ∈ adj ∧ x < 2 ^ 63 := by
      intro x hx; rw [hb] at hx; exact hX0 x (Finset.mem_sdiff.mp hx).1
    rw [ref_close_loop_unfold]
    refine PC_bind (ref_close_step h order0 order head tail offsets adjacency adj ord hor n edges i
      hlt hmem.1 hmem.2.1 op c hop) ?_
    rintro (⟨it', op', c'⟩ | _) hr
    · obtain ⟨rfl, hc', hb', hl'⟩ := hr
      have hdrop : (edges.val.map (·.val)).drop i =
          edges.val[i].val :: (edges.val.map (·.val)).drop (i + 1) := by
        rw [List.drop_eq_getElem_cons (by simpa using hlt)]; simp
      have htake : (edges.val.map (·.val)).take (i + 1) =
          (edges.val.map (·.val)).take i ++ [edges.val[i].val] := by
        rw [take_succ' _ _ (by simpa using hlt)]; simp
      -- the erased halves are incident, so the scan saw the original count
      have hinc' : ∀ x ∈ erased g n.val ord ((edges.val.map (·.val)).take i),
          g.head (x / 2) = n.val ∨ g.tail (x / 2) = n.val := by
        intro x hx
        unfold erased at hx
        simp only [List.mem_toFinset, List.mem_map, List.mem_filter] at hx
        obtain ⟨e', ⟨he', _⟩, rfl⟩ := hx
        have : halfOf g n.val e' / 2 = e' := by unfold halfOf; split_ifs <;> omega
        rw [this]
        obtain ⟨e'', he'', rfl⟩ := List.mem_map.mp (List.mem_of_mem_take he')
        exact (hinc e'' he'').2.2
      have hhits : hits g ord (bitsOf op) (ord edges.val[i].val) n.val =
          hits g ord X0 (ord edges.val[i].val) n.val := by
        rw [hb]; exact hits_sdiff g ord X0 _ _ n.val hinc'
      dsimp only
      refine PC_mono (ih (i + 1) (by omega) op' c' (by omega) ?_) ?_
      · rw [hb', htake, erased_append, hb]
        split_ifs with h0
        · ext x; simp only [Finset.mem_erase, Finset.mem_sdiff, Finset.mem_insert]; tauto
        · rfl
      · rintro ⟨c'', op''⟩ ⟨h1, h2, h3⟩
        refine ⟨?_, h2, by rw [h3, hl']⟩
        dsimp only at h1 ⊢
        rw [h1, hc', hdrop, qsum_cons, ← hhits]
        push_cast
        ring
    · exact absurd hr id

/-! ## `reference_open`: the opening pass is the model's `refOpen` fold -/

/-- The reference's concrete state relates to a model `RefState` (orders and set halves). -/
structure RRel (order0 : Slice I32) (adj : List ℕ) (W : ℕ) (order : Slice I32) (op : Slice U64)
    (r : RefState) : Prop where
  ord : ORel order0 adj r.ord order
  bits : bitsOf op = r.bits
  len : op.length = W

theorem ORel_set (order0 : Slice I32) (adj : List ℕ) (ord : ℕ → ℕ) (order order' : Slice I32)
    (hor : ORel order0 adj ord order) (e : ℕ) (he : e ∈ adj) (occ : I32) (k : ℕ)
    (hocc : occ.val = (k : ℤ)) (hlt : e < order.val.length) (hv : order'.val = order.val.set e occ) :
    ORel order0 adj (Function.update ord e k) order' := by
  refine ⟨fun e' he' => ?_, fun e' he' => ?_⟩
  · rw [hv, List.getElem?_set]
    by_cases hee : e = e'
    · subst hee; simp [hlt, hocc]
    · simp only [hee, ↓reduceIte, Function.update_of_ne (Ne.symm hee)]
      exact hor.inAdj e' he'
  · rw [hv, List.getElem?_set]
    have hee : e ≠ e' := fun hh => he' (hh ▸ he)
    simp only [hee, ↓reduceIte]
    exact hor.outAdj e' he'

theorem index_mut_ok {α : Type} {s : Slice α} {i : Usize} {p : α × (α → Slice α)}
    (h : Slice.index_mut_usize s i = ok p) : s.val[i.val]? = some p.1 ∧ p.2 = Slice.set s i := by
  unfold Slice.index_mut_usize at h
  cases hx : Slice.index_usize s i with
  | ret x =>
    rw [hx, bind_tc_ok] at h
    simp only [Result.ok.injEq] at h
    subst h
    exact ⟨index_usize_ok hx, rfl⟩
  | vis _ _ => rw [hx] at h; simp [bind_vis] at h
  | div => rw [hx] at h; simp at h

theorem slice_set_val {α : Type} (s : Slice α) (i : Usize) (x : α) :
    (Slice.set s i x).val = s.val.set i.val x := by
  simp [Slice.set, Slice.setAtNat]

theorem ref_open_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32)
    (tail : Slice Usize) (op : Slice U64) (n : Usize) (occ : I32) :
    circogen.blockpath.reference_open_loop it order tail op n occ =
      Std.bind (circogen.blockpath.reference_open_loop.body tail n occ it order op) (fun r =>
        match r with
        | .cont (it', o', op') => circogen.blockpath.reference_open_loop it' o' tail op' n occ
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.reference_open_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c⟩⟩ | y <;> rfl

theorem ref_open_step (h : Is64) (order0 : Slice I32) (adj : List ℕ) (W : ℕ) (order : Slice I32)
    (head tail : Slice Usize) (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize)
    (n : Usize) (occ : I32) (k : ℕ) (hocc : occ.val = (k : ℤ)) (edges : Slice Usize) (i : ℕ)
    (hlt : i < edges.val.length) (hadj : edges.val[i].val ∈ adj) (he62 : edges.val[i].val < 2 ^ 62)
    (op : Slice U64) (r : RefState) (hr : RRel order0 adj W order op r) :
    PC (circogen.blockpath.reference_open_loop.body tail n occ ⟨edges, i⟩ order op)
      (fun res => match res with
        | .cont (it', o', op') => it' = ⟨edges, i + 1⟩ ∧
            RRel order0 adj W o' op' (refOpen (graphOf head tail offsets adjacency) n.val k r
              edges.val[i].val)
        | .done _ => False) := by
  obtain ⟨e, he_def⟩ : ∃ e, edges.val[i] = e := ⟨_, rfl⟩
  rw [he_def] at hadj he62 ⊢
  obtain ⟨x, hx, hxv⟩ := hr.ord.inAdj e.val hadj
  have hxl : e.val < order.val.length := (List.getElem?_eq_some_iff.mp hx).1
  simp only [circogen.blockpath.reference_open_loop.body, next_some _ _ hlt, he_def, bind_ok,
    uncurry_apply_pair, lift_eq]
  refine PC_bind' (fun p hp => ?_)
  obtain ⟨hp1, hp2⟩ := index_mut_ok hp
  rw [hx] at hp1
  simp only [Option.some.injEq] at hp1
  obtain ⟨x', back⟩ := p
  dsimp only at hp1 hp2 ⊢
  subst hp1 hp2
  by_cases h0 : r.ord e.val = 0
  · have hx0 : x = 0#i32 := by rw [IScalar.eq_equiv, hxv, h0]; simp
    simp only [hx0, ↓reduceIte]
    refine PC_bind' (fun v hv => ?_)
    refine PC_bind' (fun t ht => ?_)
    have htv := index_usize_ok ht
    have htg : (graphOf head tail offsets adjacency).tail e.val = t.val := by
      simp [graphOf, htv]
    have hhalf := half_ref h (graphOf head tail offsets adjacency) n e t he62 htg v hv
    refine PC_bind' (fun i3 hi3 => ?_)
    obtain ⟨q3, hq3, hq3v⟩ := mod64 (v ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hq3] at hi3; simp only [Result.ok.injEq] at hi3; subst hi3
    refine PC_bind' (fun i6 hi6 => ?_)
    obtain ⟨q6, hq6, hq6v⟩ := div64 (v ||| core.convert.num.FromUsizeBool.from (t != n))
    rw [hq6] at hi6; simp only [Result.ok.injEq] at hi6; subst hi6
    rw [hhalf] at hq3v hq6v
    refine PC_bind' (fun w hw => ?_)
    have hwv := index_usize_ok hw
    refine PC_bind' (fun s hs => ?_)
    have hsv := update_ok hs
    apply PC_ok
    have hq3l : q3.val < 64 := by rw [hq3v]; omega
    refine ⟨rfl, ?_⟩
    simp only [refOpen, h0, ↓reduceIte]
    refine ⟨ORel_set order0 adj r.ord order _ hr.ord e.val hadj occ k hocc hxl (slice_set_val _ _ _),
      ?_, ?_⟩
    · show bitsOf s = insert _ r.bits
      rw [← hr.bits]
      exact bitsOf_set_bit op s _ w (core.num.U64.wrapping_shl 1#u64 (UScalar.cast .U32 q3))
        (by rw [← hq6v]; exact hwv) (by rw [wmask_val _ hq3l, hq3v]) (by rw [hsv, hq6v])
    · show s.val.length = W
      rw [hsv, List.length_set]; exact hr.len
  · have hx0 : ¬ x = 0#i32 := by rw [IScalar.eq_equiv, hxv]; simpa using h0
    simp only [uncurry_apply_pair, hx0, ↓reduceIte]
    apply PC_ok
    refine ⟨rfl, ?_⟩
    simp only [refOpen, h0, ↓reduceIte]
    have hset : Slice.set order e x = order := by
      apply Slice.ext
      rw [slice_set_val]
      apply List.ext_getElem?
      intro j
      rw [List.getElem?_set]
      by_cases hj : e.val = j
      · subst hj
        simp only [hxl, ↓reduceIte, hx, Option.some.injEq]
      · simp [hj]
    rw [hset]
    exact hr

theorem ref_open_PC (h : Is64) (order0 : Slice I32) (adj : List ℕ) (W : ℕ)
    (head tail : Slice Usize) (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize)
    (n : Usize) (occ : I32) (k : ℕ) (hocc : occ.val = (k : ℤ)) (edges : Slice Usize)
    (hinc : ∀ e ∈ edges.val, e.val ∈ adj ∧ e.val < 2 ^ 62) :
    ∀ (i : ℕ) (order : Slice I32) (op : Slice U64) (r : RefState), i ≤ edges.val.length →
      RRel order0 adj W order op r →
      PC (circogen.blockpath.reference_open_loop ⟨edges, i⟩ order tail op n occ)
        (fun res => RRel order0 adj W res.1 res.2
          (((edges.val.map (·.val)).drop i).foldl (refOpen (graphOf head tail offsets adjacency) n.val k) r)) := by
  intro i
  induction hk : edges.val.length - i generalizing i with
  | zero =>
    intro order op r hi hr
    have hge : ¬ i < edges.val.length := by omega
    rw [ref_open_loop_unfold]
    simp only [circogen.blockpath.reference_open_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
    apply PC_ok
    rw [List.drop_eq_nil_of_le (by simp; omega)]
    exact hr
  | succ k' ih =>
    intro order op r hi hr
    have hlt : i < edges.val.length := by omega
    have hmem := hinc _ (List.getElem_mem hlt)
    rw [ref_open_loop_unfold]
    refine PC_bind (ref_open_step h order0 adj W order head tail offsets adjacency n occ k hocc edges i
      hlt hmem.1 hmem.2 op r hr) ?_
    rintro (⟨it', o', op'⟩ | _) hres
    · obtain ⟨rfl, hr'⟩ := hres
      have hdrop : (edges.val.map (·.val)).drop i =
          edges.val[i].val :: (edges.val.map (·.val)).drop (i + 1) := by
        rw [List.drop_eq_getElem_cons (by simpa using hlt)]; simp
      rw [hdrop, List.foldl_cons]
      exact ih (i + 1) (by omega) o' op' _ (by omega) hr'
    · exact absurd hres id

/-! ## Prologue helpers: `fill`, `clone`, range `get`/`index`, the occurrence counter -/

theorem fill_zero (s : Slice U64) :
    ∃ s' : Slice U64, core.slice.Slice.fill core.clone.CloneU64 s 0#u64 = ok s' ∧
      s'.length = s.length ∧ s'.val = List.replicate s.length 0#u64 := by
  obtain ⟨s', hs, h1, h2⟩ := spec_ex (core.slice.Slice.fill.spec core.clone.CloneU64 s 0#u64
    (by simp [core.clone.CloneU64]))
  exact ⟨s', hs, h1, h2⟩

theorem bitsOf_replicate (s : Slice U64) (hs : s.val = List.replicate s.length 0#u64) :
    bitsOf s = ∅ := by
  ext h
  rw [mem_bitsOf]
  simp only [Finset.notMem_empty, iff_false]
  unfold bitAt
  rw [hs, List.getElem?_replicate]
  split_ifs <;> simp

theorem tz_replicate (s : Slice U64) (hs : s.val = List.replicate s.length 0#u64) (j : ℕ) :
    tz s j = 0 := by
  unfold tz
  rw [hs, List.getElem?_replicate]
  split_ifs <;> simp

theorem range_clone (r : core.ops.range.Range Usize) :
    core.ops.range.Range.Insts.CoreCloneClone.clone core.clone.CloneUsize r = ok r := by
  simp [core.ops.range.Range.Insts.CoreCloneClone.clone, core.clone.CloneUsize]

theorem get_range (s : Slice Usize) (r : core.ops.range.Range Usize) (h1 : r.start.val ≤ r.«end».val)
    (h2 : r.«end».val ≤ s.val.length) :
    ∃ sl : Slice Usize, core.slice.Slice.get (core.slice.index.SliceIndexRangeUsizeSlice Usize) s r =
      ok (some sl) ∧ sl.val = s.val.slice r.start.val r.«end».val := by
  have hlen : (s.val.slice r.start.val r.«end».val).length ≤ Usize.max := by
    have := s.val.slice_length_le r.start.val r.«end».val
    have := s.property
    omega
  refine ⟨Slice.from _ hlen, ?_, Slice.from_val _ _⟩
  simp only [core.slice.Slice.get, core.slice.index.SliceIndexRangeUsizeSlice,
    core.slice.index.SliceIndexRangeUsizeSlice.get]
  rw [ite_eq_left ⟨by simpa [UScalar.le_equiv] using h1, by simpa using h2⟩]

theorem index_range_ok (s : Slice Usize) (r : core.ops.range.Range Usize) (sl : Slice Usize)
    (h : core.slice.index.Slice.index (core.slice.index.SliceIndexRangeUsizeSlice Usize) s r = ok sl) :
    sl.val = s.val.slice r.start.val r.«end».val := by
  simp only [core.slice.index.Slice.index, core.slice.index.SliceIndexRangeUsizeSlice,
    core.slice.index.SliceIndexRangeUsizeSlice.index] at h
  split_ifs at h
  · simp only [Result.ok.injEq] at h; subst h; simp
  · simp at h

theorem slice_map_adjOf (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize)
    (n : ℕ) (r : core.ops.range.Range Usize) (hr : offsets.val[n]? = some r) (sl : Slice Usize)
    (hsl : sl.val = adjacency.val.slice r.start.val r.«end».val) :
    sl.val.map (·.val) = adjOf offsets adjacency n := by
  unfold adjOf
  rw [hr, hsl]
  rfl

theorem occ_succ (occ : I32) (i : ℕ) (hocc : occ.val = (i : ℤ)) (hi : i + 1 ≤ 2147483647) :
    (core.num.I32.wrapping_add occ 1#i32).val = ((i + 1 : ℕ) : ℤ) := by
  rw [core.num.I32.wrapping_add_val_eq, hocc]
  rw [show ((1#i32 : I32).val) = 1 from rfl]
  rw [bmod_small _ (by omega) (by omega)]
  push_cast; ring

/-! ## The reference's node loop -/

theorem ref_loop1_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32) (head tail : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (op : Slice U64)
    (c occ : I32) :
    circogen.blockpath.count_all_crossings_reference_loop1 it order head tail offsets adjacency op c occ =
      Std.bind (circogen.blockpath.count_all_crossings_reference_loop1.body head tail offsets adjacency
        it order op c occ) (fun r =>
        match r with
        | .cont (it', o', op', c', occ') =>
          circogen.blockpath.count_all_crossings_reference_loop1 it' o' head tail offsets adjacency op' c' occ'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.count_all_crossings_reference_loop1; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c, d, e⟩⟩ | y <;> rfl

theorem erased_nil (g : Graph) (n : ℕ) (ord : ℕ → ℕ) : erased g n ord [] = ∅ := by
  simp [erased]

theorem halfOf_div (g : Graph) (n e : ℕ) : halfOf g n e / 2 = e := by
  unfold halfOf; split_ifs <;> omega

/-- One node of the reference scan: `crossings` grows by the model's `occHits`, and the orders and
set halves become those of `refOccurrence`. -/
theorem ref_occ_PC (h : Is64) (order0 : Slice I32) (head tail list : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (E N W : ℕ)
    (hE62 : E < 2 ^ 62) (hadjE : ∀ x ∈ adjacency.val, x.val < E)
    (i : ℕ) (hlt : i < list.val.length) (hiL : i + 1 ≤ 2147483647)
    (hwf : NodeWf (graphOf head tail offsets adjacency) (adjacency.val.map (·.val)) list.val[i].val)
    (s : IdxState) (hI : Inv (adjacency.val.map (·.val)) E N i s)
    (order : Slice I32) (op : Slice U64) (c occ : I32) (hocc : occ.val = (i : ℤ))
    (hr : RRel order0 (adjacency.val.map (·.val)) W order op s.toRefState) :
    PC (circogen.blockpath.count_all_crossings_reference_loop1.body head tail offsets adjacency
        ⟨list, i⟩ order op c occ)
      (fun res => match res with
        | .cont (it', o', op', c', occ') => it' = ⟨list, i + 1⟩ ∧ occ'.val = ((i + 1 : ℕ) : ℤ) ∧
            c'.val = c.val + occHits (graphOf head tail offsets adjacency) list.val[i].val s.toRefState ∧
            RRel order0 (adjacency.val.map (·.val)) W o' op'
              (refOccurrence (graphOf head tail offsets adjacency) list.val[i].val (i + 1) s.toRefState)
        | .done _ => False) := by
  set g := graphOf head tail offsets adjacency with hg
  set adjL := adjacency.val.map (·.val) with hadjL
  obtain ⟨n, hn_def⟩ : ∃ n, list.val[i] = n := ⟨_, rfl⟩
  rw [hn_def] at hwf ⊢
  simp only [circogen.blockpath.count_all_crossings_reference_loop1.body, next_some _ _ hlt, hn_def,
    bind_ok, uncurry_apply_pair, lift_eq]
  refine PC_bind' (fun rg hrg => ?_)
  have hrgv := index_usize_ok hrg
  rw [range_clone, bind_ok]
  refine PC_bind' (fun edges hedges => ?_)
  have hev := index_range_ok adjacency rg edges hedges
  have hes : edges.val.map (·.val) = g.adj n.val := by
    rw [hg]; exact slice_map_adjOf offsets adjacency n.val rg hrgv edges hev
  have hsub_adj : ∀ e ∈ edges.val, e.val ∈ adjL := by
    intro e he
    exact hwf.sub e.val (by rw [← hes]; exact List.mem_map_of_mem he)
  have hinc : ∀ e ∈ edges.val, e.val ∈ adjL ∧ e.val < 2 ^ 62 ∧
      (g.head e.val = n.val ∨ g.tail e.val = n.val) := by
    intro e he
    have ha := hsub_adj e he
    refine ⟨ha, ?_, (hwf.incident e.val ha).mp (by rw [← hes]; exact List.mem_map_of_mem he)⟩
    obtain ⟨x, hx, hxe⟩ := List.mem_map.mp ha
    have := hadjE x hx
    omega
  have hX0 : ∀ x ∈ s.bits, x / 2 ∈ adjL ∧ x < 2 ^ 63 := fun x hx =>
    ⟨hI.mem x hx, by have := hI.lt x hx; omega⟩
  unfold circogen.blockpath.reference_close
  rw [into_iter_eq, bind_ok]
  refine PC_bind (ref_close_PC h order0 order head tail offsets adjacency adjL s.ord hr.ord n edges
    hinc s.bits hX0 0 op c (by omega) (by rw [hr.bits]; simp [erased_nil])) ?_
  rintro ⟨c1, op1⟩ ⟨hc1, hb1, hl1⟩
  dsimp only at hc1 hb1 hl1
  unfold circogen.blockpath.reference_open
  simp only [uncurry_apply_pair, into_iter_eq, bind_ok]
  have hocc1 := occ_succ occ i hocc hiL
  set r1 : RefState := { s.toRefState with
    crossings := queryAcc g n.val s.ord s.bits (g.adj n.val) s.crossings
    bits := s.bits \ erased g n.val s.ord (g.adj n.val) } with hr1
  have hrr1 : RRel order0 adjL W order op1 r1 :=
    ⟨hr.ord, by rw [hb1, hes], by rw [hl1]; exact hr.len⟩
  refine PC_bind (ref_open_PC h order0 adjL W head tail offsets adjacency n _ (i + 1) hocc1 edges
    (fun e he => ⟨(hinc e he).1, (hinc e he).2.1⟩) 0 order op1 r1 (by omega) hrr1) ?_
  rintro ⟨o2, op2⟩ hr2
  apply PC_ok
  refine ⟨rfl, hocc1, ?_, ?_⟩
  · rw [hc1, List.drop_zero, hes]
    rfl
  · dsimp only at hr2
    rw [List.drop_zero, hes] at hr2
    unfold refOccurrence
    rw [refClose_fold g n.val (g.adj n.val) s.toRefState (fun e he =>
      (hwf.incident e (hwf.sub e he)).mp he)]
    exact hr2

theorem ref_run_PC (h : Is64) (order0 : Slice I32) (head tail list : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (E N W : ℕ)
    (hN : N < 2 ^ 62) (hLN : list.val.length < N) (hL31 : list.val.length ≤ 2147483647)
    (hE62 : E < 2 ^ 62) (hadjE : ∀ x ∈ adjacency.val, x.val < E)
    (hwf : ∀ n ∈ list.val, NodeWf (graphOf head tail offsets adjacency) (adjacency.val.map (·.val)) n.val) :
    ∀ (i : ℕ) (order : Slice I32) (op : Slice U64) (c occ : I32) (s : IdxState),
      i ≤ list.val.length → Inv (adjacency.val.map (·.val)) E N i s → occ.val = (i : ℤ) →
      RRel order0 (adjacency.val.map (·.val)) W order op s.toRefState →
      PC (circogen.blockpath.count_all_crossings_reference_loop1 ⟨list, i⟩ order head tail offsets
          adjacency op c occ)
        (fun res => res.1.val = c.val + natCrossings (graphOf head tail offsets adjacency) (i + 1)
            ((list.val.map (·.val)).drop i) s.toRefState ∧
          RRel order0 (adjacency.val.map (·.val)) W res.2.1 res.2.2
            (refRun (graphOf head tail offsets adjacency) (i + 1) ((list.val.map (·.val)).drop i)
              s.toRefState)) := by
  have hE : ∀ e ∈ adjacency.val.map (·.val), e < E := by
    intro e he; obtain ⟨x, hx, rfl⟩ := List.mem_map.mp he; exact hadjE x hx
  intro i
  induction hk : list.val.length - i generalizing i with
  | zero =>
    intro order op c occ s hi hI hocc hr
    have hge : ¬ i < list.val.length := by omega
    rw [ref_loop1_unfold]
    simp only [circogen.blockpath.count_all_crossings_reference_loop1.body, next_none _ _ hge,
      bind_ok, uncurry_apply_pair]
    apply PC_ok
    rw [List.drop_eq_nil_of_le (by simp; omega)]
    exact ⟨by simp [natCrossings], hr⟩
  | succ k ih =>
    intro order op c occ s hi hI hocc hr
    have hlt : i < list.val.length := by omega
    have hw := hwf _ (List.getElem_mem hlt)
    obtain ⟨s', _, hc', hI'⟩ := idxOccurrence_step hN hLN (k := i) (by omega) hE62
      (graphOf head tail offsets adjacency) list.val[i].val hE hw.nodup hw.incident hw.sub hI
    rw [ref_loop1_unfold]
    refine PC_bind (ref_occ_PC h order0 head tail list offsets adjacency E N W hE62 hadjE i hlt
      (by omega) hw s hI order op c occ hocc hr) ?_
    rintro (⟨it', o', op', c', occ'⟩ | _) hres
    · obtain ⟨rfl, hocc', hc1, hr'⟩ := hres
      have hdrop : (list.val.map (·.val)).drop i =
          list.val[i].val :: (list.val.map (·.val)).drop (i + 1) := by
        rw [List.drop_eq_getElem_cons (by simpa using hlt)]; simp
      rw [← hc'] at hr'
      refine PC_mono (ih (i + 1) (by omega) o' op' c' occ' s' (by omega) hI' hocc' hr') ?_
      rintro ⟨cf, of, opf⟩ ⟨h1, h2⟩
      dsimp only at h1 h2 ⊢
      rw [hdrop, natCrossings, refRun, ← hc']
      refine ⟨?_, h2⟩
      rw [h1, hc1]
      push_cast
      ring
    · exact absurd hres id

/-! ## The indexed counter's node loop -/

/-- The indexed counter's concrete state relates to a model `IdxState`. -/
structure IRel (order0 : Slice I32) (adj : List ℕ) (W N : ℕ) (order : Slice I32) (op tree : Slice U64)
    (s : IdxState) : Prop where
  ord : ORel order0 adj s.ord order
  len : op.length = W
  bits : bitsOf op = s.bits
  tree : TRel N tree s.tree

theorem idx_loop_unfold (it : core.slice.iter.Iter Usize) (order : Slice I32) (head tail list : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (op tree : Slice U64)
    (c occ : I32) :
    circogen.blockpath.count_indexed_loop 0#u8 it order head tail list offsets adjacency op tree c occ =
      Std.bind (circogen.blockpath.count_indexed_loop.body 0#u8 head tail list offsets adjacency
        it order op tree c occ) (fun r =>
        match r with
        | .cont (it', o', op', t', c', occ') =>
          circogen.blockpath.count_indexed_loop 0#u8 it' o' head tail list offsets adjacency op' t' c' occ'
        | .done y => ok y) := by
  conv_lhs => unfold circogen.blockpath.count_indexed_loop; rw [loop]
  congr 1
  funext r
  rcases r with ⟨⟨a, b, c, d, e, f⟩⟩ | y <;> rfl

theorem idx_occ (h : Is64) (order0 : Slice I32) (head tail list : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (E N W : ℕ)
    (hN : N < 2 ^ 62) (hLN : list.val.length < N) (hE62 : E < 2 ^ 62)
    (hadjE : ∀ x ∈ adjacency.val, x.val < E) (hW : 2 * E ≤ 64 * W) (htailE : tail.val.length = E)
    (i : ℕ) (hlt : i < list.val.length) (hiL : i + 1 ≤ 2147483647)
    (hwf : NodeWf (graphOf head tail offsets adjacency) (adjacency.val.map (·.val)) list.val[i].val)
    (hoff : ∃ r, offsets.val[list.val[i].val]? = some r ∧ r.start.val ≤ r.«end».val ∧
      r.«end».val ≤ adjacency.val.length)
    (s : IdxState) (hI : Inv (adjacency.val.map (·.val)) E N i s)
    (order : Slice I32) (op tree : Slice U64) (c occ : I32) (hocc : occ.val = (i : ℤ))
    (hc0 : 0 ≤ c.val)
    (hcb : c.val + occHits (graphOf head tail offsets adjacency) list.val[i].val s.toRefState ≤ 2147483647)
    (hrel : IRel order0 (adjacency.val.map (·.val)) W N order op tree s) :
    ∃ (s' : IdxState) (order' : Slice I32) (op' tree' : Slice U64) (c' occ' : I32),
      circogen.blockpath.count_indexed_loop.body 0#u8 head tail list offsets adjacency ⟨list, i⟩
        order op tree c occ = ok (cont (⟨list, i + 1⟩, order', op', tree', c', occ')) ∧
      occ'.val = ((i + 1 : ℕ) : ℤ) ∧
      c'.val = c.val + occHits (graphOf head tail offsets adjacency) list.val[i].val s.toRefState ∧
      IRel order0 (adjacency.val.map (·.val)) W N order' op' tree' s' ∧
      s'.toRefState = refOccurrence (graphOf head tail offsets adjacency) list.val[i].val (i + 1)
        s.toRefState ∧
      Inv (adjacency.val.map (·.val)) E N (i + 1) s' := by
  set g := graphOf head tail offsets adjacency with hg
  set adjL := adjacency.val.map (·.val) with hadjL
  set L := list.val.length with hLdef
  obtain ⟨n, hn_def⟩ : ∃ n, list.val[i] = n := ⟨_, rfl⟩
  rw [hn_def] at hwf hoff hcb ⊢
  have hE : ∀ e ∈ adjL, e < E := by
    intro e he; obtain ⟨x, hx, rfl⟩ := List.mem_map.mp he; exact hadjE x hx
  -- the model step and its stages
  obtain ⟨s', hocc_m, hc', hI'⟩ := idxOccurrence_step hN hLN (k := i) (by omega) hE62 g n.val hE
    hwf.nodup hwf.incident hwf.sub hI
  set es := g.adj n.val with hes_def
  have hocc_m' := hocc_m
  unfold idxOccurrence at hocc_m'
  obtain ⟨sA, hA, hrest⟩ := Option.bind_eq_some_iff.mp hocc_m'
  obtain ⟨total, htot, hrest⟩ := Option.bind_eq_some_iff.mp hrest
  obtain ⟨sB, hB, hrest⟩ := Option.bind_eq_some_iff.mp hrest
  obtain ⟨sC, hC, hD⟩ := Option.bind_eq_some_iff.mp hrest
  have hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < N := fun h hh => by
    have := hI.pos h hh; omega
  obtain ⟨sA', hA', cA, vA⟩ := exclude_fold hN es ∅ s hwf.nodup (fun e _ => by simp)
    (by simpa using hI.tree) hpos
  have hAA : sA' = sA := by rw [hA'] at hA; exact Option.some.inj hA
  subst hAA
  have oA := toRef_ord cA
  have bA := toRef_bits cA
  set T := s.bits.filter (fun h => h / 2 ∉ ∅ ∪ es.toFinset) with hTdef
  have hT : ∀ h, h ∈ T ↔ h ∈ sA'.bits ∧ g.head (h / 2) ≠ n.val ∧ g.tail (h / 2) ≠ n.val := by
    intro h
    rw [bA, hTdef, Finset.mem_filter]
    simp only [Finset.empty_union, List.mem_toFinset]
    constructor
    · rintro ⟨h1, h2⟩
      have := hwf.incident (h / 2) (hI.mem h h1)
      exact ⟨h1, fun hh => h2 (this.mpr (Or.inl hh)), fun hh => h2 (this.mpr (Or.inr hh))⟩
    · rintro ⟨h1, h2, h3⟩
      have := hwf.incident (h / 2) (hI.mem h h1)
      exact ⟨h1, fun hh => by rcases this.mp hh with hh | hh <;> contradiction⟩
  have hTsub : ∀ h ∈ T, h ∈ s.bits := fun h hh => (Finset.mem_filter.mp hh).1
  have vA' : Valid N sA'.tree (histZ sA'.ord T) := by rw [oA]; exact vA
  have hposT : ∀ h ∈ T, 1 ≤ sA'.ord (h / 2) ∧ sA'.ord (h / 2) ≤ L := fun h hh => by
    rw [oA]; have := hI.pos h (hTsub h hh); omega
  have hcard : T.card < 2 ^ 64 := by
    have := card_le_of_bits (E := E) (S := T) (fun x hx => hI.lt x (hTsub x hx))
    omega
  have htotal : indexPrefix N sA'.tree L = some (T.card : ZMod (2 ^ 64)) := by
    rw [indexPrefix_eq hN vA' hLN, sum_histZ sA'.ord T (fun x hx => (hposT x hx).1)]
    congr 3
    exact Finset.filter_true_of_mem (fun x hx => (hposT x hx).2)
  rw [htotal] at htot
  have htot' : total = (T.card : ZMod (2 ^ 64)) := (Option.some.inj htot).symm
  have hle : ∀ e ∈ es, sA'.ord e ≤ L := fun e he => by
    rw [oA]; have := hI.ord_le e (hwf.sub e he); omega
  have hBq := query_fold hN hLN g n.val T total es sA' hT vA' hposT hcard htot' hle
  rw [hBq] at hB
  have hBB := (Option.some.inj hB).symm
  subst hBB
  -- the concrete slices
  obtain ⟨rg, hrg, hrg1, hrg2⟩ := hoff
  obtain ⟨sl, hsl, hslv⟩ := get_range adjacency rg hrg1 hrg2
  have hes : sl.val.map (·.val) = es := by
    rw [hes_def, hg]; exact slice_map_adjOf offsets adjacency n.val rg hrg sl hslv
  have hes2 : g.adj n.val = sl.val.map (·.val) := (hes.trans hes_def).symm
  have hsub_adj : ∀ e ∈ sl.val, e.val ∈ adjL := fun e he =>
    hwf.sub e.val (by rw [hes2]; exact List.mem_map_of_mem he)
  have heE : ∀ e ∈ sl.val, e.val < E := fun e he => hE _ (hsub_adj e he)
  have htailok : ∀ e ∈ sl.val, TailOk tail g e := by
    intro e he
    have hl : e.val < tail.val.length := by rw [htailE]; exact heE e he
    refine ⟨tail.val[e.val], List.getElem?_eq_getElem hl, ?_⟩
    simp [hg, graphOf, List.getElem?_eq_getElem hl]
  have hedgeok : ∀ (ord : ℕ → ℕ) (order : Slice I32) (op : Slice U64), ORel order0 adjL ord order →
      op.length = W → ∀ e ∈ sl.val, EdgeOk order op ord e := by
    intro ord order op hor hl e he
    have := heE e he
    exact ⟨by omega, by rw [hl]; omega, hor.inAdj e.val (hsub_adj e he)⟩
  -- loop 1
  obtain ⟨tree1, hex, htr1, _⟩ := exclude_refines h hN order op sl 0 tree s sA' (by omega) hrel.bits
    hrel.tree (hedgeok s.ord order op hrel.ord hrel.len) (by rw [List.drop_zero, hes]; exact hA)
  -- the total
  obtain ⟨r4, hr4, hr4m⟩ := index_prefix_refines h tree1 N hN htr1.1 (histZ sA'.ord T)
    (fun j hj0 hj => by rw [htr1.2 j hj0 hj]; exact vA' j hj0 hj) (Slice.len list)
  rw [Slice.len_val, indexPrefix_congr hN htr1.2, htotal] at hr4m
  obtain ⟨v2, rfl, hv2⟩ : ∃ v2 : U64, r4 = some v2 ∧ (v2.val : ZMod (2 ^ 64)) = T.card := by
    cases r4 with
    | none => simp at hr4m
    | some v2 => exact ⟨v2, rfl, by simpa using hr4m⟩
  -- loop 2
  have hqs : qsum sA'.ord (fun e => hits g sA'.ord sA'.bits (sA'.ord e) n.val) es =
      occHits g n.val s.toRefState := by
    unfold qsum occHits; rw [oA, bA]
  obtain ⟨c1, hq, hc1⟩ := query_refines h hN order tree1 sA' (histZ sA'.ord T) vA' htr1 v2
    (fun e => hits g sA'.ord sA'.bits (sA'.ord e) n.val) sl
    (fun e he => by rw [oA]; exact hrel.ord.inAdj e.val (hsub_adj e he))
    (fun e he h0 => by
      have he' : e.val ∈ es := by rw [← hes]; exact List.mem_map_of_mem he
      obtain ⟨p, hp, hsub⟩ := query_count_eq_hits hN hLN g n.val hT vA' hposT hcard rfl (hle _ he')
      exact ⟨p, hp, by rw [hv2]; exact hsub⟩)
    0 c (by omega) hc0 (by rw [List.drop_zero, hes, hqs]; exact hcb)
  rw [List.drop_zero, hes, hqs] at hc1
  -- loop 3
  obtain ⟨op1, tree2, hres, hl1, hb1, htr2, hoC⟩ := restore_refines h hN order tail n g sl 0 op tree1
    { sA' with crossings := queryAcc g n.val sA'.ord sA'.bits es sA'.crossings } sC (by omega)
    (by show bitsOf op = sA'.bits; rw [bA]; exact hrel.bits) htr1
    (fun e he => ⟨hedgeok _ order op (by rw [oA]; exact hrel.ord) hrel.len e he, htailok e he⟩)
    (by rw [List.drop_zero, hes]; exact hC)
  have hoC' : sC.ord = s.ord := by rw [hoC]; exact oA
  -- loop 4
  have hocc1 := occ_succ occ i hocc hiL
  obtain ⟨order2, op2, tree3, hop, hor2, hl2, hb2, htr3⟩ := open_refines h hN order0 adjL tail n g sl
    (core.num.I32.wrapping_add occ 1#i32) (i + 1) hocc1 0 order op1 tree2 sC s' (by omega)
    (by rw [hoC']; exact hrel.ord) hb1 htr2
    (fun e he => by
      have := heE e he
      exact ⟨hsub_adj e he, by omega, by rw [hl1, hrel.len]; omega, htailok e he⟩)
    (by rw [List.drop_zero, hes]; exact hD)
  refine ⟨s', order2, op2, tree3, c1, core.num.I32.wrapping_add occ 1#i32, ?_, hocc1, hc1,
    ⟨hor2, by rw [hl2, hl1, hrel.len], hb2, htr3⟩, hc', hI'⟩
  have hex' : circogen.blockpath.exclude_edges 0#u8 order op tree sl = ok (some (), tree1) := by
    unfold circogen.blockpath.exclude_edges; rw [into_iter_eq, bind_ok]; exact hex
  have hq' : circogen.blockpath.query_edges order head tail op tree1 sl v2 n c = ok (some c1) := by
    unfold circogen.blockpath.query_edges; rw [into_iter_eq, bind_ok]; exact hq
  have hres' : circogen.blockpath.restore_edges 0#u8 order tail op tree1 sl n = ok (some (), op1, tree2) := by
    unfold circogen.blockpath.restore_edges; rw [into_iter_eq, bind_ok]; exact hres
  have hop' : circogen.blockpath.open_edges order tail op1 tree2 sl n (core.num.I32.wrapping_add occ 1#i32) =
      ok (some (), order2, op2, tree3) := by
    unfold circogen.blockpath.open_edges; rw [into_iter_eq, bind_ok]; exact hop
  simp only [circogen.blockpath.count_indexed_loop.body, next_some _ _ hlt, hn_def, bind_ok,
    uncurry_apply_pair, lift_eq, get_usize, hrg, branch_some, range_clone, hsl, hex', hr4, hq',
    hres', hop']

theorem idx_run (h : Is64) (order0 : Slice I32) (head tail list : Slice Usize)
    (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (E N W : ℕ)
    (hN : N < 2 ^ 62) (hLN : list.val.length < N) (hL31 : list.val.length ≤ 2147483647)
    (hE62 : E < 2 ^ 62) (hadjE : ∀ x ∈ adjacency.val, x.val < E) (hW : 2 * E ≤ 64 * W)
    (htailE : tail.val.length = E)
    (hoff : ∀ n ∈ list.val, ∃ r, offsets.val[n.val]? = some r ∧ r.start.val ≤ r.«end».val ∧
      r.«end».val ≤ adjacency.val.length)
    (hwf : ∀ n ∈ list.val, NodeWf (graphOf head tail offsets adjacency) (adjacency.val.map (·.val)) n.val) :
    ∀ (i : ℕ) (order : Slice I32) (op tree : Slice U64) (c occ : I32) (s : IdxState),
      i ≤ list.val.length → Inv (adjacency.val.map (·.val)) E N i s →
      IRel order0 (adjacency.val.map (·.val)) W N order op tree s → occ.val = (i : ℤ) → 0 ≤ c.val →
      c.val + natCrossings (graphOf head tail offsets adjacency) (i + 1)
        ((list.val.map (·.val)).drop i) s.toRefState ≤ 2147483647 →
      ∃ (c' : I32) (order' : Slice I32) (op' tree' : Slice U64) (s' : IdxState),
        circogen.blockpath.count_indexed_loop 0#u8 ⟨list, i⟩ order head tail list offsets adjacency
          op tree c occ = ok (some c', order', op', tree') ∧
        c'.val = c.val + natCrossings (graphOf head tail offsets adjacency) (i + 1)
          ((list.val.map (·.val)).drop i) s.toRefState ∧
        IRel order0 (adjacency.val.map (·.val)) W N order' op' tree' s' ∧
        s'.toRefState = refRun (graphOf head tail offsets adjacency) (i + 1)
          ((list.val.map (·.val)).drop i) s.toRefState := by
  intro i
  induction hk : list.val.length - i generalizing i with
  | zero =>
    intro order op tree c occ s hi hI hrel hocc hc0 hcb
    have hge : ¬ i < list.val.length := by omega
    have hd : (list.val.map (·.val)).drop i = [] := List.drop_eq_nil_of_le (by simp; omega)
    refine ⟨c, order, op, tree, s, ?_, by rw [hd]; simp [natCrossings], hrel, by rw [hd]; rfl⟩
    rw [idx_loop_unfold]
    simp only [circogen.blockpath.count_indexed_loop.body, next_none _ _ hge, bind_ok,
      uncurry_apply_pair]
  | succ k ih =>
    intro order op tree c occ s hi hI hrel hocc hc0 hcb
    have hlt : i < list.val.length := by omega
    have hdrop : (list.val.map (·.val)).drop i =
        list.val[i].val :: (list.val.map (·.val)).drop (i + 1) := by
      rw [List.drop_eq_getElem_cons (by simpa using hlt)]; simp
    rw [hdrop, natCrossings] at hcb
    obtain ⟨s1, order1, op1, tree1, c1, occ1, hbody, hocc1, hc1, hrel1, hs1, hI1⟩ :=
      idx_occ h order0 head tail list offsets adjacency E N W hN hLN hE62 hadjE hW htailE i hlt
        (by omega) (hwf _ (List.getElem_mem hlt)) (hoff _ (List.getElem_mem hlt)) s hI order op tree
        c occ hocc hc0 (by push_cast at hcb ⊢; omega) hrel
    obtain ⟨c2, order2, op2, tree2, s2, hl2, hc2, hrel2, hs2⟩ := ih (i + 1) (by omega) order1 op1
      tree1 c1 occ1 s1 (by omega) hI1 hrel1 hocc1 (by rw [hc1]; omega)
      (by rw [hc1, hs1]; push_cast at hcb ⊢; omega)
    refine ⟨c2, order2, op2, tree2, s2, ?_, ?_, hrel2, ?_⟩
    · rw [idx_loop_unfold, hbody, bind_ok]; exact hl2
    · rw [hc2, hc1, hdrop, natCrossings, hs1]; push_cast; ring
    · rw [hs2, hdrop, refRun, hs1]

/-! ## The prologues -/

theorem ORel_reset (order0 o : Slice I32) (adj : List ℕ) (ord0 : ℕ → ℕ)
    (ho : ∀ e, o.val[e]? = resetSpec order0 adj e) (hadj : ∀ e ∈ adj, e < order0.val.length) :
    ORel order0 adj (initState adj ord0).ord o := by
  refine ⟨fun e he => ?_, fun e he => ?_⟩
  · refine ⟨0#i32, ?_, ?_⟩
    · rw [ho e]; unfold resetSpec; simp [he, List.getElem?_eq_getElem (hadj e he)]
    · show (0 : ℤ) = ((adj.foldl (fun o e => Function.update o e 0) ord0 e : ℕ) : ℤ)
      rw [foldl_update_zero adj ord0 e he]; simp
  · rw [ho e]; unfold resetSpec; simp [he]

/-! ## Main theorem -/

/-- **`count_indexed` = `count_all_crossings_reference`, over the extracted Rust.** If the
extracted reference scan returns `ok (c, order', open')` (so none of its checked `i32` additions
overflowed), then on a well-formed block graph the extracted indexed counter, with the production
`MUTATION = 0`, returns `ok (some c, order', open', tree')`: no early `None`, the same `i32` total,
the same final `arena.order` and the same `open` bitset. The total is the exact crossing count
`natCrossings` of the model (no wrapping). -/
theorem count_indexed_eq_reference (h : Is64)
    (order : Slice I32) (head tail list : Slice Usize) (offsets : Slice (core.ops.range.Range Usize))
    (adjacency : Slice Usize) («open» tree : Slice U64)
    -- sizes: `E` edges; `open` covers the `2E` halves; the indexed path needs `list.len() < tree.len()`
    (E : ℕ) (hE : E < 2 ^ 62) (hord : order.length = E) (htail : tail.length = E)
    (hopen : 2 * E ≤ 64 * «open».length)
    (htree : tree.length < 2 ^ 62) (hlist : list.length < tree.length)
    (hlist31 : list.length ≤ 2 ^ 31 - 1)
    -- every adjacency entry is an edge id
    (hadj : ∀ x ∈ adjacency.val, (x : Usize).val < E)
    -- every listed node has an offset range inside `adjacency` and a well-formed adjacency slice
    (hoff : ∀ n ∈ list.val, ∃ r, offsets.val[(n : Usize).val]? = some r ∧
      r.start.val ≤ r.«end».val ∧ r.«end».val ≤ adjacency.length)
    (hwf : ∀ n ∈ list.val, NodeWf (graphOf head tail offsets adjacency)
      (adjacency.val.map (·.val)) (n : Usize).val)
    (c : I32) (order' : Slice I32) (open' : Slice U64)
    (href : circogen.blockpath.count_all_crossings_reference order head tail list offsets
      adjacency «open» = ok (c, order', open')) :
    ∃ tree', circogen.blockpath.count_indexed 0#u8 order head tail list offsets adjacency «open»
      tree = ok (some c, order', open', tree') ∧
      c.val = (natCrossings (graphOf head tail offsets adjacency) 1 (list.val.map (·.val))
        (initState (adjacency.val.map (·.val)) (fun _ => 0)) : ℤ) := by
  set N := tree.length with hNdef
  set W := «open».length with hWdef
  set s0 : IdxState := { initState (adjacency.val.map (·.val)) (fun _ => 0) with tree := fun _ => 0 } with hs0
  have hI0 : Inv (adjacency.val.map (·.val)) E N 0 s0 := initInv E N (fun _ => 0)
  have hadjL : ∀ e ∈ (adjacency.val.map (·.val)), e < order.val.length := by
    intro e he; obtain ⟨x, hx, rfl⟩ := List.mem_map.mp he
    have := hadj x hx; rw [show order.length = order.val.length from rfl] at hord; omega
  -- the reference run
  obtain ⟨op1, hfill, hl1, hv1⟩ := fill_zero «open»
  unfold circogen.blockpath.count_all_crossings_reference at href
  rw [hfill, bind_ok, into_iter_eq, bind_ok] at href
  cases h0 : circogen.blockpath.count_all_crossings_reference_loop0 ⟨adjacency, 0⟩ order with
  | vis _ _ => rw [h0] at href; simp [bind_vis] at href
  | div => rw [h0] at href; simp at href
  | ret order1 =>
    rw [h0, bind_ok, into_iter_eq, bind_ok] at href
    have hres1 := ref_reset_PC order adjacency order1 h0
    have hrr : RRel order (adjacency.val.map (·.val)) W order1 op1 s0.toRefState :=
      ⟨ORel_reset order order1 (adjacency.val.map (·.val)) (fun _ => 0) hres1 hadjL, bitsOf_replicate op1 (by rw [hv1, hl1]),
        hl1⟩
    obtain ⟨hcr, hrr'⟩ := ref_run_PC h order head tail list offsets adjacency E N W htree hlist
      (by omega) hE hadj hwf 0 order1 op1 0#i32 0#i32 s0 (by omega) hI0 rfl hrr _ href
    dsimp only at hcr hrr'
    simp only [List.drop_zero] at hcr hrr'
    have hcr' : c.val = (natCrossings (graphOf head tail offsets adjacency) 1 (list.val.map (·.val)) s0.toRefState : ℤ) := by
      rw [hcr]; simp
    have hcmax : c.val ≤ 2147483647 := by
      have := c.hBounds; simp [IScalarTy.numBits] at this; omega
    -- the indexed run
    obtain ⟨tree1, hfillt, hlt1, hvt1⟩ := fill_zero tree
    obtain ⟨order2, hreset, hres2⟩ := reset_orders_spec order adjacency (by
      intro x hx; have := hadj x hx; rw [show order.length = order.val.length from rfl] at hord; omega)
    have hirr : IRel order (adjacency.val.map (·.val)) W N order2 op1 tree1 s0 :=
      ⟨ORel_reset order order2 (adjacency.val.map (·.val)) (fun _ => 0) hres2 hadjL, hl1,
        bitsOf_replicate op1 (by rw [hv1, hl1]),
        ⟨hlt1, fun j _ _ => by rw [tz_replicate tree1 (by rw [hvt1, hlt1])]⟩⟩
    obtain ⟨c', order3, op3, tree3, s3, hidx, hc', hirr', hs3⟩ := idx_run h order head tail list offsets
      adjacency E N W htree hlist (by omega) hE hadj hopen (by
        rw [show tail.length = tail.val.length from rfl] at htail; exact htail) hoff hwf 0 order2 op1
      tree1 0#i32 0#i32 s0 (by omega) hI0 hirr rfl (by simp) (by
        simp only [List.drop_zero]; rw [← hcr']; simpa using hcmax)
    simp only [List.drop_zero] at hc' hs3
    have hcc : c' = c := by
      rw [IScalar.eq_equiv, hc', hcr']; simp
    have hoo : order3 = order' := by
      have ho3 := hirr'.ord
      rw [show s3.ord = s3.toRefState.ord from rfl, hs3] at ho3
      exact ORel_inj ho3 hrr'.ord
    have hop : op3 = open' := bitsOf_inj (by rw [hirr'.len, hrr'.len])
      (by rw [hirr'.bits, hrr'.bits, show s3.bits = s3.toRefState.bits from rfl, hs3])
    refine ⟨tree3, ?_, hcr'⟩
    unfold circogen.blockpath.count_indexed
    rw [hfill, bind_ok, hfillt, bind_ok, hreset, bind_ok]
    simp only [uncurry_apply_pair, branch_some, bind_ok, into_iter_eq]
    rw [hidx, hcc, hoo, hop]

end GvpProofs.Circo.FenwickExtractedMain
