# Mincross without the repeated work

`crates/graphviz-pure-layout/src/dotgen/mincross/scan.rs` runs `reorder` and `transpose` with less
work. It makes **the same exchanges as `mincross.c`, from the same crossing counts, and leaves the
same state**, so `ncross`, `save_best` and every later phase see what they would have seen. This
page is the argument. The functions in `mincross.rs` mirror C and stay the reference: the shortcuts
hand over to them when a condition below does not hold (§5), and in check mode run beside them (§7).

Measurements are in the pull request that added this page.

## 1. Where the work goes

`mincross_step` runs `medians` and `reorder` on each rank, then `transpose`. On the corpus-sized
graphs that were left at about 2× (b143, b29, b106, b124, dot_dag_1000), `dot_mincross` was 21–35% of
the run, nearly all of it in three places:

- **`reorder`'s bubble passes.** n passes over a rank of n nodes whenever `reverse` or `hasfixed`
  holds. b143 makes 1.66 million comparisons on 22,440 rank positions.
- **Re-reading neighbours' orders.** `prepare_crossing_rank` rewrote the rank's copies of its
  neighbours' orders before most scans (7% of b143's run).
- **Recounting pairs.** b143 evaluates 648,404 adjacent pairs, 34,000 per `transpose`, on about
  1,200 nodes. Most evaluations repeat one whose inputs did not change.

The counting loops themselves are not the cost. Over 90% of b143's pairs have one edge per node on
each side, so sorted or merge-based counting has nothing to work on, and a tree in `rcross` would
address 1–2% of the run. Neither is done.

Exchanges cannot be skipped: 168,177 of b143's 180,191 `transpose` exchanges are ties
(`c0 == c1 > 0` under `reverse`), real state changes that later comparisons read. So `transpose`
still replays every exchange and reads every count, in order. It only stops recomputing counts it
already knows.

## 2. `reorder`

### 2.1 When the shortcut applies

`reorder_settled` runs when the view has no constraint, the rank's orders are a proven permutation
of its window (`positioned_rank`), and no median is NaN. Otherwise C's loop runs.

**Flat edges and clusters.** `prepare_mincross` sets `constrained` if any node of the view has a
cluster or any of its ranks has a flat-edge matrix. C's `left2right` (mincross.c:564) can only answer
true through one of those: its first branches compare `ND_clust`, and without a matrix it returns
false. So for an unconstrained view `left2right` is false for every pair, `muststay` and `sawclust`
stay false (mincross.c:1403), and neither can affect anything. A constrained view runs C's loop; this
page's closed form does not apply to it.

What remains is this loop, with `ep = n` and `ep--` after a pass only if `!hasfixed && !reverse`:

```text
lp = first position < ep with mval >= 0
rp = next position < ep with mval >= 0; stop if none
if p1 > p2 || (p1 >= p2 && reverse): exchange(lp, rp)
lp = rp
```

Nodes with a negative median are never `lp` or `rp`, so they never move. The others form a
subsequence a₀ … a₍ₘ₋₁₎ on fixed positions. With no NaN, `>` and `>=` are a total preorder on it;
-0.0 ties with 0.0, as it does in C, and infinities order normally.

### 2.2 One pass

A pass carries one node along the subsequence. The carry starts as a₀. At the next node x: if the
test holds, x moves one place left and the carry stays; otherwise x becomes the carry. By induction
**the carry has the largest median of the part scanned so far**.

Let d(x) be the number of nodes before x with a strictly larger median.

> **Lemma 1.** After t full passes, d(x) is max(0, d(x) − t), with or without `reverse`.
>
> *Proof.* Only the carry passes x in a pass. If d(x) > 0, the carry reaching x is strictly larger,
> so the test holds and that one larger node goes behind x: d(x) falls by exactly 1. If d(x) = 0, the
> only node that can pass x is the carry with a median ≤ x's, or x itself as the carry passing nodes
> with a median ≤ x's. Neither puts a strictly larger node before x. ∎

So after max d(x) ≤ m − 1 passes the medians are sorted, whatever `reverse` is. Exchanging two equal
medians changes which node is where but not the sequence of medians.

