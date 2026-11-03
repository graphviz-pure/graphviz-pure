# Network simplex `leave_edge` on an index

`crates/graphviz-pure-layout/src/common/ns/leave.rs` chooses the leaving tree edge of `rank2`
without walking `Tree_edge`. It returns **the same edge as `ns.c`'s `leave_edge` at every pivot,
ties included, and leaves the cursor `S_i` on the same slot**. Issue #731, stacked on #702/#703;
the equality is a Lean theorem (`proofs/GvpProofs/Ns/LeaveEdge.lean`), checked at run time by
`ns-check`.

## 1. What C does

`leave_edge` (ns.c) keeps a cursor `S_i` into `Tree_edge` between calls:

```c
j = S_i;
while (S_i < Tree_edge.size) {            // slots j .. size-1
    f = Tree_edge.list[S_i];
    if (ND_cutvalue(f) < 0) {
        if (rv) { if (ND_cutvalue(rv) > ND_cutvalue(f)) rv = f; } else rv = f;
        if (++cnt >= Search_size) return rv;     // S_i stays on this slot
    }
    S_i++;
}
if (j > 0) {                               // wrap: slots 0 .. j-1
    S_i = 0;
    while (S_i < j) { /* the same body */ S_i++; }
}
return rv;
```

So the result is the first of minimum cut value among the first `Search_size` negative cut values
met in cyclic slot order from `S_i`, where the slot of the last one counted is the next cursor; when
fewer are found it is the first minimum among all of them, and the cursor is `j` (or the array
size, if `j` was 0). The tie rule is the strict `>`: a later slot replaces `rv` only when its cut
value is smaller.

Cost: the walk visits every slot between the cursor and the slot of the 30th negative value. Early
in a run most cut values are negative and that is a few dozen slots. Late in a run few are, the
30th is never found, and the pivot walks all `n − 1` slots.

How much that matters was measured, not assumed (§6). On `dot_dag_20000` the walk is about 1% of the
instructions and the pivots' remaining cost is in `CutStore::move_cut_sets`, `compact` and `push`;
on graphs of long parallel chains (`dot_par_10_500`, `par_20_500`) the cut values go non-negative
early and the walk is 6–9% of the instructions.

## 2. The index

`NegSlots` holds the slots whose edge has a negative cut value: a 64-ary tree of `u64` words (one
bit per slot; one summary bit per nonzero word of the level below, up to a single root word), in
one `Vec<u64>` of `n/64 + n/4096 + …` words. Operations:

- `set(slot, negative)`: write the slot's bit; if the word became empty or nonempty, update the
  summary bit above, and so on up. At most one read-modify-write per level (three levels at 10⁵
  slots).
- `next_from(p)`: the least member at or above `p`. Look in `p`'s word, masked from `p`; if empty,
  go up a level, look past the word just left; found, descend taking the least bit of each word.
  One word read per level on the way up and down.

`leave_edge` then visits only members: `next_from(S_i)`, apply C's body to it (the same compare,
`cutvalue(rv) > cutvalue(f)`, on the same edges), count, stop at `Search_size`; on running off the
end continue from 0 up to `j`. The cursor is set exactly where C sets it. The cost is the number
of members visited (at most 30, plus the words skipped) instead of the number of slots.

### Keeping it right

The invariant: bit `slot` is set exactly when `cutvalue(Tree_edge[slot]) < 0`. After the index is
built (once, from the cut values `x_cutval` computed, before the first pivot) cut values change in
three places, and each calls `set`:

| change | where | slot |
| :-- | :-- | :-- |
| add or subtract `cutvalue(e)` along the cycle of f | `treeupdate` (C's functions), the loop over the cycle in `CutStore::update` (cut sets) | the tree index of the edge, or the entry's tree slot |
| `f` takes `e`'s slot, with `-cutvalue(e)` | `exchange_tree_edges` (both paths) | `tree_index(e)` |

Nothing else writes a tree edge's cut value after the pivots begin (`init_graph` and `x_cutval`
run before; `LR_balance` and `TB_balance` only read).

## 3. The proof

`proofs/GvpProofs/Ns/LeaveEdge.lean`, no `sorry`, standard axioms only (`propext`,
`Classical.choice`, `Quot.sound`). It models:

- `scanRange`/`leaveRef`: C's two loops, line for line, over `val : ℕ → ℤ` (the cut value of the
  edge in each slot), the cutoff `k`, the array length `L` and the cursor `s`; the result is the
  chosen slot and the cursor on exit.
- `idxRange`/`leaveIdx`: the same two loops over `next`, the successor query of an indexed set.

Theorems:

- `leaveIdx_eq_leaveRef`: if the set `N` is exactly the slots below `L` with `val i < 0` (`Inv`),
  and `next` answers "least member at or above `p`" (`NextSpec`), and `s ≤ L`, then
  `leaveIdx val k next L s = leaveRef val k L s`: the same slot (the same edge) and the same
  cursor, for every `val`, `k`, `L`, `s`. The proof skips the non-members between two visited
  slots (`scanRange_skip`) and shows that past the last member the scan finds nothing
  (`scanRange_nonneg`).
