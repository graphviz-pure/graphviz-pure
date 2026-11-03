import Mathlib

open Finset

/-!
# circo: the incremental crossing delta equals the full recount (#730)

Model of `crates/graphviz-pure-layout/src/circogen/blockpath.rs` (`count_all_crossings`,
`Delta::phi`, `apply_delta`), not extracted code: the Aeneas binary was not installed when this
was written. See `docs/perf/circo-crossings.md` for the argument in prose and the list of what
the model leaves to the runtime oracle (`circo-check`).

* `crossings_formula`: the pair count equals `sum fd * span - sum C(d, 2)`.
* `crossings_move`: putting node `x` at cut `c` gives `Kc + phi c`; `phi` is `Delta::phi`.
* `fast_eq_recount`: wrapped `i32` old total plus wrapped delta is the wrapped recount.
-/

-- The proof scripts try several closers per case; the linters flag the ones that were not needed.
set_option linter.unusedTactic false
set_option linter.unreachableTactic false
set_option linter.unusedSimpArgs false
set_option linter.unusedVariables false
set_option linter.unusedSectionVars false
set_option linter.unnecessarySeqFocus false

namespace GvpProofs.Circo

variable {V : Type*} [Fintype V] [DecidableEq V]
variable (adj : V → V → Prop) [DecidableRel adj] (p : V → ℕ)

/-- Neighbours of `z` placed later in the list (`fd` in the Rust). -/
def fwd (z : V) : ℕ := ∑ w, if adj z w ∧ p z < p w then 1 else 0
/-- Neighbours of `z` placed earlier (`d` in the Rust). -/
def bck (z : V) : ℕ := ∑ w, if adj w z ∧ p w < p z then 1 else 0
/-- Edges with one end before `z` and one after (`span`). -/
def spn (z : V) : ℕ := ∑ u, ∑ v, if adj u v ∧ p u < p z ∧ p z < p v then 1 else 0
/-- Sum of `f` over the nodes placed before position `t`. -/
def pre (f : V → ℕ) (t : ℕ) : ℕ := ∑ w, if p w < t then f w else 0

/-- Pair count: edges `(u,v)`, `(u',v')`, both forward, with `u' ` strictly inside `(u,v)` and
`v' ≠ v`. This is what `count_all_crossings` computes on a permutation of a strict block. -/
def crossings : ℕ :=
  ∑ u, ∑ v, ∑ u', ∑ v',
    if (adj u v ∧ p u < p v) ∧ (adj u' v' ∧ p u' < p v') ∧ p u < p u' ∧ p u' < p v ∧ v' ≠ v
    then 1 else 0

theorem pre_fwd (t : ℕ) :
    pre p (fwd adj p) t = ∑ u, ∑ v, if (adj u v ∧ p u < p v) ∧ p u < t then 1 else 0 := by
  unfold pre fwd
  refine Finset.sum_congr rfl fun u _ => ?_
  by_cases h : p u < t <;> simp [h]

theorem pre_bck (t : ℕ) :
    pre p (bck adj p) t = ∑ v, ∑ u, if (adj u v ∧ p u < p v) ∧ p v < t then 1 else 0 := by
  unfold pre bck
  refine Finset.sum_congr rfl fun v _ => ?_
  by_cases h : p v < t <;> simp [h]


/-- Edges starting before `a` and ending at or after `b`. -/
def twin (a b : ℕ) : ℕ := ∑ u, ∑ v, if adj u v ∧ p u < a ∧ b ≤ p v then 1 else 0

theorem twin_add (a b : ℕ) (h1 : a ≤ b) (h2 : b ≤ a + 1) :
    twin adj p a b + pre p (bck adj p) b = pre p (fwd adj p) a := by
  rw [pre_fwd, pre_bck, Finset.sum_comm (f := fun v u => if (adj u v ∧ p u < p v) ∧ p v < b then 1 else 0)]
  unfold twin
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun u _ => ?_
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun v _ => ?_
  split_ifs <;> simp_all <;> omega

/-- Edges across the cut at `t`: `cut°(c) = FP c - DP c`. -/
theorem cut_eq (t : ℕ) :
    twin adj p t t + pre p (bck adj p) t = pre p (fwd adj p) t :=
  twin_add adj p t t le_rfl (Nat.le_succ t)

/-- Edges spanning `z`: `span(z) = FP(p z) - DP(p z + 1)`. -/
theorem spn_eq (z : V) :
    spn adj p z + pre p (bck adj p) (p z + 1) = pre p (fwd adj p) (p z) := by
  have := twin_add adj p (p z) (p z + 1) (Nat.le_succ _) le_rfl
  rwa [show twin adj p (p z) (p z + 1) = spn adj p z by
    unfold twin spn
    refine Finset.sum_congr rfl fun u _ => Finset.sum_congr rfl fun v _ => ?_
    by_cases h : adj u v <;> simp [h]] at this

