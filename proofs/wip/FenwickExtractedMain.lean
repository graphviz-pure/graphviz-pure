import GvpProofs.Circo.FenwickExtracted

/-!
# WIP: `count_indexed = count_all_crossings_reference`, over the extracted definitions

Not part of the `GvpProofs` library (outside its glob, so `lake build` does not see it, and the
`sorry` below is allowed here only). After `scripts/proofs/lake.sh build`, check that it still
compiles with `cd proofs && ../scripts/proofs/lake.sh env lean wip/FenwickExtractedMain.lean`; the
only warning allowed is the one for the `sorry`.

What is proved in `GvpProofs/Circo/FenwickExtracted.lean` (no `sorry`): the extracted
`index_update`, `index_prefix`, `add_crossings` and `wrap_crossings`. What is left is the layer
between them and `Fenwick.lean`'s `countIndexed_eq_reference`, which the model proves on a Lean
structure (`Finset` of set halves, `ℕ → ℕ` orders). Each extracted function in
`Extracted/CircoCount/Funs.lean` needs a refinement lemma `extracted = ok (model step)` under an
abstraction map; the main theorem then follows from the model's.

## Work remaining (one lemma per extracted function)

* `bitsOf`/`ordOf` abstraction maps (below), and `Slice.set_opt` lemmas for `order` and `open`.
* `tree_step` (range loop over two halves): refines `idxHalves`. Needs a lemma that
  `word &&& (1 <<< (half % 64)) ≠ 0` is `half ∈ bitsOf open`.
* `exclude_edges`, `query_edges`, `restore_edges`, `open_edges`, `reset_orders`: slice-iterator
  loops (`core.slice.iter.IteratorSliceIter.next`), each a `List.foldlM` over the edge slice; refine
  `idxExclude`, `idxQuery`, `idxRestoreClose`, `idxOpen`. `query_edges` also needs
  `add_crossings_ok` (the checked-build assertion), so its hypothesis is "the running total stays
  in `0..i32::MAX`".
* `count_indexed_loop`: refines `idxRun`; `fill` is a `List.replicate`.
* Reference side: `reference_scan` (word loop, then `bits &= bits - 1` with `trailing_zeros`)
  refines `refQuery`: needs "clearing the lowest set bit visits the set bits in ascending order";
  `reference_close`, `reference_open`, `count_all_crossings_reference_loop0/1`.
* Overflow bookkeeping: the reference's `crossings += 1` is a checked add in the extraction (it
  fails past `i32::MAX`), as is `add_crossings`; the theorem is about runs where neither fails.

The statement below is the target. Every hypothesis is either a bound the Rust enforces before
entering the indexed path, or one of the model's `NodeWf` facts about `CGraph` adjacency.
-/

open Aeneas Aeneas.Std Result ControlFlow
open GvpProofs.Extracted.CircoCount GvpProofs.Circo.Fenwick GvpProofs.Circo.FenwickExtracted

namespace GvpProofs.Circo.FenwickExtractedMain

/-- Bit `half % 64` of word `half / 64` of the `open` bitset. -/
def bitSet (o : Slice U64) (half : ℕ) : Prop :=
  ∃ w, o.val[half / 64]? = some w ∧ w.val.testBit (half % 64)

/-- `arena.order[e]` as a natural number (positive in every state the counters reach). -/
def ordAt (order : Slice I32) (e : ℕ) : ℕ :=
  match order.val[e]? with
  | some x => x.val.toNat
  | none => 0

/-- The adjacency slice of node `n`: `adjacency[offsets[n]]`. -/
def adjOf (offsets : Slice (core.ops.range.Range Usize)) (adjacency : Slice Usize) (n : ℕ) :
    List ℕ :=
  match offsets.val[n]? with
  | some r => ((adjacency.val.drop r.start.val).take (r.«end».val - r.start.val)).map (·.val)
  | none => []

/-- The block graph the counters read, from the `head` and `tail` slices. -/
def graphOf (head tail : Slice Usize) (offsets : Slice (core.ops.range.Range Usize))
    (adjacency : Slice Usize) : Graph where
  head e := (head.val[e]?.map (·.val)).getD 0
  tail e := (tail.val[e]?.map (·.val)).getD 0
  adj := adjOf offsets adjacency

/-- **Target theorem.** If the reference scan returns `ok (c, order', open')` (so none of its
checked additions overflowed), then on a well-formed block graph the indexed counter, with the
production `MUTATION = 0`, returns `ok (some c, order', open', tree')`: no early `None`, the same
`i32` total, the same final `arena.order` and the same `open` bitset. -/
theorem count_indexed_eq_reference (h : Is64)
    (order : Slice I32) (head tail list : Slice Usize) (offsets : Slice (core.ops.range.Range Usize))
    (adjacency : Slice Usize) («open» tree : Slice U64)
    -- sizes: `E` edges; `open` covers the `2E` halves; the indexed path needs `list.len() < tree.len()`
    (E : ℕ) (hE : E < 2 ^ 62) (hord : order.length = E) (hhead : head.length = E)
    (htail : tail.length = E) (hopen : 2 * E ≤ 64 * «open».length)
    (htree : tree.length < 2 ^ 62) (hlist : list.length < tree.length)
    (hlist31 : list.length ≤ 2 ^ 31 - 1)
    -- every adjacency entry is an edge id
    (hadj : ∀ x ∈ adjacency.val, (x : Usize).val < E)
    -- every listed node has an offset range inside `adjacency` and a well-formed adjacency slice
    (hoff : ∀ n ∈ list.val, ∃ r, offsets.val[(n : Usize).val]? = some r ∧
      r.start.val ≤ r.«end».val ∧ r.«end».val ≤ adjacency.length)
    (hwf : ∀ n ∈ list.val, NodeWf (graphOf head tail offsets adjacency)
      (adjacency.val.map (·.val)) (n : Usize).val)
    (c : I32) (order' : Slice I32) (open' : Slice U64)
    (href : circogen.blockpath.count_all_crossings_reference order head tail list offsets
      adjacency «open» = ok (c, order', open')) :
    ∃ tree', circogen.blockpath.count_indexed 0#u8 order head tail list offsets adjacency «open»
      tree = ok (some c, order', open', tree') := by
  sorry

end GvpProofs.Circo.FenwickExtractedMain