- `inv_set`, `inv_apply`: a point edit of a cut value, mirrored by the point edit of the set
  (`setIdx`), keeps `Inv`; so does any list of such edits.
- `leaveRef_cursor_le`: the cursor stays in `0 ..= L`, so the premise `s ≤ L` holds at every pivot.
- `run_idx_eq_run_ref`: for any number of pivots, where a pivot is any function from the cut
  values and the chosen slot to a list of in-range `(slot, new value)` edits, the run on the index
  and the run of C's scans produce the same list of (slot, cursor), whatever the pivots do.
- `succAt_spec`: a `next` satisfying `NextSpec` exists for every finite set.
- Four worked cases (tie, second-smaller wins, wrap, empty), which `leave.rs` repeats as a unit
  test against the Rust.

What is **not** proved, and how it is covered:

| gap | covered by |
| :-- | :-- |
| `NegSlots::next_from` implements `NextSpec` (the 64-ary tree is not modelled) | unit test against `BTreeSet` over random edits at every level boundary (63, 64, 65, 4095, 4096, 4097, 262145 slots), the full and the empty index |
| every cut-value change calls `set` (the list in §2 is complete) | check mode: on every pivot the index is compared with the cut values of **every** tree slot (`assert_agrees`) |
| the Lean loops are the Rust loops | line for line, plus the worked cases and a randomized test of the Rust index against the Rust walk (cutoff 0 to 59, wrapped cursors, ties, cut values edited like `update` edits them) |
| `Int` for `i32` | only comparisons of cut values are read; the wrapping arithmetic that computes them is upstream of the function |
| Aeneas extraction | the binary appeared while the PR was in progress; a probe (not committed: `proofs/` has no Aeneas library yet) accepted all of `NegSlots` (`build`, `set`, `next_from`) and rejected `members` (`?` inside a `while`) and the walk in `ns.rs`; the statement is over a Lean model |

## 4. Check mode

Under `cfg(test)` or the `ns-check` feature every run indexes (any size), and every call of the
indexed `leave_edge` also runs C's walk from the same cursor and asserts the same edge and the same
cursor, then asserts the index against the cut values of every tree slot. The feature stays. The
proof makes the first check (same edge, same cursor) a theorem about the selection rule; the check
keeps covering what the theorem takes as hypotheses: that `NegSlots` is an ordered set, and that no
cut-value write escapes `set`. Before this change `ns-check` had nothing on `leave_edge`.

## 5. When it is used

Runs whose `Tree_edge` has fewer than `INDEX_MIN_SLOTS` slots (4096) keep C's walk: it is short,
and the build (one pass over the cut values) and the edits would not pay for themselves. The
tree's vector is also sized once, to `nodes − 1`, instead of growing by doubling while the feasible
tree is built: that removes about log₂ n reallocations per run and more than pays for the index's one
allocation, so the allocation count falls on every case of the ratchet that runs network simplex.

## 6. Numbers

Same machine, same build flags (the ratchet's musl build), `before` = the head of #703.

**Instructions** (`perf stat -e instructions:u`, exact; output bytes identical in every row):

| input | NS size (position phase) | before | after | Δ |
| :-- | --: | --: | --: | --: |
| `dot_dag_5000` | 19,476 nodes, 5,643 pivots | 4,643.4 M | 4,593.2 M | −1.1% |
| `dot_dag_20000` | 77,796 nodes, 22,462 pivots | 20,330 M | 20,078 M | −1.2% |
| `dot_par_10_500` | 19,991 nodes, 1,887 pivots | 1,535 M | 1,402 M | −8.7% |
| `par_20_500` | 39,981 nodes, 892 pivots | 1,305 M | 1,232 M | −5.7% |
| `dot_dag_1000` (index off, 3,948 slots) | 3,949 nodes | 769.5 M | 769.5 M | 0.0% |

**Wall time** (musl release, interleaved runs, load 3–7; C 16.1.0 for scale):

| input | 1.0 | before | after | after vs before | C |
| :-- | --: | --: | --: | --: | --: |
| `dot_dag_5000` | 5.2 s | 814 ms | 793 ms | −2.6% (min +3%) | 10.7 s |
| `dot_dag_20000` | 211 s | 3,784 ms | 3,723 ms | −1.6% (min −0.4%) | 310 s |
| `dot_par_10_500` | 1.14 s | 220 ms | 209 ms | −5.0% | 468 ms |
| `par_20_500` | 1.02 s | 215 ms | 216 ms | 0% | 2.5 s |

The wall-time gain is real only where the walk was long, and is small there. On large layered
DAGs it is inside the noise.

See the pull request for the ratchet table.