### 2.3 The final order

**Without `reverse`** the test is `p1 > p2`, so equal medians never exchange: the result is stable.
`ep` may shrink after each pass but cuts nothing that is unsorted. After k passes the k largest are
in the subsequence's last k places, and the element with k nodes after it is at position ≤ n−1−k,
which is below `ep = n−k`. With `hasfixed` set the later passes change nothing. There are n ≥ m
passes. The medians are sorted and equal ones keep their order, and only one permutation does both:
**the stable sort of the subsequence.**

**With `reverse`** `ep` never shrinks, so there are exactly n full passes, and the test is
`p1 >= p2`. The medians end sorted (Lemma 1). What remains is the order within each class V of equal
medians. Call x ∈ V *active* in a pass if d(x) = 0 when it starts. A larger node before x is also
before every later node of V, so the active nodes are the first ones of V in rank order.

> **Lemma 2.** A pass rotates V's active prefix left by one, and leaves the rest of V's order alone.
>
> *Proof.* Let f be the first active node. On reaching f the carry is smaller than f: no larger node
> is before f, and no equal one either, since f is V's first (an earlier node of V would be active
> too). So f becomes the carry. It then passes every node up to the first strictly larger node z, or
> to the end. Those nodes are ≤ f, so the V-nodes among them have no larger node before them: they are
> exactly the other active nodes. f ends behind them. The V-nodes after z have z before them, so they
> are not active. They are passed only by strictly larger carries, which does not change their order
> relative to each other. ∎

By Lemma 1, x is active in pass t (from 0) exactly when d(x) ≤ t, and d is non-decreasing along V.
So the class's final order is these n rotations composed. `rotate_class` groups them: while the
active prefix has c nodes for k passes, it rotates that prefix by k mod c. `rotate_ties` gets d from
a Fenwick tree, walking the classes from the largest median down, so that the tree holds exactly the
strictly larger nodes.

> **Whether anything changed.** C only tests `if (changed)`. An exchange happens at all exactly when
> some adjacent pair of the subsequence meets the test before any pass: if none does, the carry
> becomes each node in turn and nothing moves, and the same holds for every later pass; if one does,
> then either an earlier exchange happened or the carry is that pair's left node. `settle` computes
> this, and it is true even when the rotations end where they began.

The cost is O(m log m) plus the rotations, at most O(|V|²) per class, against n·m comparisons.

### 2.4 Every side effect

`reorder` calls nothing but `left2right` and `exchange`, and reads nothing else that it writes. No
caller can see a state between two exchanges. So its effects are its final state, which is this:

| what | C's `reorder` | `reorder_settled` |
| :-- | :-- | :-- |
| `ND_order` of nonnegative nodes | swapped by each `exchange` | the final permutation, §2.3 |
| `ND_order` of negative nodes | never written | never written |
| `GD_rank(Root)[r].v` | swapped by each `exchange` | the final permutation, same positions |
| `ND_mval` | never written (`medians` writes it, before) | never written |
| `GD_rank(Root)[r].valid`, `[r-1].valid` | false if `changed`; r−1 only if r > 0 | the same, from the same test |
| `candidate`, `cache_nc` | not touched | not touched |
| `ncross` | not called; only its cache is invalidated | not called |
| flat matrix, `ND_clust` | read through `left2right`, §2.1 | not read |
| `hasfixed` | read for `ep` only | not needed, §2.3 |

The port's own state also matches the reference's: compact orders are written with the arena's,
`write_crossing_orders` publishes them, and `invalidate_crossing_rank(g, r, false)` clears the
neighbours' snapshots, all when `changed` holds and through the same calls.

## 3. `transpose`

### 3.1 What a pair's counts depend on

For adjacent v, w of rank r, C computes (mincross.c:636):

```text
c0 = in_cross(v, w) + out_cross(v, w)      c1 = in_cross(w, v) + out_cross(w, v)
```

taking `in_cross` only if r > 0 and `out_cross` only if rank r+1 is not empty. The shortcut applies
when every node of the view's ranks has all its edges to ranks r−1 and r+1 (`snapshot_valid`,
`snapshot_adjacent`), every rank is a proven permutation of its window, and:

