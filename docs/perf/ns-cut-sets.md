# Network simplex on cut sets

`crates/graphviz-pure-layout/src/common/ns/cut.rs` runs the pivots of `rank2` without walking
the leaving edge's subtree. It still chooses **the same entering edge as `ns.c` at every pivot,
ties included**, and leaves the same ranks, cut values, tree lists, tree indices and parent
pointers. This page is the argument. The functions in `common/ns.rs` mirror `ns.c` and stay the
reference: the cut-set path is chosen per graph (§7), falls back to them when C's search stops
being an argmin or an invariant check fails (§5), and in check mode runs beside them at every pivot (§8).

Issue #641; measurements are in its pull request.

## 1. Why C's pivot is quadratic

`rank2` repeats `leave_edge` → `enter_edge(e)` → `update(e, f)` (ns.c:982–1001). With `v` the
endpoint of the leaving tree edge `e` below it in the tree and `S` the subtree under `v`, each
pivot walks S three times:

- `dfs_enter_outedge`/`dfs_enter_inedge` scan every edge list in S;
- `rerank` shifts every rank in S;
- `dfs_range` renumbers `lim`/`low` under the LCA of f's ends, most of which it cannot skip.

On dot's position graphs, S averages a fifth of the graph and the number of pivots grows
with the graph, so network simplex is about n². Yet each pivot evaluates only about 18
candidates: the cut between S and the rest is crossed by about 36 non-tree edges, and the
entering edge's tree cycle is about 35 edges long, at 19,000 nodes as at 78,000.

## 2. C's selection rule

Notation: the tree is rooted at `GD_nlist(g)`'s first node, which never changes. `lim` is a
post-order number and `low` the smallest `lim` in a subtree, so `w ∈ subtree(u)` exactly when
`low(u) ≤ lim(w) ≤ lim(u)`. `slack(g) = rank(head) − rank(tail) − minlen`, wrapping. A
non-tree edge `g` **crosses t's cut against t** when it goes from head(t)'s side to tail(t)'s
side of the cut that removing tree edge `t` makes.

**Candidates.** `enter_edge(e)` (ns.c:282) takes `v` = the endpoint of `e` with the smaller
`lim`, the child. If v = tail(e), `dfs_enter_inedge` keeps the non-tree in-edges of S whose tail
is outside S. If v = head(e), `dfs_enter_outedge` keeps the non-tree out-edges of S whose head
is outside S. Either way the candidates are **the non-tree edges crossing e's cut against e**.
That set does not depend on the rooting.

**Order.** Both searches use a stack. Popping `u` scans u's own list (`ND_in(u)` for the in-search,
`ND_out(u)` for the out-search) in list order, evaluating candidates as it meets them, and pushes
u's children: first those joined by an edge of that list, in list order; then those joined by a
tree edge of the other direction, in `ND_tree_out(u)` (in-search) or `ND_tree_in(u)` (out-search)
list order. "Child" is tested as `lim(other) < lim(u)`, which is `par(other) == edge`. The last
push pops first, so the scan meets **nodes in pre-order with children in reverse push order, and
a node's edges in list order**. Call this the scan order.

**Choice.** `if (slack < Slack || Enter == NULL)`: the first candidate is taken, a later one
replaces it only when strictly smaller.

**Pruning.** The second push loop runs only while `Slack > 0`.

> **Lemma 1.** If no candidate has a negative slack, `enter_edge(e)` returns the first
> candidate of minimum slack in the scan order (none if there are no candidates).
>
> *Proof.* Until `Slack ≤ 0` the scan is the full scan, so if the minimum is positive the
> result is the full scan's first minimum. If the minimum is 0, `Slack` first reaches 0 at the
> first slack-0 candidate in scan order; everything evaluated before it was positive, and nothing
> after it can be smaller than 0, so it stays the answer however the rest of the scan is pruned. ∎

With a negative candidate (only possible after wrapped ranks: a feasible tree has no negative
slack), the pruning can skip the minimum, so the cut-set path does not decide those (§5).

## 3. What the cut-set path keeps

For the current tree T:

- **(I1) Cut sets.** For every tree edge `t`, `cover[t]` is the set of non-tree edges crossing
  t's cut, each with the bit *against t*. Equivalently: the non-tree edges whose fundamental
  cycle contains `t`.
- **(I2) Slacks.** `slack[x]` is C's `slack(x)` for every edge, and `root_rank` is C's rank of
  the root.
