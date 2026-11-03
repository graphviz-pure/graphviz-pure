import Mathlib

/-!
# circo: the Fenwick-indexed crossing count equals the bitset reference count

Subject: `crates/graphviz-pure-layout/src/circogen/blockpath.rs`,
`count_indexed` (production, Fenwick tree over opening orders) against
`count_all_crossings_reference` (retained bitset scan, a mirror of C's
`lib/circogen/blockpath.c:count_all_crossings`). Prose argument being replaced:
`docs/perf/circo-crossings.md`.

Aeneas extraction was not available when this file was written, so the theorems are stated over
a Lean model that mirrors the Rust control flow. "Model vs Rust" is listed in the PR.

This section holds the definitions only. Each Rust function has one Lean function with the same
branches, in the same order.
-/

namespace GvpProofs.Circo.Fenwick

/-! ## Fenwick tree operations (`index_update`, `index_prefix`) -/

/-- `i & i.wrapping_neg()` on a 64-bit `usize`. -/
def lowbit (i : ℕ) : ℕ := i &&& ((2 ^ 64 - i) % 2 ^ 64)

/-- `i.wrapping_add(i & i.wrapping_neg())`. -/
def nextIdx (i : ℕ) : ℕ := (i + lowbit i) % 2 ^ 64

/-- `i &= i.wrapping_sub(1)`. -/
def prevIdx (i : ℕ) : ℕ := i &&& ((i + (2 ^ 64 - 1)) % 2 ^ 64)

/-- The `while i < tree.len()` loop of `index_update`, with a fuel counter so that it is total.
The fuel is `tree.len()`; the loop's index strictly increases, so it never runs out
(`updateGo_eq`). `tree.get_mut(i)?` cannot fail under the loop guard, and `wrapping_add(1)` /
`wrapping_sub(1)` on `u64` are `+ 1` / `+ -1` in `ZMod (2 ^ 64)`. -/
def updateGo {M : Type*} [AddCommGroup M] (len : ℕ) (d : M) :
    ℕ → ℕ → (ℕ → M) → (ℕ → M)
  | 0, _, t => t
  | fuel + 1, i, t =>
    if i < len then updateGo len d fuel (nextIdx i) (Function.update t i (t i + d)) else t

/-- `index_update(tree, i, insert)`: `d = 1` for `insert = true`, `d = -1` otherwise. -/
def indexUpdate {M : Type*} [AddCommGroup M] (len : ℕ) (d : M) (t : ℕ → M) (i : ℕ) :
    Option (ℕ → M) :=
  if i = 0 ∨ len ≤ i then none else some (updateGo len d len i t)

/-- The `while i > 0` loop of `index_prefix`, with fuel (`i` suffices, `prevIdx i < i`). -/
def prefixGo {M : Type*} [AddCommGroup M] (t : ℕ → M) : ℕ → ℕ → M → M
  | 0, _, sum => sum
  | fuel + 1, i, sum => if 0 < i then prefixGo t fuel (prevIdx i) (sum + t i) else sum

/-- `index_prefix(tree, i)`. -/
def indexPrefix {M : Type*} [AddCommGroup M] (len : ℕ) (t : ℕ → M) (i : ℕ) : Option M :=
  if len ≤ i then none else some (prefixGo t i i 0)

/-! ## The block graph -/

/-- What the counters read of `Circ`: `arena.head`, `arena.tail`, and, per node, the slice
`adjacency[offsets[n]]`. -/
structure Graph where
  head : ℕ → ℕ
  tail : ℕ → ℕ
  adj : ℕ → List ℕ

/-- The half-key of edge `e` that opens or closes at node `n`:
`(e << 1) | usize::from(tail[e] != n)`. -/
def halfOf (g : Graph) (n e : ℕ) : ℕ := 2 * e + (if g.tail e ≠ n then 1 else 0)

/-- The state both counters mutate: `arena.order` (positive `i32`, modelled in `ℕ`), the set
bits of the `open` bitset (as half indices), and the running `i32` total. -/
structure RefState where
  ord : ℕ → ℕ
  bits : Finset ℕ
  crossings : BitVec 32

/-! ## `count_all_crossings_reference` -/

/-- The inner scan: every set half in ascending order, `crossings += 1` when
`order[ep] > edge_order && head[ep] != n && tail[ep] != n`. `i32` addition wraps in release. -/
def refQuery (g : Graph) (ord : ℕ → ℕ) (bits : Finset ℕ) (edgeOrder n : ℕ) (c : BitVec 32) :
    BitVec 32 :=
  (bits.sort (· ≤ ·)).foldl
    (fun c half =>
      if edgeOrder < ord (half / 2) ∧ g.head (half / 2) ≠ n ∧ g.tail (half / 2) ≠ n then c + 1
      else c)
    c

/-- First loop body: `if edge_order > 0 { scan; clear the closing half }`. -/
def refClose (g : Graph) (n : ℕ) (s : RefState) (e : ℕ) : RefState :=
  if 0 < s.ord e then
    { s with
      crossings := refQuery g s.ord s.bits (s.ord e) n s.crossings
      bits := s.bits.erase (halfOf g n e) }
  else s

