import GvpProofs.Ns.CutSets

/-!
# Network simplex on cut sets after a pivot, with `lim`/`low` stale (issue #750)

`CutSets.lean` proves `Net.Valid.enterEdge_eq` on a state whose `lim`/`low` are current. The
production path (`common/ns/cut.rs`) numbers the tree only when the pivots begin and when it
hands over to C's functions (`hand_over`). `CutStore::update` re-hangs `par` and leaves
`lim`/`low` alone, so after the first pivot they are stale. This file covers every pivot after
the first.

The production state is a `Net` `N` with whatever `lim`/`low` it carries. `N.renum low lim` is
the same graph, lists, tree and parent pointers with another numbering: the reference.

| step | here |
| :-- | :-- |
| The cut-set search reads `par`, the lists, the cut set and the slacks, and no `lim`/`low` | `scannedBeforeF` (the climbs' fuel explicit), `scannedBeforeF_renum`, `renum_childOf`, `coverOK_renum` |
| Past the tree's depth the climbs' fuel does not matter | `Valid.climb_fuel`, `Valid.scannedBeforeF_eq` |
| C's `enter_edge` gives one answer on every valid numbering of a tree (§5, Lemma 2, for `enter_edge`) | `cEnter_renum` |
| **The production `enter_edge(e)` is C's on the same tree, validly numbered** | `enterEdgeP_eq`, `enterEdgeP_eq_every` |
| `Valid` without the numbering, and a finite spanning tree always has a valid numbering | `Forest`, `Tree`, `Tree.sub_antisymm`, `Tree.exists_valid` |
| A spanning tree's `par` is fixed by its edges and root, so the re-hung `par` is C's | `Tree.par_unique` |
| (I3): the re-hang leaves a spanning tree from the same root | `Valid.rehang_tree` |
| **A pivot leaves the next pivot's hypotheses**: a tree, a valid numbering, the new cuts | `pivot_step` |
| The issue's shape: after one pivot the stored `lim` is invalid, C's search on it finds no edge, and the cut sets and C's search on a valid numbering find the same one | `Example750` |

Chained over a run: at the first pivot the stored numbering is `feasible_tree`'s, which is valid.
If the state before a pivot has a valid numbering, `pivot_step` gives the state after it a
spanning tree and a valid numbering (`Tree.exists_valid`), and the cuts (I1) that the next cut
set must hold. So at every pivot, given (I1) for `e`, `enterEdgeP_eq_every` says that the
production `enter_edge(e)` returns C's `enter_edge(e)` on any valid numbering of the stored
tree. C's own numbering is one of them: by `Tree.par_unique`, C's parent pointers are the stored
ones, and C's numbering is valid by C's `dfs_range`, which is assumed, as `CutSets.lean` assumes
it. This replaces `Valid.pivot_against`'s hypothesis that the new tree, with the numbering it is
stored with, is `Valid`: `pivot_step` derives the same conclusion from the old tree alone.

## What the stale numbering breaks, and what it does not

C's search reads `lim`/`low` for its child test and its subtree test. On a stale numbering it can
search the wrong side of e's cut: `Example750.exM_stale_c` returns no edge where the answer is
`a`. Production never runs C's search on that state. Every way into C's functions goes through
`hand_over`, which renumbers from the root first, and `enterEdgeP` models that. The cut-set
search reads no numbering (`scannedBeforeF_renum`), and neither do Rust's `first_tied`, `lca`
nor `prepare`, which test children as `par(c) == e`.

## Model vs Rust

The model of `CutSets.lean` applies here too. In addition:

* `enterEdgeP` is `cut::enter_edge`. Rust's climbs in `scanned_before` go up to `v` within the
  query's `TIE_CLIMBS` budget. Past that budget `first_tied` decides. The model gives the climbs
  fuel `n`, and the theorems hold for every `n` past a bound. `first_tied` is not modelled. It
  reads `par` and the lists only (§4.1, Lemma 1a, in prose).
* The fallback is "C's search on `N.renum lowF limF`", with `lowF`/`limF` any valid numbering.
  Rust's `hand_over` renumbers with `dfs_range_impl` from the first node of `nlist`. That it
  produces a valid numbering is assumed (`hF`), not proved. `Tree.exists_valid` proves that one
  exists.
* `pivot_step` describes the re-hang loop of `CutStore::update` by what it leaves (`hp0`–`hp2`),
  and `exchange_tree_edges` by the membership and `Nodup` of the new tree lists (`hin`, `hout`).
  The lists' order is not constrained here: that is (I4), which holds because Rust calls C's
  `exchange_tree_edges`.
* Stored cut sets after `move_cut_sets` are not modelled as lists. `pivot_step` gives the cuts
  that the next cover must list, as `Valid.pivot_against` did; that the pool holds them is
  checked by `ns-check`.
* `Tree.exists_valid` needs `[Fintype V]`: graphs are finite.
-/

set_option linter.unusedSectionVars false

namespace GvpProofs.Ns.CutSets

namespace Net

variable {V E : Type} [DecidableEq V] [DecidableEq E]

/-! ## The same tree, numbered otherwise -/

/-- `N` with its `lim`/`low` replaced: the same graph, edge lists, tree and parent pointers. -/
def renum (N : Net V E) (low lim : V → ℕ) : Net V E := { N with low := low, lim := lim }

section Renum

variable {N : Net V E} {low lim : V → ℕ}

@[simp] theorem renum_tail : (N.renum low lim).tail = N.tail := rfl
@[simp] theorem renum_head : (N.renum low lim).head = N.head := rfl
@[simp] theorem renum_inE : (N.renum low lim).inE = N.inE := rfl
@[simp] theorem renum_outE : (N.renum low lim).outE = N.outE := rfl
@[simp] theorem renum_treeIn : (N.renum low lim).treeIn = N.treeIn := rfl
@[simp] theorem renum_treeOut : (N.renum low lim).treeOut = N.treeOut := rfl
@[simp] theorem renum_isTree : (N.renum low lim).isTree = N.isTree := rfl
@[simp] theorem renum_par : (N.renum low lim).par = N.par := rfl
@[simp] theorem renum_low : (N.renum low lim).low = low := rfl
@[simp] theorem renum_lim : (N.renum low lim).lim = lim := rfl
@[simp] theorem renum_renum {l' m' : V → ℕ} : (N.renum low lim).renum l' m' = N.renum l' m' := rfl
@[simp] theorem renum_own (d : Dir) : (N.renum low lim).own d = N.own d := by cases d <;> rfl
@[simp] theorem renum_near (d : Dir) : (N.renum low lim).near d = N.near d := by cases d <;> rfl
@[simp] theorem renum_scanL (d : Dir) : (N.renum low lim).scanL d = N.scanL d := by cases d <;> rfl
@[simp] theorem renum_treeL (d : Dir) : (N.renum low lim).treeL d = N.treeL d := by cases d <;> rfl
@[simp] theorem renum_kids : (N.renum low lim).kids = N.kids := rfl
@[simp] theorem renum_childOf : (N.renum low lim).childOf = N.childOf := rfl
@[simp] theorem renum_parentOf : (N.renum low lim).parentOf = N.parentOf := rfl

theorem renum_self : N.renum N.low N.lim = N := rfl

theorem inSub_renum {u w : V} : (N.renum low lim).InSub u w ↔ N.InSub u w := by
  constructor
  · intro h
    induction h with
    | refl u => exact InSub.refl u
    | kid hc _ ih => exact InSub.kid hc ih
  · intro h
    induction h with
    | refl u => exact InSub.refl u
    | kid hc _ ih => exact InSub.kid hc ih

theorem conn_renum {e : E} {x y : V} : (N.renum low lim).Conn e x y ↔ N.Conn e x y := by
  constructor
  · intro h
    induction h with
    | refl => exact Conn.refl _
    | step t ht hte hj _ ih => exact Conn.step t ht hte hj ih
  · intro h
    induction h with
    | refl => exact Conn.refl _
    | step t ht hte hj _ ih => exact Conn.step t ht hte hj ih

theorem against_renum {t g : E} : (N.renum low lim).Against t g ↔ N.Against t g :=
  and_congr conn_renum conn_renum

/-- (I1) mentions no numbering. -/
theorem coverOK_renum {e : E} {cover : List (E × Bool)} :
    (N.renum low lim).CoverOK e cover ↔ N.CoverOK e cover := by
  unfold CoverOK
  simp only [against_renum, conn_renum, renum_isTree, renum_tail, renum_head]

/-- The cut-set search of `CutStore::enter_edge` (as `cutEnter` with `scannedBefore` models it)
reads no numbering except the climb's fuel, which `scannedBeforeF` makes explicit. -/
theorem climb_renum (v : V) : ∀ n u, (N.renum low lim).climb v n u = N.climb v n u := by
  intro n
  induction n with
  | zero => intro u; rfl
  | succ n ih =>
    intro u
    simp only [climb, renum_parentOf]
    split <;> simp [ih]

theorem pushRank_renum (d : Dir) : (N.renum low lim).pushRank d = N.pushRank d := by
  funext u c
  unfold pushRank
  simp only [renum_par, renum_own, renum_scanL, renum_treeL]

theorem climbDecide_renum (v : V) (d : Dir) :
    (N.renum low lim).climbDecide v d = N.climbDecide v d := by
  funext ra rb
  unfold climbDecide
  simp only [pushRank_renum]

end Renum

/-! ## C's search reads the numbering only as subtree membership -/

section Invariance

variable {N : Net V E} {la ma lb mb : V → ℕ} {ra rb : V}

theorem Valid.inInt_iff {root : V} {M : Net V E} (hM : M.Valid root) (v x : V) :
    M.inInt v x ↔ M.InSub v x := (hM.inSub_iff v x).symm

/-- Two valid numberings of one tree agree on every subtree-membership test. -/
theorem inInt_renum_iff (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (v x : V) :
    (N.renum la ma).inInt v x ↔ (N.renum lb mb).inInt v x := by
  rw [hA.inInt_iff, hB.inInt_iff, inSub_renum, inSub_renum]

/-- ... and on every child test `lim(child) < lim(parent)` across a tree edge. -/
theorem child_renum_iff (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (d : Dir)
    {t : E} (ht : N.isTree t = true) :
    ma (N.near d t) < ma (N.own d t) ↔ mb (N.near d t) < mb (N.own d t) := by
  have a := hA.near_child_iff d (t := t) ht
  have b := hB.near_child_iff d (t := t) ht
  simp only [renum_lim, renum_near, renum_own, renum_par] at a b
  rw [a, b]

theorem child_renum_iff' (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (d : Dir)
    {t : E} (ht : N.isTree t = true) :
    ma (N.own d t) < ma (N.near d t) ↔ mb (N.own d t) < mb (N.near d t) := by
  have a := hA.own_child_iff d (t := t) ht
  have b := hB.own_child_iff d (t := t) ht
  simp only [renum_lim, renum_near, renum_own, renum_par] at a b
  rw [a, b]

theorem loop1_renum (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (d : Dir)
    (v : V) (slack : E → ℤ) (u : V) (s : St V E) {g : E} (hg : g ∈ N.scanL d u) :
    (N.renum la ma).loop1 d v slack u s g = (N.renum lb mb).loop1 d v slack u s g := by
  have ho : N.own d g = u := by
    have := (hA.mem_scanL (d := d) (u := u) (g := g)).mp (by simpa using hg)
    simpa using this
  have hi := inInt_renum_iff hA hB v (N.near d g)
  unfold loop1
  simp only [renum_isTree, renum_near, renum_lim]
  by_cases ht : N.isTree g = true
  · have hf : ¬N.isTree g = false := by simp [ht]
    have hc := child_renum_iff hA hB d ht
    rw [ho] at hc
    rw [ite_eq_right hf, ite_eq_right hf]
    by_cases h : ma (N.near d g) < ma u
    · rw [ite_eq_left h, ite_eq_left (hc.mp h)]
    · rw [ite_eq_right h, ite_eq_right (fun h' => h (hc.mpr h'))]
  · have hf : N.isTree g = false := by simpa using ht
    rw [ite_eq_left hf, ite_eq_left hf]
    by_cases h : (N.renum la ma).inInt v (N.near d g)
    · rw [ite_eq_right (not_not.mpr h), ite_eq_right (not_not.mpr (hi.mp h))]
    · rw [ite_eq_left h, ite_eq_left (fun h' => h (hi.mpr h'))]

theorem loop2_renum (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (d : Dir)
    (u : V) (s : St V E) {g : E} (hg : g ∈ N.treeL d u) :
    (N.renum la ma).loop2 d u s g = (N.renum lb mb).loop2 d u s g := by
  obtain ⟨ht, hn⟩ := (hA.mem_treeL (d := d) (u := u) (g := g)).mp (by simpa using hg)
  simp only [renum_isTree, renum_near] at ht hn
  have hc := child_renum_iff' hA hB d ht
  rw [hn] at hc
  unfold loop2
  simp only [renum_own, renum_lim]
  by_cases h0 : s.2.2 ≤ 0
  · rw [ite_eq_left h0, ite_eq_left h0]
  · rw [ite_eq_right h0, ite_eq_right h0]
    by_cases h : ma (N.own d g) < ma u
    · rw [ite_eq_left h, ite_eq_left (hc.mp h)]
    · rw [ite_eq_right h, ite_eq_right (fun h' => h (hc.mpr h'))]

theorem step_renum (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) (d : Dir)
    (v : V) (slack : E → ℤ) :
    (N.renum la ma).step d v slack = (N.renum lb mb).step d v slack := by
  funext s
  obtain ⟨todo, p⟩ := s
  cases todo with
  | nil => rfl
  | cons u rest =>
    simp only [step, renum_scanL, renum_treeL]
    rw [List.foldl_ext _ _ _ (fun s g hg => loop1_renum hA hB d v slack u s hg)]
    rw [List.foldl_ext _ _ _ (fun s g hg => loop2_renum hA hB d u s hg)]

/-- **C's `enter_edge(e)` does not depend on the numbering**: on two valid numberings of the same
tree (any roots), it returns the same edge after any number of iterations. This is what
`fall_back` relies on when it renumbers from the root instead of reproducing C's numbering. -/
theorem cEnter_renum (hA : (N.renum la ma).Valid ra) (hB : (N.renum lb mb).Valid rb) {e : E}
    (he : N.isTree e = true) (slack : E → ℤ) (k : ℕ) :
    (N.renum la ma).cEnter slack k e = (N.renum lb mb).cEnter slack k e := by
  have hc : ma (N.tail e) < ma (N.head e) ↔ mb (N.tail e) < mb (N.head e) :=
    child_renum_iff hA hB .inS he
  unfold cEnter run
  rw [step_renum hA hB .inS, step_renum hA hB .outS]
  by_cases h : ma (N.tail e) < ma (N.head e)
  · have h' := hc.mp h
    simp only [renum_lim, renum_tail, renum_head, h, h', ite_true]
  · have h' : ¬mb (N.tail e) < mb (N.head e) := fun h' => h (hc.mpr h')
    simp only [renum_lim, renum_tail, renum_head, h, h', ite_false]

end Invariance

/-! ## The tie order with explicit fuel -/

section Fuel

variable (N : Net V E)

/-- `scanned_before` with the climbs' fuel `n` given, not read from `lim(v)`. Rust climbs until it
reaches `v` (or its budget runs out, see "Model vs Rust"). -/
def scannedBeforeF (d : Dir) (v : V) (n : ℕ) (c w : E) : Bool :=
  if N.own d c = N.own d w then Net.firstOf (N.scanL d (N.own d c)) c w
  else N.climbDecide v d (N.climb v n (N.own d c)).reverse (N.climb v n (N.own d w)).reverse

variable {N}

theorem scannedBefore_eq_F (d : Dir) (v : V) :
    N.scannedBefore d v = N.scannedBeforeF d v (N.lim v + 1) := rfl

/-- `scannedBeforeF` reads no numbering. -/
@[simp] theorem scannedBeforeF_renum {low lim : V → ℕ} (d : Dir) (v : V) (n : ℕ) :
    (N.renum low lim).scannedBeforeF d v n = N.scannedBeforeF d v n := by
  funext c w
  unfold scannedBeforeF
  simp only [renum_own, renum_scanL, climb_renum, climbDecide_renum]

/-- Past the depth, the fuel does not matter. -/
theorem Valid.climb_fuel {root : V} (hN : N.Valid root) (v : V) :
    ∀ n m u, N.InSub v u → N.lim v < N.lim u + n → N.lim v < N.lim u + m →
      N.climb v n u = N.climb v m u := by
  intro n
  induction n with
  | zero =>
    intro m u hu hn _
    have := (hN.sub_bounds hu).2.2
    omega
  | succ n ih =>
    intro m u hu hn hm
    cases m with
    | zero =>
      have := (hN.sub_bounds hu).2.2
      omega
    | succ m =>
      by_cases huv : u = v
      · subst huv; simp [climb]
      · rcases hN.inSub_last hu with h | ⟨q, hq, huq⟩
        · exact absurd h huv
        · have hlt := hN.kid_lim huq
          simp only [climb, huv, ite_false, hN.parentOf_kid huq]
          rw [ih m q hq (by omega) (by omega)]

theorem Valid.scannedBeforeF_eq {root : V} (hN : N.Valid root) (d : Dir) (v : V) {n : ℕ}
    (hn : N.lim v < n) {c w : E} (hc : N.InSub v (N.own d c)) (hw : N.InSub v (N.own d w)) :
    N.scannedBeforeF d v n c w = N.scannedBefore d v c w := by
  rw [scannedBefore_eq_F]
  unfold scannedBeforeF
  rw [hN.climb_fuel v n (N.lim v + 1) _ hc (by omega) (by omega),
    hN.climb_fuel v n (N.lim v + 1) _ hw (by omega) (by omega)]

end Fuel

/-! ## The cut-set loop sees `before` only between its candidates -/

section Congr

variable {slack : E → ℤ} {b1 b2 : E → E → Bool}

theorem foldl_cutStep_congr {S : E → Prop} (h : ∀ a b, S a → S b → b1 a b = b2 a b) :
    ∀ (l : List E) (best : Option (E × ℤ)), (∀ x ∈ l, S x) → (∀ p, best = some p → S p.1) →
      l.foldl (cutStep slack b1) best = l.foldl (cutStep slack b2) best := by
  intro l
  induction l with
  | nil => intro _ _ _; rfl
  | cons k l ih =>
    intro best hl hb
    have hk : S k := hl k (by simp)
    have hstep : cutStep slack b1 best k = cutStep slack b2 best k := by
      unfold cutStep
      cases best with
      | none => rfl
      | some p =>
        obtain ⟨w, sw⟩ := p
        have hw : S w := hb (w, sw) rfl
        simp only [h k w hk hw]
    have hnew : ∀ p, cutStep slack b1 best k = some p → S p.1 := by
      intro p hp
      unfold cutStep at hp
      cases best with
      | none => simp only [Option.some.injEq] at hp; subst hp; exact hk
      | some q =>
        obtain ⟨w, sw⟩ := q
        simp only at hp
        split_ifs at hp <;> simp only [Option.some.injEq] at hp <;> subst hp
        all_goals first | exact hk | exact hb (w, sw) rfl
    rw [List.foldl_cons, List.foldl_cons, ← hstep]
    exact ih _ (fun x hx => hl x (by simp [hx])) hnew

theorem cutEnter_congr {pivot : Bool} {cover : List (E × Bool)}
    (h : ∀ a b, (a, true) ∈ cover → (b, true) ∈ cover → b1 a b = b2 a b) :
    cutEnter slack b1 pivot cover = cutEnter slack b2 pivot cover := by
  unfold cutEnter
  simp only
  have hS : ∀ x ∈ (cover.filter (fun x => x.2)).map Prod.fst, (x, true) ∈ cover := by
    intro x hx
    obtain ⟨⟨g, b⟩, hg, rfl⟩ := List.mem_map.mp hx
    obtain ⟨hg, hb⟩ := List.mem_filter.mp hg
    simp only at hb
    subst hb
    exact hg
  rw [foldl_cutStep_congr (S := fun x => (x, true) ∈ cover) (fun a b ha hb => h a b ha hb) _ none
    hS (fun _ h => by simp at h)]

end Congr

/-! ## The production `enter_edge`, after any number of pivots -/

section Production

variable {N : Net V E}

/-- **What production runs** at a pivot or `LR_balance` query, on its stored state `N`, whose
`lim`/`low` may be stale: the cut-set search (climbs with fuel `n`, reading only `par` and the
lists), and on `Fallback`, C's search after `fall_back` has renumbered the tree as `lowF`/`limF`. -/
def enterEdgeP (N : Net V E) (slack : E → ℤ) (n k : ℕ) (pivot : Bool) (cover : List (E × Bool))
    (e : E) (lowF limF : V → ℕ) : Option E :=
  match cutEnter slack (N.scannedBeforeF (N.childOf e).2 (N.childOf e).1 n) pivot cover with
  | some r => r
  | none => (N.renum lowF limF).cEnter slack k e

/-- The members of e's cut set that cross against `e` have their searched end below e's child. -/
theorem Valid.own_sub_of_cover {root : V} {M : Net V E} (hM : M.Valid root) {e : E}
    (he : M.isTree e = true) {cover : List (E × Bool)} (hcov : M.CoverOK e cover) {g : E}
    (hg : (g, true) ∈ cover) :
    M.InSub (M.childOf e).1 (M.own (M.childOf e).2 g) := by
  set v := (M.childOf e).1 with hv
  set d := (M.childOf e).2 with hd
  have hvpar : M.par v = some e ∧ ((v = M.tail e ∧ d = .inS) ∨ (v = M.head e ∧ d = .outS)) := by
    simp only [hv, hd, childOf]
    split_ifs with h
    · exact ⟨h, Or.inl ⟨rfl, rfl⟩⟩
    · refine ⟨(hM.tree_par e he).resolve_left h, Or.inr ⟨rfl, rfl⟩⟩
  obtain ⟨hpv, hdir⟩ := hvpar
  obtain ⟨h1, _, h3⟩ := (hcov.2 g true).mp hg
  have hL : g ∈ M.scanList d v v := (hM.cands_iff hpv d hdir g).mpr ⟨h1, h3.mp rfl⟩
  obtain ⟨x, hx, hgx⟩ := (hM.mem_scanList d v v g).mp hL
  rw [((hM.mem_cands).mp hgx).1]
  exact hx

/-- **Issue #750.** The production `enter_edge(e)` on a state whose `lim`/`low` are stale is C's
`enter_edge(e)` on the same tree validly numbered: `lowC`/`limC` (for example C's own numbering,
which C's `dfs_range` keeps current), with (I1) for `e`. The fallback's renumbering
`lowF`/`limF` is any valid numbering of that tree. Nothing is assumed of `N.low`/`N.lim`. -/
theorem enterEdgeP_eq {lowC limC lowF limF : V → ℕ} {rootC rootF : V}
    (hC : (N.renum lowC limC).Valid rootC) (hF : (N.renum lowF limF).Valid rootF) {e : E}
    (he : N.isTree e = true) {cover : List (E × Bool)} (hcov : N.CoverOK e cover)
    (slack : E → ℤ) (pivot : Bool) :
    ∃ K, ∀ n k, K ≤ n → K ≤ k →
      N.enterEdgeP slack n k pivot cover e lowF limF = (N.renum lowC limC).cEnter slack k e := by
  set A := N.renum lowC limC with hAdef
  have hcovA : A.CoverOK e cover := coverOK_renum.mpr hcov
  obtain ⟨K1, hK1⟩ := hC.enterEdge_eq (e := e) he hcovA slack pivot
  set v := (N.childOf e).1 with hv
  set d := (N.childOf e).2 with hd
  refine ⟨max K1 (limC v + 1), fun n k hn hk => ?_⟩
  -- the cut-set search on the stale state is the one on the renumbered state
  have hcut : cutEnter slack (N.scannedBeforeF d v n) pivot cover =
      cutEnter slack (A.scannedBefore (A.childOf e).2 (A.childOf e).1) pivot cover := by
    apply cutEnter_congr
    intro a b ha hb
    have hsa := hC.own_sub_of_cover he hcovA ha
    have hsb := hC.own_sub_of_cover he hcovA hb
    simp only [hAdef, renum_childOf] at hsa hsb ⊢
    rw [← hC.scannedBeforeF_eq d v (lt_of_lt_of_le (Nat.lt_succ_self _) (le_of_max_le_right hn)) hsa hsb, scannedBeforeF_renum]
  have hE := hK1 k (le_of_max_le_left hk)
  unfold enterEdge at hE
  unfold enterEdgeP
  rw [← hv, ← hd, hcut]
  cases hc : cutEnter slack (A.scannedBefore (A.childOf e).2 (A.childOf e).1) pivot cover with
  | some r => rw [hc] at hE; exact hE
  | none => exact cEnter_renum hF hC he slack k

end Production

/-! ## The stored tree, without a numbering

`Tree` is `Valid` without `chain` and `low_le`: what the production state keeps between pivots.
On a finite graph it always has a valid numbering (`Tree.exists_valid`), so `enterEdgeP_eq` needs
nothing about `lim`/`low` at all (`Tree.enterEdgeP_eq`). -/

/-- The edge lists are the graph's, and every tree edge is the parent edge of exactly one of its
ends: `Valid` with no numbering, no root and no spanning. -/
structure Forest (N : Net V E) : Prop where
  mem_inE : ∀ u e, e ∈ N.inE u ↔ N.head e = u
  mem_outE : ∀ u e, e ∈ N.outE u ↔ N.tail e = u
  mem_treeIn : ∀ u e, e ∈ N.treeIn u ↔ N.isTree e = true ∧ N.head e = u
  mem_treeOut : ∀ u e, e ∈ N.treeOut u ↔ N.isTree e = true ∧ N.tail e = u
  nodup_inE : ∀ u, (N.inE u).Nodup
  nodup_outE : ∀ u, (N.outE u).Nodup
  nodup_treeIn : ∀ u, (N.treeIn u).Nodup
  nodup_treeOut : ∀ u, (N.treeOut u).Nodup
  par_tree : ∀ u e, N.par u = some e → N.isTree e = true ∧ (N.tail e = u ∨ N.head e = u)
  tree_par : ∀ e, N.isTree e = true → N.par (N.tail e) = some e ∨ N.par (N.head e) = some e
  par_excl : ∀ e, N.isTree e = true → ¬(N.par (N.tail e) = some e ∧ N.par (N.head e) = some e)

/-- The parent pointers form a tree that spans the graph from `root`, and the edge lists are the
graph's: `Valid` with no numbering. -/
structure Tree (N : Net V E) (root : V) : Prop extends N.Forest where
  span : ∀ w, N.InSub root w
  par_root : N.par root = none

section TreeSec

variable {N : Net V E} {root : V}

theorem Valid.tree (hN : N.Valid root) : N.Tree root :=
  ⟨⟨hN.mem_inE, hN.mem_outE, hN.mem_treeIn, hN.mem_treeOut, hN.nodup_inE, hN.nodup_outE,
    hN.nodup_treeIn, hN.nodup_treeOut, hN.par_tree, hN.tree_par, hN.par_excl⟩, hN.span, hN.par_root⟩

theorem Tree.renum {low lim : V → ℕ} (hT : N.Tree root) : (N.renum low lim).Tree root :=
  ⟨⟨hT.mem_inE, hT.mem_outE, hT.mem_treeIn, hT.mem_treeOut, hT.nodup_inE, hT.nodup_outE,
    hT.nodup_treeIn, hT.nodup_treeOut, hT.par_tree, hT.tree_par, hT.par_excl⟩,
    fun w => inSub_renum.mpr (hT.span w), hT.par_root⟩

/-- A tree with a numbering that `dfs_range` could have produced is valid. -/
theorem Tree.valid_of_chain {low lim : V → ℕ} (hT : N.Tree root)
    (hc : ∀ u, Chain low lim (low u) (N.kids u) (lim u)) (hle : ∀ u, low u ≤ lim u) :
    (N.renum low lim).Valid root :=
  ⟨hc, hle, fun w => inSub_renum.mpr (hT.span w), hT.mem_inE, hT.mem_outE, hT.mem_treeIn,
    hT.mem_treeOut, hT.nodup_inE, hT.nodup_outE, hT.nodup_treeIn, hT.nodup_treeOut, hT.par_root,
    hT.par_tree, hT.tree_par, hT.par_excl⟩

theorem Forest.mem_kids (hT : N.Forest) {u c : V} :
    c ∈ N.kids u ↔ ∃ e, N.isTree e = true ∧ N.par c = some e ∧
      ((N.tail e = u ∧ N.head e = c) ∨ (N.head e = u ∧ N.tail e = c)) := by
  constructor
  · intro h
    simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq] at h
    rcases h with ⟨e, ⟨he, hp⟩, rfl⟩ | ⟨e, ⟨he, hp⟩, rfl⟩
    · obtain ⟨ht, htl⟩ := (hT.mem_treeOut u e).mp he
      refine ⟨e, ht, ?_, Or.inl ⟨htl, rfl⟩⟩
      rcases hT.tree_par e ht with h | h
      · rw [htl] at h; exact absurd h hp
      · exact h
    · obtain ⟨ht, hhd⟩ := (hT.mem_treeIn u e).mp he
      refine ⟨e, ht, ?_, Or.inr ⟨hhd, rfl⟩⟩
      rcases hT.tree_par e ht with h | h
      · exact h
      · rw [hhd] at h; exact absurd h hp
  · rintro ⟨e, ht, hpc, (⟨h1, h2⟩ | ⟨h1, h2⟩)⟩
    · simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq]
      refine Or.inl ⟨e, ⟨(hT.mem_treeOut u e).mpr ⟨ht, h1⟩, ?_⟩, h2⟩
      intro hpu
      exact hT.par_excl e ht ⟨by rw [h1]; exact hpu, by rw [h2]; exact hpc⟩
    · simp only [kids, List.mem_append, List.mem_map, List.mem_filter, decide_eq_true_eq]
      refine Or.inr ⟨e, ⟨(hT.mem_treeIn u e).mpr ⟨ht, h1⟩, ?_⟩, h2⟩
      intro hpu
      exact hT.par_excl e ht ⟨by rw [h2]; exact hpc, by rw [h1]; exact hpu⟩

/-- No parent edge is a loop, so no node is its own child. -/
theorem Forest.kid_ne (hT : N.Forest) {u c : V} (h : c ∈ N.kids u) : c ≠ u := by
  rintro rfl
  obtain ⟨e, ht, hp, hj⟩ := (hT.mem_kids).mp h
  rcases hj with ⟨a, b⟩ | ⟨a, b⟩
  · exact hT.par_excl e ht ⟨by rw [a]; exact hp, by rw [b]; exact hp⟩
  · exact hT.par_excl e ht ⟨by rw [b]; exact hp, by rw [a]; exact hp⟩

/-- Distinct nodes have distinct parent edges. -/
theorem Forest.par_inj (hT : N.Forest) {a b : V} {e : E} (ha : N.par a = some e)
    (hb : N.par b = some e) : a = b := by
  by_contra hne
  obtain ⟨ht, ha1⟩ := hT.par_tree a e ha
  obtain ⟨_, hb1⟩ := hT.par_tree b e hb
  rcases ha1 with ha1 | ha1 <;> rcases hb1 with hb1 | hb1
  · exact hne (ha1.symm.trans hb1)
  · exact hT.par_excl e ht ⟨by rw [ha1]; exact ha, by rw [hb1]; exact hb⟩
  · exact hT.par_excl e ht ⟨by rw [hb1]; exact hb, by rw [ha1]; exact ha⟩
  · exact hne (ha1.symm.trans hb1)

theorem Forest.parentOf_kid (hT : N.Forest) {q u : V} (h : u ∈ N.kids q) :
    N.parentOf u = some q := by
  obtain ⟨t, _, hp, hj⟩ := (hT.mem_kids).mp h
  have hne := hT.kid_ne h
  unfold parentOf
  rw [hp]
  simp only [Option.map_some, Option.some.injEq]
  rcases hj with ⟨a, b⟩ | ⟨a, b⟩
  · rw [ite_eq_right (by rw [a]; exact Ne.symm hne)]; exact a
  · rw [ite_eq_left b, a]

theorem Forest.kid_of_parentOf (hT : N.Forest) {q u : V} (h : N.parentOf u = some q) :
    u ∈ N.kids q := by
  unfold parentOf at h
  cases hp : N.par u with
  | none => rw [hp] at h; simp at h
  | some t =>
    rw [hp] at h
    simp only [Option.map_some, Option.some.injEq] at h
    obtain ⟨ht, hu⟩ := hT.par_tree u t hp
    refine (hT.mem_kids).mpr ⟨t, ht, hp, ?_⟩
    by_cases htu : N.tail t = u
    · rw [ite_eq_left htu] at h; exact Or.inr ⟨h, htu⟩
    · rw [ite_eq_right htu] at h
      exact Or.inl ⟨h, hu.resolve_left htu⟩

theorem Forest.kid_unique (hT : N.Forest) {u u' c : V} (h : c ∈ N.kids u) (h' : c ∈ N.kids u') :
    u = u' := by
  have := (hT.parentOf_kid h).symm.trans (hT.parentOf_kid h')
  exact Option.some.inj this

theorem Forest.nodup_kids (hT : N.Forest) (u : V) : (N.kids u).Nodup := by
  have hout : ∀ g, g ∈ (N.treeOut u).filter (fun e => N.par u ≠ some e) →
      N.isTree g = true ∧ N.tail g = u ∧ N.par (N.head g) = some g := by
    intro g hg
    obtain ⟨hg, hp⟩ := List.mem_filter.mp hg
    simp only [ne_eq, decide_eq_true_eq] at hp
    obtain ⟨ht, htl⟩ := (hT.mem_treeOut u g).mp hg
    refine ⟨ht, htl, ?_⟩
    rcases hT.tree_par g ht with h | h
    · rw [htl] at h; exact absurd h hp
    · exact h
  have hin : ∀ g, g ∈ (N.treeIn u).filter (fun e => N.par u ≠ some e) →
      N.isTree g = true ∧ N.head g = u ∧ N.par (N.tail g) = some g := by
    intro g hg
    obtain ⟨hg, hp⟩ := List.mem_filter.mp hg
    simp only [ne_eq, decide_eq_true_eq] at hp
    obtain ⟨ht, hhd⟩ := (hT.mem_treeIn u g).mp hg
    refine ⟨ht, hhd, ?_⟩
    rcases hT.tree_par g ht with h | h
    · exact h
    · rw [hhd] at h; exact absurd h hp
  unfold kids
  rw [List.nodup_append]
  refine ⟨?_, ?_, ?_⟩
  · refine List.Nodup.map_on ?_ ((hT.nodup_treeOut u).filter _)
    intro x hx y hy hxy
    have a := (hout x hx).2.2
    have b := (hout y hy).2.2
    rw [hxy, b] at a
    exact (Option.some.inj a).symm
  · refine List.Nodup.map_on ?_ ((hT.nodup_treeIn u).filter _)
    intro x hx y hy hxy
    have a := (hin x hx).2.2
    have b := (hin y hy).2.2
    rw [hxy, b] at a
    exact (Option.some.inj a).symm
  · intro a ha b hb hab
    obtain ⟨g1, hg1, rfl⟩ := List.mem_map.mp ha
    obtain ⟨g2, hg2, rfl⟩ := List.mem_map.mp hb
    obtain ⟨_, h1t, h1p⟩ := hout g1 hg1
    obtain ⟨_, h2h, h2p⟩ := hin g2 hg2
    rw [hab, h2p] at h1p
    obtain rfl := Option.some.inj h1p
    have hp := (List.mem_filter.mp hg1).2
    simp only [ne_eq, decide_eq_true_eq] at hp
    exact hp (by rw [← h1t]; exact h2p)

theorem inSub_last' {v c : V} (h : N.InSub v c) : c = v ∨ ∃ q, N.InSub v q ∧ c ∈ N.kids q := by
  induction h with
  | refl => exact Or.inl rfl
  | @kid u c' w hc hw ih =>
    rcases ih with rfl | ⟨q, hq, hwq⟩
    · exact Or.inr ⟨u, InSub.refl u, hc⟩
    · exact Or.inr ⟨q, InSub.kid hc hq, hwq⟩

/-! ### No cycles: the parent chain from any node reaches the root -/

/-- One step up the parent pointers. -/
def up (N : Net V E) (o : Option V) : Option V := o.bind N.parentOf

theorem up_iterate_none (N : Net V E) : ∀ m, (N.up)^[m] none = none := by
  intro m
  induction m with
  | zero => rfl
  | succ m ih => rw [Function.iterate_succ_apply', ih]; rfl

theorem Forest.anc_of_inSub (hT : N.Forest) {a b : V} (h : N.InSub a b) :
    ∃ j, (N.up)^[j] (some b) = some a := by
  induction h with
  | refl => exact ⟨0, rfl⟩
  | @kid u c w hc _ ih =>
    obtain ⟨j, hj⟩ := ih
    refine ⟨j + 1, ?_⟩
    rw [Function.iterate_succ_apply', hj]
    exact hT.parentOf_kid hc

theorem Forest.inSub_of_anc (hT : N.Forest) {b : V} :
    ∀ j a, (N.up)^[j] (some b) = some a → N.InSub a b := by
  intro j
  induction j with
  | zero => intro a h; obtain rfl := Option.some.inj h; exact InSub.refl b
  | succ j ih =>
    intro a h
    rw [Function.iterate_succ_apply'] at h
    cases hy : (N.up)^[j] (some b) with
    | none => rw [hy] at h; simp [up] at h
    | some y =>
      rw [hy] at h
      exact InSub.kid (hT.kid_of_parentOf h) (ih y hy)

/-- Subtrees are a partial order: there is no cycle of children. -/
theorem Tree.sub_antisymm (hT : N.Tree root) {u w : V} (h1 : N.InSub u w) (h2 : N.InSub w u) :
    u = w := by
  obtain ⟨i, hi⟩ := hT.anc_of_inSub h1
  obtain ⟨j, hj⟩ := hT.anc_of_inSub h2
  have hper : (N.up)^[i + j] (some u) = some u := by
    rw [Function.iterate_add_apply, hj, hi]
  by_cases hp : i + j = 0
  · have : j = 0 := by omega
    subst this
    exact Option.some.inj hj
  have hall : ∀ t, (N.up)^[(i + j) * t] (some u) = some u := by
    intro t
    induction t with
    | zero => rfl
    | succ t ih => rw [Nat.mul_succ, Nat.add_comm, Function.iterate_add_apply, ih, hper]
  obtain ⟨r, hr⟩ := hT.anc_of_inSub (hT.span u)
  have hnone : (N.up)^[r + 1] (some u) = none := by
    rw [Function.iterate_succ_apply', hr]
    simp [up, parentOf, hT.par_root]
  have hle : r + 1 ≤ (i + j) * (r + 1) := Nat.le_mul_of_pos_left _ (by omega)
  have := hall (r + 1)
  rw [show (i + j) * (r + 1) = ((i + j) * (r + 1) - (r + 1)) + (r + 1) by omega,
    Function.iterate_add_apply, hnone, up_iterate_none] at this
  simp at this

/-- Two children of one node have disjoint subtrees. -/
theorem Tree.sub_disjoint (hT : N.Tree root) {u c c' x : V} (hc : c ∈ N.kids u)
    (hc' : c' ∈ N.kids u) (hne : c ≠ c') (hx : N.InSub c x) (hx' : N.InSub c' x) : False := by
  obtain ⟨i, hi⟩ := hT.anc_of_inSub hx
  obtain ⟨j, hj⟩ := hT.anc_of_inSub hx'
  -- the ancestors of `x` form a chain: one of `c`, `c'` is above the other
  have key : ∀ {a b : V} {i j : ℕ}, a ∈ N.kids u → b ∈ N.kids u → a ≠ b →
      (N.up)^[i] (some x) = some a → (N.up)^[j] (some x) = some b → i ≤ j → False := by
    intro a b i j ha hb hab hia hjb hij
    have hab' : (N.up)^[j - i] (some a) = some b := by
      rw [← hia, ← Function.iterate_add_apply, show j - i + i = j by omega, hjb]
    have hba : N.InSub b a := hT.inSub_of_anc _ _ hab'
    rcases inSub_last' hba with h | ⟨q, hq, haq⟩
    · exact hab h
    · obtain rfl : q = u := hT.kid_unique haq ha
      exact hT.kid_ne hb (hT.sub_antisymm (InSub.kid hb (InSub.refl b)) hq).symm
  rcases le_total i j with h | h
  · exact key hc hc' hne hi hj h
  · exact key hc' hc (Ne.symm hne) hj hi h

/-- **A spanning tree's parent pointers are determined by its edges and its root.** Two stored
trees over the same edges, tree and root have the same `par`: so the pointers the re-hang leaves
are the ones C's `dfs_range` assigns, as soon as both are trees of `T − e + f` from the root. -/
theorem Tree.par_unique {N₂ : Net V E} (h₁ : N.Tree root) (h₂ : N₂.Tree root)
    (htail : N₂.tail = N.tail) (hhead : N₂.head = N.head) (htree : N₂.isTree = N.isTree) :
    N₂.par = N.par := by
  funext w
  obtain ⟨j, hj⟩ := h₁.anc_of_inSub (h₁.span w)
  induction j generalizing w with
  | zero => obtain rfl := Option.some.inj hj; rw [h₁.par_root, h₂.par_root]
  | succ j ih =>
    rw [Function.iterate_succ_apply] at hj
    cases hp : N.par w with
    | none =>
      rw [show N.up (some w) = none by simp [up, parentOf, hp], up_iterate_none] at hj
      exact absurd hj (by simp)
    | some t =>
      set p := if N.tail t = w then N.head t else N.tail t with hpdef
      have hpo : N.up (some w) = some p := by simp [up, parentOf, hp, hpdef]
      rw [hpo] at hj
      have hpp := ih p hj
      obtain ⟨htt, hends⟩ := h₁.par_tree w t hp
      have hpw : p ≠ w := by
        intro h
        rw [hpdef] at h
        split_ifs at h with h'
        · exact h₁.par_excl t htt ⟨h' ▸ hp, h ▸ hp⟩
        · exact h' h
      by_contra hne
      have h2 : N₂.par p = some t := by
        have := h₂.tree_par t (by rw [htree]; exact htt)
        rw [htail, hhead] at this
        rw [hpdef]
        split_ifs with h'
        · rcases this with h | h
          · exact absurd (h' ▸ h) hne
          · exact h
        · have hh : N.head t = w := hends.resolve_left h'
          rcases this with h | h
          · exact h
          · exact absurd (hh ▸ h) hne
      rw [hpp] at h2
      exact hpw (h₁.par_inj h2 hp)

/-! ### A tree always has a `dfs_range` numbering -/

theorem Chain.congr {low lim low' lim' : V → ℕ} :
    ∀ {a cs b}, Chain low lim a cs b → (∀ c ∈ cs, low' c = low c ∧ lim' c = lim c) →
      Chain low' lim' a cs b := by
  intro a cs b h
  induction h with
  | nil a => intro _; exact Chain.nil a
  | @cons a b c cs h1 _ ih =>
    intro hx
    have hc := hx c (by simp)
    refine Chain.cons (by rw [hc.1]; exact h1) ?_
    rw [hc.2]
    exact ih (fun x hx' => hx x (by simp [hx']))

theorem Chain.le {low lim : V → ℕ} :
    ∀ {a cs b}, Chain low lim a cs b → (∀ c ∈ cs, low c ≤ lim c) → a ≤ b := by
  intro a cs b h
  induction h with
  | nil a => intro _; exact le_rfl
  | @cons a b c cs h1 _ ih =>
    intro hx
    have := hx c (by simp)
    have := ih (fun x hx' => hx x (by simp [hx']))
    omega

/-- `low`/`lim` number `subtree(u)` as `dfs_range` does. -/
def NumOK (N : Net V E) (low lim : V → ℕ) (u : V) : Prop :=
  ∀ w, N.InSub u w → Chain low lim (low w) (N.kids w) (lim w) ∧ low w ≤ lim w

theorem NumOK.congr {low lim low' lim' : V → ℕ} {u : V} (h : N.NumOK low lim u)
    (he : ∀ w, N.InSub u w → low' w = low w ∧ lim' w = lim w) : N.NumOK low' lim' u := by
  intro w hw
  obtain ⟨hc, hl⟩ := h w hw
  have hew := he w hw
  refine ⟨?_, by rw [hew.1, hew.2]; exact hl⟩
  rw [hew.1, hew.2]
  exact hc.congr (fun x hx => he x (hw.trans (InSub.kid hx (InSub.refl x))))

/-- Every subtree can be numbered from any start, children left to right. -/
theorem Tree.exists_num [Fintype V] (hT : N.Tree root) (u : V) (a : ℕ) :
    ∃ low lim : V → ℕ, N.NumOK low lim u ∧ low u = a := by
  classical
  suffices H : ∀ n u, (Finset.univ.filter (fun w => N.InSub u w)).card = n →
      ∀ a, ∃ low lim : V → ℕ, N.NumOK low lim u ∧ low u = a from H _ u rfl a
  intro n
  induction n using Nat.strong_induction_on with
  | _ n ih =>
  intro u hn a
  have hbelow : ∀ c ∈ N.kids u, ∀ w, N.InSub c w → w ≠ u := by
    intro c hc w hw hwu
    subst hwu
    exact hT.kid_ne hc (hT.sub_antisymm hw (InSub.kid hc (InSub.refl c)))
  -- the children's subtrees are smaller
  have hkid : ∀ c ∈ N.kids u, ∀ a, ∃ low lim : V → ℕ, N.NumOK low lim c ∧ low c = a := by
    intro c hc
    refine ih _ ?_ c rfl
    rw [← hn]
    apply Finset.card_lt_card
    rw [Finset.ssubset_iff_of_subset]
    · refine ⟨u, by simp [InSub.refl], ?_⟩
      simp only [Finset.mem_filter, Finset.mem_univ, true_and]
      exact fun h => hbelow c hc u h rfl
    · intro w
      simp only [Finset.mem_filter, Finset.mem_univ, true_and]
      exact fun hw => InSub.kid hc hw
  -- the children, glued left to right
  have inner : ∀ cs : List V, cs.Nodup → (∀ c ∈ cs, c ∈ N.kids u) → ∀ a, ∃ (low lim : V → ℕ) (b : ℕ),
      (∀ c ∈ cs, N.NumOK low lim c) ∧ Chain low lim a cs b := by
    intro cs
    induction cs with
    | nil => intro _ _ a; exact ⟨fun _ => 0, fun _ => 0, a, by simp, Chain.nil a⟩
    | cons c cs ihc =>
      intro hnd hcs a
      have hc := hcs c (by simp)
      have hnot : ∀ x ∈ cs, ∀ w, N.InSub x w → ¬N.InSub c w := by
        intro x hx w hxw hcw
        have hxc : x ≠ c := fun h => (List.nodup_cons.mp hnd).1 (h ▸ hx)
        exact hT.sub_disjoint hc (hcs x (by simp [hx])) (Ne.symm hxc) hcw hxw
      obtain ⟨l1, m1, h1, h1a⟩ := hkid c hc a
      obtain ⟨l2, m2, b, h2, h2c⟩ :=
        ihc (List.nodup_cons.mp hnd).2 (fun x hx => hcs x (by simp [hx])) (m1 c + 1)
      refine ⟨fun w => if N.InSub c w then l1 w else l2 w,
        fun w => if N.InSub c w then m1 w else m2 w, b, ?_, ?_⟩
      · intro x hx
        rcases List.mem_cons.mp hx with rfl | hx
        · exact h1.congr (fun w hw => by simp [hw])
        · exact (h2 x hx).congr (fun w hw => by simp [hnot x hx w hw])
      · refine Chain.cons (by simp [InSub.refl, h1a]) ?_
        simp only [InSub.refl, ite_true]
        exact h2c.congr (fun x hx => by simp [hnot x hx x (InSub.refl x)])
  obtain ⟨l, m, b, hl, hch⟩ := inner (N.kids u) (hT.nodup_kids u) (fun _ h => h) a
  refine ⟨Function.update l u a, Function.update m u b, ?_, by simp⟩
  have hkeep : ∀ c ∈ N.kids u, ∀ w, N.InSub c w →
      Function.update l u a w = l w ∧ Function.update m u b w = m w := by
    intro c hc w hw
    have := hbelow c hc w hw
    exact ⟨Function.update_of_ne this _ _, Function.update_of_ne this _ _⟩
  intro w hw
  cases hw with
  | refl =>
    simp only [Function.update_self]
    have hcong : Chain (Function.update l u a) (Function.update m u b) a (N.kids u) b :=
      hch.congr (fun x hx => hkeep x hx x (InSub.refl x))
    refine ⟨hcong, hcong.le (fun x hx => ?_)⟩
    rw [(hkeep x hx x (InSub.refl x)).1, (hkeep x hx x (InSub.refl x)).2]
    exact ((hl x hx) x (InSub.refl x)).2
  | kid hc hcw =>
    exact ((hl _ hc).congr (fun x hx => hkeep _ hc x hx)) w hcw

/-- **A spanning tree on a finite graph has a valid numbering**: the one `dfs_range` gives it
from the root, or any other. So `Valid`'s numbering part costs nothing to assume once `Tree`
holds. -/
theorem Tree.exists_valid [Fintype V] (hT : N.Tree root) :
    ∃ low lim : V → ℕ, (N.renum low lim).Valid root := by
  obtain ⟨low, lim, h, _⟩ := hT.exists_num root 1
  exact ⟨low, lim, hT.valid_of_chain (fun u => (h u (hT.span u)).1) (fun u => (h u (hT.span u)).2)⟩

/-! ### The re-hang keeps a spanning tree (§4.2, (I3))

`CutStore::update` re-hangs the moved subtree from `f`: starting at f's end `u0` below e's child
`v`, it sets `par(u0) = f`, then walks up the old parent pointers to `v`, giving each node the old
parent edge of the node below it. Nodes off that path keep their parent edge. -/

/-- Where the re-hang starts: `if tail_is_child { head(f) } else { tail(f) }`. -/
def rehangStart (N : Net V E) (e f : E) : V :=
  if N.par (N.tail e) = some e then N.head f else N.tail f

/-- f's other end, above e's cut. -/
def rehangOther (N : Net V E) (e f : E) : V :=
  if N.par (N.tail e) = some e then N.tail f else N.head f

theorem Valid.childOf_par (hN : N.Valid root) {e : E} (he : N.isTree e = true) :
    N.par (N.childOf e).1 = some e := by
  unfold childOf
  split_ifs with h
  · exact h
  · exact (hN.tree_par e he).resolve_left h

/-- f's start end is below e's child; its other end is not. -/
theorem Valid.rehang_ends (hN : N.Valid root) {e f : E} (he : N.isTree e = true)
    (hef : N.Against e f) :
    N.InSub (N.childOf e).1 (N.rehangStart e f) ∧ ¬N.InSub (N.childOf e).1 (N.rehangOther e f) ∧
      ((N.tail f = N.rehangStart e f ∧ N.head f = N.rehangOther e f) ∨
       (N.head f = N.rehangStart e f ∧ N.tail f = N.rehangOther e f)) := by
  have hv := hN.childOf_par he
  obtain ⟨u, hu, hj⟩ := hN.parent hv
  have side := fun x => side_iff hN hv hu (x := x)
  have hne := hN.kid_ne hu
  obtain ⟨h1, h2⟩ := hef
  unfold rehangStart rehangOther
  unfold childOf at hv hu side hne ⊢
  by_cases ht : N.par (N.tail e) = some e
  · simp only [ht, ite_true] at hv hu side hne hj ⊢
    have hue : N.head e = u := by
      rcases hj with ⟨a, _⟩ | ⟨a, _⟩
      · exact absurd a hne
      · exact a
    rw [hue] at h1
    exact ⟨(side _).1.mp h2, (side _).2.mp h1, by simp⟩
  · simp only [ht, ite_false] at hv hu side hne hj ⊢
    have hue : N.tail e = u := by
      rcases hj with ⟨a, _⟩ | ⟨a, _⟩
      · exact a
      · exact absurd a hne
    rw [hue] at h2
    exact ⟨(side _).1.mp h1, (side _).2.mp h2, by simp⟩

/-- **(I3), as a tree.** The pointers the re-hang leaves (`hp0`–`hp2`: what the loop in
`CutStore::update` assigns) and the tree lists `exchange_tree_edges` leaves (`hin`, `hout`: the
tree edges `T − e + f` at each end, once each) form a spanning tree from the same root, whatever
`lim`/`low` the state carries. -/
theorem Valid.rehang_tree (hN : N.Valid root) {e f : E} (he : N.isTree e = true)
    (hf : N.isTree f = false) (hef : N.Against e f)
    {treeIn' treeOut' : V → List E} {par' : V → Option E} (low' lim' : V → ℕ)
    (hin : ∀ u g, g ∈ treeIn' u ↔ CutBasis.pivotT N.isTree e f g = true ∧ N.head g = u)
    (hout : ∀ u g, g ∈ treeOut' u ↔ CutBasis.pivotT N.isTree e f g = true ∧ N.tail g = u)
    (hnin : ∀ u, (treeIn' u).Nodup) (hnout : ∀ u, (treeOut' u).Nodup)
    (hp0 : par' (N.rehangStart e f) = some f)
    (hp1 : ∀ q c, c ∈ N.kids q → N.InSub (N.childOf e).1 q → N.InSub c (N.rehangStart e f) →
      par' q = N.par c)
    (hp2 : ∀ w, ¬(N.InSub w (N.rehangStart e f) ∧ N.InSub (N.childOf e).1 w) → par' w = N.par w) :
    (N.repivot e f treeIn' treeOut' par' low' lim').Tree root := by
  set N' := N.repivot e f treeIn' treeOut' par' low' lim' with hN'
  set v := (N.childOf e).1 with hvdef
  set u0 := N.rehangStart e f with hu0def
  set y := N.rehangOther e f with hydef
  have hT := hN.tree
  have hv : N.par v = some e := hN.childOf_par he
  obtain ⟨hu0, hy, hends⟩ : N.InSub v u0 ∧ ¬N.InSub v y ∧
      ((N.tail f = u0 ∧ N.head f = y) ∨ (N.head f = u0 ∧ N.tail f = y)) := hN.rehang_ends he hef
  have hfe : f ≠ e := by rintro rfl; rw [he] at hf; exact absurd hf (by simp)
  have hT' : ∀ x, CutBasis.pivotT N.isTree e f x = true ↔ x = f ∨ (x ≠ e ∧ N.isTree x = true) := by
    intro x
    unfold CutBasis.pivotT
    by_cases hxe : x = e
    · subst hxe; simp [Ne.symm hfe]
    · by_cases hxf : x = f
      · subst hxf; simp [hxe]
      · simp [hxe, hxf]
  have hu0y : u0 ≠ y := fun h => hy (h ▸ hu0)
  -- the path from `u0` up to `v`
  let P : V → Prop := fun w => N.InSub w u0 ∧ N.InSub v w
  have hPu0 : P u0 := ⟨InSub.refl u0, hu0⟩
  have hPv : P v := ⟨hu0, InSub.refl v⟩
  have hparE : ∀ w, N.par w = some e → w = v := fun w h => hT.par_inj h hv
  -- a path node other than `u0` takes the old parent edge of its child on the path
  have hpath : ∀ w, P w → w ≠ u0 → ∃ c, c ∈ N.kids w ∧ P c ∧ par' w = N.par c := by
    intro w hw hwu
    obtain ⟨hw1, hw2⟩ := hw
    cases hw1 with
    | refl => exact absurd rfl hwu
    | kid hc hcu => exact ⟨_, hc, ⟨hcu, hw2.trans (InSub.kid hc (InSub.refl _))⟩, hp1 w _ hc hw2 hcu⟩
  have hkidE : ∀ w c t, c ∈ N.kids w → P w → N.par c = some t → t ≠ e := by
    intro w c t hc hw hpc hte
    subst hte
    have hcv := hparE c hpc
    subst hcv
    exact hN.kid_ne hc (hT.sub_antisymm hw.2 (InSub.kid hc (InSub.refl _)))
  have hoffE : ∀ w, ¬P w → N.par w ≠ some e := fun w hw h => hw (hparE w h ▸ hPv)
  -- every parent edge after the re-hang, by where it comes from
  have hcase : ∀ w t, par' w = some t →
      (w = u0 ∧ t = f) ∨ (t ≠ f ∧ t ≠ e ∧ N.isTree t = true ∧ ∃ c, N.par c = some t ∧
        ((¬P w ∧ c = w) ∨ (P w ∧ w ≠ u0 ∧ c ∈ N.kids w ∧ P c))) := by
    intro w t hw
    by_cases hwu : w = u0
    · subst hwu; rw [hp0] at hw; exact Or.inl ⟨rfl, (Option.some.inj hw).symm⟩
    by_cases hPw : P w
    · obtain ⟨c, hc, hPc, hpc⟩ := hpath w hPw hwu
      rw [hpc] at hw
      have htt := (hT.par_tree c t hw).1
      refine Or.inr ⟨fun h => by rw [h, hf] at htt; exact absurd htt (by simp),
        hkidE w c t hc hPw hw, htt, c, hw, Or.inr ⟨hPw, hwu, hc, hPc⟩⟩
    · rw [hp2 w hPw] at hw
      have htt := (hT.par_tree w t hw).1
      refine Or.inr ⟨fun h => by rw [h, hf] at htt; exact absurd htt (by simp),
        fun h => hoffE w hPw (h ▸ hw), htt, w, hw, Or.inl ⟨hPw, rfl⟩⟩
  -- so the new parent edges are distinct
  have hinj : ∀ a b t, par' a = some t → par' b = some t → a = b := by
    intro a b t ha hb
    rcases hcase a t ha with ⟨rfl, rfl⟩ | ⟨hta, _, _, ca, hca, hA⟩ <;>
      rcases hcase b t hb with ⟨rfl, hbt⟩ | ⟨htb, _, _, cb, hcb, hB⟩
    · rfl
    · exact absurd rfl htb
    · exact absurd hbt hta
    · have hcc : ca = cb := hT.par_inj hca hcb
      subst hcc
      rcases hA with ⟨hPa, rfl⟩ | ⟨hPa, _, hka, hPc⟩ <;> rcases hB with ⟨hPb, rfl⟩ | ⟨hPb, _, hkb, hPc'⟩
      · rfl
      · exact absurd hPc' hPa
      · exact absurd hPc hPb
      · exact hT.kid_unique hka hkb
  have hF' : N'.Forest := by
    refine ⟨hN.mem_inE, hN.mem_outE, hin, hout, hN.nodup_inE, hN.nodup_outE, hnin, hnout, ?_, ?_, ?_⟩
    · -- par_tree
      intro w t hw
      show CutBasis.pivotT N.isTree e f t = true ∧ (N.tail t = w ∨ N.head t = w)
      rcases hcase w t hw with ⟨rfl, htf⟩ | ⟨htf, hte, htt, c, hpc, hc⟩
      · rw [htf]
        refine ⟨(hT' f).mpr (Or.inl rfl), ?_⟩
        rcases hends with ⟨a, _⟩ | ⟨a, _⟩
        · exact Or.inl a
        · exact Or.inr a
      · refine ⟨(hT' t).mpr (Or.inr ⟨hte, htt⟩), ?_⟩
        rcases hc with ⟨_, rfl⟩ | ⟨_, _, hk, _⟩
        · exact (hT.par_tree c t hpc).2
        · obtain ⟨t', _, hpc', hj⟩ := (hT.mem_kids).mp hk
          rw [hpc] at hpc'
          obtain rfl := Option.some.inj hpc'
          rcases hj with ⟨a, _⟩ | ⟨a, _⟩
          · exact Or.inl a
          · exact Or.inr a
    · -- tree_par
      intro t ht
      show par' (N.tail t) = some t ∨ par' (N.head t) = some t
      rcases (hT' t).mp ht with rfl | ⟨hte, htt⟩
      · rcases hends with ⟨a, _⟩ | ⟨a, _⟩
        · exact Or.inl (a ▸ hp0)
        · exact Or.inr (a ▸ hp0)
      · have htf : t ≠ f := fun h => by rw [h, hf] at htt; exact absurd htt (by simp)
        -- the child end `c` of `t` and its parent end `q`
        have key : ∀ c q, N.par c = some t → c ∈ N.kids q → par' c = some t ∨ par' q = some t := by
          intro c q hpc hcq
          by_cases hPc : P c
          · right
            have hcv : c ≠ v := by
              rintro rfl
              rw [hpc] at hv
              exact hte (Option.some.inj hv)
            rcases inSub_last' hPc.2 with h | ⟨q', hq', hcq'⟩
            · exact absurd h hcv
            · obtain rfl := hT.kid_unique hcq' hcq
              rw [hp1 q' c hcq hq' hPc.1, hpc]
          · left; rw [hp2 c hPc, hpc]
        rcases hT.tree_par t htt with h | h
        · have hk : N.tail t ∈ N.kids (N.head t) := (hT.mem_kids).mpr ⟨t, htt, h, Or.inr ⟨rfl, rfl⟩⟩
          exact key _ _ h hk
        · have hk : N.head t ∈ N.kids (N.tail t) := (hT.mem_kids).mpr ⟨t, htt, h, Or.inl ⟨rfl, rfl⟩⟩
          exact (key _ _ h hk).symm
    · -- par_excl
      intro t ht hboth
      obtain ⟨h1, h2⟩ := hboth
      have hloop : N.tail t = N.head t := hinj _ _ t h1 h2
      rcases (hT' t).mp ht with rfl | ⟨_, htt⟩
      · rcases hends with ⟨a, b⟩ | ⟨a, b⟩
        · exact hu0y (a.symm.trans (hloop.trans b))
        · exact hu0y (a.symm.trans (hloop.symm.trans b))
      · rcases hT.tree_par t htt with h | h
        · exact hT.par_excl t htt ⟨h, hloop ▸ h⟩
        · exact hT.par_excl t htt ⟨hloop ▸ h, h⟩
  -- the new parent pointers, read as nodes
  have hpo_off : ∀ w, ¬P w → N'.parentOf w = N.parentOf w := by
    intro w hw
    show (par' w).map _ = (N.par w).map _
    rw [hp2 w hw]
    rfl
  have hpo_u0 : N'.parentOf u0 = some y := by
    show (par' u0).map _ = _
    rw [hp0]
    simp only [Option.map_some, Option.some.injEq]
    show (if N.tail f = u0 then N.head f else N.tail f) = y
    rcases hends with ⟨a, b⟩ | ⟨a, b⟩
    · rw [ite_eq_left a, b]
    · rw [ite_eq_right (by rw [b]; exact Ne.symm hu0y), b]
  have hpo_path : ∀ q c, P q → q ≠ u0 → c ∈ N.kids q → N.InSub c u0 → N'.parentOf q = some c := by
    intro q c hq hqu hc hcu
    have hne := hN.kid_ne hc
    obtain ⟨t, _, hpc, hj⟩ := (hT.mem_kids).mp hc
    show (par' q).map _ = _
    rw [hp1 q c hc hq.2 hcu, hpc]
    simp only [Option.map_some, Option.some.injEq]
    show (if N.tail t = q then N.head t else N.tail t) = c
    rcases hj with ⟨a, b⟩ | ⟨a, b⟩
    · rw [ite_eq_left a, b]
    · rw [ite_eq_right (by rw [b]; exact hne), b]
  -- every node's new parent chain reaches the root
  let Reach : V → Prop := fun w => ∃ j, (N'.up)^[j] (some w) = some root
  have hstep : ∀ w p, N'.parentOf w = some p → Reach p → Reach w := by
    rintro w p hw ⟨j, hj⟩
    refine ⟨j + 1, ?_⟩
    rw [Function.iterate_succ_apply]
    show (N'.up)^[j] ((some w).bind N'.parentOf) = _
    rw [Option.bind_some, hw, hj]
  have hR1 : ∀ j w, (N.up)^[j] (some w) = some root → ¬N.InSub v w → Reach w := by
    intro j
    induction j with
    | zero => intro w hw _; obtain rfl := Option.some.inj hw; exact ⟨0, rfl⟩
    | succ j ih =>
      intro w hw hvw
      rw [Function.iterate_succ_apply] at hw
      cases hp : N.parentOf w with
      | none =>
        rw [show N.up (some w) = none by simp [up, hp], up_iterate_none] at hw
        exact absurd hw (by simp)
      | some p =>
        rw [show N.up (some w) = some p by simp [up, hp]] at hw
        have hvp : ¬N.InSub v p := fun h => hvw (h.trans (InSub.kid (hT.kid_of_parentOf hp) (InSub.refl w)))
        exact hstep w p (by rw [hpo_off w (fun h => hvw h.2), hp]) (ih p hw hvp)
  have hR1' : ∀ w, ¬N.InSub v w → Reach w := by
    intro w hw
    obtain ⟨j, hj⟩ := hT.anc_of_inSub (hT.span w)
    exact hR1 j w hj hw
  have hR3 : ∀ i q, (N.up)^[i] (some u0) = some q → N.InSub v q → Reach q := by
    intro i
    induction i with
    | zero =>
      intro q hq _
      obtain rfl := Option.some.inj hq
      exact hstep u0 y hpo_u0 (hR1' y hy)
    | succ i ih =>
      intro q hq hvq
      rw [Function.iterate_succ_apply'] at hq
      cases hc : (N.up)^[i] (some u0) with
      | none => rw [hc] at hq; simp [up] at hq
      | some c =>
        rw [hc] at hq
        have hcq : c ∈ N.kids q := hT.kid_of_parentOf hq
        have hcu : N.InSub c u0 := hT.inSub_of_anc i c hc
        have hvc : N.InSub v c := hvq.trans (InSub.kid hcq (InSub.refl c))
        have hqu : q ≠ u0 := by
          rintro rfl
          exact hN.kid_ne hcq (hT.sub_antisymm hcu (InSub.kid hcq (InSub.refl c)))
        exact hstep q c (hpo_path q c ⟨InSub.kid hcq hcu, hvq⟩ hqu hcq hcu) (ih c hc hvc)
  have hR4 : ∀ j w, (N.up)^[j] (some w) = some v → Reach w := by
    intro j
    induction j with
    | zero =>
      intro w hw
      obtain rfl := Option.some.inj hw
      obtain ⟨i, hi⟩ := hT.anc_of_inSub hu0
      exact hR3 i v hi (InSub.refl v)
    | succ j ih =>
      intro w hw
      by_cases hPw : P w
      · obtain ⟨i, hi⟩ := hT.anc_of_inSub hPw.1
        exact hR3 i w hi hPw.2
      rw [Function.iterate_succ_apply] at hw
      cases hp : N.parentOf w with
      | none =>
        rw [show N.up (some w) = none by simp [up, hp], up_iterate_none] at hw
        exact absurd hw (by simp)
      | some p =>
        rw [show N.up (some w) = some p by simp [up, hp]] at hw
        exact hstep w p (by rw [hpo_off w hPw, hp]) (ih p hw)
  have hroot : ¬P root := by
    intro h
    have := hT.sub_antisymm h.2 (hT.span v)
    rw [this, hT.par_root] at hv
    exact absurd hv (by simp)
  refine ⟨hF', fun w => ?_, ?_⟩
  · have hreach : Reach w := by
      by_cases hvw : N.InSub v w
      · obtain ⟨j, hj⟩ := hT.anc_of_inSub hvw
        exact hR4 j w hj
      · exact hR1' w hvw
    obtain ⟨j, hj⟩ := hreach
    exact hF'.inSub_of_anc j root hj
  · show par' root = none
    rw [hp2 root hroot, hT.par_root]

end TreeSec

/-! ## Chaining the pivots -/

section Chain

variable {N : Net V E}

/-- **The production `enter_edge(e)` is C's on every valid numbering of the stored tree**, with
one bound `K` on the climbs' fuel and on C's loop. `lowF`/`limF` is the fallback's renumbering
(any valid one). -/
theorem enterEdgeP_eq_every {lowF limF : V → ℕ} {rootF : V}
    (hF : (N.renum lowF limF).Valid rootF) {e : E} (he : N.isTree e = true)
    {cover : List (E × Bool)} (hcov : N.CoverOK e cover) (slack : E → ℤ) (pivot : Bool) :
    ∃ K, ∀ n k, K ≤ n → K ≤ k → ∀ (lowC limC : V → ℕ) (rootC : V),
      (N.renum lowC limC).Valid rootC →
        N.enterEdgeP slack n k pivot cover e lowF limF = (N.renum lowC limC).cEnter slack k e := by
  obtain ⟨K, hK⟩ := enterEdgeP_eq hF hF he hcov slack pivot
  exact ⟨K, fun n k hn hk lowC limC rootC hC => (hK n k hn hk).trans (cEnter_renum hF hC he slack k)⟩

/-- **A pivot leaves the next pivot's hypotheses.** From a stored state `N` (any `lim`/`low`) with
a valid numbering `lowA`/`limA`, the pivot `e → f` with the re-hang and tree lists of
`CutStore::update` leaves a state that keeps `N`'s now stale `lim`/`low` and:
1. is a spanning tree from the same root (`Tree`);
2. has a valid numbering (so `enterEdgeP_eq_every` applies at the next pivot);
3. has the cut sets of §4.2: `g` crosses the new cut of `t` against `t` exactly as the old cut
   basis says (`Valid.pivot_against`, here with nothing assumed of the new tree). -/
theorem pivot_step [Fintype V] {root : V} {lowA limA : V → ℕ}
    (hA : (N.renum lowA limA).Valid root) {e f : E} (he : N.isTree e = true)
    (hf : N.isTree f = false) (hef : N.Against e f)
    {treeIn' treeOut' : V → List E} {par' : V → Option E}
    (hin : ∀ u g, g ∈ treeIn' u ↔ CutBasis.pivotT N.isTree e f g = true ∧ N.head g = u)
    (hout : ∀ u g, g ∈ treeOut' u ↔ CutBasis.pivotT N.isTree e f g = true ∧ N.tail g = u)
    (hnin : ∀ u, (treeIn' u).Nodup) (hnout : ∀ u, (treeOut' u).Nodup)
    (hp0 : par' (N.rehangStart e f) = some f)
    (hp1 : ∀ q c, c ∈ N.kids q → N.InSub (N.childOf e).1 q → N.InSub c (N.rehangStart e f) →
      par' q = N.par c)
    (hp2 : ∀ w, ¬(N.InSub w (N.rehangStart e f) ∧ N.InSub (N.childOf e).1 w) → par' w = N.par w) :
    (N.repivot e f treeIn' treeOut' par' N.low N.lim).Tree root ∧
    (∃ low' lim' : V → ℕ,
      ((N.repivot e f treeIn' treeOut' par' N.low N.lim).renum low' lim').Valid root) ∧
    ∀ t, CutBasis.pivotT N.isTree e f t = true → ∀ g,
      ((N.repivot e f treeIn' treeOut' par' N.low N.lim).Against t g ↔
        (if t = f then -hA.cutBasis.coef e g
         else hA.cutBasis.coef t g - hA.cutBasis.coef t f * hA.cutBasis.coef e g) = 1) := by
  have hT' : (N.repivot e f treeIn' treeOut' par' N.low N.lim).Tree root :=
    hA.rehang_tree (e := e) (f := f) he hf (against_renum.mpr hef) N.low N.lim hin hout hnin hnout
      hp0 (fun q c hc hq hcu => hp1 q c hc (inSub_renum.mp hq) (inSub_renum.mp hcu))
      (fun w hw => hp2 w (fun h => hw ⟨inSub_renum.mpr h.1, inSub_renum.mpr h.2⟩))
  obtain ⟨l', m', hV'⟩ := hT'.exists_valid
  refine ⟨hT', ⟨l', m', hV'⟩, fun t ht g => ?_⟩
  have key := hA.pivot_against he hf (against_renum.mpr hef) treeIn' treeOut' par' l' m'
    (root' := root) hV' ht g
  rw [← key]
  exact against_renum.symm

end Chain

end Net

/-! ## Example: one pivot makes the numbering stale

Three nodes and the edges `a = 0 : 0 → 1`, `b = 1 : 1 → 2`, `c = 2 : 2 → 0`; the tree is `a`, `b`
from root `0`, numbered as `dfs_range` numbers it. The pivot `a → c` re-hangs node 2 from `c` and
node 1 from `b`. The production state keeps the old `lim`, so node 1 (`lim` 2) now hangs below
node 2 (`lim` 1): the shape of the issue's reproduction. At the next query on `b`, C's search on
that stale numbering looks on the wrong side of b's cut and finds no edge; the cut-set path and
C's search on a valid numbering both find `a`. So `fall_back`'s renumbering is needed, and the
cut-set path does not need it. -/

namespace Example750

open Net

/-- Before the pivot. -/
def exO : Net (Fin 3) (Fin 3) where
  tail e := e
  head e := e + 1
  inE u := [u - 1]
  outE u := [u]
  treeIn u := if u = 0 then [] else [u - 1]
  treeOut u := if u = 2 then [] else [u]
  isTree e := e ≠ 2
  par u := if u = 0 then none else some (u - 1)
  low _ := 1
  lim u := 3 - u

theorem exO_kids0 : exO.kids 0 = [1] := by decide
theorem exO_kids1 : exO.kids 1 = [2] := by decide
theorem exO_kids2 : exO.kids 2 = [] := by decide

theorem exO_valid : exO.Valid 0 where
  chain := by
    intro u
    fin_cases u
    · show Chain exO.low exO.lim 1 (exO.kids 0) 3
      rw [exO_kids0]; exact Chain.cons rfl (Chain.nil _)
    · show Chain exO.low exO.lim 1 (exO.kids 1) 2
      rw [exO_kids1]; exact Chain.cons rfl (Chain.nil _)
    · show Chain exO.low exO.lim 1 (exO.kids 2) 1
      rw [exO_kids2]; exact Chain.nil _
  low_le := by decide
  span := by
    intro w
    have h01 : (1 : Fin 3) ∈ exO.kids 0 := by rw [exO_kids0]; simp
    have h12 : (2 : Fin 3) ∈ exO.kids 1 := by rw [exO_kids1]; simp
    fin_cases w
    · exact InSub.refl _
    · exact InSub.kid h01 (InSub.refl _)
    · exact InSub.kid h01 (InSub.kid h12 (InSub.refl _))
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

theorem exO_against : exO.Against 0 2 := by
  -- c = 2 → 0 leaves a's child side {1, 2} for its parent side {0}
  have hb : exO.Conn 0 1 2 := Conn.single (t := 1) (by decide) (by decide) (Or.inl ⟨rfl, rfl⟩)
  exact ⟨hb, Conn.refl _⟩

/-- What `CutStore::update` leaves: `T − a + c`, the re-hung pointers, the old `lim`/`low`. -/
def tIn (u : Fin 3) : List (Fin 3) := if u = 0 then [2] else if u = 1 then [] else [1]
def tOut (u : Fin 3) : List (Fin 3) := if u = 0 then [] else [u]
def par' (u : Fin 3) : Option (Fin 3) := if u = 0 then none else some u

def exM : Net (Fin 3) (Fin 3) := exO.repivot 0 2 tIn tOut par' exO.low exO.lim

/-- The re-hung state is a spanning tree, by `pivot_step` from the valid old one. -/
theorem exM_tree : exM.Tree 0 :=
  (pivot_step (N := exO) (root := 0) (lowA := exO.low) (limA := exO.lim) exO_valid (e := 0) (f := 2)
    (by decide) (by decide) exO_against (treeIn' := tIn) (treeOut' := tOut) (par' := par')
    (by decide) (by decide) (by decide) (by decide) (by decide)
    (by
      intro q c hc hq hcu
      rw [exO_valid.inSub_iff] at hq hcu
      revert hc hq hcu
      fin_cases q <;> fin_cases c <;> decide)
    (by
      intro w hw
      rw [exO_valid.inSub_iff, exO_valid.inSub_iff] at hw
      revert hw
      fin_cases w <;> decide)).1

/-- ... whose stored numbering is stale: node 1 hangs below node 2 with a larger `lim`. -/
theorem exM_stale : ¬exM.Valid 0 := by
  intro h
  have hk : (1 : Fin 3) ∈ exM.kids 2 := by decide
  have := h.kid_lim hk
  exact absurd this (by decide)

/-- A valid numbering of the new tree: `0 ⊃ 2 ⊃ 1`. -/
def limV (u : Fin 3) : ℕ := if u = 0 then 3 else if u = 1 then 1 else 2

def exSlack (_ : Fin 3) : ℤ := 3

/-- The cut-set path at `b`, with the fallback's numbering `limV` if it were needed. -/
theorem exM_cut : exM.enterEdgeP exSlack 5 5 false [(0, true)] 1 (fun _ => 1) limV = some 0 := by
  decide

/-- C's search on a valid numbering agrees. -/
theorem exM_valid_c : (exM.renum (fun _ => 1) limV).cEnter exSlack 5 1 = some 0 := by decide

/-- C's search on the stale numbering does not. -/
theorem exM_stale_c : exM.cEnter exSlack 5 1 = none := by decide

end Example750

end GvpProofs.Ns.CutSets
