import Mathlib.Combinatorics.SimpleGraph.Metric
import Mathlib.Algebra.BigOperators.Group.Finset.Basic
import Mathlib.Algebra.Order.BigOperators.Group.Finset

/-!
# twopi: multi-source BFS and one reverse sweep equal the per-leaf relaxation and the per-leaf walks

Proof for `perf/729-twopi-bfs`, `crates/graphviz-pure-layout/src/twopigen/circle.rs`. Complete: no unproved goals.

C's `circle.c` runs `setNStepsToLeaf` from every leaf in turn (a depth-first relaxation, O(n·depth)) and
`setSubtreeSize` as a walk to the root from every childless node (O(leaves·depth)). The port now does a
multi-source breadth-first search and one pass over the BFS discovery order backwards. Same integers:

* `Lists.bfs_nearest`, `Lists.relaxation_nearest`: on a connected component with a leaf, both the queue
  loop and the old relaxation label every node with the number of edges to its nearest leaf, in
  Mathlib's `SimpleGraph.dist`. `Lists.bfs_eq_relaxation` is the equality. The queue loop is proved
  for *any* pop order, so it does not rely on FIFO. `Lists.exists_terminal`: it terminates.
  `Lists.noleaf`: with no leaf (a cycle) both leave every label at `inf`.
* `sweep_eq_walks`: given the discovery order of `set_n_steps_to_center` (each parent before its
  children), the reverse sweep and the walk from every childless node give every node the same size.

## Model vs Rust

The theorems are over a Lean model, not Aeneas-extracted Rust (the extraction toolchain was not
installed when this was written). Differences, all of them:

1. Nodes are a finite type `V`; Rust uses `usize` indices into `RData::d`. Labels are `ℕ`, Rust uses
   `u64`; `inf = nnodes²` and `+ 1` cannot overflow `u64` at any size that fits in memory.
2. The component is the adjacency lists `Lists.nbrs`: `other(g, e, n)` for `e` in `g.all_edges(sg, n)`,
   in order. Edges outside the component are already filtered out. The model assumes the lists are
   symmetric (`Lists.symm`), which holds because every edge is listed at both of its ends. The
   component is assumed connected (`ccomps` guarantees it) and `inf ≥ |V|`.
3. `Lists.IsLeaf` is `is_leaf`: all neighbours other than the node itself are one and the same node.
4. The Rust BFS keeps `order` and a `head` index; the model keeps the unread suffix as a list (the
   same thing). The model pushes a node every time its label drops, which the Rust does too.
5. The old relaxation is the *recursion* C writes (`Lists.go`), with a fuel parameter in place of its
   termination argument; the Rust before #729 was an iterative version of it with an explicit stack,
   checked against the recursion by the test `iterative_relaxation_matches_recursive_calls`. The old
   `n_steps_to_leaf == 0` source test is `relaxAll`.
6. For sizes, `parent`, `nc` (= `n_children`) and the discovery order `order` are inputs. The theorem
   assumes `order` is a permutation of all nodes in which a parent comes before its children
   (`hpar`). That is immediate from `set_n_steps_to_center` (a node is queued only while its parent
   is being expanded) and is asserted for every random case in
   `bfs_and_postorder_match_the_oracles_on_random_graphs`; it is not itself proved from the Rust.
   The old walk has a fuel too (the order length: every chain is shorter than that).
7. `subtree_size` starts at 0 for every node (each node is in exactly one component and laid out once).
-/

open Function

namespace GvpProofs.Twopi

variable {V : Type}

/-- The component as `leaf_adjacency` / `g.all_edges(sg, n)` present it: for every node the list of
the other ends of its edges, in order, with repeats (parallel edges) and with the node itself for
a loop. Every edge is listed at both ends. -/
structure Lists (V : Type) where
  nbrs : V → List V
  symm : ∀ u v, v ∈ nbrs u → u ∈ nbrs v

variable (L : Lists V)

namespace Lists

/-- The simple graph underneath: loops and multiplicities forgotten. -/
def graph : SimpleGraph V := SimpleGraph.fromRel fun u v => v ∈ L.nbrs u

theorem adj_iff {u v : V} : L.graph.Adj u v ↔ u ≠ v ∧ v ∈ L.nbrs u := by
  constructor
  · intro h
    rcases (SimpleGraph.fromRel_adj _ _ _).1 h with ⟨hne, h | h⟩
    · exact ⟨hne, h⟩
    · exact ⟨hne, L.symm _ _ h⟩
  · rintro ⟨hne, h⟩
    exact (SimpleGraph.fromRel_adj _ _ _).2 ⟨hne, Or.inl h⟩

/-- `is_leaf`: every neighbour other than the node itself is the same node. -/
def IsLeaf (u : V) : Prop :=
  ∀ v ∈ L.nbrs u, ∀ w ∈ L.nbrs u, v ≠ u → w ≠ u → v = w

open Classical in
/-- `init_layout`: `0` at a leaf, `inf = nnodes²` elsewhere. -/
noncomputable def init (inf : ℕ) : V → ℕ := fun u => if L.IsLeaf u then 0 else inf

/-- `d` is the number of edges from `u` to the nearest leaf. -/
def Nearest (u : V) (d : ℕ) : Prop :=
  (∃ l, L.IsLeaf l ∧ L.graph.dist l u = d) ∧ ∀ l, L.IsLeaf l → d ≤ L.graph.dist l u

/-- Along a walk the values may grow by one per edge at most. -/
theorem le_of_walk {val : V → ℕ} (hlip : ∀ x y, y ∈ L.nbrs x → val y ≤ val x + 1)
    {u l : V} (p : L.graph.Walk u l) : val u ≤ val l + p.length := by
  induction p with
  | nil => simp
  | cons h p ih =>
    have := hlip _ _ (L.symm _ _ ((L.adj_iff.1 h).2))
    simp only [SimpleGraph.Walk.length_cons]
    omega