/-- Second loop body: `if order[e] == 0 { order[e] = order; set the opening half }`. -/
def refOpen (g : Graph) (n order : ℕ) (s : RefState) (e : ℕ) : RefState :=
  if s.ord e = 0 then
    { s with ord := Function.update s.ord e order, bits := insert (halfOf g n e) s.bits }
  else s

/-- One list occurrence: node `n` at position `order`. -/
def refOccurrence (g : Graph) (n order : ℕ) (s : RefState) : RefState :=
  (g.adj n).foldl (refOpen g n order) ((g.adj n).foldl (refClose g n) s)

/-- `for (&n, order) in list.iter().zip(1..)`. -/
def refRun (g : Graph) : ℕ → List ℕ → RefState → RefState
  | _, [], s => s
  | order, n :: rest, s => refRun g (order + 1) rest (refOccurrence g n order s)

/-- The state after the reset prologue: `open.fill(0)`, `order[e] = 0` for `e` in `adjacency`. -/
def initState (adjacency : List ℕ) (ord0 : ℕ → ℕ) : RefState :=
  { ord := adjacency.foldl (fun o e => Function.update o e 0) ord0
    bits := ∅
    crossings := 0 }

/-- `count_all_crossings_reference(c, list, offsets, adjacency, open)`. -/
def countReference (g : Graph) (adjacency : List ℕ) (ord0 : ℕ → ℕ) (list : List ℕ) :
    RefState :=
  refRun g 1 list (initState adjacency ord0)

/-! ## `count_indexed` -/

/-- Reference state plus the Fenwick `tree` (wrapping `u64` entries). -/
structure IdxState extends RefState where
  tree : ℕ → ZMod (2  ^ 64)

/-- `u64::checked_sub`. -/
def checkedSub (a b : ZMod (2 ^ 64)) : Option ℕ :=
  if b.val ≤ a.val then some (a.val - b.val) else none

/-- `wrap_crossings`: `value.wrapping_add(count as i32)`. -/
def wrapCrossings (value : BitVec 32) (count : ℕ) : BitVec 32 := value + BitVec.ofNat 32 count

/-- Tree update guarded by "the half is set": the body of both `for half in ..` loops over an
edge's two halves in `count_indexed`. -/
def idxHalf (len : ℕ) (d : ZMod (2 ^ 64)) (e : ℕ) (s : IdxState) (half : ℕ) :
    Option IdxState :=
  if half ∈ s.bits then do
    let t ← indexUpdate len d s.tree (s.ord e)
    pure { s with tree := t }
  else pure s

