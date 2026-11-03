import Mathlib.Logic.Function.Iterate
import Mathlib.Algebra.BigOperators.Group.Finset.Basic
import Mathlib.Algebra.BigOperators.Ring.Finset
import Mathlib.Tactic

/-!
# Network simplex on cut sets (`docs/perf/ns-cut-sets.md` §2, §3, §4)

The Rust fast path is `common/ns/cut.rs` (`CutStore::enter_edge`, `scanned_before`, `push_rank`,
`first_of`, `update`, `shift`, `move_cut_sets`); the mirror of C is `common/ns.rs` (`enter_edge`,
`dfs_enter_inedge`, `dfs_enter_outedge`, `dfs_range_impl`, `update`). The headline theorem is
`Net.Valid.enterEdge_eq`: on a valid tree with (I1) for `e`, the cut-set `enter_edge(e)`, with its
fallback, returns what C's `enter_edge(e)` returns once its loop has run to the end, ties included.

Scope of `Valid`: it requires the `lim`/`low` numbering to be current. The production cut-set path
renumbers only at init and on fallback (`cut.rs` re-hangs `par` after a pivot; its `dfs_range` hook
is a no-op), so the theorem applies as stated to the first pivot after `feasible_tree` and to any
pivot from a renumbered state. `CutSetsStale.lean` (issue #750) covers the later pivots, whose
`lim`/`low` are stale: `Net.enterEdgeP_eq_every` says the production `enter_edge(e)` is C's
`enter_edge(e)` on any valid numbering of the stored tree, and `Net.pivot_step` derives, from the
tree before a pivot alone, that the tree after it is a spanning tree with a valid numbering, and
that its cuts are the ones `pivot_against` gives. Here `pivot_against` still takes the new tree's
`Valid` as a hypothesis; `pivot_step` discharges it.

| prose | here |
| :-- | :-- |
| §2 notation: `w ∈ subtree(u)` iff `low(u) ≤ lim(w) ≤ lim(u)` | `Net.Valid.inSub_iff`, `Net.Valid.lim_inj` |
| §2 child test `lim(other) < lim(u)` is `par(other) == edge` | `Net.Valid.tail_child_iff`, `near_child_iff` |
| §2 the cut's sides, and "does not depend on the rooting" | `Net.Conn`, `Net.Against`, `Net.side_iff` |
| §2 Candidates | `Net.Valid.cands_iff` |
| §2 Order (pre-order, children in reverse push order, list order) | `Net.scanList`, `Net.Valid.scanList_unfold`, `Net.Valid.nodup_scanList` |
| §2 Choice, Pruning | `Net.evalE`, `Net.loop1`, `Net.loop2`, `Net.step_cons` |
| Lemma 1 | `Net.Valid.block`, `Net.Valid.lemma1`, `Net.foldl_evalE_spec` |
| §3 (I1) and "equivalently: whose fundamental cycle contains t" | `Net.CoverOK`, `CutBasis.cycle_conserves`, `CutBasis.cycle_unique` |
| §4.1 the tie order | `Net.Valid.scannedBefore_spec` (`firstOf_spec`, `climbDecide_eq`, `Valid.pushRank_sublist`) |
| §4.1 the minimum, in cut-set order | `Net.foldl_cutStep_spec`, `Net.Valid.cutEnter_eq`, `Net.Valid.enterEdge_eq`, `Net.cutEnter_isSome` |
| §4.2 Ranks | `Net.Valid.rerankNode_spec`, `CutBasis.slack_shift_cut`, `Net.Valid.tail_inside_iff` |
| §4.2 Cut values | `CutBasis.pivot_cutValue`, `CutBasis.pivot_cutValue_f`, `CutBasis.cutValue_eq_check` |
| §4.2 Cut sets (I1), the four bullets | `CutBasis.pivot`, `pivot_outside`, `pivot_toggle`, `pivot_join_against`, `pivot_coef_e`, `pivot_coef_f_e`, `pivot_coef_f`, `pivot_coef_of_f`, `coef_range` |
| §4.2 (I1) for the new tree, however it is stored | `Net.Valid.pivot_against`; with nothing assumed of the new tree, `Net.pivot_step` (`CutSetsStale.lean`) |
| §4.2 (I3) parent pointers, §5 Lemma 2 for `enter_edge`, later pivots | `CutSetsStale.lean`: `Net.Valid.rehang_tree`, `Net.Tree.par_unique`, `Net.cEnter_renum`, `Net.enterEdgeP_eq_every` |

Not formalised here: that C's two `treeupdate` walks and the cut sets' LCA climb visit exactly
the edges with σ(t) ≠ 0 (the cut-value theorem is stated per edge, with σ); that `rerank` reaches
exactly the subtree under its start; §4.3 beyond its `enter_edge` (which `enterEdge_eq` covers with
`pivot = false`); the fallback's rank reconstruction, and that its renumbering is valid (§5);
§5–§8. The re-hanging of parent pointers (I3) and the production tree after a pivot (issue #750)
are in `CutSetsStale.lean`.

## Model vs Rust

* `Net` holds the fields of `NsGraph` that the search reads; `E` is the graph's edge set, so
  `ND_in`/`ND_out` list every edge once (`Valid`). `lim`/`low` are `ℕ` (they start at 1).
* Slacks are one function `slack : E → ℤ` for both paths: that is (I2). C's `INT_MAX` start is
  `Net.intMax`; only "it is positive" is used.
* The C loop runs on fuel (`Net.run`); the theorems say "for every `k` past some `K`", which is the
  `while` loop ending. The climb in `scanned_before` has fuel `lim(v) + 1`, enough on a valid tree.
* `push_rank`'s `position(..).unwrap_or(0)` is `List.idxOf` (the list's length when absent). On a
  valid tree the edge is always present.
* `CutStore`'s pool, stamps, generations and `prepare` checks are not modelled: `cutEnter` reads
  e's cut set as a list of `(edge, against)`. Every extra check falls back to C. The model's
  fallback runs C's search on the same `Net` and slacks; Rust's fallback (`cut.rs` `fall_back`)
  first reconstructs ranks and renumbers the tree, then searches. That reconstruction is §5 and is
  not modelled, so "fallback included" means: whichever branch is taken, the search that runs is
  C's search on a validly numbered tree.
* `i32` wrapping: §4.2's algebra (`CutBasis.cutValue`, `slack`) holds in any commutative ring,
  `ℤ/2³²` included. Comparisons of slacks are on `ℤ`, as `i32` compares.
-/

set_option linter.unusedSectionVars false

namespace GvpProofs.Ns.CutSets

/-! ## Lists: "comes before" -/

section ListAux
variable {α β : Type}

theorem sublist_flatMap_of_mem {f : α → List β} {l : List α} {a : α} (h : a ∈ l) :
    List.Sublist (f a) (l.flatMap f) := by
  induction l with
  | nil => simp at h
  | cons z l ih =>
    rw [List.flatMap_cons]
    rcases List.mem_cons.mp h with rfl | h
    · exact List.sublist_append_left _ _
    · exact (ih h).trans (List.sublist_append_right _ _)

theorem pair_sublist_flatMap {f : α → List β} {l : List α} {a b : α} {x y : β}
    (hab : List.Sublist [a, b] l) (hx : x ∈ f a) (hy : y ∈ f b) :
    List.Sublist [x, y] (l.flatMap f) := by
  induction l with
  | nil => simp at hab
  | cons z l ih =>
    rw [List.flatMap_cons]
    rcases List.sublist_cons_iff.mp hab with h | ⟨r, hr, h⟩
    · exact (ih h).trans (List.sublist_append_right _ _)
    · obtain ⟨rfl, rfl⟩ := List.cons_eq_cons.mp hr
      have hb : b ∈ l := List.singleton_sublist.mp h
      exact List.Sublist.append (l₁ := [x]) (List.singleton_sublist.mpr hx)
        (List.singleton_sublist.mpr (List.mem_flatMap.mpr ⟨b, hb, hy⟩))

theorem pair_sublist_append {l₁ l₂ : List α} {a b : α} (ha : a ∈ l₁) (hb : b ∈ l₂) :
    List.Sublist [a, b] (l₁ ++ l₂) :=
  List.Sublist.append (l₁ := [a]) (List.singleton_sublist.mpr ha) (List.singleton_sublist.mpr hb)

variable [DecidableEq α]

theorem idxOf_lt_of_pair_sublist {l : List α} {a b : α} (hn : l.Nodup)
    (h : List.Sublist [a, b] l) : l.idxOf a < l.idxOf b := by
  induction l with
  | nil => simp at h
  | cons z l ih =>
    have hz : z ∉ l := (List.nodup_cons.mp hn).1
    rcases List.sublist_cons_iff.mp h with h' | ⟨r, hr, h'⟩
    · have ha : a ∈ l := h'.subset (by simp)
      have hb : b ∈ l := h'.subset (by simp)
      have hna : a ≠ z := fun e => hz (e ▸ ha)
      have hnb : b ≠ z := fun e => hz (e ▸ hb)
      rw [List.idxOf_cons_ne _ (Ne.symm hna), List.idxOf_cons_ne _ (Ne.symm hnb)]
      exact Nat.succ_lt_succ (ih (List.nodup_cons.mp hn).2 h')
    · obtain ⟨rfl, rfl⟩ := List.cons_eq_cons.mp hr
      have hb : b ∈ l := List.singleton_sublist.mp h'
      have hnb : b ≠ a := fun e => hz (e ▸ hb)
      rw [List.idxOf_cons_self, List.idxOf_cons_ne _ (Ne.symm hnb)]
      exact Nat.succ_pos _

theorem pair_sublist_of_idxOf_lt {l : List α} {a b : α} (hb : b ∈ l)
    (h : l.idxOf a < l.idxOf b) : List.Sublist [a, b] l := by
  induction l with
  | nil => simp at hb
  | cons z l ih =>
    by_cases ha : a = z
    · subst ha
      by_cases hbz : b = a
      · subst hbz; simp at h
      · exact List.Sublist.cons_cons _ (List.singleton_sublist.mpr
          ((List.mem_cons.mp hb).resolve_left hbz))
    · by_cases hbz : b = z
      · subst hbz; rw [List.idxOf_cons_self] at h; omega
      · rw [List.idxOf_cons_ne _ (Ne.symm ha), List.idxOf_cons_ne _ (Ne.symm hbz)] at h
        exact List.Sublist.cons _ (ih ((List.mem_cons.mp hb).resolve_left hbz) (by omega))

end ListAux

/-! ## The tree and its post-order numbering (§2, notation) -/

/-- The fields of `NsGraph` that `enter_edge` and `dfs_range` read. `E` is the graph's edge set:
every value of it is an edge. `lim`/`low` are natural numbers: `dfs_range` starts at 1. -/
structure Net (V E : Type) where
  tail : E → V
  head : E → V
  /-- `ND_in`, `ND_out`, in list order. -/
  inE : V → List E
  outE : V → List E
  /-- `ND_tree_in`, `ND_tree_out`, in list order. -/
  treeIn : V → List E
  treeOut : V → List E
  /-- `ED_tree_index >= 0`. -/
  isTree : E → Bool
  par : V → Option E
  low : V → ℕ
  lim : V → ℕ

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] (N : Net V E)

/-- The children `dfs_range` visits from `u`, in its order: the heads of `u`'s tree out-edges, then
the tails of its tree in-edges, skipping `par(u)`. -/
def kids (u : V) : List V :=
  ((N.treeOut u).filter (fun e => N.par u ≠ some e)).map N.head ++
    ((N.treeIn u).filter (fun e => N.par u ≠ some e)).map N.tail

/-- `w ∈ subtree(u)`, by the children `dfs_range` visits. -/
inductive InSub : V → V → Prop
  | refl (u : V) : InSub u u
  | kid {u c w : V} : c ∈ N.kids u → InSub c w → InSub u w

/-- `dfs_range_impl`'s counter over a list of children: child `c` gets `low(c) = a`, and the
counter continues at `lim(c) + 1`. -/
inductive Chain (low lim : V → ℕ) : ℕ → List V → ℕ → Prop
  | nil (a : ℕ) : Chain low lim a [] a
  | cons {a b : ℕ} {c : V} {cs : List V} :
      low c = a → Chain low lim (lim c + 1) cs b → Chain low lim a (c :: cs) b

/-- What holds of the tree between pivots: `dfs_range`'s numbering is current (`chain`, `low_le`),
the tree spans the graph from `root`, and the edge lists are the graph's. -/
structure Valid (root : V) : Prop where
  chain : ∀ u, Chain N.low N.lim (N.low u) (N.kids u) (N.lim u)
  low_le : ∀ u, N.low u ≤ N.lim u
  span : ∀ w, N.InSub root w
  mem_inE : ∀ u e, e ∈ N.inE u ↔ N.head e = u
  mem_outE : ∀ u e, e ∈ N.outE u ↔ N.tail e = u
  mem_treeIn : ∀ u e, e ∈ N.treeIn u ↔ N.isTree e = true ∧ N.head e = u
  mem_treeOut : ∀ u e, e ∈ N.treeOut u ↔ N.isTree e = true ∧ N.tail e = u
  nodup_inE : ∀ u, (N.inE u).Nodup
  nodup_outE : ∀ u, (N.outE u).Nodup
  nodup_treeIn : ∀ u, (N.treeIn u).Nodup
  nodup_treeOut : ∀ u, (N.treeOut u).Nodup
  par_root : N.par root = none
  par_tree : ∀ u e, N.par u = some e → N.isTree e = true ∧ (N.tail e = u ∨ N.head e = u)
  /-- Every tree edge is the parent edge of exactly one of its ends. -/
  tree_par : ∀ e, N.isTree e = true → N.par (N.tail e) = some e ∨ N.par (N.head e) = some e
  par_excl : ∀ e, N.isTree e = true → ¬(N.par (N.tail e) = some e ∧ N.par (N.head e) = some e)

variable {N} {root : V}

theorem Chain.bounds {low lim : V → ℕ} (hle : ∀ u, low u ≤ lim u) :
    ∀ {a cs b}, Chain low lim a cs b → a ≤ b ∧ ∀ c ∈ cs, a ≤ low c ∧ lim c < b := by
  intro a cs b h
  induction h with
  | nil a => simp
  | @cons a b c cs h1 _ ih =>
    obtain ⟨ih1, ih2⟩ := ih
    have := hle c
    refine ⟨by omega, ?_⟩
    intro x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · omega
    · have := ih2 x hx; omega

theorem Chain.disjoint {low lim : V → ℕ} (hle : ∀ u, low u ≤ lim u) :
    ∀ {a cs b}, Chain low lim a cs b → ∀ c ∈ cs, ∀ c' ∈ cs, c ≠ c' →
      lim c < low c' ∨ lim c' < low c := by
  intro a cs b h
  induction h with
  | nil a => simp
  | @cons a b c cs h1 hrest ih =>
    intro x hx y hy hxy
    have hb := (Chain.bounds hle hrest).2
    rcases List.mem_cons.mp hx with hx1 | hx2
    · rcases List.mem_cons.mp hy with hy1 | hy2
      · exact absurd (hx1.trans hy1.symm) hxy
      · rw [hx1]; left; have := (hb y hy2).1; omega
    · rcases List.mem_cons.mp hy with hy1 | hy2
      · rw [hy1]; right; have := (hb x hx2).1; omega
      · exact ih x hx2 y hy2 hxy

theorem Valid.kid_bounds (hN : N.Valid root) {u c : V} (hc : c ∈ N.kids u) :
    N.low u ≤ N.low c ∧ N.lim c < N.lim u :=
  (Chain.bounds hN.low_le (hN.chain u)).2 c hc

theorem Valid.kid_lim (hN : N.Valid root) {u c : V} (hc : c ∈ N.kids u) : N.lim c < N.lim u :=
  (hN.kid_bounds hc).2

theorem InSub.trans {u w x : V} (h1 : N.InSub u w) (h2 : N.InSub w x) : N.InSub u x := by
  induction h1 with
  | refl => exact h2
  | kid hc _ ih => exact InSub.kid hc (ih h2)

theorem Valid.sub_bounds (hN : N.Valid root) {u w : V} (h : N.InSub u w) :
    N.low u ≤ N.low w ∧ N.low u ≤ N.lim w ∧ N.lim w ≤ N.lim u := by
  induction h with
  | refl u => exact ⟨le_rfl, hN.low_le u, le_rfl⟩
  | kid hc _ ih =>
    have := hN.kid_bounds hc
    omega

/-- Following children strictly lowers `lim`. -/
theorem Valid.sub_lim_lt (hN : N.Valid root) {u w : V} (h : N.InSub u w) (hne : u ≠ w) :
    N.lim w < N.lim u := by
  cases h with
  | refl => exact absurd rfl hne
  | kid hc h' =>
    have := hN.kid_lim hc
    have := (hN.sub_bounds h').2.2
    omega

theorem Valid.sub_antisymm (hN : N.Valid root) {u w : V} (h1 : N.InSub u w) (h2 : N.InSub w u) :
    u = w := by
  by_contra hne
  have := hN.sub_lim_lt h1 hne
  have := hN.sub_lim_lt h2 (Ne.symm hne)
  omega

theorem Valid.sub_of_interval (hN : N.Valid root) :
    ∀ r u w, N.InSub r u → N.InSub r w → N.low u ≤ N.lim w → N.lim w ≤ N.lim u → N.InSub u w := by
  intro r
  induction r using (measure N.lim).wf.induction with
  | h r ih =>
    intro u w hu hw h1 h2
    cases hu with
    | refl => exact hw
    | @kid _ c _ hc hcu =>
      have hcb := hN.kid_bounds hc
      have hub := hN.sub_bounds hcu
      cases hw with
      | refl => omega
      | @kid _ c' _ hc' hcw =>
        by_cases hcc : c = c'
        · subst hcc
          exact ih c hcb.2 u w hcu hcw h1 h2
        · have hwb := hN.sub_bounds hcw
          rcases Chain.disjoint hN.low_le (hN.chain r) c hc c' hc' hcc with h | h <;> omega

/-- §2, notation: `w ∈ subtree(u)` exactly when `low(u) ≤ lim(w) ≤ lim(u)`. -/
theorem Valid.inSub_iff (hN : N.Valid root) (u w : V) :
    N.InSub u w ↔ N.low u ≤ N.lim w ∧ N.lim w ≤ N.lim u := by
  constructor
  · intro h; have := hN.sub_bounds h; exact ⟨this.2.1, this.2.2⟩
  · rintro ⟨h1, h2⟩; exact hN.sub_of_interval root u w (hN.span u) (hN.span w) h1 h2

/-- `lim` numbers the nodes one to one. -/
theorem Valid.lim_inj (hN : N.Valid root) {u w : V} (h : N.lim u = N.lim w) : u = w := by
  have h1 : N.InSub u w := (hN.inSub_iff u w).mpr ⟨by rw [← h]; exact hN.low_le u, by omega⟩
  have h2 : N.InSub w u := (hN.inSub_iff w u).mpr ⟨by rw [h]; exact hN.low_le w, by omega⟩
  exact hN.sub_antisymm h1 h2


/-! ### Parents and children -/

theorem Valid.mem_kids (hN : N.Valid root) {u c : V} :
    c ∈ N.kids u ↔ ∃ e, N.isTree e = true ∧ N.par c = some e ∧
      ((N.tail e = u ∧ N.head e = c) ∨ (N.head e = u ∧ N.tail e = c)) := by
  constructor
  · intro h
    simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq] at h
    rcases h with ⟨e, ⟨he, hp⟩, rfl⟩ | ⟨e, ⟨he, hp⟩, rfl⟩
    · obtain ⟨ht, htl⟩ := (hN.mem_treeOut u e).mp he
      refine ⟨e, ht, ?_, Or.inl ⟨htl, rfl⟩⟩
      rcases hN.tree_par e ht with h | h
      · rw [htl] at h; exact absurd h hp
      · exact h
    · obtain ⟨ht, hhd⟩ := (hN.mem_treeIn u e).mp he
      refine ⟨e, ht, ?_, Or.inr ⟨hhd, rfl⟩⟩
      rcases hN.tree_par e ht with h | h
      · exact h
      · rw [hhd] at h; exact absurd h hp
  · rintro ⟨e, ht, hpc, (⟨h1, h2⟩ | ⟨h1, h2⟩)⟩
    · simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq]
      refine Or.inl ⟨e, ⟨(hN.mem_treeOut u e).mpr ⟨ht, h1⟩, ?_⟩, h2⟩
      intro hpu
      exact hN.par_excl e ht ⟨by rw [h1]; exact hpu, by rw [h2]; exact hpc⟩
    · simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq]
      refine Or.inr ⟨e, ⟨(hN.mem_treeIn u e).mpr ⟨ht, h1⟩, ?_⟩, h2⟩
      intro hpu
      exact hN.par_excl e ht ⟨by rw [h2]; exact hpc, by rw [h1]; exact hpu⟩