> **(W)** A node of the view that is not in its rank's window has an order outside that window.

A cluster's rank is a window of the root's, and a neighbour outside the window can hold a stale
order. Nothing about the window proves (W), so `build_scan` checks it before every `transpose`, in
every build, over all of the view's slots. If it fails, C's loop runs. Then:

> **(F)** c0 and c1 are functions of v's and w's edge lists, penalties and ports, which do not change
> during a `transpose`, and of the orders of their neighbours, which are in other ranks. The loops add
> in a fixed order, so equal inputs give the equal integer, wrapped or not.

> **(E)** `exchange(v, w)` swaps two adjacent orders. The counting loops use an order only through
> the sign of a difference. The sign changes only for the comparison between v's and w's own orders.
>
> *Proof.* Take a node x other than v and w, in their rank. If x is in the window, its order is
> distinct from theirs, and nothing lies between two adjacent orders, so x is on the same side of
> both, before and after. If x is outside the window, it does not move, and by (W) its order is
> outside the window, the range v and w stay in. So again x is on the same side of both. Comparing a
> node with itself, through parallel edges, gives 0 before and after. ∎

(W) is needed. If a stale order ties with w's, the exchange turns a comparison with v from 1 into 0,
so a pair's counts change with no edge to w involved. A test turns the check off and asserts that the
cross-check then fails. The check has not fired on any of the 2,407 `transpose` calls of the 548
corpus graphs measured.

### 3.2 The memo

`pairs` has one record per rank position: the node there and, if `known`, the (c0, c1) of the pair it
starts. The invariant:

> **(M)** If position i is `known`, its counts are what C would compute now for the nodes at i, i+1.

All positions start unknown, at the start of each `transpose`. So nothing survives a `reorder`, a
`restore_best` or any other write to orders, and `reverse` need not be stored: the counts do not
depend on it and the test uses the current one. Within a `transpose` the only state change is an
exchange. After `exchange(v, w)` at position i of rank r:

- **Position i** now holds (w, v). By (F) its counts are (c1, c0), and it stays `known`.
- **Positions i−1 and i+1** hold new pairs, and are forgotten.
- **Other positions of r** hold the same nodes, and by (F) keep their counts.
- **Ranks r−1 and r+1.** By (E) a pair (x, y) there changes only if x has an edge to one of v, w and
  y to the other. `forget_pairs` walks those edges and forgets the position when x and y are
  adjacent. If they are not adjacent they have no memo. Over-forgetting is harmless.
- **Other ranks** have no edge to r, so nothing changes.

So (M) holds at every read.

### 3.3 Equivalence

The scan is C's `transpose_step`: same ranks, same order, with `left2right` asked first, from fields
that do not change during a `transpose`. By (M), every count it uses is the integer C would compute.
So by induction it makes the same exchanges and returns the same `rv`, and `transpose` sees the same
`delta` and the same candidates. Reading a count has no side effect, so not computing one loses
nothing.

Three things are deferred, and none is read in between:

- **`valid` and `candidate`** are set at the end of a scan rather than at each exchange. Only
  `transpose` reads `candidate`, between scans, and only `ncross` reads `valid`.
- **`ND_order` and the rank buffers** are written once at the end. During a `transpose` only the
  compact orders are read. `left2right` uses `low`, `cluster` and the like, not orders.
- **The crossing copies** that the reference reads are not maintained. If anything moved, every
  rank's `mincross_snapshot` is cleared, so the reference rebuilds them before it reads them.

## 4. Integer widths

Nothing here changes an integer. The memo stores what these functions return, and §2 reads no count.

| where | C | port |
| :-- | :-- | :-- |
| `in_cross` total | `int64_t` | `i64` |
| `in_cross` product | `int * int` (undefined on overflow) | `i64` product, never overflows |
| `out_cross` total and product | `int`, then widened | `i32`, wrapping, then widened |
| `c0`, `c1`, `rv`, `delta` | `int64_t` | `i64` |