- **(I3) Parent pointers.** `par(u)` is u's parent edge in T rooted at the root, as C's.
- **(I4) Everything C writes**: tree lists `ND_tree_in`/`ND_tree_out` (same order), `Tree_edge`,
  `ED_tree_index`, cut values are C's, because the cut-set path performs C's own operations on
  them (`exchange_tree_edges`, and the same cut-value additions, §4.2). `leave_edge` reads
  only cut values and the order of `Tree_edge`, from its `S_i` cursor through its
  `Search_size` window, so by (I4) it leaves the same tree edge as C at every pivot.

`lim`/`low` are left as they were when the pivots began (§6). All four hold when the pivots
begin: `cover` is built by walking each non-tree edge's tree path with the then-valid
`lim`/`low`, and the slacks and `root_rank` are read off the ranks.

## 4. A pivot

### 4.1 The entering edge

`enter_edge(e)` reads the members of `cover[e]` that cross against `e`: by (I1) and §2 these are
exactly C's candidates. Their slacks come from `slack[]`, C's by (I2). If one has a negative
slack, the run falls back (§5). Otherwise the minimum is taken; between tied candidates, the
scan order is evaluated directly:

- the same end in S (the head for the in-search, the tail for the out-search): the lower index
  in that node's `ND_in`/`ND_out` list comes first (precomputed: these lists don't change);
- otherwise both ends climb the parent pointers to `v`; at the lowest common ancestor `w`, if
  `w` is one of the two ends, that one comes first (pre-order); else the child of `w` on each
  path gets its push rank (0, index in the scanned list of `w`) or (1, index in the current
  other-direction tree list of `w`), and the higher rank comes first.

That is the scan order of §2 on the current tree (by (I3) and (I4)), so by Lemma 1 the edge is
C's. The order changes from pivot to pivot (S moves and is re-rooted, swap-removal reorders
`ND_tree_in`/`ND_tree_out`), but nothing about it is stored: it is read from the current tree
when there is a tie. On dot's graphs ties come up in 20–65% of pivots and the climbs average
about 13 steps.

### 4.2 `update`

C's `update(e, f)` (ns.c:707) with `delta = slack(f)`:

**Ranks.** C calls `rerank(n, d)` for `n` and `d` chosen at ns.c:710–724; the cut-set path
makes the same choice. `rerank(n, d)` subtracts `d` from the ranks of `n` and of everything
reachable from it through tree edges other than `par(n)`:

- if `n` has a parent edge, `n` is e's child (a tree leaf's only tree edge is its parent edge),
  so the moved set is S, the subtree under `e`. An edge's slack changes exactly when one of its
  ends is in S: that is `e`, and by (I1) the non-tree edges in `cover[e]`. The bit *against e*
  says which end is inside. So the cut-set path adds or subtracts `d` on those slacks: (I2)
  holds, with no other slack touched;
- if `n` is the root (`par` NULL), C moves the whole tree, which changes no slack; the cut-set
  path subtracts `d` from `root_rank`. This happens when e's end that C picks is the root and a
  tree leaf; `f` then enters with a positive slack, as in C.

**Cut values.** C's two `treeupdate` walks climb from tail(f) and from head(f) to their LCA:
that is f's tree path. Each edge `t` on it gains `cutvalue(e)` when `d` is true and loses it
otherwise, where `d` is true exactly when f's cycle, oriented along `f`, runs `t` backwards.
The cut-set path finds the LCA by climbing from both ends one step at a time, marking, until one
reaches the other's mark (cost at most twice the longer path, never the tree depth). It collects
the path with σ(t) = +1 for forwards and −1 for backwards, and applies
`cutvalue(t) −= σ(t)·cutvalue(e)`. That is the same additions on the same edges. Then
`cutvalue(f) = −cutvalue(e)` and `cutvalue(e) = 0` as in C. σ(e) = +1, because `f` crosses
against `e`.

**Tree lists.** `exchange_tree_edges(e, f)` is C's function, called as C calls it.