theorem Valid.kid_ne (hN : N.Valid root) {u c : V} (h : c ∈ N.kids u) : c ≠ u := by
  rintro rfl; have := hN.kid_lim h; omega

/-- A node is a child of one node only. -/
theorem Valid.kid_unique (hN : N.Valid root) {u u' c : V} (h : c ∈ N.kids u) (h' : c ∈ N.kids u') :
    u = u' := by
  have hcu := hN.kid_ne h
  have hcu' := hN.kid_ne h'
  obtain ⟨e, _, hp, he⟩ := (hN.mem_kids).mp h
  obtain ⟨e', _, hp', he'⟩ := (hN.mem_kids).mp h'
  rw [hp] at hp'
  obtain rfl := Option.some.inj hp'
  rcases he with ⟨a, b⟩ | ⟨a, b⟩ <;> rcases he' with ⟨a', b'⟩ | ⟨a', b'⟩
  · rw [← a, ← a']
  · exact absurd (b.symm.trans a') hcu'
  · exact absurd (b.symm.trans a') hcu'
  · rw [← a, ← a']

/-- The parent edge of a node joins it to the node it is a child of. -/
theorem Valid.parent (hN : N.Valid root) {c : V} {e : E} (h : N.par c = some e) :
    ∃ u, c ∈ N.kids u ∧ ((N.tail e = u ∧ N.head e = c) ∨ (N.head e = u ∧ N.tail e = c)) := by
  obtain ⟨ht, hc⟩ := hN.par_tree c e h
  rcases hc with hc | hc
  · exact ⟨N.head e, (hN.mem_kids).mpr ⟨e, ht, h, Or.inr ⟨rfl, hc⟩⟩, Or.inr ⟨rfl, hc⟩⟩
  · exact ⟨N.tail e, (hN.mem_kids).mpr ⟨e, ht, h, Or.inl ⟨rfl, hc⟩⟩, Or.inl ⟨rfl, hc⟩⟩

theorem Valid.inSub_last (_hN : N.Valid root) {v c : V} (h : N.InSub v c) :
    c = v ∨ ∃ q, N.InSub v q ∧ c ∈ N.kids q := by
  induction h with
  | refl => exact Or.inl rfl
  | @kid u c' w hc hw ih =>
    rcases ih with rfl | ⟨q, hq, hwq⟩
    · exact Or.inr ⟨u, InSub.refl u, hc⟩
    · exact Or.inr ⟨q, InSub.kid hc hq, hwq⟩

/-- §2, "Child is tested as `lim(other) < lim(u)`, which is `par(other) == edge`". -/
theorem Valid.tail_child_iff (hN : N.Valid root) {t : E} (ht : N.isTree t = true) :
    N.lim (N.tail t) < N.lim (N.head t) ↔ N.par (N.tail t) = some t := by
  constructor
  · intro hl
    rcases hN.tree_par t ht with h | h
    · exact h
    · have hk : N.head t ∈ N.kids (N.tail t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inl ⟨rfl, rfl⟩⟩
      have := hN.kid_lim hk; omega
  · intro h
    have hk : N.tail t ∈ N.kids (N.head t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inr ⟨rfl, rfl⟩⟩
    exact hN.kid_lim hk

theorem Valid.head_child_iff (hN : N.Valid root) {t : E} (ht : N.isTree t = true) :
    N.lim (N.head t) < N.lim (N.tail t) ↔ N.par (N.head t) = some t := by
  constructor
  · intro hl
    rcases hN.tree_par t ht with h | h
    · have hk : N.tail t ∈ N.kids (N.head t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inr ⟨rfl, rfl⟩⟩
      have := hN.kid_lim hk; omega
    · exact h
  · intro h
    have hk : N.head t ∈ N.kids (N.tail t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inl ⟨rfl, rfl⟩⟩
    exact hN.kid_lim hk

/-! ### Fundamental cuts, without a root

Removing tree edge `e` splits the tree in two. `Conn e x y`: `x` and `y` are joined by tree edges
other than `e`, so on the same side. This is the page's "side of the cut", and it mentions no root,
no `par` and no `lim`. -/

/-- Joined by tree edges other than `e`. -/
inductive Conn (e : E) : V → V → Prop
  | refl (x : V) : Conn e x x
  | step {x y z : V} (t : E) : N.isTree t = true → t ≠ e →
      ((N.tail t = y ∧ N.head t = z) ∨ (N.head t = y ∧ N.tail t = z)) → Conn e x y → Conn e x z

theorem Conn.trans {e : E} {x y z : V} (h1 : N.Conn e x y) (h2 : N.Conn e y z) : N.Conn e x z := by
  induction h2 with
  | refl => exact h1
  | step t ht hte hj _ ih => exact Conn.step t ht hte hj ih

theorem Conn.single {e t : E} {y z : V} (ht : N.isTree t = true) (hte : t ≠ e)
    (hj : (N.tail t = y ∧ N.head t = z) ∨ (N.head t = y ∧ N.tail t = z)) : N.Conn e y z :=
  Conn.step t ht hte hj (Conn.refl y)

theorem Conn.symm {e : E} {x y : V} (h : N.Conn e x y) : N.Conn e y x := by
  induction h with
  | refl => exact Conn.refl _
  | step t ht hte hj _ ih =>
    refine (Conn.single ht hte ?_).trans ih
    rcases hj with ⟨a, b⟩ | ⟨a, b⟩
    · exact Or.inr ⟨b, a⟩
    · exact Or.inl ⟨b, a⟩

/-- The page's "crosses t's cut against t": from head(t)'s side to tail(t)'s side. -/
def Against (t g : E) : Prop := N.Conn t (N.head t) (N.tail g) ∧ N.Conn t (N.tail t) (N.head g)

section Side

variable (hN : N.Valid root) {e : E} {v : V} (hv : N.par v = some e)
include hN hv

/-- A tree edge other than `e` has both ends in `subtree(v)` or neither. -/
theorem side_edge {t : E} (ht : N.isTree t = true) (hte : t ≠ e) {y z : V}
    (hj : (N.tail t = y ∧ N.head t = z) ∨ (N.head t = y ∧ N.tail t = z)) :
    N.InSub v y ↔ N.InSub v z := by
  -- the child end `c` of `t` and its parent `q`
  have key : ∀ {c q : V}, c ∈ N.kids q → N.par c = some t → (N.InSub v q ↔ N.InSub v c) := by
    intro c q hc hpc
    constructor
    · intro hq; exact hq.trans (InSub.kid hc (InSub.refl c))
    · intro hvc
      rcases hN.inSub_last hvc with rfl | ⟨q', hq', hcq'⟩
      · rw [hv] at hpc; exact absurd (Option.some.inj hpc).symm hte
      · rw [hN.kid_unique hc hcq']; exact hq'
  rcases hN.tree_par t ht with h | h
  · have hk : N.tail t ∈ N.kids (N.head t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inr ⟨rfl, rfl⟩⟩
    have := key hk h
    rcases hj with ⟨a, b⟩ | ⟨a, b⟩
    · rw [← a, ← b]; exact this.symm
    · rw [← a, ← b]; exact this
  · have hk : N.head t ∈ N.kids (N.tail t) := (hN.mem_kids).mpr ⟨t, ht, h, Or.inl ⟨rfl, rfl⟩⟩
    have := key hk h
    rcases hj with ⟨a, b⟩ | ⟨a, b⟩
    · rw [← a, ← b]; exact this
    · rw [← a, ← b]; exact this.symm

theorem conn_inSub {x y : V} (h : N.Conn e x y) : N.InSub v x ↔ N.InSub v y := by
  induction h with
  | refl => exact Iff.rfl
  | step t ht hte hj _ ih => exact ih.trans (side_edge hN hv ht hte hj)

/-- `v`'s parent end of `e` is not below `v`. -/
theorem parent_end_not_sub {u : V} (hu : v ∈ N.kids u) : ¬N.InSub v u := by
  intro h
  have := hN.kid_lim hu
  have := (hN.sub_bounds h).2.2
  omega

theorem inSub_conn {x : V} (h : N.InSub v x) : N.Conn e v x := by
  have key : ∀ u w, N.InSub u w → N.InSub v u → N.Conn e u w := by
    intro u w huw
    induction huw with
    | refl => intro _; exact Conn.refl _
    | @kid u c w hc _ ih =>
      intro hvu
      have hvc : N.InSub v c := hvu.trans (InSub.kid hc (InSub.refl c))
      obtain ⟨t, ht, hpc, hj⟩ := (hN.mem_kids).mp hc
      have hte : t ≠ e := by
        rintro rfl
        have hcv : c = v := by
          rcases hN.parent hpc with ⟨u1, hu1, _⟩
          rcases hN.parent hv with ⟨u2, hu2, _⟩
          by_contra hne
          -- both `c` and `v` have parent edge `e`: they are its two ends
          obtain ⟨_, hc1⟩ := hN.par_tree c t hpc
          obtain ⟨_, hv1⟩ := hN.par_tree v t hv
          rcases hc1 with hc1 | hc1 <;> rcases hv1 with hv1 | hv1
          · exact hne (hc1.symm.trans hv1)
          · exact hN.par_excl t ht ⟨by rw [hc1]; exact hpc, by rw [hv1]; exact hv⟩
          · exact hN.par_excl t ht ⟨by rw [hv1]; exact hv, by rw [hc1]; exact hpc⟩
          · exact hne (hc1.symm.trans hv1)
        subst hcv
        exact parent_end_not_sub hN hv hc hvu
      exact (Conn.single ht hte hj).trans (ih hvc)
  exact key v x h (InSub.refl v)

theorem not_inSub_conn {u x : V} (hu : v ∈ N.kids u) (h : ¬N.InSub v x) : N.Conn e u x := by
  have key : ∀ a w, N.InSub a w → ¬N.InSub v w → N.Conn e a w := by
    intro a w haw
    induction haw with
    | refl => intro _; exact Conn.refl _
    | @kid a c w hc hcw ih =>
      intro hvw
      obtain ⟨t, ht, hpc, hj⟩ := (hN.mem_kids).mp hc
      have hte : t ≠ e := by
        rintro rfl
        have hcv : c = v := by
          by_contra hne
          obtain ⟨_, hc1⟩ := hN.par_tree c t hpc
          obtain ⟨_, hv1⟩ := hN.par_tree v t hv
          rcases hc1 with hc1 | hc1 <;> rcases hv1 with hv1 | hv1
          · exact hne (hc1.symm.trans hv1)
          · exact hN.par_excl t ht ⟨by rw [hc1]; exact hpc, by rw [hv1]; exact hv⟩
          · exact hN.par_excl t ht ⟨by rw [hv1]; exact hv, by rw [hc1]; exact hpc⟩
          · exact hne (hc1.symm.trans hv1)
        subst hcv
        exact hvw hcw
      exact (Conn.single ht hte hj).trans (ih hvw)
  have h1 := key root x (hN.span x) h
  have h2 := key root u (hN.span u) (parent_end_not_sub hN hv hu)
  exact h2.symm.trans h1

/-- The two sides of e's cut are `subtree(v)` and the rest, whatever the root. -/
theorem side_iff {u x : V} (hu : v ∈ N.kids u) :
    (N.Conn e v x ↔ N.InSub v x) ∧ (N.Conn e u x ↔ ¬N.InSub v x) := by
  refine ⟨⟨fun h => (conn_inSub hN hv h).mp (InSub.refl v), inSub_conn hN hv⟩, ?_⟩
  constructor
  · intro h hx
    exact parent_end_not_sub hN hv hu ((conn_inSub hN hv h).mpr hx)
  · exact not_inSub_conn hN hv hu

end Side

end Net

/-! ## C's scan (§2, "Candidates" and "Order") -/

/-- `dfs_enter_inedge` (`inS`, e's tail is the child) or `dfs_enter_outedge` (`outS`). -/
inductive Dir
  | inS
  | outS
  deriving DecidableEq

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] (N : Net V E)

/-- The node whose scanned list holds `e`: the head for the in-search. -/
def own : Dir → E → V
  | .inS, e => N.head e
  | .outS, e => N.tail e

/-- The other end, which the search tests with `seq`: the tail for the in-search. -/
def near : Dir → E → V
  | .inS, e => N.tail e
  | .outS, e => N.head e

/-- The list the search scans at a node: `ND_in` for the in-search. -/
def scanL : Dir → V → List E
  | .inS, u => N.inE u
  | .outS, u => N.outE u

/-- The tree list of the other direction: `ND_tree_out` for the in-search. -/
def treeL : Dir → V → List E
  | .inS, u => N.treeOut u
  | .outS, u => N.treeIn u

/-- `seq(low(v), lim(x), lim(v))`. -/
def inInt (v x : V) : Prop := N.low v ≤ N.lim x ∧ N.lim x ≤ N.lim v

instance (v x : V) : Decidable (N.inInt v x) := by unfold inInt; infer_instance

/-- The candidates the search meets at `u`, in list order: non-tree edges whose other end is
outside the interval of `v`. -/
def cands (d : Dir) (v u : V) : List E :=
  (N.scanL d u).filter (fun g => !N.isTree g && !decide (N.inInt v (N.near d g)))

/-- The first push loop: children joined by a tree edge of the scanned list. -/
def pushesA (d : Dir) (u : V) : List V :=
  ((N.scanL d u).filter (fun g => N.isTree g && decide (N.lim (N.near d g) < N.lim u))).map (N.near d)

/-- The second push loop, unpruned: children joined by a tree edge of the other direction. -/
def pushesB (d : Dir) (u : V) : List V :=
  ((N.treeL d u).filter (fun g => decide (N.lim (N.own d g) < N.lim u))).map (N.own d)

def pushes (d : Dir) (u : V) : List V := N.pushesA d u ++ N.pushesB d u

/-- The full scan from `u` (no pruning), with fuel: `u`'s candidates in list order, then the
children in reverse push order, each scanned whole. -/
def scanF (d : Dir) (v : V) : ℕ → V → List E
  | 0, _ => []
  | n + 1, u => N.cands d v u ++ (N.pushes d u).reverse.flatMap (scanF d v n)

/-- **The scan order** of §2, for the search under `v`, from `u`. -/
def scanList (d : Dir) (v u : V) : List E := N.scanF d v (N.lim u + 1) u

variable {N} {root : V}

theorem own_near (d : Dir) (e : E) :
    (N.own d e = N.head e ∧ N.near d e = N.tail e) ∨ (N.own d e = N.tail e ∧ N.near d e = N.head e) := by
  cases d
  · exact Or.inl ⟨rfl, rfl⟩
  · exact Or.inr ⟨rfl, rfl⟩

theorem Valid.mem_scanL (hN : N.Valid root) {d : Dir} {u : V} {g : E} :
    g ∈ N.scanL d u ↔ N.own d g = u := by
  cases d
  · exact hN.mem_inE u g
  · exact hN.mem_outE u g

theorem Valid.mem_treeL (hN : N.Valid root) {d : Dir} {u : V} {g : E} :
    g ∈ N.treeL d u ↔ N.isTree g = true ∧ N.near d g = u := by
  cases d
  · exact hN.mem_treeOut u g
  · exact hN.mem_treeIn u g

theorem Valid.nodup_scanL (hN : N.Valid root) (d : Dir) (u : V) : (N.scanL d u).Nodup := by
  cases d
  · exact hN.nodup_inE u
  · exact hN.nodup_outE u

theorem Valid.nodup_treeL (hN : N.Valid root) (d : Dir) (u : V) : (N.treeL d u).Nodup := by
  cases d
  · exact hN.nodup_treeOut u
  · exact hN.nodup_treeIn u

/-- The child test for an edge, whichever direction the search runs. -/
theorem Valid.near_child_iff (hN : N.Valid root) (d : Dir) {t : E} (ht : N.isTree t = true) :
    N.lim (N.near d t) < N.lim (N.own d t) ↔ N.par (N.near d t) = some t := by
  cases d
  · exact hN.tail_child_iff ht
  · exact hN.head_child_iff ht

theorem Valid.own_child_iff (hN : N.Valid root) (d : Dir) {t : E} (ht : N.isTree t = true) :
    N.lim (N.own d t) < N.lim (N.near d t) ↔ N.par (N.own d t) = some t := by
  cases d
  · exact hN.head_child_iff ht
  · exact hN.tail_child_iff ht

theorem Valid.ends_kid (hN : N.Valid root) {d : Dir} {t : E} {a b : V} (ht : N.isTree t = true)
    (hp : N.par a = some t)
    (hab : (N.own d t = b ∧ N.near d t = a) ∨ (N.own d t = a ∧ N.near d t = b)) : a ∈ N.kids b := by
  refine (hN.mem_kids).mpr ⟨t, ht, hp, ?_⟩
  rcases own_near (N := N) d t with ⟨h1, h2⟩ | ⟨h1, h2⟩ <;> rcases hab with ⟨a1, a2⟩ | ⟨a1, a2⟩
  · exact Or.inr ⟨h1 ▸ a1, h2 ▸ a2⟩
  · exact Or.inl ⟨h2 ▸ a2, h1 ▸ a1⟩
  · exact Or.inl ⟨h1 ▸ a1, h2 ▸ a2⟩
  · exact Or.inr ⟨h2 ▸ a2, h1 ▸ a1⟩

/-- The two push loops push exactly the children. -/
theorem Valid.mem_pushes (hN : N.Valid root) {d : Dir} {u x : V} :
    x ∈ N.pushes d u ↔ x ∈ N.kids u := by
  constructor
  · intro h
    simp only [pushes, pushesA, pushesB, List.mem_append, List.mem_map, List.mem_filter,
      Bool.and_eq_true, decide_eq_true_eq] at h
    rcases h with ⟨g, ⟨hg, ht, hl⟩, rfl⟩ | ⟨g, ⟨hg, hl⟩, rfl⟩
    · have ho := (hN.mem_scanL).mp hg
      have hp := (hN.near_child_iff d ht).mp (by rw [ho]; exact hl)
      exact hN.ends_kid ht hp (Or.inl ⟨ho, rfl⟩)
    · obtain ⟨ht, hn⟩ := (hN.mem_treeL).mp hg
      have hp := (hN.own_child_iff d ht).mp (by rw [hn]; exact hl)
      exact hN.ends_kid ht hp (Or.inr ⟨rfl, hn⟩)
  · intro h
    have hlt := hN.kid_lim h
    obtain ⟨t, ht, hp, hj⟩ := (hN.mem_kids).mp h
    simp only [pushes, pushesA, pushesB, List.mem_append, List.mem_map, List.mem_filter,
      Bool.and_eq_true, decide_eq_true_eq]
    rcases own_near (N := N) d t with ⟨h1, h2⟩ | ⟨h1, h2⟩ <;> rcases hj with ⟨a1, a2⟩ | ⟨a1, a2⟩
    · right; refine ⟨t, ⟨(hN.mem_treeL).mpr ⟨ht, by rw [h2, a1]⟩, by rw [h1, a2]; exact hlt⟩, by rw [h1, a2]⟩
    · left
      refine ⟨t, ⟨(hN.mem_scanL).mpr (by rw [h1, a1]), ht, by rw [h2, a2]; exact hlt⟩, by rw [h2, a2]⟩
    · left
      refine ⟨t, ⟨(hN.mem_scanL).mpr (by rw [h1, a1]), ht, by rw [h2, a2]; exact hlt⟩, by rw [h2, a2]⟩
    · right; refine ⟨t, ⟨(hN.mem_treeL).mpr ⟨ht, by rw [h2, a1]⟩, by rw [h1, a2]; exact hlt⟩, by rw [h1, a2]⟩

theorem Valid.scanF_eq (hN : N.Valid root) (d : Dir) (v : V) :
    ∀ n m u, N.lim u < n → N.lim u < m → N.scanF d v n u = N.scanF d v m u := by
  intro n
  induction n with
  | zero => intro _ _ _ h; omega
  | succ n ih =>
    intro m u hn hm
    cases m with
    | zero => omega
    | succ m =>
      simp only [scanF]
      congr 1
      apply List.flatMap_congr
      intro x hx
      have hk := hN.kid_lim ((hN.mem_pushes).mp (List.mem_reverse.mp hx))
      exact ih m x (by omega) (by omega)

theorem Valid.scanList_unfold (hN : N.Valid root) (d : Dir) (v u : V) :
    N.scanList d v u = N.cands d v u ++ (N.pushes d u).reverse.flatMap (N.scanList d v) := by
  simp only [scanList, scanF]
  congr 1
  apply List.flatMap_congr
  intro x hx
  have hk := hN.kid_lim ((hN.mem_pushes).mp (List.mem_reverse.mp hx))
  exact hN.scanF_eq d v _ _ x (by omega) (by omega)

theorem Valid.mem_scanList (hN : N.Valid root) (d : Dir) (v : V) :
    ∀ u g, g ∈ N.scanList d v u ↔ ∃ x, N.InSub u x ∧ g ∈ N.cands d v x := by
  intro u
  induction u using (measure N.lim).wf.induction with
  | h u ih =>
    intro g
    rw [hN.scanList_unfold, List.mem_append, List.mem_flatMap]
    constructor
    · rintro (h | ⟨x, hx, hg⟩)
      · exact ⟨u, InSub.refl u, h⟩
      · have hk := (hN.mem_pushes).mp (List.mem_reverse.mp hx)
        obtain ⟨y, hy, hgy⟩ := (ih x (hN.kid_lim hk) g).mp hg
        exact ⟨y, InSub.kid hk hy, hgy⟩
    · rintro ⟨x, hx, hg⟩
      cases hx with
      | refl => exact Or.inl hg
      | @kid _ c _ hc hcx =>
        exact Or.inr ⟨c, List.mem_reverse.mpr ((hN.mem_pushes).mpr hc),
          (ih c (hN.kid_lim hc) g).mpr ⟨x, hcx, hg⟩⟩

theorem Valid.sub_scanList (hN : N.Valid root) (d : Dir) (v : V) {u x : V} (h : N.InSub u x) :
    List.Sublist (N.scanList d v x) (N.scanList d v u) := by
  induction h with
  | refl => exact List.Sublist.refl _
  | @kid u c w hc _ ih =>
    refine ih.trans ?_
    rw [hN.scanList_unfold d v u]
    refine List.sublist_append_of_sublist_right ?_
    exact sublist_flatMap_of_mem (List.mem_reverse.mpr ((hN.mem_pushes).mpr hc))

theorem Valid.mem_cands (hN : N.Valid root) {d : Dir} {v u : V} {g : E} :
    g ∈ N.cands d v u ↔ N.own d g = u ∧ N.isTree g = false ∧ ¬N.inInt v (N.near d g) := by
  simp only [cands, List.mem_filter, Bool.and_eq_true, Bool.not_eq_true', decide_eq_false_iff_not,
    hN.mem_scanL]

/-- Distinct children have disjoint subtrees. -/
theorem Valid.kids_disjoint (hN : N.Valid root) {u x y z : V} (hx : x ∈ N.kids u) (hy : y ∈ N.kids u)
    (hxy : x ≠ y) (hxz : N.InSub x z) (hyz : N.InSub y z) : False := by
  have h1 := hN.sub_bounds hxz
  have h2 := hN.sub_bounds hyz
  rcases Chain.disjoint hN.low_le (hN.chain u) x hx y hy hxy with h | h <;> omega

theorem Valid.nodup_pushes (hN : N.Valid root) (d : Dir) (u : V) : (N.pushes d u).Nodup := by
  -- a pushed node determines the edge that pushed it: its parent edge
  have pa : ∀ g, g ∈ N.scanL d u → N.isTree g = true → N.lim (N.near d g) < N.lim u →
      N.par (N.near d g) = some g := by
    intro g hg ht hl
    exact (hN.near_child_iff d ht).mp (by rw [(hN.mem_scanL).mp hg]; exact hl)
  have pb : ∀ g, g ∈ N.treeL d u → N.lim (N.own d g) < N.lim u → N.par (N.own d g) = some g := by
    intro g hg hl
    obtain ⟨ht, hn⟩ := (hN.mem_treeL).mp hg
    exact (hN.own_child_iff d ht).mp (by rw [hn]; exact hl)
  unfold pushes pushesA pushesB
  rw [List.nodup_append]
  refine ⟨?_, ?_, ?_⟩
  · refine List.Nodup.map_on ?_ ((hN.nodup_scanL d u).filter _)
    intro a ha b hb hab
    simp only [List.mem_filter, Bool.and_eq_true, decide_eq_true_eq] at ha hb
    have h1 := pa a ha.1 ha.2.1 ha.2.2
    have h2 := pa b hb.1 hb.2.1 hb.2.2
    rw [hab, h2] at h1
    exact (Option.some.inj h1).symm
  · refine List.Nodup.map_on ?_ ((hN.nodup_treeL d u).filter _)
    intro a ha b hb hab
    simp only [List.mem_filter, decide_eq_true_eq] at ha hb
    have h1 := pb a ha.1 ha.2
    have h2 := pb b hb.1 hb.2
    rw [hab, h2] at h1
    exact (Option.some.inj h1).symm
  · intro x hxa y hyb hxy
    subst hxy
    simp only [List.mem_map, List.mem_filter, Bool.and_eq_true, decide_eq_true_eq] at hxa hyb
    obtain ⟨a, ⟨ha, hta, hla⟩, rfl⟩ := hxa
    obtain ⟨b, ⟨hb, hlb⟩, hab⟩ := hyb
    have h1 := pa a ha hta hla
    have h2 := pb b hb hlb
    rw [hab, h1] at h2
    obtain rfl := Option.some.inj h2
    have hn := ((hN.mem_treeL).mp hb).2
    rw [hab, hn] at hlb
    exact lt_irrefl _ hlb

/-- The scan meets every candidate once. -/
theorem Valid.nodup_scanList (hN : N.Valid root) (d : Dir) (v : V) :
    ∀ u, (N.scanList d v u).Nodup := by
  intro u
  induction u using (measure N.lim).wf.induction with
  | h u ih =>
    -- every edge of the scan from `x` is owned by a node of `subtree(x)`
    have owned : ∀ x g, g ∈ N.scanList d v x → N.InSub x (N.own d g) := by
      intro x g hg
      obtain ⟨y, hy, hgy⟩ := (hN.mem_scanList d v x g).mp hg
      rw [((hN.mem_cands).mp hgy).1]; exact hy
    rw [hN.scanList_unfold, List.nodup_append]
    refine ⟨?_, ?_, ?_⟩
    · exact (hN.nodup_scanL d u).filter _
    · rw [List.nodup_flatMap]
      refine ⟨fun x hx => ih x (hN.kid_lim ((hN.mem_pushes).mp (List.mem_reverse.mp hx))), ?_⟩
      refine List.Pairwise.imp_of_mem ?_ (List.nodup_reverse.mpr (hN.nodup_pushes d u))
      intro x y hx hy hxy
      have hx' := (hN.mem_pushes).mp (List.mem_reverse.mp hx)
      have hy' := (hN.mem_pushes).mp (List.mem_reverse.mp hy)
      intro g hgx hgy
      exact hN.kids_disjoint hx' hy' hxy (owned x g hgx) (owned y g hgy)
    · intro g hg g' hg' hgg
      subst hgg
      obtain ⟨x, hx, hgx⟩ := List.mem_flatMap.mp hg'
      have hx' := (hN.mem_pushes).mp (List.mem_reverse.mp hx)
      have h1 := owned x g hgx
      rw [((hN.mem_cands).mp hg).1] at h1
      exact hN.kid_ne hx' (hN.sub_antisymm (InSub.kid hx' (InSub.refl x)) h1).symm

end Net

/-! ## C's search, as a stack machine (§2, "Choice", "Pruning", Lemma 1) -/

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] (N : Net V E)

/-- `todo`, and `(Enter, Slack)`. -/
abbrev St (V E : Type) := List V × (Option E × ℤ)

/-- `if (slack < Slack || Enter == NULL) { Enter = e; Slack = slack; }` -/
def evalE (slack : E → ℤ) (p : Option E × ℤ) (g : E) : Option E × ℤ :=
  if slack g < p.2 ∨ p.1 = none then (some g, slack g) else p

/-- One pass of the first loop of `dfs_enter_inedge`/`dfs_enter_outedge` at node `u`. -/
def loop1 (d : Dir) (v : V) (slack : E → ℤ) (u : V) (s : St V E) (g : E) : St V E :=
  if N.isTree g = false then
    if ¬N.inInt v (N.near d g) then (s.1, evalE slack s.2 g) else s
  else if N.lim (N.near d g) < N.lim u then (N.near d g :: s.1, s.2) else s

/-- One pass of the second loop: `if (Slack <= 0) break;`, then the child push. -/
def loop2 (d : Dir) (u : V) (s : St V E) (g : E) : St V E :=
  if s.2.2 ≤ 0 then s
  else if N.lim (N.own d g) < N.lim u then (N.own d g :: s.1, s.2) else s

/-- `while (todo) { v = pop(todo); …both loops… }`: one iteration. -/
def step (d : Dir) (v : V) (slack : E → ℤ) (s : St V E) : St V E :=
  match s.1 with
  | [] => s
  | u :: rest => (N.treeL d u).foldl (N.loop2 d u) ((N.scanL d u).foldl (N.loop1 d v slack u) (rest, s.2))

/-- `k` iterations. -/
def run (d : Dir) (v : V) (slack : E → ℤ) (k : ℕ) : St V E → St V E := (N.step d v slack)^[k]

/-- `INT_MAX`, `Slack`'s initial value. -/
def intMax : ℤ := 2147483647

/-- C's `enter_edge(e)`, run for `k` iterations of its loop. -/
def cEnter (slack : E → ℤ) (k : ℕ) (e : E) : Option E :=
  if N.lim (N.tail e) < N.lim (N.head e) then
    (N.run .inS (N.tail e) slack k ([N.tail e], (none, intMax))).2.1
  else (N.run .outS (N.head e) slack k ([N.head e], (none, intMax))).2.1

variable {N} {root : V}

/-- Once `Slack ≤ 0`, `Enter` is set. -/
def Ok (p : Option E × ℤ) : Prop := p.2 ≤ 0 → p.1 ≠ none

/-- `Enter` is set and `Slack ≤ 0`: no candidate of nonnegative slack replaces it. -/
def Settled (p : Option E × ℤ) : Prop := p.1 ≠ none ∧ p.2 ≤ 0

theorem evalE_ok {slack : E → ℤ} {p : Option E × ℤ} (g : E) (h : Ok p) : Ok (evalE slack p g) := by
  unfold evalE Ok at *
  split_ifs <;> simp_all

theorem foldl_evalE_ok {slack : E → ℤ} (l : List E) {p : Option E × ℤ} (h : Ok p) :
    Ok (l.foldl (evalE slack) p) := by
  induction l generalizing p with
  | nil => exact h
  | cons g l ih => exact ih (evalE_ok g h)

theorem foldl_evalE_settled {slack : E → ℤ} (l : List E) {p : Option E × ℤ} (h : Settled p)
    (hl : ∀ g ∈ l, 0 ≤ slack g) : l.foldl (evalE slack) p = p := by
  induction l with
  | nil => rfl
  | cons g l ih =>
    rw [List.foldl_cons]
    have : evalE slack p g = p := by
      unfold evalE
      rw [ite_eq_right]
      have := hl g (by simp)
      have := h.2
      have := h.1
      push Not
      constructor <;> omega
    rw [this]; exact ih (fun x hx => hl x (by simp [hx]))

theorem foldl_loop1 (d : Dir) (v : V) (slack : E → ℤ) (u : V) :
    ∀ (l : List E) (todo : List V) (p : Option E × ℤ),
      l.foldl (N.loop1 d v slack u) (todo, p) =
        ((((l.filter (fun g => N.isTree g && decide (N.lim (N.near d g) < N.lim u))).map
          (N.near d)).reverse ++ todo),
          (l.filter (fun g => !N.isTree g && !decide (N.inInt v (N.near d g)))).foldl (evalE slack) p) := by
  intro l
  induction l with
  | nil => intro todo p; simp
  | cons g l ih =>
    intro todo p
    rw [List.foldl_cons]
    by_cases ht : N.isTree g = true
    · by_cases hl : N.lim (N.near d g) < N.lim u
      · have : N.loop1 d v slack u (todo, p) g = (N.near d g :: todo, p) := by
          simp [loop1, ht, hl]
        rw [this, ih]
        simp [ht, hl]
      · have : N.loop1 d v slack u (todo, p) g = (todo, p) := by
          simp [loop1, ht, hl]
        rw [this, ih]
        simp [ht, hl]
    · have ht' : N.isTree g = false := by simpa using ht
      by_cases hi : N.inInt v (N.near d g)
      · have : N.loop1 d v slack u (todo, p) g = (todo, p) := by
          simp [loop1, ht', hi]
        rw [this, ih]
        simp [ht', hi]
      · have : N.loop1 d v slack u (todo, p) g = (todo, evalE slack p g) := by
          simp [loop1, ht', hi]
        rw [this, ih]
        simp [ht', hi]

theorem foldl_loop2 (d : Dir) (u : V) :
    ∀ (l : List E) (todo : List V) (p : Option E × ℤ),
      l.foldl (N.loop2 d u) (todo, p) =
        if p.2 ≤ 0 then (todo, p)
        else ((((l.filter (fun g => decide (N.lim (N.own d g) < N.lim u))).map (N.own d)).reverse ++
          todo), p) := by
  intro l
  induction l with
  | nil => intro todo p; simp
  | cons g l ih =>
    intro todo p
    rw [List.foldl_cons]
    by_cases hp : p.2 ≤ 0
    · have : N.loop2 d u (todo, p) g = (todo, p) := by simp [loop2, hp]
      rw [this, ih]; simp [hp]
    · by_cases hl : N.lim (N.own d g) < N.lim u
      · have : N.loop2 d u (todo, p) g = (N.own d g :: todo, p) := by simp [loop2, hp, hl]
        rw [this, ih]; simp [hp, hl]
      · have : N.loop2 d u (todo, p) g = (todo, p) := by simp [loop2, hp, hl]
        rw [this, ih]; simp [hp, hl]

/-- One iteration of the loop, popping `u`: its candidates are evaluated in list order, then its
children are pushed, the second loop's only while `Slack > 0`. -/
theorem step_cons (d : Dir) (v : V) (slack : E → ℤ) (u : V) (rest : List V) (p : Option E × ℤ) :
    N.step d v slack (u :: rest, p) =
      let p1 := (N.cands d v u).foldl (evalE slack) p
      if p1.2 ≤ 0 then ((N.pushesA d u).reverse ++ rest, p1)
      else ((N.pushes d u).reverse ++ rest, p1) := by
  simp only [step]
  rw [foldl_loop1, foldl_loop2]
  simp only [cands, pushes, pushesA, pushesB, List.reverse_append, List.append_assoc]

theorem run_add (d : Dir) (v : V) (slack : E → ℤ) (a b : ℕ) (s : St V E) :
    N.run d v slack (a + b) s = N.run d v slack b (N.run d v slack a s) := by
  simp only [run]
  rw [add_comm, Function.iterate_add_apply]

theorem run_nil (d : Dir) (v : V) (slack : E → ℤ) (k : ℕ) (p : Option E × ℤ) :
    N.run d v slack k ([], p) = ([], p) := by
  induction k with
  | zero => rfl
  | succ k ih => rw [run, Function.iterate_succ_apply', ← run, ih]; rfl

/-- The search under `v`, from `u` on top of the stack: it consumes `u`'s block and leaves `rest`,
having evaluated, of the full scan order from `u`, everything that could change `(Enter, Slack)`. -/
theorem Valid.block (hN : N.Valid root) (d : Dir) (v : V) (slack : E → ℤ) :
    ∀ u, (∀ g ∈ N.scanList d v u, 0 ≤ slack g) → ∀ rest p, Ok p →
      ∃ k, N.run d v slack k (u :: rest, p) = (rest, (N.scanList d v u).foldl (evalE slack) p) := by
  intro u
  induction u using (measure N.lim).wf.induction with
  | h u ih =>
    intro hnn rest p hp
    -- the children's blocks, one after the other
    have blocks : ∀ (ks : List V), (∀ x ∈ ks, x ∈ N.kids u) → ∀ (q : Option E × ℤ), Ok q →
        ∃ k, N.run d v slack k (ks ++ rest, q) =
          (rest, (ks.flatMap (N.scanList d v)).foldl (evalE slack) q) := by
      intro ks
      induction ks with
      | nil => intro _ q _; exact ⟨0, rfl⟩
      | cons x ks ihks =>
        intro hks q hq
        have hx := hks x (by simp)
        have hnnx : ∀ g ∈ N.scanList d v x, 0 ≤ slack g := fun g hg =>
          hnn g ((hN.sub_scanList d v (InSub.kid hx (InSub.refl x))).subset hg)
        obtain ⟨k1, hk1⟩ := ih x (hN.kid_lim hx) hnnx (ks ++ rest) q hq
        obtain ⟨k2, hk2⟩ := ihks (fun y hy => hks y (by simp [hy])) _ (foldl_evalE_ok _ hq)
        refine ⟨k1 + k2, ?_⟩
        rw [run_add, List.cons_append, hk1, hk2, List.flatMap_cons, List.foldl_append]
    have hpush : ∀ x ∈ (N.pushes d u).reverse, x ∈ N.kids u :=
      fun x hx => (hN.mem_pushes).mp (List.mem_reverse.mp hx)
    have hpushA : ∀ x ∈ (N.pushesA d u).reverse, x ∈ N.kids u :=
      fun x hx => (hN.mem_pushes).mp (List.mem_append_left _ (List.mem_reverse.mp hx))
    set p1 := (N.cands d v u).foldl (evalE slack) p with hp1
    have hok1 : Ok p1 := foldl_evalE_ok _ hp
    have hunf : (N.scanList d v u).foldl (evalE slack) p =
        ((N.pushes d u).reverse.flatMap (N.scanList d v)).foldl (evalE slack) p1 := by
      rw [hN.scanList_unfold, List.foldl_append]
    by_cases hs : p1.2 ≤ 0
    · -- pruned: `Slack` is 0 now, and nothing after can replace `Enter`
      have hset : Settled p1 := ⟨hok1 hs, hs⟩
      obtain ⟨k, hk⟩ := blocks _ hpushA p1 hok1
      refine ⟨1 + k, ?_⟩
      rw [run_add]
      have h1 : N.run d v slack 1 (u :: rest, p) = ((N.pushesA d u).reverse ++ rest, p1) := by
        show N.step d v slack (u :: rest, p) = _
        rw [step_cons]; simp only [← hp1, hs, ite_true]
      rw [h1, hk, hunf]
      have hnnl : ∀ (l : List V), (∀ x ∈ l, x ∈ N.kids u) →
          ∀ g ∈ l.flatMap (N.scanList d v), 0 ≤ slack g := by
        intro l hl g hg
        obtain ⟨x, hx, hgx⟩ := List.mem_flatMap.mp hg
        exact hnn g ((hN.sub_scanList d v (InSub.kid (hl x hx) (InSub.refl x))).subset hgx)
      rw [foldl_evalE_settled _ hset (hnnl _ hpushA), foldl_evalE_settled _ hset (hnnl _ hpush)]
    · obtain ⟨k, hk⟩ := blocks _ hpush p1 hok1
      refine ⟨1 + k, ?_⟩
      rw [run_add]
      have h1 : N.run d v slack 1 (u :: rest, p) = ((N.pushes d u).reverse ++ rest, p1) := by
        show N.step d v slack (u :: rest, p) = _
        rw [step_cons]; simp only [← hp1, hs, ite_false]
      rw [h1, hk, hunf]

/-- **Lemma 1.** If no candidate has a negative slack, C's search, run to the end of its loop,
returns the result of evaluating the full scan order: the first candidate of minimum slack. -/
theorem Valid.lemma1 (hN : N.Valid root) (d : Dir) (v : V) (slack : E → ℤ)
    (hnn : ∀ g ∈ N.scanList d v v, 0 ≤ slack g) :
    ∃ K, ∀ k, K ≤ k → N.run d v slack k ([v], (none, intMax)) =
      ([], (N.scanList d v v).foldl (evalE slack) (none, intMax)) := by
  obtain ⟨K, hK⟩ := hN.block d v slack v hnn [] (none, intMax) (by simp [Ok, intMax])
  refine ⟨K, fun k hk => ?_⟩
  obtain ⟨j, rfl⟩ := Nat.exists_eq_add_of_le hk
  rw [run_add, hK, run_nil]

/-- What the evaluation fold returns: the first of minimum slack. `R` is "comes first". -/
theorem foldl_evalE_spec {slack : E → ℤ} {R : E → E → Prop} :
    ∀ (l : List E), l.Pairwise R →
      ((l.foldl (evalE slack) (none, intMax)).1 = none ↔ l = []) ∧
      ∀ m, (l.foldl (evalE slack) (none, intMax)).1 = some m →
        (l.foldl (evalE slack) (none, intMax)).2 = slack m ∧ m ∈ l ∧
        ∀ x ∈ l, x ≠ m → slack m < slack x ∨ (slack m = slack x ∧ R m x) := by
  intro l
  induction l using List.reverseRecOn with
  | nil => intro _; simp
  | append_singleton l g ih =>
    intro hp
    rw [List.pairwise_append] at hp
    obtain ⟨hpl, _, hpg⟩ := hp
    obtain ⟨ih1, ih2⟩ := ih hpl
    rw [List.foldl_append, List.foldl_cons, List.foldl_nil]
    set q := l.foldl (evalE slack) (none, intMax) with hq
    refine ⟨?_, ?_⟩
    · simp only [evalE]
      split_ifs <;> simp_all
    · intro m hm
      cases hqm : q.1 with
      | none =>
        have hl : l = [] := ih1.mp hqm
        subst hl
        simp only [evalE, hqm, or_true, ite_true] at hm ⊢
        obtain rfl := Option.some.inj hm
        simp
      | some w =>
        obtain ⟨hw1, hw2, hw3⟩ := ih2 w hqm
        by_cases hlt : slack g < q.2
        · simp only [evalE, hlt, true_or, ite_true] at hm ⊢
          obtain rfl := Option.some.inj hm
          refine ⟨rfl, by simp, ?_⟩
          intro x hx hxm
          rcases List.mem_append.mp hx with hx | hx
          · left
            by_cases hxw : x = w
            · subst hxw; omega
            · rcases hw3 x hx hxw with h | ⟨h, _⟩ <;> omega
          · simp at hx; exact absurd hx hxm
        · have hne : ¬(slack g < q.2 ∨ q.1 = none) := by simp [hlt, hqm]
          simp only [evalE, hne, ite_false] at hm ⊢
          rw [hqm] at hm
          obtain rfl := Option.some.inj hm
          refine ⟨hw1, List.mem_append_left _ hw2, ?_⟩
          intro x hx hxm
          rcases List.mem_append.mp hx with hx | hx
          · exact hw3 x hx hxm
          · simp at hx; subst hx
            rw [hw1] at hlt
            rcases lt_or_eq_of_le (not_lt.mp hlt) with h | h
            · exact Or.inl h
            · exact Or.inr ⟨h, hpg w hw2 x (by simp)⟩

end Net

/-! ## §2, "Candidates": the scan meets exactly the edges crossing e's cut against e -/

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] {N : Net V E} {root : V}

/-- **§2, Candidates.** For the tree edge `e` with child `v`, the search C runs (in-search if `v` is
e's tail) meets exactly the non-tree edges that cross e's cut against `e`. `Against` mentions no
root, so neither does this set. -/
theorem Valid.cands_iff (hN : N.Valid root) {e : E} {v : V} (hv : N.par v = some e) (d : Dir)
    (hd : (v = N.tail e ∧ d = .inS) ∨ (v = N.head e ∧ d = .outS)) (g : E) :
    g ∈ N.scanList d v v ↔ N.isTree g = false ∧ N.Against e g := by
  obtain ⟨u, hu, hj⟩ := hN.parent hv
  have side := fun x => side_iff hN hv hu (x := x)
  rw [hN.mem_scanList d v v g]
  have hint : ∀ x, N.inInt v x ↔ N.InSub v x := fun x => (hN.inSub_iff v x).symm
  constructor
  · rintro ⟨x, hx, hg⟩
    obtain ⟨ho, ht, hn⟩ := (hN.mem_cands).mp hg
    rw [hint] at hn
    refine ⟨ht, ?_⟩
    unfold Against
    rcases hd with ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩
    · have hue : N.head e = u := by
        rcases hj with ⟨a, b⟩ | ⟨a, b⟩
        · exact absurd a (hN.kid_ne hu)
        · exact a
      rw [hue]
      exact ⟨(side _).2.2 hn, (side _).1.2 (by subst ho; exact hx)⟩
    · have hue : N.tail e = u := by
        rcases hj with ⟨a, b⟩ | ⟨a, b⟩
        · exact a
        · exact absurd a (hN.kid_ne hu)
      rw [hue]
      exact ⟨(side _).1.2 (by subst ho; exact hx), (side _).2.2 hn⟩
  · rintro ⟨ht, h1, h2⟩
    rcases hd with ⟨rfl, rfl⟩ | ⟨rfl, rfl⟩
    · have hue : N.head e = u := by
        rcases hj with ⟨a, b⟩ | ⟨a, b⟩
        · exact absurd a (hN.kid_ne hu)
        · exact a
      rw [hue] at h1
      exact ⟨N.head g, (side _).1.1 h2, (hN.mem_cands).mpr ⟨rfl, ht, by rw [hint]; exact (side _).2.1 h1⟩⟩
    · have hue : N.tail e = u := by
        rcases hj with ⟨a, b⟩ | ⟨a, b⟩
        · exact a
        · exact absurd a (hN.kid_ne hu)
      rw [hue] at h2
      exact ⟨N.tail g, (side _).1.1 h1, (hN.mem_cands).mpr ⟨rfl, ht, by rw [hint]; exact (side _).2.1 h2⟩⟩

end Net

/-! ## §4.1: the scan order from the parent pointers -/

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] (N : Net V E)

/-- `parent(g, u)`: the other end of `par(u)`. -/
def parentOf (u : V) : Option V :=
  (N.par u).map (fun t => if N.tail t = u then N.head t else N.tail t)

/-- `scanned_before`'s `climb`: `u`, then its ancestors up to `v`, with fuel. -/
def climb (v : V) : ℕ → V → List V
  | 0, u => [u]
  | k + 1, u =>
    if u = v then [u]
    else match N.parentOf u with
      | none => [u]
      | some p => u :: climb v k p

/-- Rust's lexicographic `>` on `(u8, usize)`, the other way round. -/
def lexLt (a b : ℕ × ℕ) : Prop := a.1 < b.1 ∨ (a.1 = b.1 ∧ a.2 < b.2)

instance (a b : ℕ × ℕ) : Decidable (lexLt a b) := by unfold lexLt; infer_instance

/-- `push_rank`: `(0, index in the scanned list of u)` for a child joined by an edge of that list,
else `(1, index in u's other-direction tree list)`. -/
def pushRank (d : Dir) (u child : V) : ℕ × ℕ :=
  match N.par child with
  | none => (0, 0)
  | some pe => if N.own d pe = u then (0, (N.scanL d u).idxOf pe) else (1, (N.treeL d u).idxOf pe)

/-- `first_of`: `a` is met before `b` in the list. -/
def firstOf (l : List E) (a b : E) : Bool := l.find? (fun x => x = a ∨ x = b) == some a

/-- The number of leading pairs that agree: `zip(..).take_while(x == y).count()`. -/
def commonLen (a b : List V) : ℕ := ((a.zip b).takeWhile (fun xy => xy.1 = xy.2)).length

/-- The comparison at the LCA, as `scanned_before` writes it on the two climbs reversed (from `v`
down). `ra[k]` is `at(a, common + 1)`; the LCA is `at(a, common)`, or `v`. -/
def climbDecide (v : V) (d : Dir) (ra rb : List V) : Bool :=
  let k := commonLen ra rb
  match ra[k]?, rb[k]? with
  | none, _ => true
  | some _, none => false
  | some ca, some cb =>
    let lca := if k = 0 then v else (ra[k - 1]?).getD v
    decide (lexLt (N.pushRank d lca cb) (N.pushRank d lca ca))

/-- **`scanned_before(c, w)`**: whether candidate `c` comes before `w` in C's scan under `v`. -/
def scannedBefore (d : Dir) (v : V) (c w : E) : Bool :=
  if N.own d c = N.own d w then Net.firstOf (N.scanL d (N.own d c)) c w
  else N.climbDecide v d (N.climb v (N.lim v + 1) (N.own d c)).reverse
    (N.climb v (N.lim v + 1) (N.own d w)).reverse

/-- The same comparison, by recursion on the two paths below the common part. -/
def decideFrom (d : Dir) (lca : V) : List V → List V → Bool
  | [], _ => true
  | _ :: _, [] => false
  | x :: xs, y :: ys =>
    if x = y then decideFrom d x xs ys else decide (lexLt (N.pushRank d lca y) (N.pushRank d lca x))

/-- `Down a l b`: `l` is the path from a child of `a` down to `b`. -/
inductive Down : V → List V → V → Prop
  | nil (a : V) : Down a [] a
  | cons {a c b : V} {l : List V} : c ∈ N.kids a → Down c l b → Down a (c :: l) b

variable {N} {root : V}

theorem commonLen_cons (x y : V) (xs ys : List V) :
    commonLen (x :: xs) (y :: ys) = if x = y then commonLen xs ys + 1 else 0 := by
  unfold commonLen
  rw [List.zip_cons_cons, List.takeWhile_cons]
  by_cases h : x = y <;> simp [h]

theorem commonLen_le (a b : List V) : commonLen a b ≤ a.length := by
  unfold commonLen
  calc ((a.zip b).takeWhile (fun xy => xy.1 = xy.2)).length ≤ (a.zip b).length :=
        (List.takeWhile_sublist _).length_le
    _ ≤ a.length := by simp

theorem climbDecide_eq (d : Dir) (v : V) (xs ys : List V) :
    N.climbDecide v d (v :: xs) (v :: ys) = N.decideFrom d v xs ys := by
  -- generalise the default LCA
  have key : ∀ (xs ys : List V) (l : V),
      (match xs[commonLen xs ys]?, ys[commonLen xs ys]? with
        | none, _ => true
        | some _, none => false
        | some ca, some cb =>
          decide (lexLt (N.pushRank d (if commonLen xs ys = 0 then l
            else (xs[commonLen xs ys - 1]?).getD l) cb)
            (N.pushRank d (if commonLen xs ys = 0 then l
              else (xs[commonLen xs ys - 1]?).getD l) ca))) = N.decideFrom d l xs ys := by
    intro xs
    induction xs with
    | nil => intro ys l; simp [decideFrom, commonLen]
    | cons x xs ih =>
      intro ys l
      cases ys with
      | nil => simp [decideFrom, commonLen]
      | cons y ys =>
        rw [commonLen_cons]
        by_cases hxy : x = y
        · subst hxy
          simp only [ite_true, decideFrom]
          rw [← ih ys x]
          cases hk : commonLen xs ys with
          | zero => simp
          | succ n =>
            have hlen : n + 1 ≤ xs.length := hk ▸ commonLen_le xs ys
            have hz : xs[n]? = some xs[n] := List.getElem?_eq_getElem (by omega)
            simp [hz]
        · simp [hxy, decideFrom]
  unfold climbDecide
  rw [commonLen_cons, ite_eq_left rfl, ← key xs ys v]
  cases hk : commonLen xs ys <;> simp

theorem Down.inSub {a b : V} {l : List V} (h : N.Down a l b) : N.InSub a b := by
  induction h with
  | nil => exact InSub.refl _
  | cons hc _ ih => exact InSub.kid hc ih

theorem Down.snoc {a b c : V} {l : List V} (h : N.Down a l b) (hc : c ∈ N.kids b) :
    N.Down a (l ++ [c]) c := by
  induction h with
  | nil => exact Down.cons hc (Down.nil c)
  | cons hk _ ih => exact Down.cons hk (ih hc)

theorem Valid.parentOf_kid (hN : N.Valid root) {q u : V} (h : u ∈ N.kids q) :
    N.parentOf u = some q := by
  obtain ⟨t, _, hp, hj⟩ := (hN.mem_kids).mp h
  have hne := hN.kid_ne h
  unfold parentOf
  rw [hp]
  simp only [Option.map_some, Option.some.injEq]
  rcases hj with ⟨a, b⟩ | ⟨a, b⟩
  · rw [ite_eq_right (by rw [a]; exact Ne.symm hne)]; exact a
  · rw [ite_eq_left b, a]

/-- The climb from `u` up to `v`, reversed, is the path from `v` down to `u`. -/
theorem Valid.climb_down (hN : N.Valid root) (v : V) :
    ∀ k u, N.InSub v u → N.lim v < N.lim u + k →
      ∃ l, (N.climb v k u).reverse = v :: l ∧ N.Down v l u := by
  intro k
  induction k with
  | zero =>
    intro u hu hk
    have := (hN.sub_bounds hu).2.2
    omega
  | succ k ih =>
    intro u hu hk
    by_cases huv : u = v
    · subst huv; exact ⟨[], by simp [climb], Down.nil u⟩
    · rcases hN.inSub_last hu with h | ⟨q, hq, huq⟩
      · exact absurd h huv
      · have hlt := hN.kid_lim huq
        obtain ⟨l, hl, hd⟩ := ih q hq (by omega)
        refine ⟨l ++ [u], ?_, hd.snoc huq⟩
        simp only [climb, huv, ite_false, hN.parentOf_kid huq, List.reverse_cons, hl,
          List.cons_append]

/-- A child, and the edge that joins it to its parent, as each search loop sees it. -/
theorem Valid.kid_class (hN : N.Valid root) (d : Dir) {a x : V} (hx : x ∈ N.kids a) :
    ∃ pe, N.par x = some pe ∧ N.isTree pe = true ∧
      ((pe ∈ N.scanL d a ∧ N.own d pe = a ∧ N.near d pe = x) ∨
       (pe ∈ N.treeL d a ∧ N.near d pe = a ∧ N.own d pe = x)) := by
  obtain ⟨t, ht, hp, hj⟩ := (hN.mem_kids).mp hx
  refine ⟨t, hp, ht, ?_⟩
  rcases own_near (N := N) d t with ⟨h1, h2⟩ | ⟨h1, h2⟩ <;> rcases hj with ⟨a1, a2⟩ | ⟨a1, a2⟩
  · right; exact ⟨(hN.mem_treeL).mpr ⟨ht, by rw [h2, a1]⟩, by rw [h2, a1], by rw [h1, a2]⟩
  · left; exact ⟨(hN.mem_scanL).mpr (by rw [h1, a1]), by rw [h1, a1], by rw [h2, a2]⟩
  · left; exact ⟨(hN.mem_scanL).mpr (by rw [h1, a1]), by rw [h1, a1], by rw [h2, a2]⟩
  · right; exact ⟨(hN.mem_treeL).mpr ⟨ht, by rw [h2, a1]⟩, by rw [h2, a1], by rw [h1, a2]⟩

theorem Valid.pushRank_of_scan (_hN : N.Valid root) {d : Dir} {a x : V} {pe : E}
    (hp : N.par x = some pe) (ho : N.own d pe = a) :
    N.pushRank d a x = (0, (N.scanL d a).idxOf pe) := by
  simp [pushRank, hp, ho]

theorem Valid.pushRank_of_tree (_hN : N.Valid root) {d : Dir} {a x : V} {pe : E}
    (hp : N.par x = some pe) (ho : N.own d pe ≠ a) :
    N.pushRank d a x = (1, (N.treeL d a).idxOf pe) := by
  simp [pushRank, hp, ho]

/-- Children in push-rank order are in push order. -/
theorem Valid.pushRank_sublist (hN : N.Valid root) (d : Dir) {a x y : V} (hx : x ∈ N.kids a)
    (hy : y ∈ N.kids a) (h : lexLt (N.pushRank d a x) (N.pushRank d a y)) :
    List.Sublist [x, y] (N.pushes d a) := by
  obtain ⟨px, hpx, htx, hcx⟩ := hN.kid_class d hx
  obtain ⟨py, hpy, hty, hcy⟩ := hN.kid_class d hy
  have hlx := hN.kid_lim hx
  have hly := hN.kid_lim hy
  have hA : ∀ {z : V} {pz : E}, pz ∈ N.scanL d a → N.isTree pz = true → N.near d pz = z →
      N.lim z < N.lim a → pz ∈ (N.scanL d a).filter
        (fun g => N.isTree g && decide (N.lim (N.near d g) < N.lim a)) := by
    intro z pz h1 h2 h3 h4
    simp [List.mem_filter, h1, h2, h3, h4]
  have hB : ∀ {z : V} {pz : E}, pz ∈ N.treeL d a → N.own d pz = z →
      N.lim z < N.lim a → pz ∈ (N.treeL d a).filter (fun g => decide (N.lim (N.own d g) < N.lim a)) := by
    intro z pz h1 h3 h4
    simp [List.mem_filter, h1, h3, h4]
  unfold pushes pushesA pushesB
  rcases hcx with ⟨sx, ox, nx⟩ | ⟨sx, nx, ox⟩ <;> rcases hcy with ⟨sy, oy, ny⟩ | ⟨sy, ny, oy⟩
  · rw [hN.pushRank_of_scan hpx ox, hN.pushRank_of_scan hpy oy] at h
    have hlt : (N.scanL d a).idxOf px < (N.scanL d a).idxOf py := by
      rcases h with h | ⟨_, h⟩
      · simp at h
      · exact h
    have hs := pair_sublist_of_idxOf_lt sy hlt
    have hf := hs.filter (fun g => N.isTree g && decide (N.lim (N.near d g) < N.lim a))
    have e1 : [px, py].filter (fun g => N.isTree g && decide (N.lim (N.near d g) < N.lim a)) =
        [px, py] := by
      simp [htx, hty, nx, ny, hlx, hly]
    rw [e1] at hf
    have hm := hf.map (N.near d)
    simp only [List.map_cons, List.map_nil, nx, ny] at hm
    exact hm.trans (List.sublist_append_left _ _)
  · refine pair_sublist_append ?_ ?_
    · exact List.mem_map.mpr ⟨px, hA sx htx nx hlx, nx⟩
    · exact List.mem_map.mpr ⟨py, hB sy oy hly, oy⟩
  · have ox' : N.own d px ≠ a := by rw [ox]; exact hN.kid_ne hx
    rw [hN.pushRank_of_tree hpx ox', hN.pushRank_of_scan hpy oy] at h
    rcases h with h | ⟨h, _⟩ <;> simp at h
  · have ox' : N.own d px ≠ a := by rw [ox]; exact hN.kid_ne hx
    have oy' : N.own d py ≠ a := by rw [oy]; exact hN.kid_ne hy
    rw [hN.pushRank_of_tree hpx ox', hN.pushRank_of_tree hpy oy'] at h
    have hlt : (N.treeL d a).idxOf px < (N.treeL d a).idxOf py := by
      rcases h with h | ⟨_, h⟩
      · simp at h
      · exact h
    have hs := pair_sublist_of_idxOf_lt sy hlt
    have hf := hs.filter (fun g => decide (N.lim (N.own d g) < N.lim a))
    have e1 : [px, py].filter (fun g => decide (N.lim (N.own d g) < N.lim a)) = [px, py] := by
      simp [ox, oy, hlx, hly]
    rw [e1] at hf
    have hm := hf.map (N.own d)
    simp only [List.map_cons, List.map_nil, ox, oy] at hm
    exact hm.trans (List.sublist_append_right _ _)

theorem Valid.pushRank_inj (hN : N.Valid root) (d : Dir) {a x y : V} (hx : x ∈ N.kids a)
    (hy : y ∈ N.kids a) (h : N.pushRank d a x = N.pushRank d a y) : x = y := by
  obtain ⟨px, hpx, _, hcx⟩ := hN.kid_class d hx
  obtain ⟨py, hpy, _, hcy⟩ := hN.kid_class d hy
  rcases hcx with ⟨sx, ox, nx⟩ | ⟨sx, nx, ox⟩ <;> rcases hcy with ⟨sy, oy, ny⟩ | ⟨sy, ny, oy⟩
  · rw [hN.pushRank_of_scan hpx ox, hN.pushRank_of_scan hpy oy] at h
    have := (List.idxOf_inj sx).mp (Prod.mk.inj h).2
    rw [← nx, ← ny, this]
  · have oy' : N.own d py ≠ a := by rw [oy]; exact hN.kid_ne hy
    rw [hN.pushRank_of_scan hpx ox, hN.pushRank_of_tree hpy oy'] at h
    simp at h
  · have ox' : N.own d px ≠ a := by rw [ox]; exact hN.kid_ne hx
    rw [hN.pushRank_of_tree hpx ox', hN.pushRank_of_scan hpy oy] at h
    simp at h
  · have ox' : N.own d px ≠ a := by rw [ox]; exact hN.kid_ne hx
    have oy' : N.own d py ≠ a := by rw [oy]; exact hN.kid_ne hy
    rw [hN.pushRank_of_tree hpx ox', hN.pushRank_of_tree hpy oy'] at h
    have := (List.idxOf_inj sx).mp (Prod.mk.inj h).2
    rw [← ox, ← oy, this]

theorem firstOf_spec {l : List E} {a b : E} (hab : a ≠ b) (ha : a ∈ l) (hb : b ∈ l) :
    (Net.firstOf l a b = true → List.Sublist [a, b] l) ∧
    (Net.firstOf l a b = false → List.Sublist [b, a] l) := by
  induction l with
  | nil => simp at ha
  | cons z l ih =>
    by_cases hza : z = a
    · subst hza
      have hf : (z :: l).find? (fun x => decide (x = z ∨ x = b)) = some z := by simp
      simp only [Net.firstOf, hf, beq_self_eq_true, Bool.true_eq_false, false_implies, and_true,
        true_implies]
      exact List.Sublist.cons_cons _ (List.singleton_sublist.mpr
        ((List.mem_cons.mp hb).resolve_left (Ne.symm hab)))
    · by_cases hzb : z = b
      · subst hzb
        have hf : (z :: l).find? (fun x => decide (x = a ∨ x = z)) = some z := by simp
        have hne : (some z == some a) = false := by simp [hza]
        simp only [Net.firstOf, hf, hne, Bool.false_eq_true, false_implies, true_and, true_implies]
        exact List.Sublist.cons_cons _ (List.singleton_sublist.mpr
          ((List.mem_cons.mp ha).resolve_left (Ne.symm hza)))
      · have hf : (z :: l).find? (fun x => decide (x = a ∨ x = b)) =
            l.find? (fun x => decide (x = a ∨ x = b)) := by simp [hza, hzb]
        have ha' := (List.mem_cons.mp ha).resolve_left (Ne.symm hza)
        have hb' := (List.mem_cons.mp hb).resolve_left (Ne.symm hzb)
        obtain ⟨i1, i2⟩ := ih ha' hb'
        unfold Net.firstOf at i1 i2 ⊢
        rw [hf]
        exact ⟨fun h => (i1 h).cons _, fun h => (i2 h).cons _⟩

/-- The comparison at the LCA orders two candidates as the scan does. -/
theorem Valid.decideFrom_spec (hN : N.Valid root) (d : Dir) (v : V) :
    ∀ (xs : List V) (a : V) (ys : List V) (sc sw : V) (c w : E), N.Down a xs sc → N.Down a ys sw →
      c ∈ N.cands d v sc → w ∈ N.cands d v sw → sc ≠ sw →
      (N.decideFrom d a xs ys = true → List.Sublist [c, w] (N.scanList d v a)) ∧
      (N.decideFrom d a xs ys = false → List.Sublist [w, c] (N.scanList d v a)) := by
  -- an edge met from a child's whole block
  have inBlock : ∀ {a x s : V} {l : List V} {g : E}, x ∈ N.kids a → N.Down x l s →
      g ∈ N.cands d v s → g ∈ (N.pushes d a).reverse.flatMap (N.scanList d v) ∧
        g ∈ N.scanList d v x := by
    intro a x s l g hx hd hg
    have hgx : g ∈ N.scanList d v x := (hN.mem_scanList d v x g).mpr ⟨s, hd.inSub, hg⟩
    exact ⟨List.mem_flatMap.mpr ⟨x, List.mem_reverse.mpr ((hN.mem_pushes).mpr hx), hgx⟩, hgx⟩
  intro xs
  induction xs with
  | nil =>
    intro a ys sc sw c w hdc hdw hc hw hne
    cases hdc
    cases hdw with
    | nil => exact absurd rfl hne
    | @cons _ y _ l hy hdy =>
      simp only [decideFrom, true_implies, Bool.true_eq_false, false_implies, and_true]
      rw [hN.scanList_unfold]
      exact pair_sublist_append hc (inBlock hy hdy hw).1
  | cons x xs ih =>
    intro a ys sc sw c w hdc hdw hc hw hne
    cases hdc with
    | @cons _ _ _ _ hx hdx =>
      cases hdw with
      | nil =>
        simp only [decideFrom, Bool.false_eq_true, false_implies, true_implies, true_and]
        rw [hN.scanList_unfold]
        exact pair_sublist_append hw (inBlock hx hdx hc).1
      | @cons _ y _ l hy hdy =>
        simp only [decideFrom]
        by_cases hxy : x = y
        · subst hxy
          rw [ite_eq_left rfl]
          obtain ⟨h1, h2⟩ := ih x l sc sw c w hdx hdy hc hw hne
          have hs := hN.sub_scanList d v (InSub.kid hx (InSub.refl x))
          exact ⟨fun h => (h1 h).trans hs, fun h => (h2 h).trans hs⟩
        · rw [ite_eq_right hxy]
          have hcx := (inBlock hx hdx hc).2
          have hwy := (inBlock hy hdy hw).2
          rw [hN.scanList_unfold]
          constructor
          · intro h
            have hl : lexLt (N.pushRank d a y) (N.pushRank d a x) := by simpa using h
            have hs := (hN.pushRank_sublist d hy hx hl).reverse
            simp only [List.reverse_cons, List.reverse_nil, List.nil_append,
              List.singleton_append] at hs
            exact (pair_sublist_flatMap hs hcx hwy).trans (List.sublist_append_right _ _)
          · intro h
            have hl : ¬lexLt (N.pushRank d a y) (N.pushRank d a x) := by simpa using h
            have hl' : lexLt (N.pushRank d a x) (N.pushRank d a y) := by
              have hne' : N.pushRank d a x ≠ N.pushRank d a y :=
                fun he => hxy (hN.pushRank_inj d hx hy he)
              unfold lexLt at hl ⊢
              generalize N.pushRank d a x = P at hne' hl ⊢
              generalize N.pushRank d a y = Q at hne' hl ⊢
              obtain ⟨p1, p2⟩ := P
              obtain ⟨q1, q2⟩ := Q
              simp only [ne_eq, Prod.mk.injEq] at hne' hl ⊢
              omega
            have hs := (hN.pushRank_sublist d hx hy hl').reverse
            simp only [List.reverse_cons, List.reverse_nil, List.nil_append,
              List.singleton_append] at hs
            exact (pair_sublist_flatMap hs hwy hcx).trans (List.sublist_append_right _ _)

/-- **§4.1, the tie order.** For two distinct candidates of the search under `v`,
`scanned_before` answers "comes first in C's scan order". -/
theorem Valid.scannedBefore_spec (hN : N.Valid root) (d : Dir) (v : V) {c w : E}
    (hc : c ∈ N.scanList d v v) (hw : w ∈ N.scanList d v v) (hne : c ≠ w) :
    (N.scannedBefore d v c w = true → List.Sublist [c, w] (N.scanList d v v)) ∧
    (N.scannedBefore d v c w = false → List.Sublist [w, c] (N.scanList d v v)) := by
  obtain ⟨sc, hsc, hcc⟩ := (hN.mem_scanList d v v c).mp hc
  obtain ⟨sw, hsw, hcw⟩ := (hN.mem_scanList d v v w).mp hw
  have hoc := ((hN.mem_cands).mp hcc).1
  have how := ((hN.mem_cands).mp hcw).1
  unfold scannedBefore
  rw [hoc, how]
  by_cases hs : sc = sw
  · subst hs
    rw [ite_eq_left rfl]
    have hcl := (List.mem_filter.mp hcc).1
    have hwl := (List.mem_filter.mp hcw).1
    obtain ⟨h1, h2⟩ := firstOf_spec hne hcl hwl
    have hsub := hN.sub_scanList d v hsc
    have up : ∀ {a b : E}, a ∈ N.cands d v sc → b ∈ N.cands d v sc →
        List.Sublist [a, b] (N.scanL d sc) → List.Sublist [a, b] (N.scanList d v v) := by
      intro a b ha hb hab
      have hf := hab.filter (fun g => !N.isTree g && !decide (N.inInt v (N.near d g)))
      have e1 : [a, b].filter (fun g => !N.isTree g && !decide (N.inInt v (N.near d g))) = [a, b] := by
        have ha' := (List.mem_filter.mp ha).2
        have hb' := (List.mem_filter.mp hb).2
        simp [ha', hb']
      rw [e1] at hf
      refine hf.trans ?_
      refine List.Sublist.trans ?_ hsub
      rw [hN.scanList_unfold]
      exact List.sublist_append_left _ _
    exact ⟨fun h => up hcc hcw (h1 h), fun h => up hcw hcc (h2 h)⟩
  · rw [ite_eq_right hs]
    have hbc := (hN.sub_bounds hsc).2.2
    have hbw := (hN.sub_bounds hsw).2.2
    obtain ⟨xs, hxs, hdx⟩ := hN.climb_down v (N.lim v + 1) sc hsc (by omega)
    obtain ⟨ys, hys, hdy⟩ := hN.climb_down v (N.lim v + 1) sw hsw (by omega)
    rw [hxs, hys, climbDecide_eq]
    exact hN.decideFrom_spec d v xs v ys sc sw c w hdx hdy hcc hcw hs

end Net

/-! ## §4.1: the cut-set search picks C's entering edge -/

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] (N : Net V E)

/-- One candidate of `CutStore::enter_edge`'s loop: `better` is `true` with no best yet, the scan
order between equal slacks, `<` otherwise. -/
def cutStep (slack : E → ℤ) (before : E → E → Bool) (best : Option (E × ℤ)) (k : E) :
    Option (E × ℤ) :=
  match best with
  | none => some (k, slack k)
  | some (w, sw) =>
    if (if slack k = sw then before k w = true else slack k < sw) then some (k, slack k)
    else some (w, sw)

/-- `CutStore::enter_edge` over e's cut set, a list of `(edge, against)`: `none` is `Fallback`
(a negative candidate, or no candidate for a pivot), `some r` is its answer. Any other check that
falls back (`prepare`) only makes more runs take `none`. -/
def cutEnter (slack : E → ℤ) (before : E → E → Bool) (pivot : Bool) (cover : List (E × Bool)) :
    Option (Option E) :=
  let cs := (cover.filter (fun x => x.2)).map Prod.fst
  if cs.any (fun k => decide (slack k < 0)) then none
  else match cs.foldl (cutStep slack before) none with
    | none => if pivot then none else some none
    | some (k, _) => some (some k)

/-- `tail_is_child = par(tail(e)) == e`, and the search it stands for. -/
def childOf (e : E) : V × Dir :=
  if N.par (N.tail e) = some e then (N.tail e, .inS) else (N.head e, .outS)

/-- The cut-set path of `enter_edge(e)`, falling back to C's search. -/
def enterEdge (slack : E → ℤ) (k : ℕ) (pivot : Bool) (cover : List (E × Bool)) (e : E) : Option E :=
  match cutEnter slack (N.scannedBefore (N.childOf e).2 (N.childOf e).1) pivot cover with
  | some r => r
  | none => N.cEnter slack k e

/-- **(I1)** for one tree edge: `cover` lists the non-tree edges crossing e's cut, once each, with
the bit *against e*. -/
def CoverOK (e : E) (cover : List (E × Bool)) : Prop :=
  (cover.map Prod.fst).Nodup ∧ ∀ g b, (g, b) ∈ cover ↔
    (N.isTree g = false ∧ (N.Against e g ∨ (N.Conn e (N.tail e) (N.tail g) ∧
      N.Conn e (N.head e) (N.head g))) ∧ (b = true ↔ N.Against e g))

variable {N} {root : V}

/-- One step of the cut-set loop either takes `k` or keeps `w`, and the one it keeps is the
least of the two by slack, then by `R`. -/
theorem cutStep_cases {slack : E → ℤ} {before : E → E → Bool} {R : E → E → Prop} {S : E → Prop}
    (htot : ∀ a b, S a → S b → a ≠ b → R a b ∨ R b a)
    (hbef : ∀ a b, S a → S b → a ≠ b → (before a b = true ↔ R a b))
    {k w : E} (hk : S k) (hw : S w) :
    (cutStep slack before (some (w, slack w)) k = some (k, slack k) ∧
      ∀ x, (x = k ∨ x = w) → x ≠ k → slack k < slack x ∨ (slack k = slack x ∧ R k x)) ∨
    (cutStep slack before (some (w, slack w)) k = some (w, slack w) ∧
      ∀ x, (x = k ∨ x = w) → x ≠ w → slack w < slack x ∨ (slack w = slack x ∧ R w x)) := by
  unfold cutStep
  by_cases he : slack k = slack w
  · by_cases hkw : k = w
    · subst hkw
      left
      refine ⟨by dsimp only; split_ifs <;> rfl, ?_⟩
      intro x hx hxk
      rcases hx with h | h <;> exact absurd h hxk
    · by_cases hb : before k w = true
      · left
        refine ⟨by simp [he, hb], ?_⟩
        intro x hx hxk
        rcases hx with h | h
        · exact absurd h hxk
        · rw [h]; exact Or.inr ⟨he, (hbef k w hk hw hkw).mp hb⟩
      · right
        refine ⟨by simp [he, hb], ?_⟩
        intro x hx hxw
        rcases hx with h | h
        · rw [h]
          refine Or.inr ⟨he.symm, ?_⟩
          rcases htot k w hk hw hkw with h' | h'
          · exact absurd ((hbef k w hk hw hkw).mpr h') hb
          · exact h'
        · exact absurd h hxw
  · by_cases hlt : slack k < slack w
    · left
      refine ⟨by simp [he, hlt], ?_⟩
      intro x hx hxk
      rcases hx with h | h
      · exact absurd h hxk
      · rw [h]; exact Or.inl hlt
    · right
      refine ⟨by simp [he, hlt], ?_⟩
      intro x hx hxw
      rcases hx with h | h
      · rw [h]; left; omega
      · exact absurd h hxw

theorem foldl_cutStep_some_ne {slack : E → ℤ} {before : E → E → Bool} :
    ∀ (l : List E) (w : E) (sw : ℤ), l.foldl (cutStep slack before) (some (w, sw)) ≠ none := by
  intro l
  induction l with
  | nil => intro w sw; simp
  | cons z l ih =>
    intro w sw
    simp only [List.foldl_cons, cutStep]
    split_ifs <;> exact ih _ _

theorem foldl_cutStep_none {slack : E → ℤ} {before : E → E → Bool} (l : List E) :
    l.foldl (cutStep slack before) none = none ↔ l = [] := by
  cases l with
  | nil => simp
  | cons k l =>
    simp only [List.foldl_cons, cutStep, reduceCtorEq, iff_false]
    exact foldl_cutStep_some_ne l k (slack k)

/-- The cut-set loop returns the least candidate by slack, then by `before`, in whatever order the
cut set lists them, when `before` is a strict total order `R` on them. -/
theorem foldl_cutStep_spec {slack : E → ℤ} {before : E → E → Bool} {R : E → E → Prop}
    {S : E → Prop} (htr : ∀ a b c, R a b → R b c → R a c)
    (htot : ∀ a b, S a → S b → a ≠ b → R a b ∨ R b a)
    (hbef : ∀ a b, S a → S b → a ≠ b → (before a b = true ↔ R a b)) :
    ∀ (l : List E) (best : Option (E × ℤ)), (∀ x ∈ l, S x) →
      (∀ w sw, best = some (w, sw) → S w ∧ sw = slack w) →
      ∀ m sm, l.foldl (cutStep slack before) best = some (m, sm) →
        sm = slack m ∧ (m ∈ l ∨ best = some (m, sm)) ∧
        ∀ x, (x ∈ l ∨ ∃ s, best = some (x, s)) → x ≠ m →
          slack m < slack x ∨ (slack m = slack x ∧ R m x) := by
  intro l
  induction l with
  | nil =>
    intro best _ hb m sm hf
    simp only [List.foldl_nil] at hf
    subst hf
    refine ⟨(hb m sm rfl).2, Or.inr rfl, ?_⟩
    intro x hx hxm
    rcases hx with hx | ⟨s, hs⟩
    · simp at hx
    · simp only [Option.some.injEq, Prod.mk.injEq] at hs; exact absurd hs.1.symm hxm
  | cons k l ih =>
    intro best hl hb m sm hf
    have hSk : S k := hl k (by simp)
    have hSl : ∀ x ∈ l, S x := fun x hx => hl x (by simp [hx])
    simp only [List.foldl_cons] at hf
    -- the new best after `k`, and what it is below
    have hnew : ∃ w', cutStep slack before best k = some (w', slack w') ∧ S w' ∧
        (w' = k ∨ best = some (w', slack w')) ∧
        ∀ x, (x = k ∨ ∃ s, best = some (x, s)) → x ≠ w' →
          slack w' < slack x ∨ (slack w' = slack x ∧ R w' x) := by
      cases hbest : best with
      | none =>
        refine ⟨k, rfl, hSk, Or.inl rfl, ?_⟩
        intro x hx hxk
        rcases hx with h | ⟨s, hs⟩
        · exact absurd h hxk
        · simp at hs
      | some ws =>
        obtain ⟨w, sw⟩ := ws
        obtain ⟨hSw, hsw⟩ := hb w sw (by rw [hbest])
        subst hsw
        have hx2 : ∀ x, (x = k ∨ ∃ s, some (w, slack w) = some (x, s)) → (x = k ∨ x = w) := by
          intro x hx
          rcases hx with h | ⟨s, hs⟩
          · exact Or.inl h
          · simp only [Option.some.injEq, Prod.mk.injEq] at hs; exact Or.inr hs.1.symm
        rcases cutStep_cases (R := R) htot hbef hSk hSw with ⟨hs, hk⟩ | ⟨hs, hk⟩
        · exact ⟨k, hs, hSk, Or.inl rfl, fun x hx hxk => hk x (hx2 x hx) hxk⟩
        · exact ⟨w, hs, hSw, Or.inr rfl, fun x hx hxw => hk x (hx2 x hx) hxw⟩
    obtain ⟨w', hw', hSw', hfrom, hbelow⟩ := hnew
    rw [hw'] at hf
    obtain ⟨h1, h2, h3⟩ := ih (some (w', slack w')) hSl
      (fun w sw h => by
        simp only [Option.some.injEq, Prod.mk.injEq] at h; rw [← h.1, ← h.2]; exact ⟨hSw', rfl⟩)
      m sm hf
    refine ⟨h1, ?_, ?_⟩
    · rcases h2 with h2 | h2
      · exact Or.inl (List.mem_cons_of_mem _ h2)
      · simp only [Option.some.injEq, Prod.mk.injEq] at h2
        rcases hfrom with hk | hb'
        · exact Or.inl (by rw [← h2.1, hk]; exact List.mem_cons_self)
        · right; rw [hb']; obtain ⟨e1, _⟩ := h2; subst e1; rw [h1]
    · intro x hx hxm
      by_cases hxl : x ∈ l
      · exact h3 x (Or.inl hxl) hxm
      · have hx' : x = k ∨ ∃ s, best = some (x, s) := by
          rcases hx with hx | hx
          · rcases List.mem_cons.mp hx with h | h
            · exact Or.inl h
            · exact absurd h hxl
          · exact Or.inr hx
        by_cases hxw : x = w'
        · exact h3 x (Or.inr ⟨slack w', by rw [hxw]⟩) hxm
        · -- through the best after `k`: m ≤ w' < x
          have hmw := h3 w' (Or.inr ⟨slack w', rfl⟩)
          have hwx := hbelow x hx' hxw
          by_cases hwm : w' = m
          · subst hwm; exact hwx
          · rcases hmw hwm with h | ⟨h, hr⟩ <;> rcases hwx with h' | ⟨h', hr'⟩
            · exact Or.inl (lt_trans h h')
            · exact Or.inl (by omega)
            · exact Or.inl (by omega)
            · exact Or.inr ⟨h.trans h', htr _ _ _ hr hr'⟩

theorem pairwise_idxOf {l : List E} (hn : l.Nodup) : l.Pairwise (fun a b => l.idxOf a < l.idxOf b) := by
  rw [List.pairwise_iff_forall_sublist]
  intro a b h
  exact idxOf_lt_of_pair_sublist hn h

/-- **§4.1.** On a valid tree with (I1) for `e` (and the cut sets' slacks C's, (I2): the same
`slack`), whenever the cut-set search answers, its answer is what C's `enter_edge(e)` returns once
its loop has run to the end. Ties included: `scanned_before` is C's scan order. -/
theorem Valid.cutEnter_eq (hN : N.Valid root) {e : E} (he : N.isTree e = true)
    {cover : List (E × Bool)} (hcov : N.CoverOK e cover) (slack : E → ℤ) (pivot : Bool)
    {r : Option E}
    (hcut : cutEnter slack (N.scannedBefore (N.childOf e).2 (N.childOf e).1) pivot cover = some r) :
    ∃ K, ∀ k, K ≤ k → N.cEnter slack k e = r := by
  -- e's child and the search C runs for it
  set v := (N.childOf e).1 with hv
  set d := (N.childOf e).2 with hd
  have hvpar : N.par v = some e ∧ ((v = N.tail e ∧ d = .inS) ∨ (v = N.head e ∧ d = .outS)) := by
    simp only [hv, hd, childOf]
    split_ifs with h
    · exact ⟨h, Or.inl ⟨rfl, rfl⟩⟩
    · refine ⟨(hN.tree_par e he).resolve_left h, Or.inr ⟨rfl, rfl⟩⟩
  obtain ⟨hpv, hdir⟩ := hvpar
  set L := N.scanList d v v with hL
  -- the members of e's cut set that cross against `e` are C's candidates (§2, (I1))
  set cs := (cover.filter (fun x => x.2)).map Prod.fst with hcs
  have hmem : ∀ g, g ∈ cs ↔ g ∈ L := by
    intro g
    rw [hL, hN.cands_iff hpv d hdir g]
    simp only [hcs, List.mem_map, List.mem_filter]
    constructor
    · rintro ⟨⟨g', b⟩, ⟨hgb, hb⟩, rfl⟩
      obtain ⟨h1, _, h3⟩ := (hcov.2 g' b).mp hgb
      exact ⟨h1, h3.mp hb⟩
    · rintro ⟨h1, h2⟩
      exact ⟨(g, true), ⟨(hcov.2 g true).mpr ⟨h1, Or.inl h2, Iff.intro (fun _ => h2) (fun _ => rfl)⟩,
        rfl⟩, rfl⟩
  have hnd : L.Nodup := hN.nodup_scanList d v v
  -- no negative candidate, else the cut sets fell back
  unfold cutEnter at hcut
  simp only [← hcs] at hcut
  by_cases hneg : (cs.any fun k => decide (slack k < 0)) = true
  · rw [ite_eq_left hneg] at hcut; exact absurd hcut (by simp)
  rw [ite_eq_right hneg] at hcut
  have hnn : ∀ g ∈ L, 0 ≤ slack g := by
    intro g hg
    have := (hmem g).mpr hg
    simp only [List.any_eq_true, decide_eq_true_eq, not_exists, not_and, not_lt] at hneg
    exact hneg g this
  -- C's search: Lemma 1
  obtain ⟨K, hK⟩ := hN.lemma1 d v slack hnn
  have hC : ∀ k, K ≤ k → N.cEnter slack k e = (L.foldl (evalE slack) (none, intMax)).1 := by
    intro k hk
    unfold cEnter
    rcases hdir with ⟨hvt, hdi⟩ | ⟨hvh, hdo⟩
    · have hlt : N.lim (N.tail e) < N.lim (N.head e) := (hN.tail_child_iff he).mpr (hvt ▸ hpv)
      rw [ite_eq_left hlt, ← hvt, ← hdi, hK k hk]
    · have hnlt : ¬N.lim (N.tail e) < N.lim (N.head e) := by
        intro h
        have h1 := (hN.tail_child_iff he).mp h
        exact hN.par_excl e he ⟨h1, hvh ▸ hpv⟩
      rw [ite_eq_right hnlt, ← hvh, ← hdo, hK k hk]
  refine ⟨K, fun k hk => ?_⟩
  rw [hC k hk]
  -- the order: position in C's scan
  let R : E → E → Prop := fun a b => L.idxOf a < L.idxOf b
  have hbef : ∀ a b, a ∈ cs → b ∈ cs → a ≠ b →
      (N.scannedBefore d v a b = true ↔ R a b) := by
    intro a b ha hb hab
    obtain ⟨h1, h2⟩ := hN.scannedBefore_spec d v ((hmem a).mp ha) ((hmem b).mp hb) hab
    constructor
    · intro h; exact idxOf_lt_of_pair_sublist hnd (h1 h)
    · intro h
      by_contra hf
      have := idxOf_lt_of_pair_sublist hnd (h2 (by simpa using hf))
      exact absurd (lt_trans h this) (lt_irrefl _)
  obtain ⟨c1, c2⟩ := foldl_evalE_spec (slack := slack) L (pairwise_idxOf hnd)
  have htot : ∀ a b, a ∈ cs → b ∈ cs → a ≠ b → R a b ∨ R b a := by
    intro a b ha hb hab
    have : L.idxOf a ≠ L.idxOf b := fun h => hab ((List.idxOf_inj ((hmem a).mp ha)).mp h)
    simp only [R]; omega
  have hspec := foldl_cutStep_spec (slack := slack) (before := N.scannedBefore d v) (R := R)
    (S := (· ∈ cs)) (fun a b c h1 h2 => lt_trans h1 h2) htot hbef cs none (fun _ h => h)
    (fun _ _ h => by simp at h)
  -- the two answers are the same: the least by slack, then by position in C's scan
  cases hf : cs.foldl (cutStep slack (N.scannedBefore d v)) none with
  | none =>
    rw [hf] at hcut
    have hcs : cs = [] := (foldl_cutStep_none cs).mp hf
    have hL0 : L = [] := by
      cases hL' : L with
      | nil => rfl
      | cons g l => exact absurd ((hmem g).mpr (by rw [hL']; simp)) (by rw [hcs]; simp)
    cases pivot
    · simp only [Bool.false_eq_true, ite_false] at hcut
      obtain rfl := Option.some.inj hcut
      exact c1.mpr hL0
    · simp at hcut
  | some ms =>
    obtain ⟨m, sm⟩ := ms
    rw [hf] at hcut
    simp only [Option.some.injEq] at hcut
    subst hcut
    obtain ⟨_, hm2, hm3⟩ := hspec m sm hf
    have hmc : m ∈ cs := hm2.resolve_right (by simp)
    have hL0 : L ≠ [] := fun h => by have := (hmem m).mp hmc; rw [h] at this; simp at this
    cases hC0 : (L.foldl (evalE slack) (none, intMax)).1 with
    | none => exact absurd (c1.mp hC0) hL0
    | some m' =>
      obtain ⟨_, hm'L, hm'3⟩ := c2 m' hC0
      by_contra hne
      have hne' : m' ≠ m := fun h => hne (by rw [h])
      have a1 := hm3 m' (Or.inl ((hmem m').mpr hm'L)) hne'
      have a2 := hm'3 m ((hmem m).mp hmc) (Ne.symm hne')
      rcases a1 with a1 | ⟨a1, r1⟩ <;> rcases a2 with a2 | ⟨a2, r2⟩
      · omega
      · omega
      · omega
      · exact absurd (lt_trans r1 r2) (lt_irrefl _)

/-- **The cut-set path of `enter_edge(e)` is C's `enter_edge(e)`**, at every pivot and every
`LR_balance` query: either the cut sets answer, with C's edge, or they fall back and C's search
answers. -/
theorem Valid.enterEdge_eq (hN : N.Valid root) {e : E} (he : N.isTree e = true)
    {cover : List (E × Bool)} (hcov : N.CoverOK e cover) (slack : E → ℤ) (pivot : Bool) :
    ∃ K, ∀ k, K ≤ k → N.enterEdge slack k pivot cover e = N.cEnter slack k e := by
  unfold enterEdge
  cases hcut : cutEnter slack (N.scannedBefore (N.childOf e).2 (N.childOf e).1) pivot cover with
  | none => exact ⟨0, fun k _ => rfl⟩
  | some r =>
    obtain ⟨K, hK⟩ := hN.cutEnter_eq he hcov slack pivot hcut
    exact ⟨K, fun k hk => (hK k hk).symm⟩

/-- With no negative candidate slack (a feasible tree, §2), the cut sets answer: they never hand
an `LR_balance` query to C, nor a pivot that has a candidate. -/
theorem cutEnter_isSome {slack : E → ℤ} {before : E → E → Bool} {cover : List (E × Bool)}
    (hnn : ∀ g, (g, true) ∈ cover → 0 ≤ slack g) (pivot : Bool)
    (hc : pivot = true → ∃ g, (g, true) ∈ cover) : (cutEnter slack before pivot cover).isSome := by
  unfold cutEnter
  have hneg : ¬((cover.filter (fun x => x.2)).map Prod.fst).any (fun k => decide (slack k < 0)) = true := by
    simp only [List.any_eq_true, List.mem_map, List.mem_filter, decide_eq_true_eq, not_exists,
      not_and]
    rintro k ⟨⟨g, b⟩, ⟨hg, hb⟩, rfl⟩
    simp only at hb
    subst hb
    simpa using hnn g hg
  simp only [hneg]
  cases hf : ((cover.filter (fun x => x.2)).map Prod.fst).foldl (cutStep slack before) none with
  | none =>
    cases pivot
    · simp
    · obtain ⟨g, hg⟩ := hc rfl
      have := (foldl_cutStep_none _).mp hf
      have hm : g ∈ (cover.filter (fun x => x.2)).map Prod.fst :=
        List.mem_map.mpr ⟨(g, true), List.mem_filter.mpr ⟨hg, rfl⟩, rfl⟩
      rw [this] at hm; simp at hm
  | some p => simp

end Net

/-! ## §4.2: a pivot, in cut space

Here the tree is any edge set `T` that spans and has no cycle, with no root: a *cut basis* gives,
for every tree edge `t`, a potential that steps by one across `t` and by zero across every other
tree edge. `coef t x` is the page's τ_x(t). `Valid.cutBasis` builds one from `lim`/`low`, and
`CutBasis.coef_unique` says every cut basis of `T` has the same coefficients, so the statements
below are about the tree, not about how it is stored. -/

section CutSpace

variable {V E : Type} [DecidableEq V] [DecidableEq E]

/-- A cut basis of the spanning tree `T` (`tail`, `head` fixed). -/
structure CutBasis (tail head : E → V) (T : E → Bool) where
  pot : E → V → ℤ
  step_self : ∀ t, T t = true → pot t (head t) - pot t (tail t) = 1
  step_other : ∀ t t', T t = true → T t' = true → t' ≠ t → pot t (tail t') = pot t (head t')
  /-- `T` spans: a potential flat across every tree edge is constant. -/
  conn : ∀ π : V → ℤ, (∀ t, T t = true → π (tail t) = π (head t)) → ∀ x y, π x = π y

namespace CutBasis

variable {tail head : E → V} {T : E → Bool} (B : CutBasis tail head T)

/-- τ_x(t): `+1` when `x` crosses t's cut against `t` (from head(t)'s side to tail(t)'s), `−1`
when it crosses along `t`, `0` when it does not cross. -/
def coef (t x : E) : ℤ := B.pot t (tail x) - B.pot t (head x)

theorem coef_self {t : E} (ht : T t = true) : B.coef t t = -1 := by
  have := B.step_self t ht; unfold coef; omega

theorem coef_tree {t t' : E} (ht : T t = true) (ht' : T t' = true) (hne : t' ≠ t) :
    B.coef t t' = 0 := by
  have := B.step_other t t' ht ht' hne; unfold coef; omega

/-- The coefficients are the tree's: any two cut bases of `T` agree. -/
theorem coef_unique (B' : CutBasis tail head T) {t : E} (ht : T t = true) (x : E) :
    B.coef t x = B'.coef t x := by
  have h := B.conn (fun y => B.pot t y - B'.pot t y) (by
    intro t' ht'
    by_cases he : t' = t
    · subst he; have := B.step_self t' ht; have := B'.step_self t' ht; omega
    · have := B.step_other t t' ht ht' he; have := B'.step_other t t' ht ht' he; omega)
  have := h (tail x) (head x)
  unfold coef; omega

/-- A cut basis potential takes two values, one on each side. -/
theorem pot_two {t : E} (ht : T t = true) (x : V) :
    B.pot t x = B.pot t (tail t) ∨ B.pot t x = B.pot t (tail t) + 1 := by
  set c := B.pot t (tail t)
  have h := B.conn (fun y => (B.pot t y - c) * (B.pot t y - c - 1)) (by
    intro t' ht'
    by_cases he : t' = t
    · subst he
      have := B.step_self t' ht
      have h1 : B.pot t' (head t') = c + 1 := by omega
      simp only [h1]; ring
    · simp only [B.step_other t t' ht ht' he]) x (tail t)
  beta_reduce at h
  rw [sub_self, zero_mul] at h
  rcases mul_eq_zero.mp h with h | h
  · left; omega
  · right; omega

/-- "A fundamental cycle has no coefficient ±2." -/
theorem coef_range {t : E} (ht : T t = true) (x : E) :
    B.coef t x = -1 ∨ B.coef t x = 0 ∨ B.coef t x = 1 := by
  unfold coef
  rcases B.pot_two ht (tail x) with h1 | h1 <;> rcases B.pot_two ht (head x) with h2 | h2 <;> omega

/-- (I1), "equivalently": the cycle `g + Σ τ_g(t)·t` is a circulation (it pairs to zero with every
potential difference)… -/
theorem cycle_conserves [Fintype E] (g : E) (π : V → ℤ) :
    (π (tail g) - π (head g)) +
      ∑ t ∈ Finset.univ.filter (fun t => T t = true), B.coef t g * (π (tail t) - π (head t)) = 0 := by
  set F := Finset.univ.filter (fun t => T t = true)
  -- π minus its tree steps times the basis is flat across the tree, hence constant
  have h := B.conn (fun y => π y - ∑ t ∈ F, (π (head t) - π (tail t)) * B.pot t y) (by
    intro t0 ht0
    have key : ∀ t ∈ F, (π (head t) - π (tail t)) * B.pot t (tail t0) -
        (π (head t) - π (tail t)) * B.pot t (head t0) =
        if t = t0 then -(π (head t0) - π (tail t0)) else 0 := by
      intro t ht
      have ht' : T t = true := (Finset.mem_filter.mp ht).2
      split_ifs with he
      · subst he; have := B.step_self t ht'; rw [← mul_sub]; rw [show B.pot t (tail t) - B.pot t (head t) = -1 by omega]; ring
      · rw [B.step_other t t0 ht' ht0 (Ne.symm he)]; ring
    have hs : ∑ t ∈ F, (π (head t) - π (tail t)) * B.pot t (tail t0) -
        ∑ t ∈ F, (π (head t) - π (tail t)) * B.pot t (head t0) = -(π (head t0) - π (tail t0)) := by
      rw [← Finset.sum_sub_distrib, Finset.sum_congr rfl key, Finset.sum_ite_eq']
      simp [F, ht0]
    linarith) (tail g) (head g)
  have hs : ∑ t ∈ F, B.coef t g * (π (tail t) - π (head t)) =
      -(∑ t ∈ F, (π (head t) - π (tail t)) * B.pot t (tail g) -
        ∑ t ∈ F, (π (head t) - π (tail t)) * B.pot t (head g)) := by
    rw [← Finset.sum_sub_distrib, ← Finset.sum_neg_distrib]
    refine Finset.sum_congr rfl (fun t _ => ?_)
    unfold coef; ring
  rw [hs]; linarith

/-- …and the only one of that shape through `g`: a circulation `g + Σ c(t)·t` over tree edges has
`c(t) = τ_g(t)`. So `g` is in `cover[t]` exactly when its fundamental cycle contains `t`. -/
theorem cycle_unique [Fintype E] (g : E) (c : E → ℤ)
    (hc : ∀ π : V → ℤ, (π (tail g) - π (head g)) +
      ∑ t ∈ Finset.univ.filter (fun t => T t = true), c t * (π (tail t) - π (head t)) = 0)
    {t0 : E} (ht0 : T t0 = true) : c t0 = B.coef t0 g := by
  have h := hc (B.pot t0)
  have hs : ∑ t ∈ Finset.univ.filter (fun t => T t = true),
      c t * (B.pot t0 (tail t) - B.pot t0 (head t)) = -c t0 := by
    rw [Finset.sum_eq_single t0]
    · rw [show B.pot t0 (tail t0) - B.pot t0 (head t0) = -1 by have := B.step_self t0 ht0; omega]
      ring
    · intro t ht hne
      rw [B.step_other t0 t ht0 (Finset.mem_filter.mp ht).2 hne]; ring
    · intro h'; simp [ht0] at h'
  rw [hs] at h
  unfold coef; linarith

/-- The tree after the pivot: `T − e + f`. -/
def pivotT (T : E → Bool) (e f : E) : E → Bool := fun x =>
  if x = e then false else if x = f then true else T x

/-- The cut basis of `T − e + f`, from T's (§4.2, "Cut sets (I1)"). It needs `f` to cross e's cut
against `e` (σ(e) = +1), which C's candidates do. -/
def pivot {e f : E} (he : T e = true) (hf : T f = false) (hef : B.coef e f = 1) :
    CutBasis tail head (pivotT T e f) where
  pot t := if t = f then fun y => -B.pot e y else fun y => B.pot t y - B.coef t f * B.pot e y
  step_self := by
    intro t ht
    have hfe : f ≠ e := by rintro rfl; rw [he] at hf; exact absurd hf (by simp)
    unfold pivotT at ht
    by_cases hte : t = e
    · simp [hte] at ht
    · by_cases htf : t = f
      · subst htf; simp only [ite_true]; unfold coef at hef; omega
      · simp only [hte, htf, ite_false] at ht ⊢
        have h1 := B.step_self t ht
        have h2 := B.step_other e t he ht hte
        rw [h2]; omega
  step_other := by
    intro t t' ht ht' hne
    have hfe : f ≠ e := by rintro rfl; rw [he] at hf; exact absurd hf (by simp)
    unfold pivotT at ht ht'
    by_cases hte : t = e
    · simp [hte] at ht
    by_cases hte' : t' = e
    · simp [hte'] at ht'
    by_cases htf : t = f
    · subst htf
      simp only [ite_true]
      have ht'T : T t' = true := by simpa [hte', hne] using ht'
      rw [B.step_other e t' he ht'T hte']
    · simp only [htf, ite_false]
      have htT : T t = true := by simpa [hte, htf] using ht
      by_cases htf' : t' = f
      · subst htf'
        have h := hef
        unfold coef at h ⊢
        linear_combination (-(B.pot t (tail t') - B.pot t (head t'))) * h
      · have ht'T : T t' = true := by simpa [hte', htf'] using ht'
        rw [B.step_other t t' htT ht'T hne, B.step_other e t' he ht'T hte']
  conn := by
    intro π hπ x y
    set c := π (head e) - π (tail e)
    have hfe : f ≠ e := by rintro rfl; rw [he] at hf; exact absurd hf (by simp)
    have hflat := B.conn (fun z => π z - c * B.pot e z) (by
      intro t ht
      by_cases hte : t = e
      · subst hte; have := B.step_self t ht; simp only [c]
        linear_combination (π (head t) - π (tail t)) * this
      · have h1 := hπ t (by simp [pivotT, hte, ht])
        rw [h1, B.step_other e t he ht hte])
    have hfs := hπ f (by simp [pivotT, hfe])
    have := hflat (tail f) (head f)
    have hc : c = 0 := by
      unfold coef at hef
      have : c * (B.pot e (tail f) - B.pot e (head f)) = 0 := by linarith
      rw [hef] at this; linarith
    have := hflat x y
    rw [hc] at this
    linarith

variable {e f : E} (he : T e = true) (hf : T f = false) (hef : B.coef e f = 1)

/-- In `T − e + f`, f's cut is e's with the sign flipped: `cover[f]` is `cover[e]` with every bit
flipped (and `e` in place of `f`). -/
theorem pivot_coef_f (x : E) : (B.pivot he hf hef).coef f x = -B.coef e x := by
  simp only [coef, pivot, ite_true]; ring

/-- For every other tree edge, `τ'_g(t) = τ_g(t) − σ(t)·τ_g(e)` with σ = τ_f. -/
theorem pivot_coef {t : E} (htf : t ≠ f) (x : E) :
    (B.pivot he hf hef).coef t x = B.coef t x - B.coef t f * B.coef e x := by
  simp only [coef, pivot, htf, ite_false]; ring

/-- §4.2, first bullet: a non-tree edge outside `cover[e]` keeps its cycle. -/
theorem pivot_outside {t g : E} (htf : t ≠ f) (hg : B.coef e g = 0) :
    (B.pivot he hf hef).coef t g = B.coef t g := by
  rw [B.pivot_coef he hf hef htf, hg]; ring

/-- §4.2, second bullet: for `g ∈ cover[e]` and `t ≠ e` on f's path (σ(t) ≠ 0), `g` toggles in
`cover[t]`… -/
theorem pivot_toggle {t g : E} (ht : T t = true) (hte : t ≠ e) (hg : B.coef e g ≠ 0)
    (hσ : B.coef t f ≠ 0) :
    (B.pivot he hf hef).coef t g ≠ 0 ↔ B.coef t g = 0 := by
  have htf : t ≠ f := by rintro rfl; rw [ht] at hf; exact absurd hf (by simp)
  have ht' : pivotT T e f t = true := by simp [pivotT, hte, htf, ht]
  have r1 := B.coef_range ht g
  have r2 := B.coef_range he g
  have r3 := B.coef_range ht f
  have r4 := (B.pivot he hf hef).coef_range ht' g
  rw [B.pivot_coef he hf hef htf] at r4 ⊢
  rcases r2 with r2 | r2 | r2 <;> rcases r3 with r3 | r3 | r3 <;>
    simp only [r2, r3] at r4 hg hσ ⊢ <;> omega

/-- …and where it joins, it is against `t` exactly when *g against e* differs from σ(t) = +1. -/
theorem pivot_join_against {t g : E} (ht : T t = true) (hg0 : B.coef t g = 0)
    (hg : B.coef e g ≠ 0) (hσ : B.coef t f ≠ 0) (htf : t ≠ f) :
    ((B.pivot he hf hef).coef t g = 1 ↔ (B.coef e g = 1 ↔ ¬B.coef t f = 1)) := by
  rw [B.pivot_coef he hf hef htf, hg0]
  have r2 := B.coef_range he g
  have r3 := B.coef_range ht f
  rcases r2 with r2 | r2 | r2 <;> rcases r3 with r3 | r3 | r3 <;> simp only [r2, r3] at hg hσ ⊢
  all_goals norm_num at *

/-- §4.2, third bullet: `e`, now non-tree, crosses each `t ≠ e` of f's path against `t` exactly
when σ(t) = +1, and crosses `f`'s cut against `f`. -/
theorem pivot_coef_e {t : E} (ht : T t = true) (hte : t ≠ e) (htf : t ≠ f) :
    (B.pivot he hf hef).coef t e = B.coef t f := by
  rw [B.pivot_coef he hf hef htf, B.coef_tree ht he (Ne.symm hte), B.coef_self he]; ring

theorem pivot_coef_f_e : (B.pivot he hf hef).coef f e = 1 := by
  rw [B.pivot_coef_f he hf hef, B.coef_self he]; ring

/-- §4.2, fourth bullet: `f` is a tree edge and leaves every cut set. -/
theorem pivot_coef_of_f {t : E} (ht : pivotT T e f t = true) (htf : t ≠ f) :
    (B.pivot he hf hef).coef t f = 0 := by
  have hfe : f ≠ e := by rintro rfl; rw [he] at hf; exact absurd hf (by simp)
  exact (B.pivot he hf hef).coef_tree ht (by simp [pivotT, hfe]) (Ne.symm htf)

/-! ### Cut values (§4.2, "Cut values") -/

variable {R : Type} [CommRing R] [Fintype E]

/-- t's cut value as a signed sum: `−Σ_x τ_x(t)·weight(x)`, which is `weight(t)` plus the weights
crossing along `t` minus those crossing against it (`check.rs`'s sum). -/
def cutValue (w : E → R) (t : E) : R := -∑ x, (B.coef t x : R) * w x

theorem cutValue_eq_check (w : E → R) {t : E} (ht : T t = true) :
    B.cutValue w t = w t - ∑ x ∈ Finset.univ.filter (fun x => T x = false), (B.coef t x : R) * w x := by
  unfold cutValue
  rw [← Finset.sum_filter_add_sum_filter_not Finset.univ (fun x => T x = true)]
  have h1 : ∑ x ∈ Finset.univ.filter (fun x => T x = true), (B.coef t x : R) * w x = -w t := by
    rw [Finset.sum_eq_single t]
    · rw [B.coef_self ht]; push_cast; ring
    · intro x hx hne
      rw [B.coef_tree ht (Finset.mem_filter.mp hx).2 hne]; push_cast; ring
    · intro h; simp [ht] at h
  have h2 : Finset.univ.filter (fun x => ¬T x = true) = Finset.univ.filter (fun x => T x = false) := by
    ext x; simp
  rw [h1, h2]; ring

/-- `cutvalue(t) −= σ(t)·cutvalue(e)` on f's path, and nothing elsewhere (σ(t) = 0). -/
theorem pivot_cutValue (w : E → R) {t : E} (htf : t ≠ f) :
    (B.pivot he hf hef).cutValue w t = B.cutValue w t - (B.coef t f : R) * B.cutValue w e := by
  unfold cutValue
  simp only [B.pivot_coef he hf hef htf]
  push_cast
  rw [show (∑ x, ((B.coef t x : R) - (B.coef t f : R) * (B.coef e x : R)) * w x) =
      ∑ x, (B.coef t x : R) * w x - (B.coef t f : R) * ∑ x, (B.coef e x : R) * w x by
    rw [Finset.mul_sum, ← Finset.sum_sub_distrib]
    exact Finset.sum_congr rfl (fun x _ => by ring)]
  ring

/-- `cutvalue(f) = −cutvalue(e)`. -/
theorem pivot_cutValue_f (w : E → R) :
    (B.pivot he hf hef).cutValue w f = -B.cutValue w e := by
  unfold cutValue
  simp only [B.pivot_coef_f he hf hef]
  push_cast
  simp

/-! ### Slacks (§4.2, "Ranks") -/

/-- `slack(x) = rank(head) − rank(tail) − minlen`. In any commutative ring: `i32` wrapping is one. -/
def slack (rank : V → R) (minlen : E → R) (x : E) : R := rank (head x) - rank (tail x) - minlen x

/-- Moving the ranks by `d` times a potential `p` (`rerank` moves one side, `p` its indicator)
changes a slack by `d` times p's step: only edges that cross. -/
theorem slack_shift (rank : V → R) (minlen : E → R) (p : V → R) (d : R) (x : E) :
    slack (tail := tail) (head := head) (fun y => rank y - d * p y) minlen x =
      slack (tail := tail) (head := head) rank minlen x + d * (p (tail x) - p (head x)) := by
  unfold slack; ring

/-- With `p` the potential of e's cut: only `e` (by `−d`) and the members of `cover[e]` (by `±d`,
the sign their bit gives) change slack. -/
theorem slack_shift_cut (rank : V → R) (minlen : E → R) (d : R) (x : E) :
    slack (tail := tail) (head := head) (fun y => rank y - d * (B.pot e y : R)) minlen x =
      slack (tail := tail) (head := head) rank minlen x + d * (B.coef e x : R) := by
  rw [slack_shift]; unfold coef; push_cast; ring

end CutBasis

end CutSpace

/-! ### The rooted tree is a cut basis -/

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E] {N : Net V E} {root : V}

/-- The potential of t's cut on the stored tree: the indicator of the child's subtree, negated
when the child is the tail. -/
def cutPot (N : Net V E) (t : E) (y : V) : ℤ :=
  if N.par (N.head t) = some t then (if N.inInt (N.head t) y then 1 else 0)
  else (if N.inInt (N.tail t) y then -1 else 0)

/-- `lim`/`low` give a cut basis of the tree. -/
def Valid.cutBasis (hN : N.Valid root) : CutBasis N.tail N.head N.isTree where
  pot := N.cutPot
  step_self := by
    intro t ht
    have hint : ∀ c y, N.inInt c y ↔ N.InSub c y := fun c y => (hN.inSub_iff c y).symm
    unfold cutPot
    by_cases h1 : N.par (N.head t) = some t
    · have a : N.inInt (N.head t) (N.head t) := (hint _ _).mpr (InSub.refl _)
      have b : ¬N.inInt (N.head t) (N.tail t) := by
        rw [hint]
        exact parent_end_not_sub hN h1 ((hN.mem_kids).mpr ⟨t, ht, h1, Or.inl ⟨rfl, rfl⟩⟩)
      simp [h1, a, b]
    · have hp := (hN.tree_par t ht).resolve_right h1
      have a : ¬N.inInt (N.tail t) (N.head t) := by
        rw [hint]
        exact parent_end_not_sub hN hp ((hN.mem_kids).mpr ⟨t, ht, hp, Or.inr ⟨rfl, rfl⟩⟩)
      have b : N.inInt (N.tail t) (N.tail t) := (hint _ _).mpr (InSub.refl _)
      simp [h1, a, b]
  step_other := by
    intro t t' ht ht' hne
    unfold cutPot
    have hint : ∀ c y, N.inInt c y ↔ N.InSub c y := fun c y => (hN.inSub_iff c y).symm
    by_cases h1 : N.par (N.head t) = some t
    · have hs := side_edge hN h1 ht' hne (Or.inl ⟨rfl, rfl⟩)
      have hb : N.inInt (N.head t) (N.tail t') ↔ N.inInt (N.head t) (N.head t') := by
        rw [hint, hint]; exact hs
      simp only [h1, ite_true]
      by_cases a : N.inInt (N.head t) (N.tail t')
      · simp [a, hb.mp a]
      · simp [a, (not_congr hb).mp a]
    · have hp := (hN.tree_par t ht).resolve_right h1
      have hs := side_edge hN hp ht' hne (Or.inl ⟨rfl, rfl⟩)
      have hb : N.inInt (N.tail t) (N.tail t') ↔ N.inInt (N.tail t) (N.head t') := by
        rw [hint, hint]; exact hs
      simp only [h1, ite_false]
      by_cases a : N.inInt (N.tail t) (N.tail t')
      · simp [a, hb.mp a]
      · simp [a, (not_congr hb).mp a]
  conn := by
    intro π hπ x y
    have key : ∀ z, π z = π root := by
      intro z
      have h := hN.span z
      suffices ∀ a b, N.InSub a b → π b = π a from this root z h
      intro a b hab
      induction hab with
      | refl => rfl
      | kid hc _ ih =>
        rw [ih]
        obtain ⟨t, ht, _, hj⟩ := (hN.mem_kids).mp hc
        have := hπ t ht
        rcases hj with ⟨a1, a2⟩ | ⟨a1, a2⟩
        · rw [← a1, ← a2]; exact this.symm
        · rw [← a1, ← a2]; exact this
    rw [key x, key y]


/-- "Crossing t's cut against t is τ_g(t) = +1" on the stored tree. -/
theorem Valid.coef_eq_one_iff (hN : N.Valid root) {t : E} (ht : N.isTree t = true) (g : E) :
    hN.cutBasis.coef t g = 1 ↔ N.Against t g := by
  have hint : ∀ c y, N.inInt c y ↔ N.InSub c y := fun c y => (hN.inSub_iff c y).symm
  show N.cutPot t (N.tail g) - N.cutPot t (N.head g) = 1 ↔ _
  unfold cutPot Against
  by_cases h1 : N.par (N.head t) = some t
  · have hu : N.head t ∈ N.kids (N.tail t) := (hN.mem_kids).mpr ⟨t, ht, h1, Or.inl ⟨rfl, rfl⟩⟩
    have s1 := (side_iff hN h1 hu (x := N.tail g)).1
    have s2 := (side_iff hN h1 hu (x := N.head g)).2
    rw [s1, s2]
    simp only [h1, ite_true]
    by_cases a : N.InSub (N.head t) (N.tail g) <;> by_cases b : N.InSub (N.head t) (N.head g) <;>
      simp [hint, a, b]
  · have hp := (hN.tree_par t ht).resolve_right h1
    have hu : N.tail t ∈ N.kids (N.head t) := (hN.mem_kids).mpr ⟨t, ht, hp, Or.inr ⟨rfl, rfl⟩⟩
    have s1 := (side_iff hN hp hu (x := N.tail g)).2
    have s2 := (side_iff hN hp hu (x := N.head g)).1
    rw [s1, s2]
    simp only [h1, ite_false]
    by_cases a : N.InSub (N.tail t) (N.tail g) <;> by_cases b : N.InSub (N.tail t) (N.head g) <;>
      simp [hint, a, b]

/-- The graph after a pivot: the same edges, the tree `T − e + f`, and whatever tree lists, parent
pointers and numbering the pivot leaves. -/
def repivot (N : Net V E) (e f : E) (treeIn' treeOut' : V → List E) (par' : V → Option E)
    (low' lim' : V → ℕ) : Net V E where
  tail := N.tail
  head := N.head
  inE := N.inE
  outE := N.outE
  treeIn := treeIn'
  treeOut := treeOut'
  isTree := CutBasis.pivotT N.isTree e f
  par := par'
  low := low'
  lim := lim'

/-- (I1) after a pivot, whatever root and numbering the new tree is stored with: for a tree edge
`t ≠ f` of `T − e + f`, `g` crosses t's new cut against `t` exactly when
`τ_g(t) − σ(t)·τ_g(e) = 1`, all on the old tree; for `f`, exactly when `g` crossed e's cut
along `e`. -/
theorem Valid.pivot_against (hN : N.Valid root) {e f : E} (he : N.isTree e = true)
    (hf : N.isTree f = false) (hef : N.Against e f)
    (treeIn' treeOut' : V → List E) (par' : V → Option E) (low' lim' : V → ℕ) {root' : V}
    (hN' : (N.repivot e f treeIn' treeOut' par' low' lim').Valid root')
    {t : E} (ht : CutBasis.pivotT N.isTree e f t = true) (g : E) :
    (N.repivot e f treeIn' treeOut' par' low' lim').Against t g ↔
      (if t = f then -hN.cutBasis.coef e g
       else hN.cutBasis.coef t g - hN.cutBasis.coef t f * hN.cutBasis.coef e g) = 1 := by
  have hef' : hN.cutBasis.coef e f = 1 := (hN.coef_eq_one_iff he f).mpr hef
  have k1 := CutBasis.coef_unique (hN.cutBasis.pivot he hf hef') hN'.cutBasis ht g
  have k2 : (hN.cutBasis.pivot he hf hef').coef t g =
      (if t = f then -hN.cutBasis.coef e g
       else hN.cutBasis.coef t g - hN.cutBasis.coef t f * hN.cutBasis.coef e g) := by
    split_ifs with htf
    · subst htf; exact CutBasis.pivot_coef_f _ he hf hef' g
    · exact CutBasis.pivot_coef _ he hf hef' htf g
  rw [← hN'.coef_eq_one_iff ht g, ← k2]
  exact Iff.of_eq (congrArg (· = 1) k1.symm)

/-- Rust's `shift`: for a member `g` of e's cut set, `tail(g)` is in the moved side (`subtree(v)`)
exactly when its bit *against e* is clear if and only if e's tail is the child. -/
theorem Valid.tail_inside_iff (hN : N.Valid root) {e g : E} {v : V} (hv : N.par v = some e)
    (hd : v = N.tail e ∨ v = N.head e)
    (hcross : N.Against e g ∨ (N.Conn e (N.tail e) (N.tail g) ∧ N.Conn e (N.head e) (N.head g))) :
    N.InSub v (N.tail g) ↔ (¬N.Against e g ↔ v = N.tail e) := by
  obtain ⟨u, hu, hj⟩ := hN.parent hv
  have s1 : ∀ x, N.Conn e v x ↔ N.InSub v x := fun x => (side_iff hN hv hu (x := x)).1
  have s2 : ∀ x, N.Conn e u x ↔ ¬N.InSub v x := fun x => (side_iff hN hv hu (x := x)).2
  have hne := hN.kid_ne hu
  unfold Against at hcross ⊢
  rcases hd with rfl | rfl
  · have hue : N.head e = u := by
      rcases hj with ⟨a, _⟩ | ⟨a, _⟩
      · exact absurd a hne
      · exact a
    rw [hue] at hcross ⊢
    simp only [s1, s2] at hcross ⊢
    simp only [iff_true]
    tauto
  · have hue : N.tail e = u := by
      rcases hj with ⟨a, _⟩ | ⟨a, _⟩
      · exact a
      · exact absurd a hne
    rw [hue] at hcross ⊢
    simp only [s1, s2] at hcross ⊢
    have hvt : ¬N.head e = u := hne
    simp only [hvt, iff_false]
    tauto

/-- The node `update` reranks from (ns.c:710): e's tail if it is a tree leaf, else e's head if it
is, else e's child. -/
def rerankNode (N : Net V E) (e : E) : V :=
  if (N.treeIn (N.tail e)).length + (N.treeOut (N.tail e)).length = 1 then N.tail e
  else if (N.treeIn (N.head e)).length + (N.treeOut (N.head e)).length = 1 then N.head e
  else if N.lim (N.tail e) < N.lim (N.head e) then N.tail e else N.head e

/-- §4.2, Ranks: `rerank`'s start is either e's child, so the moved set is the subtree under `e`,
or the root, so the whole tree moves ("a tree leaf's only tree edge is its parent edge"). -/
theorem Valid.rerankNode_spec (hN : N.Valid root) {e : E} (he : N.isTree e = true) :
    N.par (N.rerankNode e) = none ∨ N.par (N.rerankNode e) = some e := by
  have leaf : ∀ n, (n = N.tail e ∨ n = N.head e) →
      (N.treeIn n).length + (N.treeOut n).length = 1 → N.par n = none ∨ N.par n = some e := by
    intro n hn hdeg
    cases hp : N.par n with
    | none => exact Or.inl rfl
    | some p =>
      right
      obtain ⟨hpt, hpn⟩ := hN.par_tree n p hp
      have hmem : ∀ x, N.isTree x = true → (N.tail x = n ∨ N.head x = n) →
          x ∈ N.treeIn n ++ N.treeOut n := by
        intro x hx hxn
        rcases hxn with h | h
        · exact List.mem_append_right _ ((hN.mem_treeOut n x).mpr ⟨hx, h⟩)
        · exact List.mem_append_left _ ((hN.mem_treeIn n x).mpr ⟨hx, h⟩)
      have hl : (N.treeIn n ++ N.treeOut n).length = 1 := by rw [List.length_append]; exact hdeg
      obtain ⟨a, ha⟩ := List.length_eq_one_iff.mp hl
      have h1 := hmem p hpt hpn
      have h2 := hmem e he (by rcases hn with rfl | rfl <;> simp)
      rw [ha] at h1 h2
      simp only [List.mem_singleton] at h1 h2
      rw [h1, h2]
  unfold rerankNode
  split_ifs with h1 h2 h3
  · exact leaf _ (Or.inl rfl) h1
  · exact leaf _ (Or.inr rfl) h2
  · exact Or.inr ((hN.tail_child_iff he).mp h3)
  · right
    rcases hN.tree_par e he with h | h
    · exact absurd ((hN.tail_child_iff he).mpr h) h3
    · exact h

end Net


/-! ## The hypotheses can hold

Two nodes, the tree edge `0 : 0 → 1` and the non-tree edge `1 : 1 → 0`, numbered as `dfs_range`
numbers them from root `0`. Edge `1` crosses edge 0's cut against it; both paths pick it. -/

namespace Example

/-- The graph. -/
def exN : Net (Fin 2) (Fin 2) where
  tail e := if e = 0 then 0 else 1
  head e := if e = 0 then 1 else 0
  inE u := if u = 0 then [1] else [0]
  outE u := if u = 0 then [0] else [1]
  treeIn u := if u = 0 then [] else [0]
  treeOut u := if u = 0 then [0] else []
  isTree e := e = 0
  par u := if u = 0 then none else some 0
  low _ := 1
  lim u := if u = 0 then 2 else 1

theorem exN_kids0 : exN.kids 0 = [1] := by decide
theorem exN_kids1 : exN.kids 1 = [] := by decide

theorem exN_valid : exN.Valid 0 where
  chain := by
    intro u
    fin_cases u
    · show Net.Chain exN.low exN.lim 1 (exN.kids 0) 2
      rw [exN_kids0]; exact Net.Chain.cons rfl (Net.Chain.nil _)
    · show Net.Chain exN.low exN.lim 1 (exN.kids 1) 1
      rw [exN_kids1]; exact Net.Chain.nil _
  low_le := by decide
  span := by
    intro w
    fin_cases w
    · exact Net.InSub.refl _
    · exact Net.InSub.kid (by rw [exN_kids0]; simp) (Net.InSub.refl _)
  mem_inE := by decide
  mem_outE := by decide
  mem_treeIn := by decide
  mem_treeOut := by decide
  nodup_inE := by decide
  nodup_outE := by decide
  nodup_treeIn := by decide
  nodup_treeOut := by decide
  par_root := rfl
  par_tree := by decide
  tree_par := by decide
  par_excl := by decide

theorem exN_against : exN.Against 0 1 :=
  ⟨Net.Conn.refl _, Net.Conn.refl _⟩

theorem exN_cover : exN.CoverOK 0 [(1, true)] := by
  refine ⟨by decide, ?_⟩
  intro g b
  fin_cases g
  · simp [exN]
  · simp only [List.mem_singleton, Prod.mk.injEq]
    have : exN.isTree 1 = false := by decide
    simp only [Fin.mk_one, this, true_and]
    constructor
    · rintro rfl; exact ⟨Or.inl exN_against, fun _ => exN_against, fun _ => rfl⟩
    · rintro ⟨_, h⟩; exact h.mpr exN_against

/-- The slacks: anything nonnegative. -/
def exSlack (_ : Fin 2) : ℤ := 3

theorem exN_cut : exN.enterEdge exSlack 5 true [(1, true)] 0 = some 1 := by decide

theorem exN_c : exN.cEnter exSlack 5 0 = some 1 := by decide

theorem exN_same : ∃ K, ∀ k, K ≤ k → exN.enterEdge exSlack k true [(1, true)] 0 = exN.cEnter exSlack k 0 :=
  exN_valid.enterEdge_eq (by decide) exN_cover exSlack true

end Example

end GvpProofs.Ns.CutSets