/-- Unordered pairs of distinct keys: `C(n, 2)`. -/
theorem pairs_lt (s : Finset V) (hinj : ∀ u v, p u = p v → u = v) :
    ∑ u ∈ s, ∑ u' ∈ s, (if p u < p u' then 1 else 0) = s.card.choose 2 := by
  have hsw : ∑ u ∈ s, ∑ u' ∈ s, (if p u < p u' then 1 else 0)
      = ∑ u ∈ s, ∑ u' ∈ s, (if p u' < p u then 1 else 0) := Finset.sum_comm
  have htot : ∑ u ∈ s, ∑ u' ∈ s, ((if p u < p u' then 1 else 0) + (if p u' < p u then 1 else 0)
      + (if u = u' then 1 else 0)) = s.card * s.card := by
    have : ∀ u u' : V, ((if p u < p u' then 1 else 0) + (if p u' < p u then 1 else 0)
        + (if u = u' then 1 else 0) : ℕ) = 1 := by
      intro u u'
      by_cases h : u = u'
      · subst h; simp
      · have : p u ≠ p u' := fun e => h (hinj _ _ e)
        have h1 : ¬ u = u' := h
        rcases Nat.lt_or_gt_of_ne this with h | h <;> simp [h, h1, not_lt_of_gt h]
    simp [this, Finset.sum_const]
  simp only [Finset.sum_add_distrib] at htot
  have hdiag : ∑ u ∈ s, ∑ u' ∈ s, (if u = u' then 1 else 0) = s.card := by simp
  rw [Nat.choose_two_right]
  have : 2 * ∑ u ∈ s, ∑ u' ∈ s, (if p u < p u' then 1 else 0) + s.card = s.card * s.card := by
    omega
  have h2 : s.card * (s.card - 1) = 2 * ∑ u ∈ s, ∑ u' ∈ s, (if p u < p u' then 1 else 0) := by
    cases hc : s.card with
    | zero => simp_all
    | succ n => simp_all [Nat.mul_succ]
  rw [h2]; simp


