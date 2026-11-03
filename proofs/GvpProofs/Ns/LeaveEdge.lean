import Mathlib.Data.Finset.Max

/-!
# `leave_edge` on an ordered index of negative cut values (issue #731)

C's `leave_edge` (`lib/common/ns.c`) walks the `Tree_edge` array cyclically from the cursor `S_i`,
remembers the most negative cut value it meets, and returns once it has seen `Search_size`
(30) negative ones, leaving `S_i` on the slot where it stopped. On a graph with a few negative
cut values it walks the whole array at every pivot.

The port keeps an ordered index of the slots whose cut value is negative
(`crates/graphviz-pure-layout/src/common/ns/leave.rs`) and visits only those. This file proves
that the indexed search returns the same slot and leaves the cursor on the same slot as the scan,
for every cut-value array, start and cutoff, and that an index kept up to date by point edits
stays right through a whole run of pivots.

## What is modelled

* `val i` is `ND_cutvalue(Tree_edge[i])`. Only its sign and its order are read, so `Int` stands
  for `i32` without any overflow case (the wrapping happens where the cut values are computed,
  before this function reads them).
* `leaveRef` is the scan, loop for loop (`scanRange` is one `while (S_i < hi)` loop).
* `leaveIdx` is the indexed search. The index is a `Finset` of slots and `next p` is its
  successor query (the Rust `NegSlots::next_from`); a hypothesis `NextSpec` says what `next`
  answers, so the theorem holds for any correct ordered set, not for one data structure.
* A pivot changes cut values at some slots and the index is edited at the same slots (`setSlot`).
  `run_idx_eq_run_ref` quantifies over every pivot, i.e. over every way the edits may depend on
  the chosen slot and on the cut values.

What is not modelled is listed in the pull request ("Model vs Rust").
-/

namespace GvpProofs.Ns.LeaveEdge

/-- Cut values by tree slot: `val i = ND_cutvalue(Tree_edge[i])`. -/
abbrev Cuts := Nat → Int

/-- The scan's state: the best slot so far (`rv`) and the negatives seen (`cnt`). -/
structure Acc where
  rv : Option Nat
  cnt : Nat

/-- C's replacement rule: the first negative becomes `rv`; a later one replaces it only when
`ND_cutvalue(rv) > ND_cutvalue(f)`, strictly, so on a tie the first one met stays. -/
def pick (val : Cuts) : Option Nat → Nat → Nat
  | none, i => i
  | some r, i => if val r > val i then i else r

/-- The body of C's loop for a slot with a negative cut value. -/
def visit (val : Cuts) (a : Acc) (i : Nat) : Acc :=
  { rv := some (pick val a.rv i), cnt := a.cnt + 1 }

/-- How a loop ends: `return rv` with `S_i = p`, or it ran off the end of its range. -/
inductive Out where
  | stop (rv : Option Nat) (p : Nat)
  | done (a : Acc)

