import Mathlib
import GvpProofs.Circo.Fenwick
import GvpProofs.Extracted.CircoCount.Funs

/-!
# circo: Fenwick lemmas over the *extracted* Rust

Subject: `crates/graphviz-pure-layout/src/circogen/blockpath.rs`, as Aeneas extracts it into
`GvpProofs/Extracted/CircoCount/Funs.lean` (`scripts/proofs/extract.sh`, checked by `--check`).
`Fenwick.lean` proves the Fenwick lemmas and the simulation `count_indexed = reference` on a Lean
*model* of the Rust. This file is the first step of restating them over the shipped functions
themselves: the statements below mention `circogen.blockpath.index_update`,
`circogen.blockpath.index_prefix`, `circogen.blockpath.add_crossings` and
`circogen.blockpath.wrap_crossings`, which Aeneas generated from the Rust, and no hand-written
model of them.

## Theorems

* `index_prefix_refines`, `index_prefix_sum`: the extracted `index_prefix` returns `ok`, with the
  model's `indexPrefix` answer (`Fenwick.indexPrefix_eq`) read in `ZMod (2 ^ 64)`.
* `index_update_refines`, `index_update_valid`: the extracted `index_update` returns `ok`, declines
  exactly when the model's `indexUpdate` does, and otherwise leaves the model's tree on slots
  `1..N-1` (`Fenwick.indexUpdate_valid`).
* `wrap_crossings_spec`, `add_crossings_ok`, `add_crossings_fail`: the `i32` accumulator.
* The main theorem (`count_indexed` equals `count_all_crossings_reference`) is stated, with
  an unproved placeholder, in `proofs/wip/FenwickExtractedMain.lean`, together with the list of lemmas that remain.

## What the extracted statements need that the model's did not

* **`Result`.** Every extracted function returns `Result`; a statement must show it is `ok`. For the
  Fenwick operations it always is on a slice of length `N` (`index_*_refines` have no failure
  case). `add_crossings` is the exception: the extraction keeps `cfg!(debug_assertions)` (Charon
  builds in the dev profile), so it *fails* when the total would pass `i32::MAX` where the shipped
  release build wraps (`add_crossings_fail`). The model's `i32` total wraps (`BitVec 32`); extracted
  statements about totals are about runs where no addition overflows. The wrapping step itself,
  `wrap_crossings`, is total and is the model's `wrapCrossings` (`wrap_crossings_spec`).
* **Pointer width: `Is64`.** `usize` is `UScalar .Usize`, whose width is `System.Platform.numBits`,
  32 or 64. The model hard-wired 64 (`lowbit`, `nextIdx`, `prevIdx` mod `2 ^ 64`). Every theorem
  here takes `h : Is64`. On a 32-bit target (the wasm build) the bit identities change and the
  theorems are not claimed.
* **Slices, not functions.** The model's tree is `ℕ → ZMod (2 ^ 64)`; the extraction's is
  `Slice U64`. `tz` reads it (slot `j`, or `0` when absent) and the statements carry
  `tree.length = N`, `N < 2 ^ 62`. The model needed the bound too, but not the length, and it
  never mentioned an out-of-range read; here `Slice.get` returns `none`, and the proofs show the
  loop never reaches one.
* **Mutable borrows.** Aeneas turns `&mut [u64]` into a returned slice (a backward function), so
  `index_update` returns `(Option Unit × Slice U64)`; the statement is about the returned slice.
  The refactor of #745 rewrote `tree.get_mut(i)?` as a `match` for this reason (Aeneas's `?` on
  `Option<&mut T>` needs a second model of `branch`).
* **Slot 0 and slots `≥ N`.** The model's `indexUpdate` returns a total function that is exact on
  `1..N-1`; the extracted slice also keeps slot `0` and the rest untouched. `index_update_refines`
  therefore equates `tz tree'` with the model only on `0 < j < N`, which is all `Valid` reads.
* **The fuel.** The model's loops carry a fuel counter; the extracted ones are `loop` fixpoints.
  Termination is proved here with a measure (`N - i` for the update, the index itself for the
  prefix), `loop.spec_decr_nat`.

## Reuse from the model

`lowbit_pos`, `lowbit_le`, `nextIdx_eq`, `prevIdx_eq`, `touch_step`, `updateGo_eq`, `indexPrefix_eq`
and `indexUpdate_valid` are used unchanged: the bit identities were proved once, over `ℕ`, and the
extracted `usize` operations are shown to compute them (`next_idx`, `prefix_step`).
-/