The shortcut calls the same `in_cross` and `out_cross` on the same inputs, so whatever they return,
wrapped or not, is what it remembers. Its additions wrap where the reference's plain `+` wraps in a
release build; they cannot overflow on real penalties.

**Wrapping can stop `transpose` from ending.** Without wrapping, the sum of crossing products falls
by `delta` each round and never rises, so `delta >= 1` cannot last. A wrapped `out_cross` breaks that,
and a small DOT file reaches it. **#682** has eight nodes with `group` set, 40 parallel edges between
half of the pairs and one edge between the others, so merged penalties are 40,000 and two products
add up past `i32`. C, the merge base and this code all run past every limit tried on it, so it is not
a regression, and the matching behaviour is correct for fidelity. `scan/tests.rs` has its ranks. The
shortcut reads what the reference reads, for as long as it reads it, and allocates nothing while
doing so, so memory stays flat. #682 records the open question of whether C's loop ever ends.

## 5. When C's functions run

- **`reorder`:** a constrained view, a rank that is not a proven permutation of its window, an empty
  rank, a node the view has no slot for, or a NaN median.
- **`transpose`:** any rank of the view that is not a proven permutation, or that has a node with no
  slot in the view, or with an edge in its own rank or skipping a rank; or a node outside a window
  with an order inside it, (W) in §3.1. The decision is taken before anything changes, for the whole
  `transpose`.
- **A constrained view** still uses the memo in `transpose`: `left2right` is asked of each pair first,
  as in C, and it does not change what a pair counts.

## 6. The memory bound

The memo is not a table keyed by pair. It has **one 24-byte record per rank position**, and a pair's
counts are forgotten when the pair stops being adjacent, so no number of exchanges or rounds can make
it grow. `prepare_mincross` reserves everything once for the view, before any scan:

| array | size |
| :-- | :-- |
| `pairs` | 24 bytes per rank position |
| `items` | 16 bytes per node of the widest rank |
| `work` | 3 × 4 bytes per node of the widest rank, plus 4 |
| `ranks` | one record per rank of the view |

A test asserts that these capacities do not change during a run. Check mode asserts all four against
the largest view's sizes after each `reorder` and `transpose`.

## 7. Check mode

Under `cfg(test)` or the `mincross-check` feature:

- **`transpose`** runs C's loop first and logs every count it reads. It then restores the state and
  runs the scan, which compares each count **as it reads it**. The first wrong count stops the run.
  At the end the state must match: orders, arena orders, medians, rank buffers, `valid`, `candidate`
  and `cache_nc`.
- **`reorder`** runs the shortcut, then restores the state and runs C's loop with every constraint
  test on, and compares the same state.

A check that only compares at the end cannot work here. A wrong memo, or counts that wrap, can keep a
`transpose` from ending, and its log would then grow without limit. So the log holds at most
2²⁴ reads (400 MB, and 2¹⁸ in a unit test). A run that reaches the limit stops with an error, every
read so far having been compared. When the shortcut declines and C's loop runs for real, that run has
the same limit.

The tests in `scan/tests.rs` are:

- `settle` against C's loops on plain arrays, 200,000 random ranks in all four modes, ties and
  negative medians included;
- whole `mincross_step` runs on random ranks: plain, with counts that wrap, with clusters and
  flat-edge matrices, on a window of the root's ranks as a cluster has, and with stale orders tying
  with such a window;
- that check (W) declines exactly the views that break it;
- the ranks of #682, which stop at the read limit, as C's loop and as the fallback;
- three tests that break the shortcut on purpose (no remote invalidation; ties left unrotated; (W)
  not checked) and assert that the cross-check fails.

CI runs the workspace's tests in check mode, so every corpus graph that reaches a shortcut is
compared too.

## 8. What was tried and dropped

- **Memoising across `mincross_step` calls.** About 88% of a rank's nodes move in each `reorder`, so
  little survives.
- **Merge-based counting and a tree in `rcross`.** §1: no work to save.
- **The neighbour's slot inline in the singleton span**, to skip one load per side. Under 1% on every
  graph measured, so the store stays as it was.