**Cut sets (I1).** Write a non-tree edge's fundamental cycle as a signed edge sum `C_g = g +
Σ τ_g(t)·t` over the tree edges `t` of its path, with τ_g(t) = +1 when the cycle, oriented
along `g`, runs `t` forwards. Crossing t's cut *against t* is τ_g(t) = +1, since the cycle
returns across `t` the other way. Let σ = τ_f. In T' = T − e + f:

- for `g ∉ cover[e]`, C_g uses only edges of T', so it is still g's fundamental cycle;
- for `g ∈ cover[e]`, `g ≠ f`: `C_g − τ_g(e)·C_f` has no `e` (σ(e) = +1) and uses only edges of
  T' besides `g`, so it is g's fundamental cycle in T'. On f's path, an edge `t ≠ e` that was on
  g's path cancels (its coefficient τ_g(t) − τ_g(e)σ(t) is 0, as a fundamental cycle has no
  coefficient ±2): `g` leaves `cover[t]`. One that was not joins, against `t` exactly when
  −τ_g(e)σ(t) = +1, which is when *g against e* differs from *σ(t) = +1*. Its coefficient on `f`
  is −τ_g(e): against `f` exactly when it was not against `e`;
- `e`, now non-tree, has cycle `C_f` (σ(e) = +1), so it joins `cover[t]` for every `t ≠ e` on
  f's path, against `t` when σ(t) = +1, and is against `f`;
- `f` is now a tree edge and leaves every cut set: those of f's path.

So `cover[f]` is `cover[e]` with every bit flipped and `f` replaced by `e` (against); it takes
over e's storage. For each `t` on f's path, the members of `cover[e]` toggle in `cover[t]`.
Membership is tested by stamping cover[t]'s members with a per-edit number and their index, so a
removal is a swap with the last entry. Cost: Σ over f's path of (|cover[t]| + |cover[e]|).

**Parent pointers (I3).** C's `dfs_range(lca, par(lca), low(lca))` after `invalidate_path` from
f's ends to the LCA renumbers every node whose subtree changed. Pruning skips only children whose
`par` and `low` are unchanged, which `invalidate_path` rules out for every node on f's path. So
afterwards every `par` is the parent edge in T' rooted at the root. Re-rooting S at x, f's end in
S, changes the parent of exactly the nodes on the tree path from x up to v: `par(x) = f`, and
each node above x on that path gets the edge to the node below it. The cut-set path reverses
exactly those, so (I3) holds without `dfs_range`.

### 4.3 `LR_balance`

`LR_balance` (ns.c:778; balance 2, dot's x coordinates) calls `enter_edge(e)` for every tree
edge with cut value 0, and when the entering slack `delta` is above 1, `rerank`s e's child by
`delta / 2` (truncated, as C's `-delta / 2`). The tree does not change, so (I1) and (I3) stay;
the entering edge is §4.1's, and the `rerank` is §4.2's slack shift of `cover[e]` and `e`.

## 5. Ranks at the end, and the fallback

When the pivots end (normally, at `maxiter`, or on an error return) and before `TB_balance`,
`scan_and_normalize` or the end of `LR_balance`, the ranks are written: the root gets
`root_rank`, and walking down the tree, a child joined by tree edge `t` gets its parent's rank
plus `minlen(t) + slack[t]` (if the child is the head of `t`) or minus it (if the tail). By (I2)
and because wrapping addition is a ring, these are exactly C's ranks.

If `enter_edge` finds a negative candidate slack, Lemma 1 no longer applies. The ranks are
written, `lim`/`low` are renumbered from the root (non-incremental `dfs_range`), and the rest of
the run, this pivot included, uses C's functions. That is exact by Lemma 2.

The same hand-over happens in a pivot's **check phase**, which runs in `enter_edge` right after
the cut sets pick `f` and before that edge is used or anything changes. It fails if:

- `e` has no tree slot;
- `f` is not in e's cut set;
- the LCA climb fails, or an edge of f's cycle has no tree slot;
- the pool could outgrow its budget (§7);
- the cut set offers no candidate at all.

Then the cut sets' choice is discarded. The ranks are written, `lim`/`low` are renumbered, and
**C's own `enter_edge(e)` picks the entering edge**, so every edge used after a hand-over is C's
choice by construction, not the cut sets'. C's functions then run that pivot and the rest of the
run, exact by Lemma 2.

`update` uses the checked pivot. Nothing changes between `enter_edge` and `update`, so its own
check cannot fail for the pivot `enter_edge` checked. If it ever ran for another pivot and its
check failed, it would also discard the cut sets' `f` and ask C's `enter_edge(e)` again.
`LR_balance` does the same when `f` is missing from e's cut set.

No real input has produced a broken cut set. They keep one from going on silently, and the tests
break the cut sets on purpose at each of these points (`broken_cut_sets_hand_the_pivot_to_c`,
`a_full_pool_hands_the_run_to_c`) and get C's results.

> **Lemma 2.** C's network simplex uses `lim`/`low` only to test subtree membership and which
> end of a tree edge is the child, so any valid post-order numbering of the current tree gives
> the same ranks, cut values, tree lists and parent pointers.
>
> *Proof.* `seq(low(u), lim(w), lim(u))` in `enter_edge`'s scan and test, in `treeupdate`, and
> in `x_val` is membership. `lim(a) < lim(b)` for the ends of a tree edge (in `enter_edge`,
> `update`, `LR_balance`, `treeupdate` and `invalidate_path`) is the child test.
> `invalidate_path` stops at `lim(to_node) ≥ lim(lca)`, which for a descendant means
> `to_node = lca`. `dfs_range`'s pruning only skips renumbering, and its output is a valid
> post-order with C's parent edges either way. ∎

## 6. What is not the same

`ND_low`/`ND_lim` after network simplex. The cut-set path leaves them as `feasible_tree`
numbered them, and C leaves its last incremental renumbering. Nothing reads them afterwards:
dot's `mincross` assigns `ND_low` before its `flatindex` reads it (mincross.c:1103), the port's
`fix_label_order` reads `lim`/`low` only on label-graph nodes it numbers itself, and neato's
constraint graphs are dropped after `rank`. Everything else `rank2` writes is identical,
including `ND_par`.

## 7. The selector

Building the cut sets costs Σ over non-tree edges of their tree-path length, and a pivot costs
about |cut set| × |f's path|, against C's walks of a fraction of the graph. `begin` estimates
the mean tree-path length `l` from up to `CUT_SETS_SAMPLE` evenly spaced non-tree edges. For `m`
non-tree edges and `n` nodes, the cut sets then hold about `l·m` entries and a pivot costs about
`l·(l·m/n)`. The cut-set path runs when all three hold:

- **`n ≥ CUT_SETS_MIN_NODES` (2,500).** Below that, C's walks are short and building the cut sets
  does not pay.
- **`l·(l·m/n)/n ≤ CUT_SETS_MAX_COST` (1.0)**, the time estimate.
- **`l·m ≤ CUT_SETS_MAX_ENTRIES·(n + m)` (10)**, the memory estimate. An entry is 4 bytes, against
  a few hundred bytes per node and edge that dot already holds, so the cut sets add at most a few
  percent to the peak heap. The term also turns away graphs whose tree cycles are long for their
  size. There the time estimate is wrong: it assumes C walks a fixed fraction of the graph per
  pivot, which is not true of every shape.

Graphs routed to C's functions pay only the sample walks. A store that grew past
`CUT_SETS_KEEP_WORDS` (2¹⁸ words, 1 MB) is released when its run ends, so a large graph's cut
sets do not stay allocated for the rest of the layout; smaller stores are reused by the next run.

### The memory bound

The selector's estimate is a sample of the initial tree, and the cut sets change as cycles
change during the pivots. The bound that holds is `CUT_SETS_MAX_WORDS` (24) pool words per node
and non-tree edge:

- the build keeps C's functions if its layout already needs more;
- before a pivot changes anything, its check phase adds the most that pivot can append to the
  pool. A cut set on f's cycle gains at most e's members (all but `f`, plus `e` itself), so it
  holds at most its length plus e's at any point of the pivot. It moves only when full, each
  time to the end of the pool with capacity 2·cap + 4 and a header, and the check replays
  exactly those moves for that worst case. If the total would pass the budget, the pool is
  compacted first. If it still would, the rest of the run hands over to C's functions (§5);
- the pool's capacity is reserved up to the budget, never past it.

So the cut sets never hold more than 96 bytes per node and non-tree edge in the pool, plus
16 bytes per edge and 20 bytes per node for the per-edge and per-tree-edge records. On the
graphs the selector picks, the pool starts at about 10–15 words per element.

### What it chooses, and what that costs

`l·m/(n+m)` is the estimated number of entries per node and non-tree edge. The cycles and peak
RSS are x86-64-v3 builds; the peak heap is dhat, as in the perf ratchet. Both compare against
the same code with only C's functions.

| graph | network simplex runs (nodes, `l·m/(n+m)`, cost) | path | cycles | peak heap |
| :-- | :-- | :-- | --: | --: |
| dot_dag_1000 | position (3,949, 8.3, 0.08) | cut sets | 1.16× faster | +3.26% |
| dot_dag_5000 | rank (5,000, 8.9, 0.07), position (19,476, 8.5, 0.02) | cut sets | 3.26× faster | +2.80% |
| dot_dag_20000 | rank (20,000, 9.2, 0.02), position (77,796, 8.4, 0.004) | cut sets | 9.23× faster | +2.80% |
| dot_dag_5000 with `samehead`/`sametail`, `constraint=false` on half the edges | position (20,168, 12.8, 0.04) | C's | | 0 |
| a DAG whose edges span up to 400 nodes (4,000 nodes) | position (16,723, 66, 1.12) | C's | | 0 |
| b29 | position (7,029, 38, 0.92) | C's | | 0 |
| b143 | position (3,041, 34, 1.70) | C's | | 0 |

The rows that now take C's functions show what the memory term prevents:

- **The `samehead` DAG** would run 1.56× faster on cut sets, at +5.75% peak heap.
- **The span-400 DAG**, forced onto the cut sets, runs network simplex 2.85× slower, at +15%
  peak RSS. Its time estimate (1.12) already keeps it off them. Another draw of the same shape,
  a reviewer's (16,744 nodes, cost 0.805), passed the time estimate alone and ran 1.1× slower
  overall with +15% RSS. The memory estimate (66 entries per element on mine) turns this shape
  away whatever the time estimate says.
- **b29** would run 1.17× faster, at +14.6% peak heap.
- **b143** would run its network simplex 1.3–1.45× faster, at +11% peak heap.
- A reviewer's DAG with `samehead`/`sametail`/`constraint=false` had a position graph of
  140,100 nodes (l ≈ 320, 108 entries per element). The cut sets reached 110 MB and peak RSS
  rose 36%. It now stays on C's functions.

**On every graph above that the selector sends to the cut sets, they are faster and add under
4% to the peak heap.** The selector is still an estimate from a sample: a graph unlike these
could be slower on cut sets, though never larger than the memory bound above. The output is the
same on either path.

### The corpus

The constants were first tuned by timing every network simplex run of the corpus on both paths:
three runs each, the fastest kept. The corpus is `tests/wild`, `tests/fixtures`, the 43 neato
graphs with an `ortho` overlap mode (whose constraint graphs `constraint.c` ranks), and
`dot_dag_1000`/`dot_dag_5000`. That is 1,885 runs from 594 graphs: 2,530 ms with C's functions
everywhere, 622 ms with cut sets everywhere, 512 ms with the faster of the two, run by run.

Below 2,500 nodes the runs are slower on cut sets (rechecked with the final code):

| run | nodes | cost | C's functions | cut sets |
| :-- | --: | --: | --: | --: |
| b106, position | 2,231 | 0.95 | 27.2 ms | 31.4 ms |
| b124, position | 1,950 | 0.51 | 12.0 ms | 11.4 ms |
| ngk10_4, position | 1,209 | 0.71 | 3.5 ms | 4.4 ms |
| dot_dag_1000, rank | 1,000 | 0.45 | 2.7 ms | 3.5 ms |

With the memory term, none of the 592 corpus graphs that run network simplex (1,881 runs) takes
the cut sets: their tree cycles are too long for their size. The cut sets are for large graphs
shaped like dot's synthetic DAGs. neato's constraint graphs have at most 118 nodes in the corpus
and are dense (estimated cost 10–60): cut sets would be 8× slower on them, and the selector keeps
them on C's functions. Check mode runs every one of these graphs on cut sets anyway, so their
exactness is tested.

## 8. How it is checked

- **Check mode** (`cfg(test)` in the layout crate, or `--features graphviz-pure-layout/ns-check`
  anywhere): the cut-set path runs on every graph whatever the selector says, and without the
  pool budget unless a test sets one. Next to it, C's
  `rerank`, `invalidate_path` and `dfs_range` keep the real ranks and `lim`/`low` current. At
  every pivot and every `LR_balance` query it asserts:
  - that `cover[e]` is exactly the set of non-tree edges crossing e's cut, with the right bits;
  - that its signed weights plus weight(e) are e's cut value;
  - that every candidate slack is C's;
  - that the entering edge is C's `enter_edge`;
  - that the tree path and LCA are C's `treeupdate` walks (same edges, same signs);
  - that the re-hung parent pointers are what `dfs_range` assigns;
  - that after every pivot every slack is C's;
  - that every rank written at the end is C's.

  CI runs the workspace tests in this mode.
- **Property test** (`common/ns/cut/tests.rs`): thousands of random DAGs (random root, parallel
  edges, every balance mode, several `searchsize`s, runs cut short by `maxiter`) ranked by C's
  functions alone and by the cut-set path. It compares the result, every rank, parent edge,
  mark, tree list, cut value and tree index. It asserts that the cases include tied entering
  edges, C's whole-tree `rerank`, cut-set relocation and compaction.
- **Fallback tests**, each comparing against C's functions alone:
  - wrapping minimum lengths give negative candidate slacks;
  - `broken_cut_sets_hand_the_pivot_to_c` takes the entering edge out of e's cut set at three
    points: before the pivot's check phase, before `update` (which must check the pivot
    again), and in an `LR_balance` query. It asserts every point fell back and that C's search
    re-picked the edge;
  - `a_full_pool_hands_the_run_to_c` sets the pool budget to the build's own size, or slightly
    more.
- The fidelity ratchets of `cargo test --workspace` run with the selector as shipped.