set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false

open Aeneas Aeneas.Std Result ControlFlow
open GvpProofs.Extracted.CircoCount GvpProofs.Circo.Fenwick

namespace GvpProofs.Circo.FenwickExtracted

abbrev Is64 : Prop := System.Platform.numBits = 64

theorem usize_val_lt (x : Usize) (h : Is64) : x.val < 2 ^ 64 := by
  have := x.hBounds
  simp [UScalarTy.numBits, h] at this
  omega

/-- Tree contents as `ZMod (2 ^ 64)` (absent slot 0). -/
def tz (s : Slice U64) (j : ℕ) : ZMod (2 ^ 64) :=
  match s.val[j]? with
  | some x => (x.val : ZMod (2 ^ 64))
  | none => 0

theorem getElem?_set_opt_some {α : Type*} (l : List α) (v : α) :
    ∀ (i j : ℕ), (l.set_opt i (some v))[j]? = if j = i ∧ i < l.length then some v else l[j]? := by
  induction l with
  | nil => intro i j; simp
  | cons x tl ih =>
    intro i j
    rcases i with _ | i <;> rcases j with _ | j
    · simp
    · simp
    · simp
    · rw [List.set_opt_cons_nzero _ _ _ _ (by simp)]
      simp [ih]

theorem tz_set_opt (t : Slice U64) (i : Usize) (v : U64) (j : ℕ) :
    tz (t.set_opt i (some v)) j =
      if j = i.val ∧ i.val < t.val.length then (v.val : ZMod (2 ^ 64)) else tz t j := by
  have : (t.set_opt i (some v)).val = t.val.set_opt i.val (some v) := by simp [Slice.set_opt]
  unfold tz
  rw [this, getElem?_set_opt_some]
  split_ifs <;> simp