theorem fwd_split (u' v : V) (h : p u' < p v) :
    (∑ v', if adj u' v' ∧ p u' < p v' ∧ v' ≠ v then 1 else 0) + (if adj u' v then 1 else 0)
      = fwd adj p u' := by
  unfold fwd
  have h1 : (if adj u' v then 1 else 0) = ∑ v', if adj u' v' ∧ p u' < p v' ∧ v' = v then 1 else 0 := by
    rw [Finset.sum_eq_single v]
    · simp [h]
    · intro b _ hb; simp [hb]
    · simp
  rw [h1, ← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun v' _ => ?_
  by_cases hv : v' = v <;> simp [hv, and_assoc]

/-- Closing pairs: triples `u < u' < v` with `u, u'` both earlier neighbours of `v`. -/
def triples : ℕ := ∑ u, ∑ v, ∑ u', if adj u v ∧ p u < p u' ∧ p u' < p v ∧ adj u' v then 1 else 0

theorem crossings_add_triples :
    crossings adj p + triples adj p
      = ∑ u, ∑ v, ∑ u', if adj u v ∧ p u < p u' ∧ p u' < p v then fwd adj p u' else 0 := by
  unfold crossings triples
  simp only [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun u _ => Finset.sum_congr rfl fun v _ => Finset.sum_congr rfl fun u' _ => ?_
  by_cases hc : adj u v ∧ p u < p u' ∧ p u' < p v
  · obtain ⟨ha, h1, h2⟩ := hc
    have hp : p u < p v := lt_trans h1 h2
    simp only [ha, h1, h2, hp, true_and, and_true, ite_true]
    rw [← fwd_split adj p u' v h2]
    simp [add_comm, and_assoc]
  · simp only [hc, ite_false]
    have : ∀ v', ¬ ((adj u v ∧ p u < p v) ∧ (adj u' v' ∧ p u' < p v') ∧ p u < p u' ∧ p u' < p v ∧ v' ≠ v) := by
      intro v' ⟨⟨ha, _⟩, _, h1, h2, _⟩; exact hc ⟨ha, h1, h2⟩
    have h2 : ¬ (adj u v ∧ p u < p u' ∧ p u' < p v ∧ adj u' v) := fun ⟨a, b, c, _⟩ => hc ⟨a, b, c⟩
    simp [this, h2]

theorem sum_fwd_spn :
    ∑ u, ∑ v, ∑ u', (if adj u v ∧ p u < p u' ∧ p u' < p v then fwd adj p u' else 0)
      = ∑ z, fwd adj p z * spn adj p z := by
  calc _ = ∑ u, ∑ u', ∑ v, (if adj u v ∧ p u < p u' ∧ p u' < p v then fwd adj p u' else 0) :=
        Finset.sum_congr rfl fun u _ => Finset.sum_comm
    _ = ∑ u', ∑ u, ∑ v, (if adj u v ∧ p u < p u' ∧ p u' < p v then fwd adj p u' else 0) :=
        Finset.sum_comm
    _ = _ := by
      refine Finset.sum_congr rfl fun u' _ => ?_
      unfold spn
      rw [Finset.mul_sum]
      refine Finset.sum_congr rfl fun u _ => ?_
      rw [Finset.mul_sum]
      refine Finset.sum_congr rfl fun v _ => ?_
      by_cases h : adj u v ∧ p u < p u' ∧ p u' < p v <;> simp [h, mul_comm]

theorem triples_eq (hinj : ∀ u v, p u = p v → u = v) :
    triples adj p = ∑ v, (bck adj p v).choose 2 := by
  unfold triples
  rw [Finset.sum_comm]
  refine Finset.sum_congr rfl fun v _ => ?_
  have hb : bck adj p v = (univ.filter fun w => adj w v ∧ p w < p v).card := by
    unfold bck; rw [Finset.card_filter]
  rw [hb, ← pairs_lt p _ hinj]
  simp only [Finset.sum_filter]
  refine Finset.sum_congr rfl fun u _ => ?_
  by_cases hu : adj u v ∧ p u < p v
  · simp only [hu, and_self, ite_true]
    refine Finset.sum_congr rfl fun u' _ => ?_
    by_cases hu' : adj u' v ∧ p u' < p v
    · by_cases hlt : p u < p u' <;> simp [hu', hlt, hu.1] <;> tauto
    · simp [hu']; intro _ _ h; exact absurd h (by tauto)
  · simp only [hu, ite_false]
    refine Finset.sum_eq_zero fun u' _ => ?_
    have : ¬ (adj u v ∧ p u < p u' ∧ p u' < p v ∧ adj u' v) := by
      rintro ⟨a, b, c, _⟩; exact hu ⟨a, lt_trans b c⟩
    simp [this]

/-- The pair count equals the degree formula (as an equation in `ℕ`). -/
theorem crossings_formula (hinj : ∀ u v, p u = p v → u = v) :
    crossings adj p + ∑ v, (bck adj p v).choose 2 = ∑ z, fwd adj p z * spn adj p z := by
  rw [← triples_eq adj p hinj, crossings_add_triples, sum_fwd_spn]


/-! ### Moving one node

Fix a node `x` and the order `ρ` of all the others. `pc c` places every `z ≠ x` at `2 ρ z + 1`
and `x` at `2 c`, so `x` comes before exactly the nodes of rank `≥ c`. Only the relative
order of the positions matters, and no list needs renumbering. -/

/-- The graph without `x` and its edges. -/
def adjO (x : V) (u v : V) : Prop := adj u v ∧ u ≠ x ∧ v ≠ x

instance (x : V) : DecidableRel (adjO adj x) := fun u v =>
  inferInstanceAs (Decidable (adj u v ∧ u ≠ x ∧ v ≠ x))

/-- Positions with `x` before the nodes of rank `c` and after those below it. -/
def pc (x : V) (ρ : V → ℕ) (c : ℕ) (z : V) : ℕ := if z = x then 2 * c else 2 * ρ z + 1

variable (x : V) (ρ : V → ℕ)

theorem ind_eq (P : Prop) [Decidable P] :
    (if P then 1 else 0) = ∑ w : V, if w = x ∧ P then 1 else 0 := by
  rw [Finset.sum_eq_single x]
  · simp
  · intro b _ hb; simp [hb]
  · simp

theorem fwd_pc (hs : ∀ u v, adj u v → adj v u) (c : ℕ) (z : V) (hz : z ≠ x) :
    fwd adj (pc x ρ c) z
      = fwd (adjO adj x) ρ z + (if adj x z ∧ ρ z < c then 1 else 0) := by
  have key : ∀ w, (if adj z w ∧ pc x ρ c z < pc x ρ c w then 1 else 0)
      = (if adjO adj x z w ∧ ρ z < ρ w then 1 else 0)
        + (if w = x ∧ adj x z ∧ ρ z < c then 1 else 0) := by
    intro w
    by_cases hw : w = x
    · subst hw
      by_cases h : adj z w
      · have h' := hs _ _ h
        simp [pc, adjO, hz, h, h'] <;> split_ifs <;> omega
      · have h' : ¬ adj w z := fun h' => h (hs _ _ h')
        simp [pc, adjO, hz, h, h']
    · simp [pc, adjO, hz, hw] <;> split_ifs <;> omega
  unfold fwd
  rw [Finset.sum_congr rfl fun w _ => key w, Finset.sum_add_distrib, ← ind_eq x]

theorem bck_pc (hs : ∀ u v, adj u v → adj v u) (c : ℕ) (z : V) (hz : z ≠ x) :
    bck adj (pc x ρ c) z
      = bck (adjO adj x) ρ z + (if adj x z ∧ c ≤ ρ z then 1 else 0) := by
  have key : ∀ w, (if adj w z ∧ pc x ρ c w < pc x ρ c z then 1 else 0)
      = (if adjO adj x w z ∧ ρ w < ρ z then 1 else 0)
        + (if w = x ∧ adj x z ∧ c ≤ ρ z then 1 else 0) := by
    intro w
    by_cases hw : w = x
    · subst hw
      simp [pc, adjO, hz] <;> split_ifs <;> omega
    · simp [pc, adjO, hz, hw] <;> split_ifs <;> omega
  unfold bck
  rw [Finset.sum_congr rfl fun w _ => key w, Finset.sum_add_distrib, ← ind_eq x]


/-- `x`'s neighbours that `x` precedes at cut `c`. -/
def above (c : ℕ) : ℕ := ∑ y, if adj x y ∧ c ≤ ρ y then 1 else 0
/-- `x`'s neighbours below the cut. -/
def below (c : ℕ) : ℕ := ∑ y, if adj x y ∧ ρ y < c then 1 else 0
/-- `x`'s degree. -/
def deg : ℕ := ∑ y, if adj x y then 1 else 0

theorem above_add_below (c : ℕ) : above adj x ρ c + below adj x ρ c = deg adj x := by
  unfold above below deg
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun y _ => ?_
  by_cases h : adj x y <;> by_cases h2 : c ≤ ρ y <;> simp [h, h2, not_le.mp]

theorem fwd_pc_x (hi : ∀ u, ¬ adj u u) (c : ℕ) :
    fwd adj (pc x ρ c) x = above adj x ρ c := by
  unfold fwd above
  refine Finset.sum_congr rfl fun w _ => ?_
  by_cases hw : w = x
  · subst hw; simp [hi]
  · simp [pc, hw] <;> split_ifs <;> omega

theorem bck_pc_x (hs : ∀ u v, adj u v → adj v u) (hi : ∀ u, ¬ adj u u) (c : ℕ) :
    bck adj (pc x ρ c) x = below adj x ρ c := by
  unfold bck below
  refine Finset.sum_congr rfl fun w _ => ?_
  by_cases hw : w = x
  · subst hw
    simp [hi]
  · by_cases h : adj w x
    · have h' := hs _ _ h
      simp [pc, hw, h, h'] <;> split_ifs <;> omega
    · have h' : ¬ adj x w := fun h' => h (hs _ _ h')
      simp [h, h']


theorem collapse1 (Q : V → Prop) [DecidablePred Q] :
    ∑ u : V, ∑ v : V, (if u = x ∧ Q v then 1 else 0) = ∑ v : V, if Q v then 1 else 0 := by
  rw [Finset.sum_eq_single x]
  · simp
  · intro b _ hb; simp [hb]
  · simp

theorem collapse2 (Q : V → Prop) [DecidablePred Q] :
    ∑ u : V, ∑ v : V, (if v = x ∧ Q u then 1 else 0) = ∑ u : V, if Q u then 1 else 0 := by
  refine Finset.sum_congr rfl fun u _ => ?_
  rw [Finset.sum_eq_single x]
  · simp
  · intro b _ hb; simp [hb]
  · simp

theorem spn_pc_x (c : ℕ) : spn adj (pc x ρ c) x = twin (adjO adj x) ρ c c := by
  unfold spn twin
  refine Finset.sum_congr rfl fun u _ => Finset.sum_congr rfl fun v _ => ?_
  by_cases hu : u = x
  · subst hu; simp [pc, adjO]
  by_cases hv : v = x
  · subst hv; simp [pc, adjO]
  simp [pc, adjO, hu, hv] <;> split_ifs <;> simp_all <;> omega

/-- Edges of `x` that span a node `z` at cut `c`, split by which side `x` is on. -/
def ds1 (c : ℕ) (z : V) : ℕ := ∑ y, if adj x y ∧ c ≤ ρ z ∧ ρ z < ρ y then 1 else 0
def ds2 (c : ℕ) (z : V) : ℕ := ∑ y, if adj x y ∧ ρ y < ρ z ∧ ρ z < c then 1 else 0

theorem spn_pc (hs : ∀ u v, adj u v → adj v u) (hi : ∀ u, ¬ adj u u) (c : ℕ) (z : V) (hz : z ≠ x) :
    spn adj (pc x ρ c) z
      = spn (adjO adj x) ρ z + ds1 adj x ρ c z + ds2 adj x ρ c z := by
  have key : ∀ u v, (if adj u v ∧ pc x ρ c u < pc x ρ c z ∧ pc x ρ c z < pc x ρ c v then 1 else 0)
      = (if adjO adj x u v ∧ ρ u < ρ z ∧ ρ z < ρ v then 1 else 0)
        + (if u = x ∧ adj x v ∧ c ≤ ρ z ∧ ρ z < ρ v then 1 else 0)
        + (if v = x ∧ adj x u ∧ ρ u < ρ z ∧ ρ z < c then 1 else 0) := by
    intro u v
    by_cases hu : u = x
    · subst hu
      by_cases hv : v = u
      · subst hv; simp [hi, adjO]
      · simp [pc, adjO, hz, hv] <;> split_ifs <;> simp_all <;> omega
    · by_cases hv : v = x
      · subst hv
        by_cases h : adj u v
        · have h' := hs _ _ h
          simp [pc, adjO, hz, hu, h, h'] <;> split_ifs <;> simp_all <;> omega
        · have h' : ¬ adj v u := fun h' => h (hs _ _ h')
          simp [h, h', adjO, hu]
      · simp [pc, adjO, hz, hu, hv] <;> split_ifs <;> simp_all <;> omega
  unfold spn ds1 ds2
  rw [Finset.sum_congr rfl fun u _ => Finset.sum_congr rfl fun v _ => key u v]
  simp only [Finset.sum_add_distrib]
  rw [collapse1 x (fun v => adj x v ∧ c ≤ ρ z ∧ ρ z < ρ v),
    collapse2 x (fun u => adj x u ∧ ρ u < ρ z ∧ ρ z < c)]


/-- `fd_pre`, `d_pre`: prefix sums over the graph without `x`. -/
def FP (c : ℕ) : ℕ := pre ρ (fwd (adjO adj x) ρ) c
def DP (c : ℕ) : ℕ := pre ρ (bck (adjO adj x) ρ) c

theorem fwdO_x : fwd (adjO adj x) ρ x = 0 := by
  unfold fwd; exact Finset.sum_eq_zero fun w _ => by simp [adjO]

theorem bckO_x : bck (adjO adj x) ρ x = 0 := by
  unfold bck; exact Finset.sum_eq_zero fun w _ => by simp [adjO]

theorem range_sum (f : V → ℕ) (a b : ℕ) (h : a ≤ b) :
    (∑ z, if a ≤ ρ z ∧ ρ z < b then f z else 0) + pre ρ f a = pre ρ f b := by
  unfold pre
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun z _ => ?_
  split_ifs <;> first | omega | simp_all <;> omega

theorem bckO_rank (hρ : ∀ u v, u ≠ x → v ≠ x → ρ u = ρ v → u = v) (y : V) (hy : y ≠ x) :
    DP adj x ρ (ρ y + 1) = DP adj x ρ (ρ y) + bck (adjO adj x) ρ y := by
  unfold DP pre
  have h1 : ∀ z, (if ρ z < ρ y + 1 then bck (adjO adj x) ρ z else 0)
      = (if ρ z < ρ y then bck (adjO adj x) ρ z else 0)
        + (if ρ z = ρ y then bck (adjO adj x) ρ z else 0) := by
    intro z; split_ifs <;> first | omega | simp_all <;> omega
  rw [Finset.sum_congr rfl fun z _ => h1 z, Finset.sum_add_distrib]
  congr 1
  rw [Finset.sum_eq_single y]
  · simp
  · intro z _ hz
    by_cases hx : z = x
    · subst hx; simp [bckO_x]
    · have : ρ z = ρ y → z = y := hρ z y hx hy
      simp only [ite_eq_right_iff]
      intro h; exact absurd (this h) hz
  · simp


/-- Neighbours of `x` ranked below `y`: the index of `y` in the sorted `nbr` array. -/
def kk (y : V) : ℕ := ∑ y', if adj x y' ∧ ρ y' < ρ y then 1 else 0

/-- Weighted count of nodes strictly between the cut and `y` (the `W` of `Delta::phi`). -/
def Wt (c : ℕ) (y : V) : ℤ :=
  if c ≤ ρ y then (FP adj x ρ (ρ y) : ℤ) - FP adj x ρ c
  else (FP adj x ρ c : ℤ) - FP adj x ρ (ρ y + 1)

/-- One neighbour's contribution (the loop body of `Delta::phi`). -/
def Tt (c : ℕ) (y : V) : ℤ :=
  if c ≤ ρ y then Wt adj x ρ c y - ((DP adj x ρ (ρ y + 1) : ℤ) - DP adj x ρ (ρ y))
  else Wt adj x ρ c y + ((FP adj x ρ (ρ y) : ℤ) - DP adj x ρ (ρ y + 1)) + kk adj x ρ y

/-- What `x` adds to the count when it sits before the nodes of rank `c` (`Delta::phi`). -/
def phi (c : ℕ) : ℤ :=
  (above adj x ρ c : ℤ) * ((FP adj x ρ c : ℤ) - DP adj x ρ c)
    - (((below adj x ρ c).choose 2 : ℕ) : ℤ)
    + ∑ y, if adj x y then Tt adj x ρ c y else 0

/-- The count of the list without `x` (independent of the cut). -/
def Kc : ℤ :=
  ∑ z, ((fwd (adjO adj x) ρ z * spn (adjO adj x) ρ z : ℕ) : ℤ)
    - ∑ z, (((bck (adjO adj x) ρ z).choose 2 : ℕ) : ℤ)

theorem sum_split {M : Type*} [AddCommMonoid M] (g h : V → M) (hgh : ∀ z, z ≠ x → g z = h z)
    (hx : h x = 0) : ∑ z, g z = g x + ∑ z, h z := by
  rw [← Finset.add_sum_erase univ g (mem_univ x),
    Finset.sum_congr rfl (fun z hz => hgh z (Finset.ne_of_mem_erase hz)),
    Finset.sum_erase univ hx]

theorem t1_y (c : ℕ) (y : V) :
    (∑ z, fwd (adjO adj x) ρ z *
        ((if adj x y ∧ c ≤ ρ z ∧ ρ z < ρ y then 1 else 0)
          + (if adj x y ∧ ρ y < ρ z ∧ ρ z < c then 1 else 0)))
      + (if adj x y then (if c ≤ ρ y then FP adj x ρ c else FP adj x ρ (ρ y + 1)) else 0)
      = if adj x y then (if c ≤ ρ y then FP adj x ρ (ρ y) else FP adj x ρ c) else 0 := by
  by_cases hxy : adj x y
  · by_cases hc : c ≤ ρ y
    · have := range_sum ρ (fwd (adjO adj x) ρ) c (ρ y) hc
      have e : ∀ z, fwd (adjO adj x) ρ z *
          ((if adj x y ∧ c ≤ ρ z ∧ ρ z < ρ y then 1 else 0)
            + (if adj x y ∧ ρ y < ρ z ∧ ρ z < c then 1 else 0))
          = if c ≤ ρ z ∧ ρ z < ρ y then fwd (adjO adj x) ρ z else 0 := by
        intro z; split_ifs <;> simp_all <;> omega
      rw [Finset.sum_congr rfl fun z _ => e z]
      simp only [hxy, hc, ite_true]
      exact this
    · have hc' : ρ y + 1 ≤ c := by omega
      have := range_sum ρ (fwd (adjO adj x) ρ) (ρ y + 1) c hc'
      have e : ∀ z, fwd (adjO adj x) ρ z *
          ((if adj x y ∧ c ≤ ρ z ∧ ρ z < ρ y then 1 else 0)
            + (if adj x y ∧ ρ y < ρ z ∧ ρ z < c then 1 else 0))
          = if ρ y + 1 ≤ ρ z ∧ ρ z < c then fwd (adjO adj x) ρ z else 0 := by
        intro z; split_ifs <;> simp_all <;> omega
      rw [Finset.sum_congr rfl fun z _ => e z]
      simp only [hxy, hc, ite_true, ite_false]
      exact this
  · simp [hxy]

theorem t1 (c : ℕ) :
    ∑ z, fwd (adjO adj x) ρ z * (ds1 adj x ρ c z + ds2 adj x ρ c z)
      + ∑ y, (if adj x y then (if c ≤ ρ y then FP adj x ρ c else FP adj x ρ (ρ y + 1)) else 0)
      = ∑ y, if adj x y then (if c ≤ ρ y then FP adj x ρ (ρ y) else FP adj x ρ c) else 0 := by
  have h : ∑ z, fwd (adjO adj x) ρ z * (ds1 adj x ρ c z + ds2 adj x ρ c z)
      = ∑ y, ∑ z, fwd (adjO adj x) ρ z *
        ((if adj x y ∧ c ≤ ρ z ∧ ρ z < ρ y then 1 else 0)
          + (if adj x y ∧ ρ y < ρ z ∧ ρ z < c then 1 else 0)) := by
    unfold ds1 ds2
    rw [Finset.sum_comm]
    refine Finset.sum_congr rfl fun z _ => ?_
    rw [← Finset.sum_add_distrib, Finset.mul_sum]
  rw [h, ← Finset.sum_add_distrib]
  exact Finset.sum_congr rfl fun y _ => t1_y adj x ρ c y


theorem t2 (c : ℕ) :
    ∑ z, (if adj x z ∧ ρ z < c then 1 else 0)
        * (spn (adjO adj x) ρ z + ds1 adj x ρ c z + ds2 adj x ρ c z)
      + ∑ y, (if adj x y ∧ ρ y < c then DP adj x ρ (ρ y + 1) else 0)
      = ∑ y, if adj x y ∧ ρ y < c then FP adj x ρ (ρ y) + kk adj x ρ y else 0 := by
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun z _ => ?_
  by_cases h : adj x z ∧ ρ z < c
  · have hs := spn_eq (adjO adj x) ρ z
    have h1 : ds1 adj x ρ c z = 0 := by
      unfold ds1; exact Finset.sum_eq_zero fun y _ => by simp; intros; omega
    have h2 : ds2 adj x ρ c z = kk adj x ρ z := by
      unfold ds2 kk
      exact Finset.sum_congr rfl fun y _ => by
        by_cases hy : adj x y ∧ ρ y < ρ z
        · simp [hy, h.2]
        · have : ¬ (adj x y ∧ ρ y < ρ z ∧ ρ z < c) := fun ⟨a, b, _⟩ => hy ⟨a, b⟩
          simp [hy, this]
    simp only [h, and_self, ite_true, one_mul, h1, h2, add_zero]
    unfold FP DP
    omega
  · simp [h]

theorem t3 (hi : ∀ u, ¬ adj u u) (hρ : ∀ u v, u ≠ x → v ≠ x → ρ u = ρ v → u = v) (c : ℕ) :
    ∑ z, (if adj x z ∧ c ≤ ρ z then 1 else 0) * bck (adjO adj x) ρ z
      + ∑ y, (if adj x y ∧ c ≤ ρ y then DP adj x ρ (ρ y) else 0)
      = ∑ y, if adj x y ∧ c ≤ ρ y then DP adj x ρ (ρ y + 1) else 0 := by
  rw [← Finset.sum_add_distrib]
  refine Finset.sum_congr rfl fun z _ => ?_
  by_cases h : adj x z ∧ c ≤ ρ z
  · have hz : z ≠ x := by rintro rfl; exact hi _ h.1
    have := bckO_rank adj x ρ hρ z hz
    simp only [h, and_self, ite_true, one_mul]
    omega
  · simp [h]


theorem choose_two_add (n d : ℕ) (hd : d = 0 ∨ d = 1) :
    (n + d).choose 2 = n.choose 2 + d * n := by
  rcases hd with rfl | rfl
  · simp
  · rw [Nat.choose_succ_succ]; simp [Nat.add_comm]

/-- **Moving one node.** Putting `x` before the nodes of rank `c` gives the pair count of the
list without `x` (`Kc`, independent of `c`) plus `phi c`, which reads `x`'s neighbour ranks and
prefix sums of the list without `x` only. -/
theorem crossings_move (hs : ∀ u v, adj u v → adj v u) (hi : ∀ u, ¬ adj u u)
    (hρ : ∀ u v, u ≠ x → v ≠ x → ρ u = ρ v → u = v) (c : ℕ) :
    (crossings adj (pc x ρ c) : ℤ) = Kc adj x ρ + phi adj x ρ c := by
  have hinj : ∀ u v, pc x ρ c u = pc x ρ c v → u = v := by
    intro u v h
    by_cases hu : u = x <;> by_cases hv : v = x
    · rw [hu, hv]
    · simp [pc, hu, hv] at h; omega
    · simp [pc, hu, hv] at h; omega
    · simp [pc, hu, hv] at h; exact hρ u v hu hv (by omega)
  have hA := crossings_formula adj (pc x ρ c) hinj
  set fO := fwd (adjO adj x) ρ with hfO
  set bO := bck (adjO adj x) ρ with hbO
  set sO := spn (adjO adj x) ρ with hsO
  have hX : ∑ z, fwd adj (pc x ρ c) z * spn adj (pc x ρ c) z
      = above adj x ρ c * twin (adjO adj x) ρ c c
        + ∑ z, (fO z + (if adj x z ∧ ρ z < c then 1 else 0))
            * (sO z + ds1 adj x ρ c z + ds2 adj x ρ c z) := by
    rw [sum_split x _ (fun z => (fO z + (if adj x z ∧ ρ z < c then 1 else 0))
            * (sO z + ds1 adj x ρ c z + ds2 adj x ρ c z))]
    · rw [fwd_pc_x adj x ρ hi, spn_pc_x]
    · intro z hz
      rw [fwd_pc adj x ρ hs c z hz, spn_pc adj x ρ hs hi c z hz]
    · simp [hfO, fwdO_x, hi]
  have hY : ∑ z, (bck adj (pc x ρ c) z).choose 2
      = (below adj x ρ c).choose 2
        + ∑ z, (bO z + (if adj x z ∧ c ≤ ρ z then 1 else 0)).choose 2 := by
    rw [sum_split x _ (fun z => (bO z + (if adj x z ∧ c ≤ ρ z then 1 else 0)).choose 2)]
    · rw [bck_pc_x adj x ρ hs hi]
    · intro z hz
      rw [bck_pc adj x ρ hs c z hz]
    · simp [hbO, bckO_x, hi]
  have hexp1 : ∑ z, (fO z + (if adj x z ∧ ρ z < c then 1 else 0))
            * (sO z + ds1 adj x ρ c z + ds2 adj x ρ c z)
      = ∑ z, fO z * sO z + ∑ z, fO z * (ds1 adj x ρ c z + ds2 adj x ρ c z)
        + ∑ z, (if adj x z ∧ ρ z < c then 1 else 0)
            * (sO z + ds1 adj x ρ c z + ds2 adj x ρ c z) := by
    rw [← Finset.sum_add_distrib, ← Finset.sum_add_distrib]
    exact Finset.sum_congr rfl fun z _ => by ring
  have hexp2 : ∑ z, (bO z + (if adj x z ∧ c ≤ ρ z then 1 else 0)).choose 2
      = ∑ z, (bO z).choose 2
        + ∑ z, (if adj x z ∧ c ≤ ρ z then 1 else 0) * bO z := by
    rw [← Finset.sum_add_distrib]
    refine Finset.sum_congr rfl fun z _ => ?_
    rw [choose_two_add _ _ (by split_ifs <;> simp)]
  have hcut := cut_eq (adjO adj x) ρ c
  have e1 := t1 adj x ρ c
  have e2 := t2 adj x ρ c
  have e3 := t3 adj x ρ hi hρ c
  -- the neighbour sum of `phi`, term by term
  have hphi : ∑ y, (if adj x y then Tt adj x ρ c y else 0)
      = ((∑ y, (if adj x y then (if c ≤ ρ y then FP adj x ρ (ρ y) else FP adj x ρ c) else 0) : ℕ) : ℤ)
        - ((∑ y, (if adj x y then (if c ≤ ρ y then FP adj x ρ c else FP adj x ρ (ρ y + 1)) else 0) : ℕ) : ℤ)
        + ((∑ y, (if adj x y ∧ ρ y < c then FP adj x ρ (ρ y) + kk adj x ρ y else 0) : ℕ) : ℤ)
        - ((∑ y, (if adj x y ∧ ρ y < c then DP adj x ρ (ρ y + 1) else 0) : ℕ) : ℤ)
        - ((∑ y, (if adj x y ∧ c ≤ ρ y then DP adj x ρ (ρ y + 1) else 0) : ℕ) : ℤ)
        + ((∑ y, (if adj x y ∧ c ≤ ρ y then DP adj x ρ (ρ y) else 0) : ℕ) : ℤ) := by
    push_cast
    simp only [← Finset.sum_sub_distrib, ← Finset.sum_add_distrib]
    refine Finset.sum_congr rfl fun y _ => ?_
    by_cases h : adj x y
    · by_cases hc : c ≤ ρ y
      · have hl : ¬ ρ y < c := by omega
        simp [Tt, Wt, h, hc, hl]; ring
      · have hl : ρ y < c := by omega
        simp [Tt, Wt, h, hc, hl]; ring
    · simp [h]
  have hcutz : (twin (adjO adj x) ρ c c : ℤ) = (FP adj x ρ c : ℤ) - DP adj x ρ c := by
    have := hcut; unfold FP DP; omega
  rw [hX, hY, hexp1, hexp2] at hA
  have hAz := congrArg (Nat.cast : ℕ → ℤ) hA
  have e1z := congrArg (Nat.cast : ℕ → ℤ) e1
  have e2z := congrArg (Nat.cast : ℕ → ℤ) e2
  have e3z := congrArg (Nat.cast : ℕ → ℤ) e3
  unfold Kc phi
  rw [hphi]
  push_cast at hAz e1z e2z e3z hcutz ⊢
  rw [hcutz] at hAz
  linarith


/-! ### Wrapped `i32` arithmetic

`toI32 z` is the `i32` that `z as i32` / repeated `wrapping_add` produce: the representative of
`z` modulo `2^32` in `[-2^31, 2^31)`. `ℤ / 2^32` is a group, so wrapping addition of the old
total and the delta is the wrapped sum. -/

/-- The `i32` two's-complement value of an integer. -/
def toI32 (z : ℤ) : ℤ := (z + 2147483648) % 4294967296 - 2147483648

theorem toI32_add (a b : ℤ) : toI32 (toI32 a + toI32 b) = toI32 (a + b) := by
  unfold toI32; omega

/-- `wrap_crossings`: `value.wrapping_add(count as i32)`. -/
def wrapAdd (value count : ℤ) : ℤ := toI32 (value + toI32 count)

/-- The reference recount adds its per-query counts one at a time with `wrapAdd`; the result is
the wrapped total, whatever the grouping into queries. -/
theorem wrap_foldl (l : List ℤ) (init : ℤ) (h : toI32 init = init) :
    l.foldl wrapAdd init = toI32 (init + l.sum) := by
  induction l generalizing init with
  | nil => simpa using h.symm
  | cons a l ih =>
    have h' : toI32 (wrapAdd init a) = wrapAdd init a := by unfold wrapAdd toI32; omega
    rw [List.foldl_cons, ih _ h', List.sum_cons]
    unfold wrapAdd toI32; omega

/-- **Fast path = C-mirror path.** The wrapped old total plus the wrapped delta
`phi c' - phi c0` is the wrapped total the full recount returns for the moved list. -/
theorem fast_eq_recount (hs : ∀ u v, adj u v → adj v u) (hi : ∀ u, ¬ adj u u)
    (hρ : ∀ u v, u ≠ x → v ≠ x → ρ u = ρ v → u = v) (c0 c' : ℕ) :
    wrapAdd (toI32 (crossings adj (pc x ρ c0)))
        (phi adj x ρ c' - phi adj x ρ c0)
      = toI32 (crossings adj (pc x ρ c')) := by
  have h0 := crossings_move adj x ρ hs hi hρ c0
  have h1 := crossings_move adj x ρ hs hi hρ c'
  unfold wrapAdd
  rw [toI32_add]
  congr 1
  omega

end GvpProofs.Circo