/-- One of C's loops: `while (S_i < hi) { f = Tree_edge[S_i]; if (cutvalue(f) < 0) { ...;
if (++cnt >= Search_size) return rv; } S_i++; }`, started at `S_i = i`. -/
def scanRange (val : Cuts) (k hi : Nat) (i : Nat) (a : Acc) : Out :=
  if i < hi then
    if val i < 0 then
      if k ≤ (visit val a i).cnt then .stop (visit val a i).rv i
      else scanRange val k hi (i + 1) (visit val a i)
    else scanRange val k hi (i + 1) a
  else .done a
termination_by hi - i

/-- C: `lib/common/ns.c:leave_edge`. `L` is `Tree_edge.size`, `s` is `S_i` on entry and `k` is
`Search_size`. Returns the chosen slot and `S_i` on exit. -/
def leaveRef (val : Cuts) (k L s : Nat) : Option Nat × Nat :=
  match scanRange val k L s ⟨none, 0⟩ with
  | .stop rv p => (rv, p)
  | .done a =>
    if 0 < s then
      match scanRange val k s 0 a with
      | .stop rv p => (rv, p)
      | .done a' => (a'.rv, s)
    else (a.rv, L)

/-- The same loop over the index: `next p` is the least indexed slot at or above `p`. The fuel
only makes the recursion structural; it is the length of the range, which the loop never exceeds
because each step moves `p` past the slot it visited. -/
def idxRange (val : Cuts) (k : Nat) (next : Nat → Option Nat) (hi : Nat) :
    (fuel p : Nat) → Acc → Out
  | 0, _, a => .done a
  | fuel + 1, p, a =>
    match next p with
    | none => .done a
    | some q =>
      if q < hi then
        if k ≤ (visit val a q).cnt then .stop (visit val a q).rv q
        else idxRange val k next hi fuel (q + 1) (visit val a q)
      else .done a

/-- The indexed `leave_edge`: the same two ranges as `leaveRef`, visiting indexed slots only. -/
def leaveIdx (val : Cuts) (k : Nat) (next : Nat → Option Nat) (L s : Nat) : Option Nat × Nat :=
  match idxRange val k next L (L - s) s ⟨none, 0⟩ with
  | .stop rv p => (rv, p)
  | .done a =>
    if 0 < s then
      match idxRange val k next s s 0 a with
      | .stop rv p => (rv, p)
      | .done a' => (a'.rv, s)
    else (a.rv, L)

/-! ## The index -/

/-- The index agrees with the cut values: slot `i` is in it exactly when `i` is a tree slot
with a negative cut value. -/
def Inv (val : Cuts) (N : Finset Nat) (L : Nat) : Prop :=
  ∀ i, i ∈ N ↔ (i < L ∧ val i < 0)

/-- What `next` must answer for the set `N`: the least member at or above `p`, if any. -/
structure NextSpec (N : Finset Nat) (next : Nat → Option Nat) : Prop where
  some_least : ∀ p q, next p = some q → p ≤ q ∧ q ∈ N ∧ ∀ r ∈ N, p ≤ r → q ≤ r
  none_above : ∀ p, next p = none → ∀ r ∈ N, r < p

/-- Such a `next` exists for every finite set (the bitset of the port is one implementation). -/
noncomputable def succAt (N : Finset Nat) (p : Nat) : Option Nat := by
  classical
  exact if h : (N.filter (p ≤ ·)).Nonempty then some ((N.filter (p ≤ ·)).min' h) else none

theorem succAt_spec (N : Finset Nat) : NextSpec N (succAt N) := by
  classical
  constructor
  · intro p q hq
    unfold succAt at hq
    split at hq
    · rename_i h
      have hq' : (N.filter (p ≤ ·)).min' h = q := Option.some.inj hq
      have hmem := Finset.min'_mem _ h
      rw [hq'] at hmem
      rw [Finset.mem_filter] at hmem
      refine ⟨hmem.2, hmem.1, ?_⟩
      intro r hr hpr
      rw [← hq']
      exact Finset.min'_le _ _ (Finset.mem_filter.mpr ⟨hr, hpr⟩)
    · simp at hq
  · intro p hp r hr
    unfold succAt at hp
    split at hp
    · simp at hp
    · rename_i h
      by_contra hlt
      exact h ⟨r, Finset.mem_filter.mpr ⟨hr, Nat.not_lt.mp hlt⟩⟩

/-! ## The indexed search equals the scan -/

/-- A range without a negative slot is walked to its end and changes nothing. -/
theorem scanRange_nonneg (val : Cuts) (k hi : Nat) :
    ∀ n i (a : Acc), hi - i = n → (∀ r, i ≤ r → r < hi → ¬ val r < 0) →
      scanRange val k hi i a = .done a := by
  intro n
  induction n with
  | zero =>
    intro i a hn _
    rw [scanRange]
    have : ¬ i < hi := by omega
    simp [this]
  | succ n ih =>
    intro i a hn hall
    rw [scanRange]
    have hi' : i < hi := by omega
    have hnn : ¬ val i < 0 := hall i (Nat.le_refl i) hi'
    simp only [hi', hnn, ite_true, ite_false]
    exact ih (i + 1) a (by omega) (fun r hr hrh => hall r (by omega) hrh)

/-- Walking over slots without a negative value up to `q` lands on `q` with the state unchanged. -/
theorem scanRange_skip (val : Cuts) (k hi : Nat) :
    ∀ n i q (a : Acc), q - i = n → i ≤ q → q ≤ hi → (∀ r, i ≤ r → r < q → ¬ val r < 0) →
      scanRange val k hi i a = scanRange val k hi q a := by
  intro n
  induction n with
  | zero =>
    intro i q a hn hiq _ _
    have : i = q := by omega
    rw [this]
  | succ n ih =>
    intro i q a hn hiq hq hall
    have hiq' : i < q := by omega
    have hi' : i < hi := by omega
    have hnn : ¬ val i < 0 := hall i (Nat.le_refl i) hiq'
    rw [scanRange]
    simp only [hi', hnn, ite_true, ite_false]
    exact ih (i + 1) q a (by omega) (by omega) hq (fun r hr hrq => hall r (by omega) hrq)

/-- One range: visiting only indexed slots is walking the whole range. -/
theorem idxRange_eq_scanRange (val : Cuts) (k hi : Nat) (N : Finset Nat) (L : Nat)
    (next : Nat → Option Nat) (hinv : Inv val N L) (hnext : NextSpec N next) (hhi : hi ≤ L) :
    ∀ fuel p (a : Acc), hi - p ≤ fuel →
      idxRange val k next hi fuel p a = scanRange val k hi p a := by
  intro fuel
  induction fuel with
  | zero =>
    intro p a hf
    have hp : ¬ p < hi := by omega
    rw [scanRange]
    simp [idxRange, hp]
  | succ fuel ih =>
    intro p a hf
    rw [idxRange]
    cases hn : next p with
    | none =>
      symm
      refine scanRange_nonneg val k hi (hi - p) p a rfl ?_
      intro r hpr hrh hneg
      have hmem : r ∈ N := (hinv r).mpr ⟨by omega, hneg⟩
      have := hnext.none_above p hn r hmem
      omega
    | some q =>
      obtain ⟨hpq, hqN, hleast⟩ := hnext.some_least p q hn
      have hqneg : val q < 0 := ((hinv q).mp hqN).2
      by_cases hq : q < hi
      · have hskip : scanRange val k hi p a = scanRange val k hi q a :=
          scanRange_skip val k hi (q - p) p q a rfl hpq (by omega) (by
            intro r hpr hrq hneg
            have hmem : r ∈ N := (hinv r).mpr ⟨by omega, hneg⟩
            have := hleast r hmem hpr
            omega)
        rw [hskip, scanRange]
        by_cases hk : k ≤ (visit val a q).cnt
        · simp [hq, hqneg, hk]
        · simp only [hq, hqneg, hk, ite_true, ite_false]
          exact ih (q + 1) (visit val a q) (by omega)
      · symm
        simp only [hq, ite_false]
        refine scanRange_nonneg val k hi (hi - p) p a rfl ?_
        intro r hpr hrh hneg
        have hmem : r ∈ N := (hinv r).mpr ⟨by omega, hneg⟩
        have := hleast r hmem hpr
        omega

/-- **The indexed `leave_edge` returns the slot the scan returns, and leaves the cursor where the
scan leaves it**, for every cut-value array, cursor and cutoff, whenever the index agrees with the
cut values and answers successor queries. -/
theorem leaveIdx_eq_leaveRef (val : Cuts) (k L s : Nat) (N : Finset Nat)
    (next : Nat → Option Nat) (hinv : Inv val N L) (hnext : NextSpec N next) (hs : s ≤ L) :
    leaveIdx val k next L s = leaveRef val k L s := by
  unfold leaveIdx leaveRef
  rw [idxRange_eq_scanRange val k L N L next hinv hnext (Nat.le_refl L) (L - s) s ⟨none, 0⟩
    (Nat.le_refl _)]
  cases hscan : scanRange val k L s ⟨none, 0⟩ with
  | stop rv p => rfl
  | done a =>
    by_cases h0 : 0 < s
    · simp only [h0, ite_true]
      rw [idxRange_eq_scanRange val k s N L next hinv hnext hs s 0 a (by omega)]
    · simp only [h0, ite_false]

/-! ## Keeping the index current -/

/-- A cut value changes at slot `i`; the index follows: the slot is in it exactly when the new
value is negative. In the port this is `NegSlots::set` called after `set_cutvalue`. -/
def setIdx (N : Finset Nat) (i : Nat) (x : Int) : Finset Nat :=
  if x < 0 then insert i N else N.erase i

/-- One edit keeps the index right. -/
theorem inv_set {val : Cuts} {N : Finset Nat} {L i : Nat} (hinv : Inv val N L) (hi : i < L)
    (x : Int) : Inv (Function.update val i x) (setIdx N i x) L := by
  intro j
  by_cases hji : j = i
  · subst hji
    by_cases hx : x < 0
    · simp [setIdx, hx, hi]
    · simp [setIdx, hx]
  · have hne : j ≠ i := hji
    have h := hinv j
    by_cases hx : x < 0
    · simp [setIdx, hx, hne, h]
    · simp [setIdx, hx, hne, h]

/-- The cut values after a list of `(slot, new value)` edits, in order. -/
def applyVal : Cuts → List (Nat × Int) → Cuts
  | v, [] => v
  | v, (i, x) :: es => applyVal (Function.update v i x) es

/-- The index after the same edits. -/
def applyIdx : Finset Nat → List (Nat × Int) → Finset Nat
  | N, [] => N
  | N, (i, x) :: es => applyIdx (setIdx N i x) es

theorem inv_apply {L : Nat} (es : List (Nat × Int)) :
    ∀ {val : Cuts} {N : Finset Nat}, Inv val N L → (∀ e ∈ es, e.1 < L) →
      Inv (applyVal val es) (applyIdx N es) L := by
  induction es with
  | nil => intro val N h _; exact h
  | cons e es ih =>
    intro val N h hes
    obtain ⟨i, x⟩ := e
    exact ih (inv_set h (hes (i, x) (List.mem_cons_self ..)) x)
      (fun e he => hes e (List.mem_cons_of_mem _ he))

/-! ## The cursor stays in the array -/

theorem scanRange_stop_lt (val : Cuts) (k hi : Nat) :
    ∀ n i (a : Acc) rv p, hi - i = n → scanRange val k hi i a = .stop rv p → p < hi := by
  intro n
  induction n with
  | zero =>
    intro i a rv p hn h
    have hp : ¬ i < hi := by omega
    rw [scanRange] at h
    simp [hp] at h
  | succ n ih =>
    intro i a rv p hn h
    have hi' : i < hi := by omega
    rw [scanRange] at h
    by_cases hneg : val i < 0
    · by_cases hk : k ≤ (visit val a i).cnt
      · simp only [hi', hneg, hk, ite_true] at h
        have := Out.stop.inj h
        omega
      · simp only [hi', hneg, hk, ite_true, ite_false] at h
        exact ih (i + 1) _ rv p (by omega) h
    · simp only [hi', hneg, ite_true, ite_false] at h
      exact ih (i + 1) a rv p (by omega) h

theorem leaveRef_cursor_le (val : Cuts) (k L s : Nat) (hs : s ≤ L) :
    (leaveRef val k L s).2 ≤ L := by
  unfold leaveRef
  cases h1 : scanRange val k L s ⟨none, 0⟩ with
  | stop rv p =>
    have := scanRange_stop_lt val k L (L - s) s _ rv p rfl h1
    simp only
    omega
  | done a =>
    by_cases h0 : 0 < s
    · simp only [h0, ite_true]
      cases h2 : scanRange val k s 0 a with
      | stop rv p =>
        have := scanRange_stop_lt val k s (s - 0) 0 a rv p rfl h2
        simp only
        omega
      | done a' => simpa using hs
    · simp only [h0, ite_false]
      exact Nat.le_refl L

/-! ## A whole run -/

/-- The slots `leave_edge` returned and the cursor it left, at each of `n` pivots. At each pivot
the cut values change by `edits val r`: any list of in-range `(slot, value)` edits, which may
depend on the cut values and on the slot `r` that was chosen (this is `update`: the cycle of the
entering edge, then the exchange of the two tree edges). -/
def runRef (k L : Nat) (edits : Cuts → Option Nat → List (Nat × Int)) :
    Nat → Cuts → Nat → List (Option Nat × Nat)
  | 0, _, _ => []
  | n + 1, val, s =>
    let r := leaveRef val k L s
    r :: runRef k L edits n (applyVal val (edits val r.1)) r.2

/-- The same run with the indexed search, the index edited at the same slots. `nextOf N` is the
successor query of the index holding `N`. -/
def runIdx (k L : Nat) (edits : Cuts → Option Nat → List (Nat × Int))
    (nextOf : Finset Nat → Nat → Option Nat) :
    Nat → Cuts → Finset Nat → Nat → List (Option Nat × Nat)
  | 0, _, _, _ => []
  | n + 1, val, N, s =>
    let r := leaveIdx val k (nextOf N) L s
    r :: runIdx k L edits nextOf n (applyVal val (edits val r.1)) (applyIdx N (edits val r.1)) r.2

/-- **A run of pivots on the index chooses the same leaving slot, and leaves the cursor on the
same slot, at every pivot, as a run of C's scans**, whatever the pivots do to the cut values, as
long as the index is edited wherever a cut value changes and starts right. -/
theorem run_idx_eq_run_ref (k L : Nat) (edits : Cuts → Option Nat → List (Nat × Int))
    (hedits : ∀ val r, ∀ e ∈ edits val r, e.1 < L)
    (nextOf : Finset Nat → Nat → Option Nat) (hnext : ∀ N, NextSpec N (nextOf N)) :
    ∀ n (val : Cuts) (N : Finset Nat) (s : Nat), Inv val N L → s ≤ L →
      runIdx k L edits nextOf n val N s = runRef k L edits n val s := by
  intro n
  induction n with
  | zero => intro val N s _ _; rfl
  | succ n ih =>
    intro val N s hinv hs
    have h1 := leaveIdx_eq_leaveRef val k L s N (nextOf N) hinv (hnext N) hs
    simp only [runIdx, runRef, h1]
    congr 1
    exact ih _ _ _ (inv_apply _ hinv (hedits val _)) (leaveRef_cursor_le val k L s hs)

/-! ## Worked cases

The scan, run by hand on small arrays, so the model can be read against C: the tie rule, the
cutoff, the wrap, and where the cursor is left. -/

/-- A cut-value array as a list; slots past its end read as 0. -/
def ofList (l : List Int) : Cuts := fun i => l.getD i 0

-- cutoff 2 from slot 1: slots 2 and 3 are the first two negatives, tied at -3; the first stays
example : leaveRef (ofList [-1, 0, -3, -3, 5]) 2 5 1 = (some 2, 3) := by
  simp [leaveRef, scanRange, visit, pick, ofList]

-- cutoff 2 from slot 1 on [-1, 0, -3, -4, 5]: the second negative is smaller and wins
example : leaveRef (ofList [-1, 0, -3, -4, 5]) 2 5 1 = (some 3, 3) := by
  simp [leaveRef, scanRange, visit, pick, ofList]

-- one negative only: the scan wraps to the start (slot 0 is the minimum) and ends on `s`
example : leaveRef (ofList [-9, 0, 0, -1, 5]) 30 5 2 = (some 0, 2) := by
  simp [leaveRef, scanRange, visit, pick, ofList]

-- no negatives, from slot 0: the cursor ends at `L`, as in C
example : leaveRef (ofList [0, 1, 2]) 30 3 0 = (none, 3) := by
  simp [leaveRef, scanRange, ofList]

end GvpProofs.Ns.LeaveEdge