theorem next_idx (h : Is64) (i : Usize) :
    (core.num.Usize.wrapping_add i (i &&& core.num.Usize.wrapping_sub 0#usize i)).val = nextIdx i.val := by
  simp [nextIdx, lowbit, Usize.size, UScalar.size_def, Usize.numBits_def, UScalarTy.numBits, h]

theorem wrapping_add_cast (x : U64) : ((core.num.U64.wrapping_add x 1#u64).val : ZMod (2 ^ 64)) = (x.val : ZMod (2 ^ 64)) + 1 := by
  simp [UScalar.size_def, UScalarTy.numBits]

theorem wrapping_sub_cast (x : U64) : ((core.num.U64.wrapping_sub x 1#u64).val : ZMod (2 ^ 64)) = (x.val : ZMod (2 ^ 64)) - 1 := by
  have e := core.num.U64.wrapping_sub_val_eq x 1#u64
  have e2 : UScalar.size .U64 = 2 ^ 64 := by simp [UScalar.size_def, UScalarTy.numBits]
  rw [e2] at e
  rw [e, ZMod.natCast_mod]
  rw [show ((1#u64).val) = 1 from rfl, Nat.cast_add, Nat.cast_sub (by norm_num : 1 ≤ 2 ^ 64),
    ZMod.natCast_self]
  simp [sub_eq_add_neg]

theorem prefix_step (h : Is64) (i : Usize) :
    (i &&& core.num.Usize.wrapping_sub i 1#usize).val = prevIdx i.val := by
  simp [prevIdx, UScalar.size_def, UScalarTy.numBits, h]

theorem spec_ex {α : Type} {x : Result α} {p : α → Prop} (h : WP.spec x p) :
    ∃ y, x = ok y ∧ p y := by
  cases x <;> simp_all

theorem index_prefix_loop_spec (h : Is64) (tree : Slice U64) (N : ℕ) (hN : N < 2 ^ 62)
    (hlen : tree.length = N) (a : ℕ → ZMod (2 ^ 64)) (hv : Valid N (tz tree) a)
    (i0 : Usize) (s0 : U64) (hi0 : i0.val < N) :
    circogen.blockpath.index_prefix_loop tree i0 s0 ⦃ r =>
      ∃ s : U64, r = some s ∧
        (s.val : ZMod (2 ^ 64)) = s0.val + ∑ k ∈ Finset.Ioc 0 i0.val, a k ⦄ := by
  unfold circogen.blockpath.index_prefix_loop
  apply loop.spec_decr_nat (fun x : Usize × U64 => x.1.val)
    (fun x => x.1.val < N ∧ (x.2.val : ZMod (2 ^ 64)) + ∑ k ∈ Finset.Ioc 0 x.1.val, a k =
      s0.val + ∑ k ∈ Finset.Ioc 0 i0.val, a k)
    (fun r => ∃ s : U64, r = some s ∧
        (s.val : ZMod (2 ^ 64)) = s0.val + ∑ k ∈ Finset.Ioc 0 i0.val, a k)
  · rintro ⟨i, s⟩ ⟨hi, hinv⟩
    dsimp only at hi hinv ⊢
    have hlen' : tree.val.length = N := hlen
    unfold circogen.blockpath.index_prefix_loop.body
    by_cases h0 : i > 0#usize
    · simp only [h0, ↓reduceIte]
      have hlt : i.val < tree.val.length := by
        omega
      obtain ⟨x, hx⟩ : ∃ x, tree.val[i.val]? = some x := ⟨_, List.getElem?_eq_getElem hlt⟩
      have hget : core.slice.Slice.get (core.slice.index.SliceIndexUsizeSlice U64) tree i = ok (some x) := by
        simp [core.slice.Slice.get, core.slice.index.SliceIndexUsizeSlice, hx]
      rw [hget]
      simp [core.option.Option.Insts.CoreOpsTry_traitTry.branch, lift]
      have e1 : Usize.size = 2 ^ 64 := by simp [Usize.size, UScalar.size_def, Usize.numBits_def, UScalarTy.numBits, h]
      have e2 : U64.size = 2 ^ 64 := by simp [U64.size, UScalar.size_def, U64.numBits_def, UScalarTy.numBits]
      have hi64 := usize_val_lt i h
      have hp : i.val &&& (i.val + (2 ^ 64 - 1)) % 2 ^ 64 = i.val - lowbit i.val :=
        prevIdx_eq i.val hi64
      have hpos := lowbit_pos i.val h0 hi64
      have hle := lowbit_le i.val
      have htz : tz tree i.val = (x.val : ZMod (2 ^ 64)) := by simp [tz, hx]
      have hvi := hv i.val h0 (by omega)
      rw [htz] at hvi
      rw [e1, e2, hp]
      refine ⟨by omega, ?_, by omega⟩
      rw [ZMod.natCast_mod, ← hinv, Nat.cast_add, hvi, ← Finset.sum_Ioc_consecutive a
        (Nat.zero_le (i.val - lowbit i.val)) (Nat.sub_le i.val (lowbit i.val))]
      abel
    · have hz : i.val = 0 := by
        have : ¬ (0 < i.val) := fun hh => h0 (by simpa using hh)
        omega
      simp only [h0, ↓reduceIte]
      simp [hz] at hinv ⊢
      exact hinv
  · exact ⟨hi0, rfl⟩


/-- the per-step delta: `+1` for insert, `-1` for remove -/
def delta (insert : Bool) : ZMod (2 ^ 64) := if insert then 1 else -1

theorem index_update_loop_spec (h : Is64) (N : ℕ) (hN : N < 2 ^ 62) (insert : Bool)
    (i0 : Usize) (hi0p : 0 < i0.val) (hi0 : i0.val < N) :
    ∀ (tree : Slice U64), tree.length = N →
    circogen.blockpath.index_update_loop tree i0 insert ⦃ r =>
      ∃ tree' : Slice U64, r = (some (), tree') ∧ tree'.length = N ∧
        ∀ j, 0 < j → j < N → tz tree' j = tz tree j +
          (if j - lowbit j < i0.val ∧ i0.val ≤ j then delta insert else 0) ⦄ := by
  intro tree hlen
  unfold circogen.blockpath.index_update_loop
  apply loop.spec_decr_nat (fun x : Slice U64 × Usize => N - x.2.val)
    (fun x => x.1.length = N ∧ 0 < x.2.val ∧ x.2.val < 2 ^ 63 ∧
      ∀ j, 0 < j → j < N → tz x.1 j + (if j - lowbit j < x.2.val ∧ x.2.val ≤ j then delta insert else 0) =
        tz tree j + (if j - lowbit j < i0.val ∧ i0.val ≤ j then delta insert else 0))
    (fun r => ∃ tree' : Slice U64, r = (some (), tree') ∧ tree'.length = N ∧
        ∀ j, 0 < j → j < N → tz tree' j = tz tree j +
          (if j - lowbit j < i0.val ∧ i0.val ≤ j then delta insert else 0))
  · rintro ⟨t, i⟩ ⟨hl, hip, hi63, hinv⟩
    dsimp only at hl hip hi63 hinv ⊢
    unfold circogen.blockpath.index_update_loop.body
    have hlen' : t.val.length = N := hl
    by_cases hlt : i.val < N
    · have hlt' : i < Slice.len t := by
        simp [Slice.len, Slice.length, UScalar.lt_equiv]
        omega
      obtain ⟨x, hx⟩ : ∃ x, t.val[i.val]? = some x := ⟨_, List.getElem?_eq_getElem (by omega)⟩
      simp only [hlt', ↓reduceIte]
      simp [core.slice.Slice.get_mut, hx]
      obtain ⟨v1, hv1, hv1c⟩ : ∃ v1 : U64,
          (if insert = true then ok (core.num.U64.wrapping_add x 1#u64)
            else ok (core.num.U64.wrapping_sub x 1#u64) : Result U64) = ok v1 ∧
          (v1.val : ZMod (2 ^ 64)) = x.val + delta insert := by
        cases insert
        · exact ⟨core.num.U64.wrapping_sub x 1#u64, by simp, by rw [wrapping_sub_cast]; simp [delta, sub_eq_add_neg]⟩
        · exact ⟨core.num.U64.wrapping_add x 1#u64, by simp, by rw [wrapping_add_cast]; simp [delta]⟩
      rw [hv1]
      simp only [core.num.Usize.wrapping_neg, lift]
      simp
      have e1 : Usize.size = 2 ^ 64 := by
        simp [Usize.size, Usize.numBits_def, UScalarTy.numBits, h]
      have hpos := lowbit_pos i.val hip (by omega)
      have hle := lowbit_le i.val
      have hnx : (i.val + (i.val &&& (Usize.size - i.val) % Usize.size)) % Usize.size =
          i.val + lowbit i.val := by
        rw [e1]; exact nextIdx_eq i.val (by omega)
      rw [hnx]
      refine ⟨hlen', by omega,
        by omega, ?_, by omega⟩
      intro j hj0 hj
      have key := touch_step i.val hip (by omega) j hj0 (by omega)
      have hx' : tz t i.val = (x.val : ZMod (2 ^ 64)) := by simp [tz, hx]
      rw [tz_set_opt]
      have hj' := hinv j hj0 hj
      by_cases hji : j = i.val
      · subst hji
        have h1 : ¬ (i.val - lowbit i.val < i.val + lowbit i.val ∧ i.val + lowbit i.val ≤ i.val) := by
          omega
        have h2 : (i.val - lowbit i.val < i.val ∧ i.val ≤ i.val) := by omega
        simp only [h1, h2, hlt, hlen', and_self, ↓reduceIte, add_zero] at hj' ⊢
        rw [hx'] at hj'
        rw [← hj', hv1c]
      · simp only [hji, false_and, ↓reduceIte]
        rw [← hj']
        have : (j - lowbit j < i.val ∧ i.val ≤ j) ↔
            (j - lowbit j < i.val + lowbit i.val ∧ i.val + lowbit i.val ≤ j) := by
          constructor
          · intro hh; rcases key.mp hh with h' | h'
            · exact absurd h' hji
            · exact h'
          · intro hh; exact key.mpr (Or.inr hh)
        simp only [this]
    · have hge : ¬ i < Slice.len t := by
        simp [Slice.len, UScalar.lt_equiv]
        omega
      simp only [hge, ↓reduceIte]
      show WP.spec (ok (done ((some ()), t))) _
      rw [WP.spec_ok]
      refine ⟨t, rfl, hl, ?_⟩
      intro j hj0 hj
      have := hinv j hj0 hj
      have hno : ¬ (j - lowbit j < i.val ∧ i.val ≤ j) := by omega
      simpa [hno] using this
  · dsimp only
    exact ⟨hlen, hi0p, by omega, fun j _ _ => rfl⟩

/-- **`index_prefix` refines the model's `indexPrefix`.** On a Fenwick tree of `a` (`Valid`), the
extracted function returns `ok` and, read in `ZMod (2 ^ 64)`, the model's answer: `none` when the
key is out of range, else the prefix sum. -/
theorem index_prefix_refines (h : Is64) (tree : Slice U64) (N : ℕ) (hN : N < 2 ^ 62)
    (hlen : tree.length = N) (a : ℕ → ZMod (2 ^ 64)) (hv : Valid N (tz tree) a) (i : Usize) :
    ∃ r, circogen.blockpath.index_prefix tree i = ok r ∧
      r.map (fun s : U64 => (s.val : ZMod (2 ^ 64))) = indexPrefix N (tz tree) i.val := by
  unfold circogen.blockpath.index_prefix
  have hl : (Slice.len tree).val = N := by simp [Slice.len, hlen]
  by_cases hi : N ≤ i.val
  · have : i ≥ Slice.len tree := by simp [UScalar.le_equiv, hl, hi]
    simp [this, indexPrefix, hi]
  · have : ¬ i ≥ Slice.len tree := by simp [UScalar.le_equiv, hl, hi]
    simp only [this, ↓reduceIte]
    obtain ⟨r, hr, s, hs, hsv⟩ := spec_ex (index_prefix_loop_spec h tree N hN hlen a hv i 0#u64
      (by omega))
    refine ⟨r, hr, ?_⟩
    rw [hs, indexPrefix_eq hN hv (by omega)]
    simp [hsv]

/-- **`index_update` refines the model's `indexUpdate`.** The extracted function returns `ok` on
every slice of length `N`; it declines (`none`, slice unchanged) exactly when the model does,
and otherwise returns the model's tree on slots `1..N-1`. -/
theorem index_update_refines (h : Is64) (tree : Slice U64) (N : ℕ) (hN : N < 2 ^ 62)
    (hlen : tree.length = N) (i : Usize) (insert : Bool) :
    ∃ (r : Option Unit) (tree' : Slice U64),
      circogen.blockpath.index_update tree i insert = ok (r, tree') ∧ tree'.length = N ∧
      match indexUpdate N (delta insert) (tz tree) i.val with
      | none => r = none ∧ tree' = tree
      | some t => r = some () ∧ ∀ j, 0 < j → j < N → tz tree' j = t j := by
  unfold circogen.blockpath.index_update
  have hl : (Slice.len tree).val = N := by simp [Slice.len, hlen]
  by_cases h0 : i = 0#usize
  · have : i.val = 0 := by simp [h0]
    simp [h0, indexUpdate, hlen]
  · have h0' : i.val ≠ 0 := fun hh => h0 (by simpa [UScalar.eq_equiv] using hh)
    simp only [h0, ↓reduceIte]
    by_cases hi : N ≤ i.val
    · have : i ≥ Slice.len tree := by simp [UScalar.le_equiv, hl, hi]
      simp [this, indexUpdate, hi, hlen]
    · have : ¬ i ≥ Slice.len tree := by simp [UScalar.le_equiv, hl, hi]
      simp only [this, ↓reduceIte]
      obtain ⟨⟨r, tree'⟩, hr, tree'', hs, hl', hj⟩ := spec_ex
        (index_update_loop_spec h N hN insert i (by omega) (by omega) tree hlen)
      refine ⟨r, tree', hr, ?_⟩
      simp only [Prod.mk.injEq] at hs
      obtain ⟨rfl, rfl⟩ := hs
      refine ⟨hl', ?_⟩
      simp only [indexUpdate, h0', hi, or_self, ↓reduceIte]
      refine ⟨trivial, fun j hj0 hjN => ?_⟩
      rw [hj j hj0 hjN, updateGo_eq N hN (delta insert) N i.val (tz tree) (by omega) (by omega) j
        hj0 hjN]


/-- Fenwick lemma `indexPrefix_eq`, restated over the extracted `index_prefix`: on the tree of
`a`, a key below the length returns the prefix sum `a 1 + … + a i` in `ZMod (2 ^ 64)`. -/
theorem index_prefix_sum (h : Is64) (tree : Slice U64) (N : ℕ) (hN : N < 2 ^ 62)
    (hlen : tree.length = N) (a : ℕ → ZMod (2 ^ 64)) (hv : Valid N (tz tree) a) (i : Usize)
    (hi : i.val < N) :
    ∃ s : U64, circogen.blockpath.index_prefix tree i = ok (some s) ∧
      (s.val : ZMod (2 ^ 64)) = ∑ k ∈ Finset.Ioc 0 i.val, a k := by
  obtain ⟨r, hr, hm⟩ := index_prefix_refines h tree N hN hlen a hv i
  rw [indexPrefix_eq hN hv hi] at hm
  cases r with
  | none => simp at hm
  | some s => exact ⟨s, hr, by simpa using hm⟩

/-- Fenwick lemma `indexUpdate_valid`, restated over the extracted `index_update`: adding `±1` at
a key in `1..N-1` returns `ok (some ())` and a tree of `a + δ`. -/
theorem index_update_valid (h : Is64) (tree : Slice U64) (N : ℕ) (hN : N < 2 ^ 62)
    (hlen : tree.length = N) (a : ℕ → ZMod (2 ^ 64)) (hv : Valid N (tz tree) a)
    (i : Usize) (insert : Bool) (hi0 : 0 < i.val) (hi : i.val < N) :
    ∃ tree' : Slice U64, circogen.blockpath.index_update tree i insert = ok (some (), tree') ∧
      tree'.length = N ∧
      Valid N (tz tree') (fun k => a k + if k = i.val then delta insert else 0) := by
  obtain ⟨r, tree', hr, hl', hm⟩ := index_update_refines h tree N hN hlen i insert
  obtain ⟨t, ht, hv'⟩ := indexUpdate_valid hN hv (delta insert) hi0 hi
  rw [ht] at hm
  obtain ⟨rfl, hj⟩ := hm
  refine ⟨tree', hr, hl', fun j hj0 hjN => ?_⟩
  rw [hj j hj0 hjN]
  exact hv' j hj0 hjN

/-- `wrap_crossings` is the model's `wrapCrossings` on bit-vectors: `value + count (mod 2 ^ 32)`.
It never fails (`wrapping_add`). -/
theorem wrap_crossings_spec (value : I32) (count : U64) :
    ∃ v', circogen.blockpath.wrap_crossings value count = ok v' ∧
      v'.bv = value.bv + BitVec.ofNat 32 count.val := by
  unfold circogen.blockpath.wrap_crossings
  refine ⟨core.num.I32.wrapping_add value (UScalar.hcast IScalarTy.I32 count), by simp [lift], ?_⟩
  simp [UScalar.hcast]

/-- `add_crossings` in the extraction keeps the checked-build assertion
(`cfg!(debug_assertions)`, which Charon leaves on): below the `i32` ceiling it returns `ok` and then
behaves as `wrap_crossings`. -/
theorem add_crossings_ok (value : I32) (count : U64) (h0 : 0 ≤ value.val)
    (hc : count.val ≤ 2147483647 - value.val) :
   ∃ v', circogen.blockpath.add_crossings value count = ok v' := by
  have e1 : (I32.rMax % 18446744073709551616).toNat = 2147483647 := by
    simp [I32.rMax]
  have e2 : (value.val % 18446744073709551616).toNat = value.val.toNat := by
    have := value.hBounds
    simp at this
    omega
  have e3 : U64.size = 2 ^ 64 := by simp [U64.size, UScalar.size_def, U64.numBits_def, UScalarTy.numBits]
  unfold circogen.blockpath.add_crossings
  simp [lift, core.num.I32.MAX, e1, e2, e3]
  have hcond : (count.val ≤ (2147483647 + (18446744073709551616 - value.val.toNat)) % 18446744073709551616) := by
    omega
  simp [massert, hcond]
  obtain ⟨v', hv'⟩ := wrap_crossings_spec value count
  exact ⟨v', hv'.1⟩

/-- Above the ceiling the extracted `add_crossings` fails (`massert`), where the shipped release
build wraps. This is the one place the extracted statements are about the checked build. -/
theorem add_crossings_fail (value : I32) (count : U64) (h0 : 0 ≤ value.val)
    (hc : 2147483647 - value.val < count.val) :
   circogen.blockpath.add_crossings value count = fail .assertionFailure := by
  have e1 : (I32.rMax % 18446744073709551616).toNat = 2147483647 := by
    simp [I32.rMax]
  have e2 : (value.val % 18446744073709551616).toNat = value.val.toNat := by
    have := value.hBounds
    simp at this
    omega
  have e3 : U64.size = 2 ^ 64 := by simp [U64.size, UScalar.size_def, U64.numBits_def, UScalarTy.numBits]
  unfold circogen.blockpath.add_crossings
  have hb := value.hBounds
  simp at hb
  have hcond : ¬ (count.val ≤ (2147483647 + (18446744073709551616 - value.val.toNat)) % 18446744073709551616) := by
    omega
  simp [lift, core.num.I32.MAX, e1, e2, e3, massert, hcond]

end GvpProofs.Circo.FenwickExtracted
