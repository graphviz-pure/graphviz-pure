import Mathlib

/-!
# `canreach` in `make_lr_constraints`: per-query DFS = cached reach set

C: `lib/dotgen/position.c:canreach` (`go` over `ND_out`, no memo).
Rust: `canreach` in `crates/graphviz-pure-layout/src/dotgen/position.rs`.

`make_lr_constraints` asks `canreach(u, v)` while it adds auxiliary edges, so the graph grows
between queries. A reach set cached at one moment stays exact if, before it answers, it takes in
whatever the new edges reach. This file proves two things.

1. `dfs_iff`: the stack-and-seen DFS (the shape of the per-query Rust, which is C's `go` with a
   visited set) answers `ReflTransGen`.
2. `resume_exact`: a cached set that was exact for an earlier edge set, once closed under the
   current edges, is exact for the current ones. So answering `v` as "is `v` in the cached set" is
   the DFS's answer, whatever edges were added in between and in whatever order.

Model, not extraction: vertices are an arbitrary `Fintype` with decidable equality, the graph is
`out : V → List V` (the heads of `ND_out`, in list order), edges are only ever appended.
-/

namespace GvpProofs.CanReach

open Relation

variable {V : Type*} [DecidableEq V]

/-- The edge relation of an out-list graph. -/
def Edge (out : V → List V) (x y : V) : Prop := y ∈ out x

/-- One run of the Rust DFS: pop, answer `true` on the target, skip a vertex seen before, else
mark it and push its heads. `true` iff the target is found. -/
def dfs [Fintype V] (out : V → List V) (v : V) : List V → Finset V → Bool
  | [], _ => false
  | x :: rest, seen =>
    if x = v then true
    else if x ∈ seen then dfs out v rest seen
    else dfs out v (out x ++ rest) (insert x seen)
termination_by stk seen => (Fintype.card V - seen.card, stk.length)
decreasing_by
  all_goals first
    | (apply Prod.Lex.right; simp)
    | (apply Prod.Lex.left
       have : seen.card < Fintype.card V := by
         apply Finset.card_lt_univ_of_notMem (by assumption)
       rw [Finset.card_insert_of_notMem (by assumption)]
       omega)

/-- `canreach u v`: the DFS from `[u]` with nothing seen. -/
def canreach [Fintype V] (out : V → List V) (u v : V) : Bool := dfs out v [u] ∅

/-- The DFS invariant: everything seen or on the stack is reachable from `u`, `u` is accounted
for, and nothing seen has an edge to something neither seen nor on the stack. -/
structure Inv (out : V → List V) (u v : V) (stk : List V) (seen : Finset V) : Prop where
  reach : ∀ x, x ∈ seen ∨ x ∈ stk → ReflTransGen (Edge out) u x
  start : u ∈ seen ∨ u ∈ stk
  closed : ∀ x ∈ seen, ∀ y, Edge out x y → y ∈ seen ∨ y ∈ stk
  notv : v ∉ seen

theorem dfs_spec [Fintype V] (out : V → List V) (u v : V) (stk : List V) (seen : Finset V)
    (h : Inv out u v stk seen) :
    dfs out v stk seen = true ↔ ReflTransGen (Edge out) u v := by
  induction stk, seen using dfs.induct out v with
  | case1 seen =>
    -- empty stack: `seen` is closed, holds `u`, and misses `v`
    rw [dfs]
    simp only [Bool.false_eq_true, false_iff]
    intro hr
    have hc : ∀ y, ReflTransGen (Edge out) u y → y ∈ seen := by
      intro y hy
      induction hy with
      | refl => simpa using h.start
      | tail _ hxy ih =>
        rcases h.closed _ ih _ hxy with h1 | h1
        · exact h1
        · simp at h1
    exact h.notv (hc v hr)
  | case2 rest seen =>
    -- the target is popped
    rw [dfs]
    simp only [ite_true, true_iff]
    exact h.reach v (Or.inr (by simp))
  | case3 x rest seen hx hs ih =>
    -- already seen: drop it
    rw [dfs]
    simp only [hx, hs, ite_true, ite_false]
    refine ih ?_
    refine ⟨fun y hy => h.reach y ?_, ?_, ?_, h.notv⟩
    · rcases hy with hy | hy
      · exact Or.inl hy
      · exact Or.inr (List.mem_cons_of_mem _ hy)
    · rcases h.start with h1 | h1
      · exact Or.inl h1
      · rcases List.mem_cons.1 h1 with rfl | h2
        · exact Or.inl hs
        · exact Or.inr h2
    · intro a ha b hab
      rcases h.closed a ha b hab with h1 | h1
      · exact Or.inl h1
      · rcases List.mem_cons.1 h1 with rfl | h2
        · exact Or.inl hs
        · exact Or.inr h2
  | case4 x rest seen hx hs ih =>
    -- a new vertex: mark it, push its heads
    rw [dfs]
    simp only [hx, hs, ite_false]
    refine ih ?_
    have hxr : ReflTransGen (Edge out) u x := h.reach x (Or.inr (by simp))
    refine ⟨fun y hy => ?_, ?_, ?_, ?_⟩
    · rcases hy with hy | hy
      · rcases Finset.mem_insert.1 hy with rfl | hy
        · exact hxr
        · exact h.reach y (Or.inl hy)
      · rcases List.mem_append.1 hy with hy | hy
        · exact hxr.tail hy
        · exact h.reach y (Or.inr (List.mem_cons_of_mem _ hy))
    · rcases h.start with h1 | h1
      · exact Or.inl (Finset.mem_insert_of_mem h1)
      · rcases List.mem_cons.1 h1 with rfl | h2
        · exact Or.inl (Finset.mem_insert_self _ _)
        · exact Or.inr (List.mem_append_right _ h2)
    · intro a ha b hab
      rcases Finset.mem_insert.1 ha with rfl | ha
      · exact Or.inr (List.mem_append_left _ hab)
      · rcases h.closed a ha b hab with h1 | h1
        · exact Or.inl (Finset.mem_insert_of_mem h1)
        · rcases List.mem_cons.1 h1 with rfl | h2
          · exact Or.inl (Finset.mem_insert_self _ _)
          · exact Or.inr (List.mem_append_right _ h2)
    · intro hv
      rcases Finset.mem_insert.1 hv with rfl | hv
      · exact hx rfl
      · exact h.notv hv

