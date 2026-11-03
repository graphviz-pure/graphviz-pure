import Mathlib

/-!
# Triangle adjacency: the all-pairs scan of `connecttris` = pairing by undirected edge

C: `lib/pathplan/shortest.c:connecttris`. Rust: `triangulate_c` in
`crates/graphviz-pure-geom/src/pathplan/shortest.rs` (the oracle, kept under `cfg(test)` and
`pathplan-check`), against which `clip::triangulate` is tested.

C compares every pair of triangles `i < j` and every pair of their edges, and when the two edges
are the same undirected edge it sets `right_index` on both. A later match overwrites an earlier
one, so the final value of a slot is its LAST write.

This file proves that, when no undirected edge sits on more than two edge slots (true for every
simple polygon: a diagonal has two sides, a polygon side has one), the last write is the same
as pairing by an edge-keyed map: bucket the slots by `key = (min, max)` of the two vertex
indices, and link the two slots of a bucket that holds exactly two, in different triangles.

Model, not extraction: `Tris.edge i e` is `(pnl0, pnl1)` of `tris[i].e[e]`; the vertex indices
are naturals; `NONE` is `Option.none`. The overwriting loop is the explicit list `writes`.
-/

namespace GvpProofs.TriAdjacency

/-- Edge `e` (0..2) of triangle `i`. -/
abbrev Slot := ℕ × Fin 3

/-- A triangle list: the number of triangles and, per slot, `(pnl0, pnl1)`. -/
structure Tris where
  n : ℕ
  edge : Slot → ℕ × ℕ

/-- C's test: `a.pnl0 == b.pnl0 && a.pnl1 == b.pnl1 || a.pnl0 == b.pnl1 && a.pnl1 == b.pnl0`. -/
def same (a b : ℕ × ℕ) : Bool :=
  (a.1 == b.1 && a.2 == b.2) || (a.1 == b.2 && a.2 == b.1)

/-- The edge-keyed map's key: the undirected edge, as an ordered vertex pair. -/
def key (a : ℕ × ℕ) : ℕ × ℕ := (min a.1 a.2, max a.1 a.2)

theorem same_iff_key (a b : ℕ × ℕ) : same a b = true ↔ key a = key b := by
  obtain ⟨a1, a2⟩ := a
  obtain ⟨b1, b2⟩ := b
  simp only [same, key, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq, Prod.mk.injEq]
  omega

/-- The writes C's four nested loops make, in order: `(slot, value of right_index)`. -/
def writes (T : Tris) : List (Slot × ℕ) :=
  (List.range T.n).flatMap fun i =>
    ((List.range T.n).filter (i < ·)).flatMap fun j =>
      (List.finRange 3).flatMap fun ei =>
        (List.finRange 3).flatMap fun ej =>
          if same (T.edge (i, ei)) (T.edge (j, ej)) then [((i, ei), j), ((j, ej), i)] else []

/-- `right_index` of a slot after the loops: its last write, if any. -/
def cRight (T : Tris) (s : Slot) : Option ℕ :=
  ((writes T).reverse.find? fun w => decide (w.1 = s)).map (·.2)

/-- Every slot of every triangle, in index order. -/
def slots (T : Tris) : List Slot :=
  (List.range T.n).flatMap fun i => (List.finRange 3).map fun e => (i, e)