/-- What a labelling needs for its values to be the distances to the nearest leaf. -/
theorem nearest_of_closed [Fintype V] (hconn : L.graph.Connected) {inf : ℕ} (hinf : Fintype.card V ≤ inf)
    {val : V → ℕ} (hleaf : ∀ u, L.IsLeaf u → val u = 0)
    (hlip : ∀ x y, y ∈ L.nbrs x → val y ≤ val x + 1)
    (hsound : ∀ u, val u < inf → ∃ l, L.IsLeaf l ∧ ∃ p : L.graph.Walk l u, p.length ≤ val u)
    (u : V) {l₀ : V} (h₀ : L.IsLeaf l₀) : L.Nearest u (val u) := by
  have upper : ∀ l, L.IsLeaf l → val u ≤ L.graph.dist l u := by
    intro l hl
    obtain ⟨p, -, hp⟩ := hconn.exists_path_of_dist u l
    have := L.le_of_walk hlip p
    rw [hleaf l hl, hp, SimpleGraph.dist_comm] at this
    simpa using this
  have hlt : val u < inf := by
    obtain ⟨p, hpath, hp⟩ := hconn.exists_path_of_dist u l₀
    have h1 := L.le_of_walk hlip p
    have h2 := hpath.length_lt
    rw [hleaf l₀ h₀] at h1
    omega
  obtain ⟨l, hl, p, hp⟩ := hsound u hlt
  refine ⟨⟨l, hl, le_antisymm ((SimpleGraph.dist_le p).trans hp) (upper l hl)⟩, upper⟩


/-- The labels `d` of two algorithms agree once both are the nearest-leaf distance. -/
theorem Nearest.unique {u : V} {a b : ℕ} (ha : L.Nearest u a) (hb : L.Nearest u b) : a = b := by
  obtain ⟨⟨l, hl, hla⟩, ha'⟩ := ha
  obtain ⟨⟨m, hm, hmb⟩, hb'⟩ := hb
  have h1 := ha' m hm
  have h2 := hb' l hl
  omega

end Lists

/-! ### The new path: multi-source breadth-first search

State: the labels `val` (`n_steps_to_leaf`) and the queue (`order`) with the part before `head`
dropped. One step pops `n`, reads `ns = val n + 1` once and visits `n`'s neighbours in order.
-/

section BFS
variable [DecidableEq V]

/-- `(val, queue)`. -/
abbrev St (V : Type) := (V → ℕ) × List V

/-- One neighbour of the inner loop of `set_n_steps_to_leaf`. -/
def relax (ns : ℕ) (st : St V) (w : V) : St V :=
  if ns < st.1 w then (update st.1 w ns, st.2 ++ [w]) else st

theorem sum_update_lt [Fintype V] (val : V → ℕ) (w : V) (ns : ℕ) (h : ns < val w) :
    ∑ u, update val w ns u < ∑ u, val u := by
  apply Finset.sum_lt_sum
  · intro u _
    by_cases hu : u = w
    · subst hu; simp [h.le]
    · simp [hu]
  · exact ⟨w, Finset.mem_univ w, by simpa using h⟩