/-- The per-query DFS answers reachability. -/
theorem dfs_iff [Fintype V] (out : V → List V) (u v : V) :
    canreach out u v = true ↔ ReflTransGen (Edge out) u v := by
  refine dfs_spec out u v [u] ∅ ⟨?_, Or.inr (by simp), ?_, by simp⟩
  · intro x hx
    rcases hx with hx | hx
    · simp at hx
    · simp at hx
      subst hx
      exact ReflTransGen.refl
  · intro x hx
    simp at hx

/-! ## The cache -/

/-- Edges are only ever appended: every out-list of `out'` extends the one of `out`. -/
def Extends (out out' : V → List V) : Prop := ∀ x, out x <+: out' x

omit [DecidableEq V] in
theorem Extends.edge {out out' : V → List V} (h : Extends out out') {x y : V}
    (hxy : Edge out x y) : Edge out' x y :=
  (h x).subset hxy

omit [DecidableEq V] in
theorem Extends.reach {out out' : V → List V} (h : Extends out out') {x y : V}
    (hxy : ReflTransGen (Edge out) x y) : ReflTransGen (Edge out') x y :=
  by
  induction hxy with
  | refl => exact ReflTransGen.refl
  | tail _ e ih => exact ih.tail (h.edge e)

/-- `S` is exactly what `u` reaches over `out`. -/
def Exact (out : V → List V) (u : V) (S : Set V) : Prop :=
  ∀ v, v ∈ S ↔ ReflTransGen (Edge out) u v

/-- Close `S` under the current edges: everything reachable from a member. This is what the
Rust cache does before it answers: it scans the out-edges that were appended since it last
looked, starting from the members, and follows them into new vertices. -/
def resume (out : V → List V) (S : Set V) : Set V := {v | ∃ s ∈ S, ReflTransGen (Edge out) s v}

omit [DecidableEq V] in
/-- A cached set that was exact for fewer edges, closed under the edges there are now, is exact
for them: the cache answers what a fresh DFS from `u` answers. -/
theorem resume_exact {out out' : V → List V} {u : V} {S : Set V}
    (hS : Exact out u S) (hext : Extends out out') : Exact out' u (resume out' S) := by
  intro v
  constructor
  · rintro ⟨s, hs, hsv⟩
    exact ((hext.reach ((hS s).1 hs))).trans hsv
  · intro huv
    exact ⟨u, (hS u).2 ReflTransGen.refl, huv⟩

omit [DecidableEq V] in
/-- And a cached set that nothing new touches needs no work: if no vertex of `S` gained an edge,
`S` is already closed, so the cached answer stands. -/
theorem resume_eq_of_untouched {out out' : V → List V} {u : V} {S : Set V}
    (hS : Exact out u S) (hsame : ∀ x ∈ S, out' x = out x) : resume out' S = S := by
  ext v
  constructor
  · rintro ⟨s, hs, hsv⟩
    induction hsv with
    | refl => exact hs
    | @tail b c _ hxy ih =>
      have hb : b ∈ S := ih
      have hbc : Edge out b c := by
        simpa [Edge, hsame b hb] using hxy
      have hub := (hS b).1 hb
      exact (hS c).2 (hub.tail hbc)
  · intro hv
    exact ⟨v, hv, ReflTransGen.refl⟩

end GvpProofs.CanReach