/-- The map's bucket for the edge of `s`, without `s`: the other slots on the same undirected edge. -/
def others (T : Tris) (s : Slot) : List Slot :=
  (slots T).filter fun s' => decide (s' ≠ s) && decide (key (T.edge s') = key (T.edge s))

/-- Pairing by the edge-keyed map: a slot links to the triangle of the one other slot on its
edge, when that slot is in a different triangle. -/
def mapRight (T : Tris) (s : Slot) : Option ℕ :=
  match others T s with
  | [s'] => if s'.1 ≠ s.1 then some s'.1 else none
  | _ => none

/-- No undirected edge sits on more than two slots. -/
def AtMostTwo (T : Tris) : Prop := ∀ s ∈ slots T, (others T s).length ≤ 1

theorem mem_slots (T : Tris) (s : Slot) : s ∈ slots T ↔ s.1 < T.n := by
  obtain ⟨i, e⟩ := s
  simp [slots]

/-- What the loops write: `(s, t)` is written exactly when some edge of triangle `t ≠ s.1` is the
same undirected edge as `s`'s. -/
theorem mem_writes (T : Tris) (s : Slot) (t : ℕ) :
    (s, t) ∈ writes T ↔
      s.1 < T.n ∧ t < T.n ∧ s.1 ≠ t ∧
        ∃ e : Fin 3, key (T.edge s) = key (T.edge (t, e)) := by
  obtain ⟨i, ei⟩ := s
  simp only [writes, List.mem_flatMap, List.mem_range, List.mem_filter, List.mem_finRange,
    true_and, decide_eq_true_eq]
  constructor
  · rintro ⟨i', hi', j, ⟨hj, hij⟩, e1, e2, hm⟩
    split_ifs at hm with h
    · simp only [List.mem_cons, List.not_mem_nil, or_false, Prod.mk.injEq] at hm
      rcases hm with ⟨⟨h1, h2⟩, h3⟩ | ⟨⟨h1, h2⟩, h3⟩
      · subst h1 h2 h3
        exact ⟨hi', hj, Nat.ne_of_lt hij, e2, (same_iff_key _ _).1 h⟩
      · subst h1 h2 h3
        exact ⟨hj, hi', Nat.ne_of_gt hij, e1, ((same_iff_key _ _).1 h).symm⟩
    · simp at hm
  · rintro ⟨hi, ht, hne, e2, hk⟩
    rcases Nat.lt_or_gt_of_ne hne with hlt | hgt
    · refine ⟨i, hi, t, ⟨ht, hlt⟩, ei, e2, ?_⟩
      simp [(same_iff_key _ _).2 hk]
    · refine ⟨t, ht, i, ⟨hi, hgt⟩, e2, ei, ?_⟩
      simp [(same_iff_key _ _).2 hk.symm]

/-- The edge-keyed pairing writes what C's last write leaves, whenever an edge has at most two
slots. -/
theorem cRight_eq_mapRight (T : Tris) (hT : AtMostTwo T) (s : Slot) (hs : s ∈ slots T) :
    cRight T s = mapRight T s := by
  have hlen := hT s hs
  -- membership in `others`
  have mem_others : ∀ s', s' ∈ others T s ↔
      s'.1 < T.n ∧ s' ≠ s ∧ key (T.edge s') = key (T.edge s) := by
    intro s'
    simp [others, mem_slots]
  have wfst : ∀ w ∈ writes T, w.1 = s → (s, w.2) ∈ writes T := by
    rintro ⟨a, b⟩ hw hws
    simp only at hws
    subst hws
    exact hw
  -- a write `(s, t)` means triangle `t` has a slot in `others`
  have write_other : ∀ t, (s, t) ∈ writes T → ∃ s' ∈ others T s, s'.1 = t := by
    intro t ht
    obtain ⟨_, htn, hne, e, hk⟩ := (mem_writes T s t).1 ht
    refine ⟨(t, e), (mem_others _).2 ⟨htn, ?_, hk.symm⟩, rfl⟩
    intro h
    exact hne (congrArg Prod.fst h).symm
  have other_write : ∀ s' ∈ others T s, s'.1 ≠ s.1 → (s, s'.1) ∈ writes T := by
    intro s' hs' hne
    obtain ⟨h1, h2, h3⟩ := (mem_others s').1 hs'
    exact (mem_writes T s s'.1).2 ⟨(mem_slots T s).1 hs, h1, fun h => hne h.symm, s'.2, h3.symm⟩
  unfold cRight mapRight
  match h : others T s, hlen with
  | [], _ =>
    have : ∀ w ∈ (writes T).reverse, ¬ (w.1 = s) := by
      intro w hw hws
      rw [List.mem_reverse] at hw
      obtain ⟨_, hmem, _⟩ := write_other w.2 (wfst w hw hws)
      rw [h] at hmem
      simp at hmem
    rw [List.find?_eq_none.2 (by simpa using this)]
    rfl
  | [s'], _ =>
    by_cases hne : s'.1 ≠ s.1
    · show _ = (if s'.1 ≠ s.1 then some s'.1 else none)
      have hw : (s, s'.1) ∈ writes T := other_write s' (by rw [h]; simp) hne
      have hall : ∀ w ∈ (writes T).reverse, w.1 = s → w.2 = s'.1 := by
        intro w hw hws
        rw [List.mem_reverse] at hw
        obtain ⟨s'', hmem, heq⟩ := write_other w.2 (wfst w hw hws)
        rw [h] at hmem
        simp only [List.mem_singleton] at hmem
        rw [← heq, hmem]
      have hex : ∃ w ∈ (writes T).reverse, w.1 = s :=
        ⟨(s, s'.1), List.mem_reverse.2 hw, rfl⟩
      obtain ⟨w0, hw0, hw0s⟩ := hex
      have hsome : ((writes T).reverse.find? fun w => decide (w.1 = s)).isSome := by
        rw [List.find?_isSome]
        exact ⟨w0, hw0, by simpa using hw0s⟩
      obtain ⟨w, hfind⟩ := Option.isSome_iff_exists.1 hsome
      have hwmem := List.mem_of_find?_eq_some hfind
      have hws := List.find?_some hfind
      simp only [decide_eq_true_eq] at hws
      rw [hfind]
      simp [hall w hwmem hws, hne]
    · show _ = (if s'.1 ≠ s.1 then some s'.1 else none)
      have hne' : s'.1 = s.1 := not_not.1 hne
      simp only [hne', ne_eq, not_true_eq_false, ↓reduceIte]
      have : ∀ w ∈ (writes T).reverse, ¬ (w.1 = s) := by
        intro w hw hws
        rw [List.mem_reverse] at hw
        obtain ⟨s'', hmem, heq⟩ := write_other w.2 (wfst w hw hws)
        rw [h] at hmem
        simp only [List.mem_singleton] at hmem
        rw [hmem] at heq
        have h1 := (mem_writes T s w.2).1 (wfst w hw hws)
        have h2 : s'.1 = s.1 := not_not.1 hne
        omega
      rw [List.find?_eq_none.2 (by simpa using this)]
      rfl
  | _ :: _ :: _, hl => simp at hl

/-- Corollary: with at most two slots per edge, two triangles are linked exactly when they share
an edge (the relation the funnel walk reads), and the map finds every such link. -/
theorem linked_iff_share_edge (T : Tris) (hT : AtMostTwo T) (s : Slot) (hs : s ∈ slots T)
    (t : ℕ) (ht : mapRight T s = some t) :
    t < T.n ∧ t ≠ s.1 ∧ ∃ e : Fin 3, key (T.edge s) = key (T.edge (t, e)) := by
  rw [← cRight_eq_mapRight T hT s hs] at ht
  unfold cRight at ht
  obtain ⟨w, hw, rfl⟩ := Option.map_eq_some_iff.1 ht
  have hmem := List.mem_reverse.1 (List.mem_of_find?_eq_some hw)
  have hws := List.find?_some hw
  simp only [decide_eq_true_eq] at hws
  subst hws
  obtain ⟨_, h2, h3, h4⟩ := (mem_writes T w.1 w.2).1 hmem
  exact ⟨h2, fun h => h3 h.symm, h4⟩

/-! ## The premise is needed

Three triangles on the undirected edge `{0, 2}`: `(0,1,2)`, `(0,2,3)`, `(0,2,4)` as `make_tri`
makes them. The all-pairs loop leaves asymmetric links (the last pair written wins) where the map
links none, so `AtMostTwo` cannot be dropped from `cRight_eq_mapRight`. -/

/-- `(pnl0, pnl1)` of the three edges of `(a, b, c)`, as `make_tri` sets them. -/
def triEdges (a b c : ℕ) : Fin 3 → ℕ × ℕ := ![(a, b), (b, c), (c, a)]

/-- Triangles `(0,1,2)`, `(0,2,3)`, `(0,2,4)`. -/
def fan : Tris where
  n := 3
  edge s := match s.1 with
    | 0 => triEdges 0 1 2 s.2
    | 1 => triEdges 0 2 3 s.2
    | _ => triEdges 0 2 4 s.2

example : (cRight fan (0, 2), cRight fan (1, 0), cRight fan (2, 0)) =
    (some 2, some 2, some 1) := by decide

example : (mapRight fan (0, 2), mapRight fan (1, 0), mapRight fan (2, 0)) =
    (none, none, none) := by decide

end GvpProofs.TriAdjacency