/-- `for half in e << 1..(e << 1) + 2` (halves `2e` and `2e + 1`). -/
def idxHalves (len : ℕ) (d : ZMod (2 ^ 64)) (e : ℕ) (s : IdxState) : Option IdxState :=
  (List.range' (2 * e) 2).foldlM (idxHalf len d e) s

/-- Loop 1: take every set half of the incident edge out of the tree. -/
def idxExclude (len : ℕ) (s : IdxState) (e : ℕ) : Option IdxState := idxHalves len (-1) e s

/-- Loop 2: `if edge_order > 0 { count = total - prefix(edge_order); crossings += count }`. -/
def idxQuery (len : ℕ) (total : ZMod (2 ^ 64)) (s : IdxState) (e : ℕ) : Option IdxState :=
  if 0 < s.ord e then do
    let p ← indexPrefix len s.tree (s.ord e)
    let count ← checkedSub total p
    pure { s with crossings := wrapCrossings s.crossings count }
  else pure s

/-- Loop 3: restore the incident halves, then apply the reference erasure of the closing half
(bit and tree unit, only when the bit is set). -/
def idxRestoreClose (g : Graph) (n len : ℕ) (s : IdxState) (e : ℕ) : Option IdxState := do
  let s ← idxHalves len 1 e s
  if 0 < s.ord e then
    let half := halfOf g n e
    if half ∈ s.bits then do
      let t ← indexUpdate len (-1) s.tree (s.ord e)
      pure { s with bits := s.bits.erase half, tree := t }
    else pure s
  else pure s

/-- Loop 4: `if order[e] == 0 { order[e] = order; set the half; index_update(tree, order) }`. -/
def idxOpen (g : Graph) (n len order : ℕ) (s : IdxState) (e : ℕ) : Option IdxState :=
  if s.ord e = 0 then do
    let t ← indexUpdate len 1 s.tree order
    pure
      { s with
        ord := Function.update s.ord e order
        bits := insert (halfOf g n e) s.bits
        tree := t }
  else pure s

/-- One list occurrence of `count_indexed`. `total` is `index_prefix(tree, list.len())`. -/
def idxOccurrence (g : Graph) (listLen len n order : ℕ) (s : IdxState) : Option IdxState := do
  let s ← (g.adj n).foldlM (idxExclude len) s
  let total ← indexPrefix len s.tree listLen
  let s ← (g.adj n).foldlM (idxQuery len total) s
  let s ← (g.adj n).foldlM (idxRestoreClose g n len) s
  (g.adj n).foldlM (idxOpen g n len order) s

/-- `for (&n, order) in list.iter().zip(1..)`. -/
def idxRun (g : Graph) (listLen len : ℕ) : ℕ → List ℕ → IdxState → Option IdxState
  | _, [], s => some s
  | order, n :: rest, s => do
    let s ← idxOccurrence g listLen len n order s
    idxRun g listLen len (order + 1) rest s

/-- `count_indexed(c, list, offsets, adjacency, open, tree)`; `none` is the `?` early return
(the caller then runs the reference). -/
def countIndexed (g : Graph) (adjacency : List ℕ) (ord0 : ℕ → ℕ) (list : List ℕ)
    (len : ℕ) : Option RefState :=
  (idxRun g list.length len 1 list { initState adjacency ord0 with tree := fun _ => 0 }).map
    IdxState.toRefState

/-- `count_with_policy` with `CHECK = false`: the dispatch between the two counters.
`len` is `tree.len()`. (The `list.len() <= 16` and `> i32::MAX` clauses only choose the
reference, which is always correct, so they are not modelled; see the theorem.) -/
def countWithPolicy (g : Graph) (adjacency : List ℕ) (ord0 : ℕ → ℕ) (list : List ℕ)
    (len : ℕ) : RefState :=
  if len ≤ list.length then countReference g adjacency ord0 list
  else
    match countIndexed g adjacency ord0 list len with
    | some r => r
    | none => countReference g adjacency ord0 list

/-! ## The Fenwick data structure

`Valid N t a` says that the `u64` array `t` (length `N`, slot 0 unused) is the Fenwick tree of the
array `a`: slot `j` holds the sum of `a` over `(j - lowbit j, j]`. Over `ZMod (2 ^ 64)` the
wrapping arithmetic of the Rust code is exact ring arithmetic. The lemmas are stated for any
additive group.
-/

section Bits

theorem bv_odd (x : BitVec 64) (h : x.getLsbD 0 = true) : x &&& -x = 1 := by bv_decide
theorem bv_even (x : BitVec 64) (h : x < 0x8000000000000000#64) :
    (2 * x) &&& -(2 * x) = 2 * (x &&& -x) := by bv_decide
theorem bv_prev (x : BitVec 64) : x &&& (x - 1) = x - (x &&& -x) := by bv_decide

theorem lowbit_eq_bv (i : ℕ) (hi : i < 2 ^ 64) :
    lowbit i = (BitVec.ofNat 64 i &&& -BitVec.ofNat 64 i).toNat := by
  have : i % 18446744073709551616 = i := Nat.mod_eq_of_lt hi
  simp [lowbit, BitVec.toNat_and, BitVec.toNat_neg, this]

theorem lowbit_odd (a : ℕ) (h : 2 * a + 1 < 2 ^ 64) : lowbit (2 * a + 1) = 1 := by
  rw [lowbit_eq_bv _ h, bv_odd]
  · rfl
  · simp [BitVec.getLsbD_ofNat]

theorem lowbit_even (a : ℕ) (h : a < 2 ^ 63) : lowbit (2 * a) = 2 * lowbit a := by
  have h1 : 2 * a < 2 ^ 64 := by omega
  rw [lowbit_eq_bv _ h1, lowbit_eq_bv _ (by omega)]
  have : BitVec.ofNat 64 (2 * a) = 2 * BitVec.ofNat 64 a := by
    rw [BitVec.ofNat_mul]; rfl
  rw [this, bv_even]
  · have hm : a % 18446744073709551616 = a := Nat.mod_eq_of_lt (by omega)
    simp [BitVec.toNat_mul, hm]
    have := Nat.and_le_left (n := a) (m := (18446744073709551616 - a) % 18446744073709551616)
    omega
  · rw [BitVec.lt_def]; simp [BitVec.toNat_ofNat]; omega

theorem lowbit_le (i : ℕ) : lowbit i ≤ i := Nat.and_le_left

theorem prevIdx_eq (i : ℕ) (hi : i < 2 ^ 64) : prevIdx i = i - lowbit i := by
  have h := congrArg BitVec.toNat (bv_prev (BitVec.ofNat 64 i))
  have hl := lowbit_eq_bv _ hi
  have hle := lowbit_le i
  have hm : i % 18446744073709551616 = i := Nat.mod_eq_of_lt hi
  simp [BitVec.toNat_and, BitVec.toNat_sub, BitVec.toNat_neg, hm] at h
  have hl' : lowbit i = i &&& (18446744073709551616 - i) % 18446744073709551616 := by
    simp [lowbit]
  rw [← hl'] at h
  have e : (18446744073709551616 - lowbit i + i) % 18446744073709551616 = i - lowbit i := by
    have : 18446744073709551616 - lowbit i + i = (i - lowbit i) + 18446744073709551616 := by omega
    rw [this, Nat.add_mod_right]; exact Nat.mod_eq_of_lt (by omega)
  rw [e] at h
  simpa [prevIdx, Nat.add_comm] using h

end Bits

theorem lowbit_pos (i : ℕ) (h0 : 0 < i) (h : i < 2 ^ 64) : 0 < lowbit i := by
  induction i using Nat.strong_induction_on with
  | _ i ih =>
    rcases Nat.even_or_odd' i with ⟨a, rfl | rfl⟩
    · have := lowbit_even a (by omega)
      have := ih a (by omega) (by omega) (by omega)
      omega
    · rw [lowbit_odd a h]; omega

theorem nextIdx_eq (i : ℕ) (h : i < 2 ^ 63) : nextIdx i = i + lowbit i := by
  have := lowbit_le i
  unfold nextIdx
  exact Nat.mod_eq_of_lt (by omega)

/-- The heart of the update proof. The slots an update from `i` touches are `i`, then those of the
update from `i + lowbit i`; slot `j` is touched by an update at `i` iff `j - lowbit j < i ≤ j`. -/
theorem touch_step (i : ℕ) (hi0 : 0 < i) (hi : i < 2 ^ 63) (j : ℕ) (hj0 : 0 < j)
    (hj : j < 2 ^ 63) :
    (j - lowbit j < i ∧ i ≤ j) ↔
      (j = i ∨ (j - lowbit j < i + lowbit i ∧ i + lowbit i ≤ j)) := by
  induction i using Nat.strong_induction_on generalizing j with
  | _ i ih =>
    have hli := lowbit_le i
    have hlj := lowbit_le j
    rcases Nat.even_or_odd' i with ⟨a, rfl | rfl⟩ <;>
      rcases Nat.even_or_odd' j with ⟨b, rfl | rfl⟩
    · have ha := lowbit_even a (by omega)
      have hb := lowbit_even b (by omega)
      have := ih a (by omega) (by omega) (by omega) b (by omega) (by omega)
      rw [ha, hb]
      omega
    · rw [lowbit_even a (by omega), lowbit_odd b (by omega)]
      omega
    · rw [lowbit_odd a (by omega), lowbit_even b (by omega)]
      omega
    · rw [lowbit_odd a (by omega), lowbit_odd b (by omega)]
      omega

theorem updateGo_eq {M : Type*} [AddCommGroup M] (len : ℕ) (hlen : len < 2 ^ 62) (d : M) :
    ∀ (fuel i : ℕ) (t : ℕ → M), len - i ≤ fuel → 0 < i →
      ∀ j, 0 < j → j < len →
        updateGo len d fuel i t j = t j + (if j - lowbit j < i ∧ i ≤ j then d else 0)
  | 0, i, t, hf, hi, j, hj0, hj => by
    have : ¬ (j - lowbit j < i ∧ i ≤ j) := by omega
    simp [updateGo, this]
  | fuel + 1, i, t, hf, hi, j, hj0, hj => by
    by_cases hil : i < len
    · have hnext := nextIdx_eq i (by omega)
      have hpos := lowbit_pos i hi (by omega)
      have ih := updateGo_eq len hlen d fuel (i + lowbit i) (Function.update t i (t i + d))
        (by omega) (by omega) j hj0 hj
      have key := touch_step i hi (by omega) j hj0 (by omega)
      simp only [updateGo, hil, ↓reduceIte, hnext, ih]
      by_cases hji : j = i
      · subst hji
        have h1 : ¬ (j - lowbit j < j + lowbit j ∧ j + lowbit j ≤ j) := by omega
        have h2 : (j - lowbit j < j ∧ j ≤ j) := by omega
        simp only [Function.update_self, h1, h2, ↓reduceIte, add_zero, and_self]
      · by_cases hs : j - lowbit j < i + lowbit i ∧ i + lowbit i ≤ j
        · have h2 : (j - lowbit j < i ∧ i ≤ j) := key.mpr (Or.inr hs)
          simp [Function.update_of_ne hji, hs, h2]
        · have h2 : ¬ (j - lowbit j < i ∧ i ≤ j) := fun h => by
            rcases key.mp h with h | h
            · exact hji h
            · exact hs h
          simp [Function.update_of_ne hji, hs, h2]
    · have : ¬ (j - lowbit j < i ∧ i ≤ j) := by omega
      simp [updateGo, hil, this]

theorem prefixGo_eq {M : Type*} [AddCommGroup M] (t a : ℕ → M) (N : ℕ) (hN : N < 2 ^ 62)
    (hv : ∀ j, 0 < j → j < N → t j = ∑ k ∈ Finset.Ioc (j - lowbit j) j, a k) :
    ∀ (fuel i : ℕ) (sum : M), i ≤ fuel → i < N →
      prefixGo t fuel i sum = sum + ∑ k ∈ Finset.Ioc 0 i, a k
  | 0, i, sum, hf, hi => by
    have : i = 0 := by omega
    subst this
    simp [prefixGo]
  | fuel + 1, i, sum, hf, hi => by
    by_cases hi0 : 0 < i
    · have hp := prevIdx_eq i (by omega)
      have hpos := lowbit_pos i hi0 (by omega)
      have ih := prefixGo_eq t a N hN hv fuel (i - lowbit i) (sum + t i) (by omega) (by omega)
      have e : prefixGo t (fuel + 1) i sum = prefixGo t fuel (i - lowbit i) (sum + t i) := by
        simp [prefixGo, hi0, hp]
      rw [e, ih, hv i hi0 hi, add_assoc, ← Finset.sum_Ioc_consecutive a
        (Nat.zero_le (i - lowbit i)) (Nat.sub_le i (lowbit i))]
      abel
    · have : i = 0 := by omega
      subst this
      simp [prefixGo]

/-- `Valid N t a`: `t` is the Fenwick tree of `a`. -/
def Valid {M : Type*} [AddCommGroup M] (N : ℕ) (t a : ℕ → M) : Prop :=
  ∀ j, 0 < j → j < N → t j = ∑ k ∈ Finset.Ioc (j - lowbit j) j, a k

theorem valid_zero {M : Type*} [AddCommGroup M] (N : ℕ) : Valid N (fun _ : ℕ => (0 : M)) (fun _ => 0) := by
  intro j _ _
  simp

theorem valid_congr {M : Type*} [AddCommGroup M] {N : ℕ} {t a b : ℕ → M} (h : ∀ k, a k = b k)
    (hv : Valid N t a) : Valid N t b := by
  have : a = b := funext h
  subst this
  exact hv

/-- `index_prefix` returns the prefix sum `a[1] + … + a[i]`. -/
theorem indexPrefix_eq {M : Type*} [AddCommGroup M] {N : ℕ} (hN : N < 2 ^ 62) {t a : ℕ → M}
    (hv : Valid N t a) {i : ℕ} (hi : i < N) :
    indexPrefix N t i = some (∑ k ∈ Finset.Ioc 0 i, a k) := by
  have := prefixGo_eq t a N hN hv i i 0 le_rfl hi
  simp [indexPrefix, Nat.not_le.mpr hi, this]

/-- `index_update` on a valid tree for a key in `1..N` adds `d` to `a[i]`. -/
theorem indexUpdate_valid {M : Type*} [AddCommGroup M] {N : ℕ} (hN : N < 2 ^ 62) {t a : ℕ → M}
    (hv : Valid N t a) (d : M) {i : ℕ} (hi0 : 0 < i) (hi : i < N) :
    ∃ t', indexUpdate N d t i = some t' ∧
      Valid N t' (fun k => a k + if k = i then d else 0) := by
  refine ⟨updateGo N d N i t, by simp [indexUpdate, Nat.pos_iff_ne_zero.mp hi0, Nat.not_le.mpr hi], ?_⟩
  intro j hj0 hj
  rw [updateGo_eq N hN d N i t (by omega) hi0 j hj0 hj, hv j hj0 hj,
    Finset.sum_add_distrib]
  congr 1
  simp only [Finset.sum_ite_eq', Finset.mem_Ioc]


/-! ## The tree indexes the set bits

`histZ ord S j` is the number of set halves of `S` whose edge has order `j`, as an element of
`ZMod (2 ^ 64)`. The Fenwick tree is valid for it. -/

/-- Number of halves in `S` whose edge has opening order `j`. -/
def histZ (ord : ℕ → ℕ) (S : Finset ℕ) (j : ℕ) : ZMod (2 ^ 64) :=
  ((S.filter (fun h => ord (h / 2) = j)).card : ZMod (2 ^ 64))

theorem histZ_erase (ord : ℕ → ℕ) (S : Finset ℕ) (x : ℕ) (hx : x ∈ S) (j : ℕ) :
    histZ ord (S.erase x) j = histZ ord S j + (if j = ord (x / 2) then -1 else 0) := by
  unfold histZ
  by_cases h : j = ord (x / 2)
  · have hmem : x ∈ S.filter (fun h => ord (h / 2) = j) := by simp [hx, h]
    have : (S.erase x).filter (fun h => ord (h / 2) = j)
        = (S.filter (fun h => ord (h / 2) = j)).erase x := by
      rw [Finset.filter_erase]
    rw [this, Finset.card_erase_of_mem hmem, Nat.cast_sub (Finset.card_pos.mpr ⟨x, hmem⟩)]
    simp [h, sub_eq_add_neg]
  · have : (S.erase x).filter (fun h => ord (h / 2) = j) = S.filter (fun h => ord (h / 2) = j) := by
      ext y
      simp only [Finset.mem_filter, Finset.mem_erase]
      constructor
      · rintro ⟨⟨_, hy⟩, hy2⟩; exact ⟨hy, hy2⟩
      · rintro ⟨hy, hy2⟩; exact ⟨⟨fun e => h (by rw [← hy2, e]), hy⟩, hy2⟩
    rw [this]; simp [h]

theorem histZ_insert (ord : ℕ → ℕ) (S : Finset ℕ) (x : ℕ) (hx : x ∉ S) (j : ℕ) :
    histZ ord (insert x S) j = histZ ord S j + (if j = ord (x / 2) then 1 else 0) := by
  unfold histZ
  rw [Finset.filter_insert]
  by_cases h : ord (x / 2) = j
  · have : x ∉ S.filter (fun h => ord (h / 2) = j) := by simp [hx]
    simp [h, Finset.card_insert_of_notMem this]
  · simp [h, Ne.symm h]

theorem histZ_congr (ord ord' : ℕ → ℕ) (S : Finset ℕ) (h : ∀ x ∈ S, ord' (x / 2) = ord (x / 2)) :
    histZ ord' S = histZ ord S := by
  funext j
  unfold histZ
  congr 2
  exact Finset.filter_congr (fun x hx => by rw [h x hx])

/-- Prefix of the histogram: the number of halves with order in `1..i`. -/
theorem sum_histZ (ord : ℕ → ℕ) (S : Finset ℕ) (hpos : ∀ x ∈ S, 1 ≤ ord (x / 2)) (i : ℕ) :
    ∑ k ∈ Finset.Ioc 0 i, histZ ord S k
      = ((S.filter (fun h => ord (h / 2) ≤ i)).card : ZMod (2 ^ 64)) := by
  have h := Finset.card_eq_sum_card_fiberwise (f := fun h => ord (h / 2))
    (s := S.filter (fun h => ord (h / 2) ≤ i)) (t := Finset.Ioc 0 i)
    (by intro x hx
        have hx := Finset.mem_filter.mp (Finset.mem_coe.mp hx)
        exact Finset.mem_coe.mpr (Finset.mem_Ioc.mpr ⟨hpos x hx.1, hx.2⟩))
  rw [h, Nat.cast_sum]
  refine Finset.sum_congr rfl (fun k hk => ?_)
  unfold histZ
  congr 2
  ext x
  simp only [Finset.mem_filter, Finset.mem_Ioc] at hk ⊢
  constructor
  · rintro ⟨hx, hxk⟩; exact ⟨⟨hx, by omega⟩, hxk⟩
  · rintro ⟨⟨hx, _⟩, hxk⟩; exact ⟨hx, hxk⟩

/-- Erasing a set half takes one unit out of the tree. -/
theorem valid_erase {len : ℕ} (hN : len < 2 ^ 62) {t : ℕ → ZMod (2 ^ 64)} {ord : ℕ → ℕ}
    {S : Finset ℕ} (hv : Valid len t (histZ ord S)) {x : ℕ} (hx : x ∈ S)
    (h0 : 0 < ord (x / 2)) (hl : ord (x / 2) < len) :
    ∃ t', indexUpdate len (-1) t (ord (x / 2)) = some t' ∧ Valid len t' (histZ ord (S.erase x)) := by
  obtain ⟨t', ht', hv'⟩ := indexUpdate_valid hN hv (-1) h0 hl
  refine ⟨t', ht', valid_congr (fun k => ?_) hv'⟩
  rw [histZ_erase ord S x hx]

theorem valid_insert {len : ℕ} (hN : len < 2 ^ 62) {t : ℕ → ZMod (2 ^ 64)} {ord : ℕ → ℕ}
    {S : Finset ℕ} (hv : Valid len t (histZ ord S)) {x : ℕ} (hx : x ∉ S)
    (h0 : 0 < ord (x / 2)) (hl : ord (x / 2) < len) :
    ∃ t', indexUpdate len 1 t (ord (x / 2)) = some t' ∧ Valid len t' (histZ ord (insert x S)) := by
  obtain ⟨t', ht', hv'⟩ := indexUpdate_valid hN hv 1 h0 hl
  refine ⟨t', ht', valid_congr (fun k => ?_) hv'⟩
  rw [histZ_insert ord S x hx]

/-! ## Edge-level steps of the two tree-maintenance loops -/

theorem idxHalves_eq (len : ℕ) (d : ZMod (2 ^ 64)) (e : ℕ) (s : IdxState) :
    idxHalves len d e s = (idxHalf len d e s (2 * e)).bind (fun s => idxHalf len d e s (2 * e + 1)) := by
  simp [idxHalves, List.range', List.foldlM]

theorem idxHalf_erase {len : ℕ} (hN : len < 2 ^ 62) {s : IdxState} {e x : ℕ} {U : Finset ℕ}
    (hv : Valid len s.tree (histZ s.ord U)) (hxe : x / 2 = e) (hU : ∀ h ∈ U, h ∈ s.bits)
    (hxU : x ∈ s.bits → x ∈ U)
    (hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < len) :
    ∃ s', idxHalf len (-1) e s x = some s' ∧ s'.toRefState = s.toRefState ∧
      Valid len s'.tree (histZ s.ord (U.erase x)) := by
  by_cases hx : x ∈ s.bits
  · obtain ⟨t', ht', hv'⟩ := valid_erase hN hv (hxU hx) (by have := hpos x hx; omega)
      (hpos x hx).2
    rw [hxe] at ht'
    refine ⟨{ s with tree := t' }, ?_, rfl, hv'⟩
    simp [idxHalf, hx, ht']
  · have : x ∉ U := fun h => hx (hU x h)
    refine ⟨s, by simp [idxHalf, hx], rfl, ?_⟩
    rw [Finset.erase_eq_of_notMem this]; exact hv

theorem idxHalf_insert {len : ℕ} (hN : len < 2 ^ 62) {s : IdxState} {e x : ℕ} {U U' : Finset ℕ}
    (hv : Valid len s.tree (histZ s.ord U)) (hxe : x / 2 = e) (hxU : x ∉ U)
    (hU' : ∀ h, h ∈ U' ↔ h ∈ U ∨ (h = x ∧ x ∈ s.bits))
    (hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < len) :
    ∃ s', idxHalf len 1 e s x = some s' ∧ s'.toRefState = s.toRefState ∧
      Valid len s'.tree (histZ s.ord U') := by
  by_cases hx : x ∈ s.bits
  · obtain ⟨t', ht', hv'⟩ := valid_insert hN hv hxU (by have := hpos x hx; omega)
      (hpos x hx).2
    rw [hxe] at ht'
    have hU'e : U' = insert x U := by
      ext h; rw [hU' h, Finset.mem_insert]; simp [hx, or_comm]
    refine ⟨{ s with tree := t' }, ?_, rfl, ?_⟩
    · simp [idxHalf, hx, ht']
    · rw [hU'e]; exact hv'
  · have hU'e : U' = U := by
      ext h; rw [hU' h]; simp [hx]
    refine ⟨s, by simp [idxHalf, hx], rfl, ?_⟩
    rw [hU'e]; exact hv

theorem idxExclude_step {len : ℕ} (hN : len < 2 ^ 62) {s : IdxState} {e : ℕ} {ps : Finset ℕ}
    (he : e ∉ ps)
    (hv : Valid len s.tree (histZ s.ord (s.bits.filter (fun h => h / 2 ∉ ps))))
    (hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < len) :
    ∃ s', idxExclude len s e = some s' ∧ s'.toRefState = s.toRefState ∧
      Valid len s'.tree (histZ s.ord (s.bits.filter (fun h => h / 2 ∉ insert e ps))) := by
  obtain ⟨s1, h1, c1, v1⟩ := idxHalf_erase hN hv (e := e) (x := 2 * e) (by omega)
    (fun h hh => (Finset.mem_filter.mp hh).1)
    (fun hx => Finset.mem_filter.mpr ⟨hx, by simpa using he⟩) hpos
  have hb : s1.bits = s.bits := by rw [show s1.bits = s1.toRefState.bits from rfl, c1]
  have ho : s1.ord = s.ord := by rw [show s1.ord = s1.toRefState.ord from rfl, c1]
  obtain ⟨s2, h2, c2, v2⟩ := idxHalf_erase hN (s := s1) (e := e) (x := 2 * e + 1)
    (U := (s.bits.filter (fun h => h / 2 ∉ ps)).erase (2 * e)) (by rw [ho]; exact v1) (by omega)
    (fun h hh => by rw [hb]; exact (Finset.mem_filter.mp (Finset.mem_of_mem_erase hh)).1)
    (fun hx => by
      rw [hb] at hx
      exact Finset.mem_erase.mpr ⟨by omega, Finset.mem_filter.mpr ⟨hx, by rwa [show (2 * e + 1) / 2 = e by omega]⟩⟩)
    (by rw [hb, ho]; exact hpos)
  refine ⟨s2, ?_, by rw [c2, c1], ?_⟩
  · rw [idxExclude, idxHalves_eq, h1]; exact h2
  · have ho2 : s2.ord = s.ord := by
      rw [show s2.ord = s2.toRefState.ord from rfl, c2]; exact ho
    have hset : ((s.bits.filter (fun h => h / 2 ∉ ps)).erase (2 * e)).erase (2 * e + 1)
        = s.bits.filter (fun h => h / 2 ∉ insert e ps) := by
      ext h
      simp only [Finset.mem_erase, Finset.mem_filter, Finset.mem_insert]
      have : h / 2 = e ↔ h = 2 * e ∨ h = 2 * e + 1 := by omega
      constructor
      · rintro ⟨h1, h2, h3, h4⟩
        exact ⟨h3, fun hh => hh.elim (fun hh => by omega) h4⟩
      · rintro ⟨h3, h4⟩
        refine ⟨by intro hh; apply h4; left; omega, by intro hh; apply h4; left; omega, h3, ?_⟩
        intro hh; exact h4 (Or.inr hh)
    rw [hset, ho] at v2
    exact v2

theorem idxRestore_step {len : ℕ} (hN : len < 2 ^ 62) {s : IdxState} {e : ℕ} {rest : Finset ℕ}
    (he : e ∈ rest)
    (hv : Valid len s.tree (histZ s.ord (s.bits.filter (fun h => h / 2 ∉ rest))))
    (hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < len) :
    ∃ s', idxHalves len 1 e s = some s' ∧ s'.toRefState = s.toRefState ∧
      Valid len s'.tree (histZ s.ord (s.bits.filter (fun h => h / 2 ∉ rest.erase e))) := by
  obtain ⟨s1, h1, c1, v1⟩ := idxHalf_insert hN hv (e := e) (x := 2 * e) (by omega)
    (by simp [he]) (U' := s.bits.filter (fun h => h / 2 ∉ rest ∨ h = 2 * e))
    (fun h => by
      simp only [Finset.mem_filter]
      constructor
      · rintro ⟨hb, hh | hh⟩
        · exact Or.inl ⟨hb, hh⟩
        · exact Or.inr ⟨hh, hh ▸ hb⟩
      · rintro (⟨hb, hh⟩ | ⟨hh, hb⟩)
        · exact ⟨hb, Or.inl hh⟩
        · exact ⟨hh ▸ hb, Or.inr hh⟩) hpos
  have hb : s1.bits = s.bits := by rw [show s1.bits = s1.toRefState.bits from rfl, c1]
  have ho : s1.ord = s.ord := by rw [show s1.ord = s1.toRefState.ord from rfl, c1]
  obtain ⟨s2, h2, c2, v2⟩ := idxHalf_insert hN (s := s1) (e := e) (x := 2 * e + 1)
    (U := s.bits.filter (fun h => h / 2 ∉ rest ∨ h = 2 * e))
    (U' := s.bits.filter (fun h => h / 2 ∉ rest ∨ h = 2 * e ∨ h = 2 * e + 1))
    (by rw [ho]; exact v1) (by omega)
    (by
      simp only [Finset.mem_filter]
      rintro ⟨_, h3 | h3⟩
      · exact h3 (by rwa [show (2 * e + 1) / 2 = e by omega])
      · omega)
    (fun h => by
      simp only [Finset.mem_filter, hb]
      constructor
      · rintro ⟨hb, hh | hh | hh⟩
        · exact Or.inl ⟨hb, Or.inl hh⟩
        · exact Or.inl ⟨hb, Or.inr hh⟩
        · exact Or.inr ⟨hh, hh ▸ hb⟩
      · rintro (⟨hb, hh | hh⟩ | ⟨hh, hb⟩)
        · exact ⟨hb, Or.inl hh⟩
        · exact ⟨hb, Or.inr (Or.inl hh)⟩
        · exact ⟨hh ▸ hb, Or.inr (Or.inr hh)⟩)
    (by rw [hb, ho]; exact hpos)
  refine ⟨s2, ?_, by rw [c2, c1], ?_⟩
  · rw [idxHalves_eq, h1]; exact h2
  · have hset : s.bits.filter (fun h => h / 2 ∉ rest ∨ h = 2 * e ∨ h = 2 * e + 1)
        = s.bits.filter (fun h => h / 2 ∉ rest.erase e) := by
      apply Finset.filter_congr
      intro h _
      have : h / 2 = e ↔ h = 2 * e ∨ h = 2 * e + 1 := by omega
      simp only [Finset.mem_erase]
      constructor
      · rintro (hh | hh | hh)
        · exact fun h2 => hh h2.2
        · exact fun h2 => h2.1 (by omega)
        · exact fun h2 => h2.1 (by omega)
      · intro hh
        by_cases h3 : h / 2 ∈ rest
        · right
          have : h / 2 = e := by
            by_contra hne
            exact hh ⟨hne, h3⟩
          omega
        · left; exact h3
    rw [hset, ho] at v2
    exact v2

theorem idxRestoreClose_step {len : ℕ} (hN : len < 2 ^ 62) (g : Graph) (n : ℕ) {s : IdxState}
    {e : ℕ} {rest : Finset ℕ} (he : e ∈ rest)
    (hv : Valid len s.tree (histZ s.ord (s.bits.filter (fun h => h / 2 ∉ rest))))
    (hpos : ∀ h ∈ s.bits, 1 ≤ s.ord (h / 2) ∧ s.ord (h / 2) < len) :
    ∃ s', idxRestoreClose g n len s e = some s' ∧ s'.ord = s.ord ∧
      s'.crossings = s.crossings ∧
      s'.bits = (if 0 < s.ord e then s.bits.erase (halfOf g n e) else s.bits) ∧
      Valid len s'.tree (histZ s.ord (s'.bits.filter (fun h => h / 2 ∉ rest.erase e))) := by
  obtain ⟨s1, h1, c1, v1⟩ := idxRestore_step hN he hv hpos
  have hb : s1.bits = s.bits := by rw [show s1.bits = s1.toRefState.bits from rfl, c1]
  have ho : s1.ord = s.ord := by rw [show s1.ord = s1.toRefState.ord from rfl, c1]
  have hc : s1.crossings = s.crossings := by
    rw [show s1.crossings = s1.toRefState.crossings from rfl, c1]
  have hhalf : (halfOf g n e) / 2 = e := by unfold halfOf; split_ifs <;> omega
  by_cases h0 : 0 < s.ord e
  · by_cases hm : halfOf g n e ∈ s.bits
    · have hmU : halfOf g n e ∈ s.bits.filter (fun h => h / 2 ∉ rest.erase e) :=
        Finset.mem_filter.mpr ⟨hm, by rw [hhalf]; simp⟩
      obtain ⟨t', ht', hv'⟩ := valid_erase hN v1 hmU (by rw [hhalf]; exact h0)
        (by rw [hhalf]; exact (hhalf ▸ (hpos _ hm).2))
      rw [hhalf] at ht'
      refine ⟨{ s1 with bits := s1.bits.erase (halfOf g n e), tree := t' }, ?_, ho, hc, ?_, ?_⟩
      · simp [idxRestoreClose, h1, ho, hb, h0, hm, ht']
      · simp [hb, h0]
      · show Valid len t' (histZ s.ord (Finset.filter _ (s1.bits.erase _)))
        rw [hb, Finset.filter_erase]
        exact hv'
    · refine ⟨s1, ?_, ho, hc, ?_, ?_⟩
      · simp [idxRestoreClose, h1, ho, hb, h0, hm]
      · simp [hb, h0, Finset.erase_eq_of_notMem hm]
      · rw [hb]; exact v1
  · refine ⟨s1, ?_, ho, hc, ?_, ?_⟩
    · simp [idxRestoreClose, h1, ho, h0]
    · simp [hb, h0]
    · rw [hb]; exact v1

end GvpProofs.Circo.Fenwick