/-- What the inner loop over a neighbour list does, independent of the graph. -/
theorem fold_relax [Fintype V] (ns : ℕ) (ws : List V) :
    ∀ st : St V,
    let st' := ws.foldl (relax ns) st
    (∀ u, st'.1 u ≤ st.1 u) ∧
    (∀ u, st'.1 u ≠ st.1 u → ns < st.1 u ∧ st'.1 u = ns ∧ u ∈ ws ∧ u ∈ st'.2) ∧
    (∀ x ∈ st.2, x ∈ st'.2) ∧
    (∀ w ∈ ws, st'.1 w ≤ ns) ∧
    (∑ u, st'.1 u + st'.2.length ≤ ∑ u, st.1 u + st.2.length) := by
  induction ws with
  | nil => intro st; simp
  | cons w ws ih =>
    intro st
    obtain ⟨a1, b1, c1, d1, e1⟩ := ih (relax ns st w)
    simp only [List.foldl_cons]
    by_cases hc : ns < st.1 w
    · have hr : relax ns st w = (update st.1 w ns, st.2 ++ [w]) := by simp [relax, hc]
      rw [hr] at a1 b1 c1 d1 e1 ⊢
      simp only at a1 b1 c1 d1 e1 ⊢
      refine ⟨?_, ?_, ?_, ?_, ?_⟩
      · intro u
        refine (a1 u).trans ?_
        by_cases hu : u = w
        · subst hu; simp [hc.le]
        · simp [hu]
      · intro u hu
        by_cases huw : u = w
        · subst huw
          have h1 : (ws.foldl (relax ns) (update st.1 u ns, st.2 ++ [u])).1 u = ns := by
            by_contra hne
            have := b1 u (by simpa using hne)
            simp at this
          refine ⟨hc, h1, by simp, c1 u (by simp)⟩
        · by_cases hsame : (ws.foldl (relax ns) (update st.1 w ns, st.2 ++ [w])).1 u
              = (update st.1 w ns u)
          · exact absurd (by simpa [huw] using hsame) hu
          · obtain ⟨x1, x2, x3, x4⟩ := b1 u hsame
            simp [huw] at x1
            exact ⟨by simpa [huw] using x1, x2, by simp [x3], x4⟩
      · intro x hx
        exact c1 x (by simp [hx])
      · intro y hy
        rcases List.mem_cons.1 hy with rfl | hy
        · refine (a1 y).trans ?_
          simp
        · exact d1 y hy
      · have := sum_update_lt st.1 w ns hc
        simp at e1 ⊢
        omega
    · have hr : relax ns st w = st := by simp [relax, hc]
      rw [hr] at a1 b1 c1 d1 e1 ⊢
      refine ⟨a1, ?_, c1, ?_, e1⟩
      · intro u hu
        obtain ⟨x1, x2, x3, x4⟩ := b1 u hu
        exact ⟨x1, x2, by simp [x3], x4⟩
      · intro y hy
        rcases List.mem_cons.1 hy with rfl | hy
        · refine (a1 y).trans ?_
          omega
        · exact d1 y hy


namespace Lists

/-- One iteration of the `while head < queue.len()` loop of `set_n_steps_to_leaf`. -/
inductive Step : St V → St V → Prop
  | pop (val : V → ℕ) (n : V) (q : List V) :
      Step (val, n :: q) ((L.nbrs n).foldl (relax (val n + 1)) (val, q))

/-- The state after `init_layout` and the source loop: leaves have label `0` and are queued. -/
noncomputable def initSt (nodes : List V) (inf : ℕ) : St V :=
  (L.init inf, nodes.filter fun u => L.init inf u = 0)

/-- Everything the loop keeps true. -/
structure Inv [Fintype V] (inf : ℕ) (s : St V) : Prop where
  leaf : ∀ u, L.IsLeaf u → s.1 u = 0
  bound : ∀ u, s.1 u ≤ inf
  sound : ∀ u, s.1 u < inf → ∃ l, L.IsLeaf l ∧ ∃ p : L.graph.Walk l u, p.length ≤ s.1 u
  /-- A node that left the queue has had all its neighbours brought within one of it. -/
  closed : ∀ m, m ∉ s.2 → ∀ w ∈ L.nbrs m, s.1 w ≤ s.1 m + 1

theorem Step.inv [Fintype V] {inf : ℕ} {s t : St V} (hs : L.Inv inf s) (h : Step L s t) :
    L.Inv inf t := by
  cases h with
  | pop val n q =>
    obtain ⟨hl, hb, hso, hcl⟩ := hs
    dsimp only at hl hb hso hcl
    obtain ⟨a, b, c, d, e⟩ := fold_relax (val n + 1) (L.nbrs n) (val, q)
    set t := (L.nbrs n).foldl (relax (val n + 1)) (val, q) with ht
    dsimp only at a b c d e
    refine ⟨?_, ?_, ?_, ?_⟩
    · intro u hu
      have := a u
      have := hl u hu
      omega
    · intro u
      exact (a u).trans (hb u)
    · intro u hu
      by_cases hsame : t.1 u = val u
      · rw [hsame] at hu ⊢
        exact hso u hu
      · obtain ⟨h1, h2, h3, h4⟩ := b u hsame
        have hub : val u ≤ inf := hb u
        obtain ⟨l, hl', p, hp⟩ := hso n (by omega)
        have hadj : L.graph.Adj n u := L.adj_iff.2 ⟨by rintro rfl; omega, h3⟩
        exact ⟨l, hl', p.concat hadj, by simp only [SimpleGraph.Walk.length_concat]; omega⟩
    · intro m hm w hw
      have hmq : m ∉ q := fun hq => hm (c m hq)
      by_cases hmn : m = n
      · subst hmn
        have hnn : t.1 m = val m := by
          by_contra hne
          have := (b m hne).1
          omega
        have := d w hw
        omega
      · have hmt : t.1 m = val m := by
          by_contra hne
          exact hm (b m hne).2.2.2
        have := hcl m (by simp [hmq, hmn]) w hw
        have := a w
        omega

omit [DecidableEq V] in
theorem Inv.init [Fintype V] {inf : ℕ} (nodes : List V) (hn : ∀ v, v ∈ nodes) :
    L.Inv inf (L.initSt nodes inf) where
  leaf u hu := by simp [initSt, Lists.init, hu]
  bound u := by simp only [initSt, Lists.init]; split_ifs <;> omega
  sound u hu := by
    by_cases h : L.IsLeaf u
    · exact ⟨u, h, SimpleGraph.Walk.nil, by simp⟩
    · simp [initSt, Lists.init, h] at hu
  closed m hm w _ := by
    have hm' : L.init inf m ≠ 0 := by
      intro h0
      exact hm (by simp [initSt, List.mem_filter, hn m, h0])
    have h1 : L.init inf m = inf := by
      simp only [Lists.init] at hm' ⊢
      split_ifs at hm' ⊢ <;> simp_all
    have h2 : L.init inf w ≤ inf := by
      simp only [Lists.init]; split_ifs <;> omega
    simp only [initSt]
    omega

/-- The loop makes progress: labels plus queue length shrink. -/
theorem Step.measure [Fintype V] {s t : St V} (h : Step L s t) :
    ∑ u, t.1 u + t.2.length < ∑ u, s.1 u + s.2.length := by
  cases h with
  | pop val n q =>
    have := (fold_relax (val n + 1) (L.nbrs n) (val, q)).2.2.2.2
    simp only [List.length_cons] at *
    omega

/-- The loop terminates: from any state some run reaches an empty queue. -/
theorem exists_terminal [Fintype V] (s : St V) :
    ∃ t, Relation.ReflTransGen (Step L) s t ∧ t.2 = [] := by
  suffices H : ∀ m, ∀ s : St V, ∑ u, s.1 u + s.2.length = m →
      ∃ t, Relation.ReflTransGen (Step L) s t ∧ t.2 = [] from H _ s rfl
  intro m
  induction m using Nat.strong_induction_on with
  | _ m ih =>
    intro s hm
    rcases s with ⟨val, q⟩
    cases q with
    | nil => exact ⟨_, Relation.ReflTransGen.refl, rfl⟩
    | cons n q =>
      have hstep := Step.pop (L := L) val n q
      obtain ⟨t, ht, hq⟩ := ih _ (hm ▸ hstep.measure L) _ rfl
      exact ⟨t, Relation.ReflTransGen.head hstep ht, hq⟩

theorem Inv.run [Fintype V] {inf : ℕ} {s t : St V} (hs : L.Inv inf s)
    (h : Relation.ReflTransGen (Step L) s t) : L.Inv inf t := by
  induction h with
  | refl => exact hs
  | tail _ hst ih => exact hst.inv L ih

/-- **New path.** Any run of the queue loop that ends with an empty queue labels every node with its
distance to the nearest leaf. -/
theorem bfs_nearest [Fintype V] (hconn : L.graph.Connected) {inf : ℕ}
    (hinf : Fintype.card V ≤ inf) (nodes : List V) (hn : ∀ v, v ∈ nodes)
    {l₀ : V} (h₀ : L.IsLeaf l₀) {t : St V}
    (h : Relation.ReflTransGen (Step L) (L.initSt nodes inf) t) (hq : t.2 = []) (u : V) :
    L.Nearest u (t.1 u) := by
  have hi := (Inv.init L (inf := inf) nodes hn).run L h
  exact L.nearest_of_closed hconn hinf hi.leaf (fun x y hy => hi.closed x (by simp [hq]) y hy)
    hi.sound u h₀

end Lists

end BFS


/-! ### The old path: the C-shaped relaxation

`set_n_steps_to_leaf(r, n)` of the shipped code before #729, as the recursion C writes: from every leaf in turn,
visit the neighbours of `n` except the node we came from, and descend into each one whose label drops.
`ns` is read on entry and passed down, exactly as C's `nsteps` (it is not re-read after a recursive call
lowers `val n`). The recursion terminates because every call lowers a label; the fuel `k` stands in for
that argument, and a theorem below fixes how much is enough.
-/

section Old
variable [DecidableEq V]

namespace Lists

/-- The body of the `for ep in edges` loop for neighbour `w`. -/
def goStep (rec : V → Option V → ℕ → (V → ℕ) → (V → ℕ)) (n : V) (prev : Option V)
    (ns : ℕ) (val : V → ℕ) (w : V) : V → ℕ :=
  if prev = some w then val
  else if ns < val w then rec w (some n) (ns + 1) (update val w ns)
  else val

/-- `setNStepsToLeaf(g, n, prev)` with entry label `ns = val n + 1`, fuel `k`. -/
def go : ℕ → V → Option V → ℕ → (V → ℕ) → (V → ℕ)
  | 0, _, _, _, val => val
  | k + 1, n, prev, ns, val => (L.nbrs n).foldl (goStep (go k) n prev ns) val

/-- The source loop of `find_center_node`: run the relaxation from each node still labelled `0`. -/
def relaxAll (K : ℕ) : List V → (V → ℕ) → (V → ℕ)
  | [], val => val
  | n :: ns, val => relaxAll K ns (if val n = 0 then go L K n none (val n + 1) val else val)

/-- Every positive label is at most `inf` and has a walk from a leaf at most as long. -/
def Sound (inf : ℕ) (c : V → ℕ) : Prop :=
  ∀ u, c u < inf → ∃ l, L.IsLeaf l ∧ ∃ p : L.graph.Walk l u, p.length ≤ c u

theorem Sound.update [Fintype V] {inf : ℕ} {c : V → ℕ} (hs : L.Sound inf c) {n w : V} {ns : ℕ}
    (hw : w ∈ L.nbrs n) (hnw : n ≠ w) (hn : c n + 1 ≤ ns) (hlt : ns < inf) :
    L.Sound inf (update c w ns) := by
  intro u hu
  by_cases huw : u = w
  · subst huw
    obtain ⟨l, hl, p, hp⟩ := hs n (by omega)
    have hadj : L.graph.Adj n u := L.adj_iff.2 ⟨hnw, hw⟩
    exact ⟨l, hl, p.concat hadj, by simp only [SimpleGraph.Walk.length_concat]; simp; omega⟩
  · simp only [update_of_ne huw] at hu ⊢
    exact hs u hu

/-- One call's contract. `Σ val ≤ k` is the fuel; `val n + 1 = ns` and `prev`'s label two below
`ns` is what the caller guarantees on entry. -/
def Post (inf k : ℕ) (n : V) (prev : Option V) (ns : ℕ) (val : V → ℕ) : Prop :=
  (∀ u, go L k n prev ns val u ≤ val u) ∧ L.Sound inf (go L k n prev ns val) ∧
  ∀ x y, y ∈ L.nbrs x → go L k n prev ns val x + 1 < go L k n prev ns val y →
    x ≠ n ∧ go L k n prev ns val x = val x ∧ val x + 1 < val y

/-- `Post` for fuel `k` as the induction hypothesis the loop needs. -/
def Pre (inf k : ℕ) [Fintype V] : Prop :=
  ∀ (n : V) (prev : Option V) (ns : ℕ) (val : V → ℕ),
    ∑ u, val u ≤ k → val n + 1 = ns → (∀ p, prev = some p → val p + 2 ≤ ns) →
    (∀ u, val u ≤ inf) → L.Sound inf val → L.Post inf k n prev ns val

theorem go_loop [Fintype V] {inf k : ℕ} (IH : L.Pre inf k) (n : V) (prev : Option V) (ns : ℕ)
    (val0 : V → ℕ) (hsum : ∑ u, val0 u ≤ k + 1) (hns : val0 n + 1 = ns) (hb : ∀ u, val0 u ≤ inf) :
    ∀ ws : List V, (∀ w ∈ ws, w ∈ L.nbrs n) → ∀ c : V → ℕ,
      (∀ u, c u ≤ val0 u) → L.Sound inf c →
      (∀ x y, y ∈ L.nbrs x → c x + 1 < c y → c x = val0 x ∧ val0 x + 1 < val0 y) →
      (∀ u, ws.foldl (goStep (go L k) n prev ns) c u ≤ c u) ∧
      L.Sound inf (ws.foldl (goStep (go L k) n prev ns) c) ∧
      (∀ x y, y ∈ L.nbrs x →
        ws.foldl (goStep (go L k) n prev ns) c x + 1 < ws.foldl (goStep (go L k) n prev ns) c y →
        ws.foldl (goStep (go L k) n prev ns) c x = val0 x ∧ val0 x + 1 < val0 y) ∧
      (∀ w ∈ ws, prev ≠ some w → ws.foldl (goStep (go L k) n prev ns) c w ≤ ns) := by
  intro ws
  induction ws with
  | nil =>
    intro _ c hc hs hv
    simp only [List.foldl_nil, List.not_mem_nil, false_imp_iff, implies_true, and_true]
    exact ⟨fun _ => le_rfl, hs, hv⟩
  | cons w ws ih =>
    intro hws c hc hs hv
    have hwn : w ∈ L.nbrs n := hws w (by simp)
    have hws' : ∀ w' ∈ ws, w' ∈ L.nbrs n := fun w' h => hws w' (by simp [h])
    simp only [List.foldl_cons]
    -- the effect of the first neighbour
    obtain ⟨a1, s1, v1, p1⟩ : (∀ u, goStep (go L k) n prev ns c w u ≤ c u) ∧
        L.Sound inf (goStep (go L k) n prev ns c w) ∧
        (∀ x y, y ∈ L.nbrs x → goStep (go L k) n prev ns c w x + 1 < goStep (go L k) n prev ns c w y →
          goStep (go L k) n prev ns c w x = val0 x ∧ val0 x + 1 < val0 y) ∧
        (prev ≠ some w → goStep (go L k) n prev ns c w w ≤ ns) := by
      unfold goStep
      by_cases hp : prev = some w
      · simp only [hp, ite_true]
        exact ⟨fun _ => le_rfl, hs, hv, fun h => absurd rfl h⟩
      · simp only [hp, ite_false]
        by_cases hlt : ns < c w
        · simp only [hlt, ite_true]
          have hcn : c n + 1 ≤ ns := by have := hc n; omega
          have hnw : n ≠ w := by rintro rfl; omega
          have hcw : c w ≤ inf := (hc w).trans (hb w)
          have hs' := hs.update L hwn hnw hcn (by omega)
          have hsumc : ∑ u, c u ≤ ∑ u, val0 u := Finset.sum_le_sum fun u _ => hc u
          have hsumu := sum_update_lt c w ns hlt
          obtain ⟨A, S, B⟩ := IH w (some n) (ns + 1) (update c w ns) (by omega) (by simp) (by
            intro p hp
            simp only [Option.some.injEq] at hp
            subst hp
            rw [update_of_ne hnw]
            have := hc n
            omega) (by
            intro u
            by_cases hu : u = w
            · subst hu; simp; omega
            · simp [hu]; exact (hc u).trans (hb u)) hs'
          refine ⟨fun u => (A u).trans ?_, S, ?_, ?_⟩
          · by_cases hu : u = w
            · subst hu; simp [hlt.le]
            · simp [hu]
          · intro x y hy hviol
            obtain ⟨hxw, hx1, hx2⟩ := B x y hy hviol
            rw [update_of_ne hxw] at hx1 hx2
            have hy' : update c w ns y ≤ c y := by
              by_cases hu : y = w
              · subst hu; simp [hlt.le]
              · simp [hu]
            exact hv x y hy (by omega) |>.imp_left (fun h => hx1.trans h) |> fun h => ⟨h.1, h.2⟩
          · intro _
            have := A w
            simp at this
            exact this
        · simp only [hlt, ite_false]
          exact ⟨fun _ => le_rfl, hs, hv, fun _ => by omega⟩
    obtain ⟨a2, s2, v2, p2⟩ := ih hws' _ (fun u => (a1 u).trans (hc u)) s1 v1
    refine ⟨fun u => (a2 u).trans (a1 u), s2, v2, ?_⟩
    intro y hy hne
    rcases List.mem_cons.1 hy with rfl | hy
    · exact (a2 y).trans (p1 hne)
    · exact p2 y hy hne


/-- **The contract of one call**, for every fuel: it only lowers labels, keeps them sound, and
leaves no pair `x → y` with `y` more than one above `x`, except pairs that were already out of line
at an unchanged `x ≠ n`. -/
theorem go_pre [Fintype V] (inf : ℕ) : ∀ k, L.Pre inf k := by
  intro k
  induction k with
  | zero =>
    intro n prev ns val hsum hns hprev hb hs
    have hz : ∀ u, val u = 0 := by
      intro u
      have : val u ≤ ∑ v, val v :=
        Finset.single_le_sum (f := val) (fun _ _ => Nat.zero_le _) (Finset.mem_univ u)
      omega
    refine ⟨fun _ => le_rfl, hs, ?_⟩
    intro x y _ h
    have := hz x
    have := hz y
    simp only [go] at h
    omega
  | succ k ih =>
    intro n prev ns val hsum hns hprev hb hs
    obtain ⟨a, s, v, p⟩ := L.go_loop ih n prev ns val hsum hns hb (L.nbrs n) (fun w h => h) val
      (fun _ => le_rfl) hs (fun x y _ h => ⟨rfl, h⟩)
    have hgo : go L (k + 1) n prev ns val = (L.nbrs n).foldl (goStep (go L k) n prev ns) val := rfl
    unfold Post
    rw [hgo]
    refine ⟨a, s, ?_⟩
    intro x y hy hviol
    obtain ⟨h1, h2⟩ := v x y hy hviol
    refine ⟨?_, h1, h2⟩
    rintro rfl
    by_cases hp : prev = some y
    · have := hprev y hp
      have := a y
      omega
    · have := p y hy hp
      omega

omit [DecidableEq V] in
theorem init_violation [Fintype V] {inf : ℕ} (x y : V) (h : L.init inf x + 1 < L.init inf y) :
    L.init inf x = 0 := by
  by_cases hx : L.IsLeaf x
  · simp [Lists.init, hx]
  · by_cases hy : L.IsLeaf y <;> simp [Lists.init, hx, hy] at h

/-- The source loop keeps the contract: after running from every node still at `0`, nothing is
out of line any more. -/
theorem relaxAll_spec [Fintype V] {inf K : ℕ} (nodes : List V) :
    ∀ val : V → ℕ, (∀ u, val u ≤ inf) → L.Sound inf val → ∑ u, val u ≤ K →
      (∀ x y, y ∈ L.nbrs x → val x + 1 < val y → x ∈ nodes ∧ val x = 0) →
      (∀ u, relaxAll L K nodes val u ≤ val u) ∧ L.Sound inf (relaxAll L K nodes val) ∧
      ∀ x y, y ∈ L.nbrs x → ¬ (relaxAll L K nodes val x + 1 < relaxAll L K nodes val y) := by
  induction nodes with
  | nil =>
    intro val _ hs _ hv
    refine ⟨fun _ => le_rfl, hs, fun x y hy h => ?_⟩
    exact absurd (hv x y hy h).1 (by simp)
  | cons n t ih =>
    intro val hb hs hsum hv
    simp only [relaxAll]
    by_cases h0 : val n = 0
    · rw [ite_eq_left h0]
      obtain ⟨A, S, B⟩ := go_pre L inf K n none (val n + 1) val hsum rfl (by simp) hb hs
      have hsum' : ∑ u, go L K n none (val n + 1) val u ≤ K :=
        (Finset.sum_le_sum fun u _ => A u).trans hsum
      obtain ⟨a, s, r⟩ := ih (go L K n none (val n + 1) val) (fun u => (A u).trans (hb u)) S hsum'
        (by
          intro x y hy hviol
          obtain ⟨hxn, hx1, hx2⟩ := B x y hy hviol
          obtain ⟨hmem, hz⟩ := hv x y hy (by omega)
          rcases List.mem_cons.1 hmem with rfl | hmem
          · exact absurd rfl hxn
          · exact ⟨hmem, by omega⟩)
      exact ⟨fun u => (a u).trans (A u), s, r⟩
    · rw [ite_eq_right h0]
      obtain ⟨a, s, r⟩ := ih val hb hs hsum
        (by
          intro x y hy hviol
          obtain ⟨hmem, hz⟩ := hv x y hy hviol
          rcases List.mem_cons.1 hmem with rfl | hmem
          · exact absurd hz h0
          · exact ⟨hmem, hz⟩)
      exact ⟨a, s, r⟩

/-- **Old path.** The C-shaped relaxation labels every node with its distance to the nearest leaf,
provided the fuel `K` covers the total label mass `Σ init` (every call lowers it). -/
theorem relaxation_nearest [Fintype V] (hconn : L.graph.Connected) {inf : ℕ}
    (hinf : Fintype.card V ≤ inf) (nodes : List V) (hn : ∀ v, v ∈ nodes)
    {l₀ : V} (h₀ : L.IsLeaf l₀) {K : ℕ} (hK : ∑ u, L.init inf u ≤ K) (u : V) :
    L.Nearest u (relaxAll L K nodes (L.init inf) u) := by
  have hi := Inv.init L (inf := inf) nodes hn
  obtain ⟨a, s, r⟩ := relaxAll_spec L (inf := inf) nodes (L.init inf) hi.bound hi.sound hK
    (by
      intro x y _ h
      have hx := init_violation L x y h
      exact ⟨hn x, hx⟩)
  exact L.nearest_of_closed hconn hinf (fun u hu => by have := a u; have := hi.leaf u hu; simp only [initSt] at this; omega)
    (fun x y hy => by have := r x y hy; omega) s u h₀

/-- **Fast path = C-mirror path.** Any finished run of the breadth-first loop and the old relaxation
produce the same integer for every node. -/
theorem bfs_eq_relaxation [Fintype V] (hconn : L.graph.Connected) {inf : ℕ}
    (hinf : Fintype.card V ≤ inf) (nodes : List V) (hn : ∀ v, v ∈ nodes)
    {l₀ : V} (h₀ : L.IsLeaf l₀) {K : ℕ} (hK : ∑ u, L.init inf u ≤ K) {t : St V}
    (h : Relation.ReflTransGen (Step L) (L.initSt nodes inf) t) (hq : t.2 = []) (u : V) :
    t.1 u = relaxAll L K nodes (L.init inf) u :=
  Nearest.unique L (L.bfs_nearest hconn hinf nodes hn h₀ h hq u)
    (L.relaxation_nearest hconn hinf nodes hn h₀ hK u)

theorem terminal_of_empty [Fintype V] (s t : St V) (h : Relation.ReflTransGen (Step L) s t)
    (hq : s.2 = []) : t = s := by
  rcases Relation.ReflTransGen.cases_head h with rfl | ⟨s', hs', _⟩
  · rfl
  · cases hs' with
    | pop val n q => simp at hq

/-- Components without a leaf (a cycle): neither path changes anything. -/
theorem noleaf [Fintype V] {inf : ℕ} (hinf : 0 < inf) (hno : ∀ l, ¬ L.IsLeaf l) (nodes : List V)
    (K : ℕ) {t : St V} (h : Relation.ReflTransGen (Step L) (L.initSt nodes inf) t) :
    t.1 = L.init inf ∧ relaxAll L K nodes (L.init inf) = L.init inf := by
  have hz : ∀ u, L.init inf u ≠ 0 := fun u => by simp [Lists.init, hno u]; omega
  constructor
  · have hnil : (L.initSt nodes inf).2 = [] := by
      simp only [initSt]
      apply List.filter_eq_nil_iff.2
      intro a _; simpa using hz a
    exact congrArg Prod.fst (terminal_of_empty L _ _ h hnil)
  · suffices H : ∀ nodes : List V, relaxAll L K nodes (L.init inf) = L.init inf from H nodes
    intro nodes
    induction nodes with
    | nil => rfl
    | cons n t ih => simp [relaxAll, hz n, ih]

end Lists

end Old


/-! ### Subtree sizes: one pass in reverse discovery order = a walk to the root per leaf

`set_n_steps_to_center` leaves each node a `parent`, a child count `nc` and a place in the discovery
order `order` (the BFS queue), every parent before its children. The old `set_subtree_size` walks from each
childless node to the root adding 1 to every node on the way. The new one visits `order` backwards,
adds 1 at a childless node, and adds the node's finished total to its parent.
-/

section Subtree
variable {V : Type} [Fintype V] [DecidableEq V] (parent : V → Option V) (nc : V → ℕ)

/-- The parent chain `n, parent n, …` that `while let Some(p) = parent` walks, cut after `k` nodes. -/
def chain : ℕ → V → List V
  | 0, _ => []
  | k + 1, w => w :: (match parent w with | none => [] | some p => chain k p)

/-- `size[p] += 1` for every `p` on the chain. -/
def addChain (s : V → ℕ) (c : List V) : V → ℕ := c.foldl (fun s p => update s p (s p + 1)) s

omit [Fintype V] in
theorem addChain_apply (c : List V) : ∀ (s : V → ℕ) (v : V), addChain s c v = s v + c.count v := by
  induction c with
  | nil => intro s v; simp [addChain]
  | cons p c ih =>
    intro s v
    have := ih (update s p (s p + 1)) v
    simp only [addChain, List.foldl_cons] at this ⊢
    rw [this, List.count_cons]
    by_cases h : v = p
    · subst h; simp; omega
    · simp [h, Ne.symm h]

/-- The old pass: from every childless node (in `sg.nodes` order) walk to the root. -/
def oldSizes (K : ℕ) (nodes : List V) (s0 : V → ℕ) : V → ℕ :=
  nodes.foldl (fun s n => if nc n = 0 then addChain s (chain parent K n) else s) s0

/-- The new pass for one node `n` of the reverse-order sweep. -/
def newStep (s : V → ℕ) (n : V) : V → ℕ :=
  let s1 := if nc n = 0 then update s n (s n + 1) else s
  match parent n with
  | none => s1
  | some p => update s1 p (s1 p + s1 n)

/-- The new pass: nodes in reverse discovery order. -/
def newSizes (order : List V) (s0 : V → ℕ) : V → ℕ := order.reverse.foldl (newStep parent nc) s0

/-- The number of childless nodes at or below `v`, as the old pass counts them. -/
def leafCount (K : ℕ) (v : V) : ℕ := ∑ w, if nc w = 0 then (chain parent K w).count v else 0

omit [Fintype V] in
theorem oldSizes_apply (K : ℕ) (nodes : List V) :
    ∀ (s0 : V → ℕ) (v : V), oldSizes parent nc K nodes s0 v =
      s0 v + (nodes.map fun w => if nc w = 0 then (chain parent K w).count v else 0).sum := by
  induction nodes with
  | nil => intro s0 v; simp [oldSizes]
  | cons n t ih =>
    intro s0 v
    have := ih (if nc n = 0 then addChain s0 (chain parent K n) else s0) v
    simp only [oldSizes, List.foldl_cons, List.map_cons, List.sum_cons] at this ⊢
    rw [this]
    by_cases h : nc n = 0
    · simp [h, addChain_apply]; omega
    · simp [h]

/-- The old pass computes `leafCount`. -/
theorem oldSizes_eq (K : ℕ) (nodes : List V) (hnd : nodes.Nodup) (hn : ∀ v, v ∈ nodes) :
    oldSizes parent nc K nodes (fun _ => 0) = leafCount parent nc K := by
  funext v
  rw [oldSizes_apply, leafCount, ← List.sum_toFinset _ hnd]
  have : nodes.toFinset = Finset.univ := by ext x; simp [hn x]
  simp [this]

/-- Counting along the chain splits over the children: `v` is on `w`'s chain iff it is `w` or
an ancestor, and an ancestor is entered through exactly one child. -/
theorem chain_count (idx : V → ℕ) (hidx : ∀ v p, parent v = some p → idx p < idx v) :
    ∀ k w, idx w < k → ∀ v, (chain parent k w).count v = (if w = v then 1 else 0) +
      ∑ c ∈ Finset.univ.filter (fun c => parent c = some v), (chain parent k w).count c := by
  intro k
  induction k with
  | zero => intro w hw; omega
  | succ k ih =>
    intro w hw v
    cases hp : parent w with
    | none =>
      simp [chain, hp, List.count_singleton]
    | some p =>
      have h1 := ih p (by have := hidx w p hp; omega) v
      simp only [chain, hp, List.count_cons] at h1 ⊢
      simp only [beq_iff_eq, Finset.sum_add_distrib, Finset.sum_ite_eq, Finset.mem_filter,
        Finset.mem_univ, true_and, hp, Option.some.injEq]
      rw [h1]
      split_ifs <;> omega

/-- `leafCount` obeys the recurrence the sweep implements: a node's total is its own `1` if childless
plus its children's totals. -/
theorem leafCount_rec (idx : V → ℕ) (hidx : ∀ v p, parent v = some p → idx p < idx v) {K : ℕ}
    (hK : ∀ w, idx w < K) (v : V) :
    leafCount parent nc K v = (if nc v = 0 then 1 else 0) +
      ∑ c ∈ Finset.univ.filter (fun c => parent c = some v), leafCount parent nc K c := by
  have key : ∀ w, (if nc w = 0 then (chain parent K w).count v else 0) =
      (if w = v then (if nc v = 0 then 1 else 0) else 0) +
      ∑ c ∈ Finset.univ.filter (fun c => parent c = some v),
        (if nc w = 0 then (chain parent K w).count c else 0) := by
    intro w
    rw [chain_count parent idx hidx K w (hK w) v]
    by_cases h : nc w = 0
    · by_cases hw : w = v
      · subst hw; simp [h]
      · simp [h, hw]
    · by_cases hw : w = v
      · subst hw; simp [h]
      · simp [h, hw]
  unfold leafCount
  simp_rw [key]
  rw [Finset.sum_add_distrib, Finset.sum_comm]
  simp

/-- Sum of `F` over the already-swept nodes whose parent is `v`. -/
def pending (F : V → ℕ) (o : List V) (v : V) : ℕ :=
  ∑ c ∈ o.toFinset.filter (fun c => parent c = some v), F c

omit [Fintype V] in
theorem pending_cons (F : V → ℕ) (a : V) (t : List V) (ha : a ∉ t) (v : V) :
    pending parent F (a :: t) v = pending parent F t v + (if parent a = some v then F a else 0) := by
  unfold pending
  by_cases h : parent a = some v
  · rw [List.toFinset_cons, Finset.filter_insert, ite_eq_left h,
      Finset.sum_insert (by simp [ha]), ite_eq_left h]
    omega
  · rw [List.toFinset_cons, Finset.filter_insert, ite_eq_right h, ite_eq_right h]
    omega

omit [Fintype V] in
theorem newStep_apply (hself : ∀ v, parent v ≠ some v) (s : V → ℕ) (n v : V) :
    newStep parent nc s n v = s v + (if v = n ∧ nc n = 0 then 1 else 0) +
      (if parent n = some v then s n + (if nc n = 0 then 1 else 0) else 0) := by
  unfold newStep
  cases hp : parent n with
  | none => by_cases h : nc n = 0 <;> by_cases hv : v = n <;> simp [h, hv]
  | some p =>
    have hpn : p ≠ n := fun h => hself n (by rw [hp, h])
    by_cases h : nc n = 0 <;> by_cases hv : v = n <;> by_cases hvp : v = p <;>
      simp [h, hv, hvp, hpn, Ne.symm hpn] <;> try omega
    all_goals (intro h'; exact absurd h'.symm hvp)

/-- **The sweep, by induction on the list it has swept**, children first: after sweeping `o` (a block of
the discovery order closed under taking children), a swept node holds its total and any other node holds
the sum of the totals of its swept children. -/
theorem sweep_form (F : V → ℕ)
    (hF : ∀ v, F v = (if nc v = 0 then 1 else 0) +
      ∑ c ∈ Finset.univ.filter (fun c => parent c = some v), F c)
    (hself : ∀ v, parent v ≠ some v) :
    ∀ o : List V, o.Nodup → o.Pairwise (fun a b => parent a ≠ some b) →
      (∀ a ∈ o, ∀ c, parent c = some a → c ∈ o) →
      ∀ v, o.foldr (fun n s => newStep parent nc s n) (fun _ => 0) v =
        (if v ∈ o ∧ nc v = 0 then 1 else 0) + pending parent F o v := by
  intro o
  induction o with
  | nil => intro _ _ _ v; simp [pending]
  | cons a t ih =>
    intro hnd hpw hkids v
    have hat : a ∉ t := (List.nodup_cons.1 hnd).1
    have hnd' := (List.nodup_cons.1 hnd).2
    have hpw' := List.pairwise_cons.1 hpw
    have hkids' : ∀ b ∈ t, ∀ c, parent c = some b → c ∈ t := by
      intro b hb c hc
      rcases List.mem_cons.1 (hkids b (by simp [hb]) c hc) with rfl | h
      · exact absurd hc (hpw'.1 b hb)
      · exact h
    have IH := ih hnd' hpw'.2 hkids'
    simp only [List.foldr_cons]
    rw [newStep_apply parent nc hself]
    set st := t.foldr (fun n s => newStep parent nc s n) (fun _ => 0) with hst
    have hsa : st a = pending parent F t a := by simpa [hat] using IH a
    have hkid_t : ∀ c, parent c = some a → c ∈ t := by
      intro c hc
      rcases List.mem_cons.1 (hkids a (by simp) c hc) with rfl | h
      · exact absurd hc (hself c)
      · exact h
    have hpa : pending parent F t a =
        ∑ c ∈ Finset.univ.filter (fun c => parent c = some a), F c := by
      unfold pending
      apply Finset.sum_congr _ (fun _ _ => rfl)
      ext c
      simp only [Finset.mem_filter, List.mem_toFinset, Finset.mem_univ, true_and]
      exact ⟨fun h => h.2, fun h => ⟨hkid_t c h, h⟩⟩
    have hFa : st a + (if nc a = 0 then 1 else 0) = F a := by
      rw [hsa, hpa, hF a]; omega
    rw [hFa, pending_cons parent F a t hat v, IH v]
    by_cases hva : v = a
    · subst hva
      have : ¬ parent v = some v := hself v
      simp [hat, this]
      omega
    · simp [hva]
      omega

/-- The new pass computes `leafCount`. `hpar` is the BFS property: a node is discovered after its
parent, so every parent has a smaller position in `order`. -/
theorem newSizes_eq (order : List V) (hnd : order.Nodup) (hn : ∀ v, v ∈ order)
    (hpar : ∀ v p, parent v = some p → order.idxOf p < order.idxOf v) :
    newSizes parent nc order (fun _ => 0) = leafCount parent nc order.length := by
  have hself : ∀ v, parent v ≠ some v := fun v h => by
    have := hpar v v h; omega
  have hK : ∀ w, order.idxOf w < order.length := fun w => List.idxOf_lt_length_of_mem (hn w)
  have hpw : order.Pairwise (fun a b => parent a ≠ some b) := by
    rw [List.pairwise_iff_getElem]
    intro i j hi hj hij h
    have := hpar _ _ h
    rw [hnd.idxOf_getElem j hj, hnd.idxOf_getElem i hi] at this
    omega
  funext v
  have := sweep_form parent nc (leafCount parent nc order.length)
    (leafCount_rec parent nc order.idxOf hpar hK) hself order hnd hpw (fun a _ c _ => hn c) v
  rw [newSizes, List.foldl_reverse]
  simp only [this, hn v, true_and]
  have hpend : pending parent (leafCount parent nc order.length) order v =
      ∑ c ∈ Finset.univ.filter (fun c => parent c = some v), leafCount parent nc order.length c := by
    unfold pending
    apply Finset.sum_congr _ (fun _ _ => rfl)
    ext c
    simp [hn c]
  rw [hpend, leafCount_rec parent nc order.idxOf hpar hK v]

/-- **Fast path = C-mirror path.** The sweep in reverse discovery order and the walk from every
childless node to the root produce the same size for every node. -/
theorem sweep_eq_walks (order nodes : List V) (hnd : order.Nodup) (hn : ∀ v, v ∈ order)
    (hnodes : nodes.Nodup) (hnodes' : ∀ v, v ∈ nodes)
    (hpar : ∀ v p, parent v = some p → order.idxOf p < order.idxOf v) :
    newSizes parent nc order (fun _ => 0) = oldSizes parent nc order.length nodes (fun _ => 0) := by
  rw [newSizes_eq parent nc order hnd hn hpar, oldSizes_eq parent nc _ nodes hnodes hnodes']

end Subtree

end GvpProofs.Twopi
