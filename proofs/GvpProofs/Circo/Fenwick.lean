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

/-- Tree update guarded by "the half is set": the shape of both `for half in ..` loops over an
edge's two halves (`2e`, `2e + 1`) in `count_indexed`. -/
def idxHalves (len : ℕ) (d : ZMod (2 ^ 64)) (e : ℕ) (s : IdxState) : Option IdxState :=
  (List.range' (2 * e) 2).foldlM
    (fun s half =>
      if half ∈ s.bits then do
        let t ← indexUpdate len d s.tree (s.ord e)
        pure { s with tree := t }
      else pure s)
    s

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

end GvpProofs.Circo.Fenwick
