# Network simplex on cut sets

`crates/graphviz-pure-layout/src/common/ns/cut.rs` runs the pivots of `rank2` without walking
the leaving edge's subtree. It still chooses **the same entering edge as `ns.c` at every pivot,
ties included**, and leaves the same ranks, cut values, tree lists, tree indices and parent
pointers. This page is the argument. The functions in `common/ns.rs` mirror `ns.c` and stay the
reference: the cut-set path is chosen per graph (§7), falls back to them when C's search stops
being an argmin or an invariant check fails (§5), and in check mode runs beside them at every pivot (§8).

The cut sets are either stored, in a pool edited at each pivot, or computed when asked for (§9).
Everything else here holds for both: they answer the same question.

Issues #641 and #702; measurements are in their pull requests.

Proof of record: `proofs/GvpProofs/Ns/CutSets.lean` (Lean 4, #732). It covers §2 with Lemma 1,
(I1) of §3, §4.1 as far as comparing two tied candidates by climbing, and the cut-set, cut-value and
slack algebra of §4.2, for a pivot from a tree whose `lim`/`low` are current (the first pivot, or
any pivot after a renumbering). Its model was written for stored cut sets with climbs that have no
budget, so these are argued here in prose only: the climbs' budget and the scan past it (§4.1,
Lemma 1a), `LR_balance`'s allowance (§4.3) and computed cut sets (§9, Lemma 3). Not yet covered
either: later production pivots with stale numbering (#750), the fallback's renumbering (§5), the
`treeupdate` walks and the LCA climb, the parent pointers (I3), and §4.3 beyond `enter_edge`.

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
  cycle contains `t`. On the computed path this is not stored: `cover[e]` is produced when `e`
  is asked about (§9).
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
when there is a tie.

**The climbs have a budget.** A climb costs the depth of the tree, and each tie costs one, so a
query costs ties × depth. On b29 that is 0.4 ties and 25 steps per query, where C's scan visits
714 nodes. On long chains it is quadratic: on `par_20_500` (§7) a query has 450 ties and 333,189
steps, where C's scan visits 223 nodes. So a query may climb `TIE_CLIMBS` (1,024) steps in all.
Past that, the climbs stop, and `first_tied` marks the candidates of the minimum slack and runs
C's scan from `v`, on the current tree, up to the first marked edge.

> **Lemma 1a.** With no negative candidate, the first marked edge is C's answer, and the walk
> visits only nodes that C's scan visits on the same query.
>
> *Proof.* The walk pops, scans and pushes as §2's order says, so it meets edges in scan order,
> and by Lemma 1 the first candidate of minimum slack in scan order is C's answer. C's pruning
> loop runs while `Slack > 0`. Before the first candidate of minimum slack, `Slack` is above that
> minimum, which is at least 0, so nothing has been pruned, and C has visited the same nodes up to
> that point. The walk stops there. ∎

The budget changes which of the two settles a tie, and neither can give a different answer, so the
output does not depend on it: `ties_agree_at_any_budget_for_the_climbs` runs with budgets from 0
to 21 steps. §9 has what a query costs.

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

On computed cuts this needs a limit. A computed cut costs its smaller side W whatever the query,
while C's scan stops as soon as it meets a slack of 0, and here no pivot pays for a `rerank` or a
`dfs_range`, so a run of slack-0 queries is much cheaper on C's functions. A run of other queries is
much dearer there. The position run of `dot_components_1000` takes 12.8 ms with C's `LR_balance` and
6.8 ms with computed cuts, and `par_20_500` (§7) takes 85.5 ms and 232 ms, with about the same total
walk per node.

What separates them is what C must do. If the minimum slack is above 0, nothing prunes C's scan, so
it visits all of S, and above 1 its `rerank` visits S again. So each query on computed cuts spends
|W| from an allowance and earns what C must visit: 0 at a slack of 0, |S| at 1, and 2|S| above that.
The allowance starts at `BALANCE_SPARE_WALKS` (8) nodes per node of the run. When it is gone, §5's
hand-over gives the rest of `LR_balance` to C's functions, with no failure involved.

The allowance is checked before each query, and the query that runs it out can overspend it by its
own walk, which is at most n/2 nodes, since W is the smaller side. So the computed path never walks
more than 8.5n nodes beyond what C's functions must visit on the same queries: the 8n of the allowance
and that last query. After the hand-over its work is C's. Since |W| ≤ |S|, only slack-0 queries can
spend the allowance. The allowance changes who does the work and nothing else: every query gives C's answer on
either side of the hand-over.

## 5. Ranks at the end, and the fallback

When the pivots end (normally, at `maxiter`, or on an error return) and before `TB_balance`,
`scan_and_normalize` or the end of `LR_balance`, the ranks are written: the root gets
`root_rank`, and walking down the tree, a child joined by tree edge `t` gets its parent's rank
plus `minlen(t) + slack[t]` (if the child is the head of `t`) or minus it (if the tail). By (I2)
and because wrapping addition is a ring, these are exactly C's ranks.

If `enter_edge` finds a negative candidate slack, Lemma 1 no longer applies. The ranks are
written, `lim`/`low` are renumbered from the root (non-incremental `dfs_range`), and the rest of
the run, this pivot included, uses C's functions. That is exact by Lemma 2. Those numbers are stale
until then, because the cut-set path does not maintain them, so the renumbering has to reach every
node. Check mode keeps them current itself and cannot see a mistake here, which is why
`a_fallback_renumbers_the_tree_it_hands_to_c` turns the check hooks off and runs as shipped.

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

A run takes one of three paths, decided at its first pivot, because many runs have none:

- **Fewer than `COMPUTED_MIN_NODES` (64) nodes: C's functions.** Their walks are short, and
  building even the computed path's links does not pay.
- **Stored cut sets**, when all three of these hold (`worth_it`).
- **Otherwise, computed cut sets** (§9). A pool that does not fit its budget at the build also
  computes them.

Building stored cut sets costs Σ over non-tree edges of their tree-path length, and a pivot
costs about |cut set| × |f's path|. `worth_it` estimates the mean tree-path length `l` from up to
`CUT_SETS_SAMPLE` evenly spaced non-tree edges. For `m` non-tree edges and `n` nodes, the cut sets
then hold about `l·m` entries and a pivot costs about `l·(l·m/n)`. They are stored when:

- **`n ≥ CUT_SETS_MIN_NODES` (10,000).** Below that, computing a cut is cheaper than editing the
  stored ones. Computing costs about the smaller side of the cut, which grows with the graph, and
  an edit does not. Timed on DAGs from `scripts/bench/synth.py`, position runs cross between 7,880
  nodes (34.3 ms computed, 38.1 ms stored) and 9,924 (51.4 ms, 46.1 ms). At 11,694 they are at
  78.6 ms and 64.5 ms. Rank runs have not crossed at 20,000 (61.3 ms, 77.2 ms), and the position runs
  are the ones that matter, being four times larger and many times slower, so the threshold follows
  them.
- **`l·(l·m/n)/n ≤ CUT_SETS_MAX_COST` (1.0)**, the time estimate.
- **`l·m ≤ CUT_SETS_MAX_ENTRIES·(n + m)` (10)**, the memory estimate. An entry is 4 bytes, against
  a few hundred bytes per node and edge that dot already holds, so the cut sets add at most a few
  percent to the peak heap. The term also turns away graphs whose tree cycles are long for their
  size.

A store that grew past `CUT_SETS_KEEP_WORDS` (2¹⁸ words, 1 MB) is released when its run ends, so
a large graph's cut sets do not stay allocated for the rest of the layout; smaller stores are
reused by the next run.

### The memory bound

This is the pool's bound; the computed path has no pool (§9). The selector's estimate is a sample
of the initial tree, and the cut sets change as cycles change during the pivots. The bound that
holds is `CUT_SETS_MAX_WORDS` (24) pool words per node and non-tree edge:

- the build computes the cut sets instead if its layout already needs more;
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

Each network simplex run with at least one pivot was timed on all three paths, the faster of two
rounds: `tests/wild`, `tests/fixtures` and the three large `dot_dag` graphs, 297 runs. The pool's
budget was lifted for this, so that its column shows what storing would cost if it were allowed. It
is not allowed: the selector's memory estimate turns away every one of these runs but the `dot_dag`
ones.

| nodes in the run | runs | pivots | C's functions | computed | stored |
| :-- | --: | --: | --: | --: | --: |
| 0–63 | 180 | 717 | 2.0 ms | 4.2 ms | 5.0 ms |
| 64–127 | 39 | 783 | 1.8 ms | 1.9 ms | 3.0 ms |
| 128–1,023 | 64 | 5,163 | 21.0 ms | 13.2 ms | 29.8 ms |
| 1,024–4,095 | 8 | 6,054 | 112.1 ms | 33.9 ms | 101.2 ms |
| 4,096–8,191 | 3 | 7,650 | 379.1 ms | 75.7 ms | 453.3 ms |
| 16,384 and up | 3 | 34,624 | 28,262 ms | 3,768 ms | 636 ms |

Individual runs:

| run | nodes | pivots | C's functions | computed | stored | path |
| :-- | --: | --: | --: | --: | --: | :-- |
| dot_dag_20000, position | 77,796 | 22,462 | 26,384 ms | 3,495 ms | 457 ms | stored |
| dot_dag_5000, position | 19,476 | 5,643 | 1,444 ms | 211 ms | 101 ms | stored |
| dot_dag_20000, rank | 20,000 | 6,519 | 433 ms | 61.3 ms | 77.2 ms | stored |
| b29, position | 7,029 | 4,056 | 277 ms | 46.4 ms | 230 ms | computed |
| badvoro, position | 5,327 | 1,997 | 74.2 ms | 21.3 ms | 207 ms | computed |
| dot_dag_1000, position | 3,949 | 1,043 | 37.5 ms | 9.2 ms | 17.7 ms | computed |
| dot_dag_5000, rank | 5,000 | 1,597 | 28.0 ms | 8.0 ms | 16.0 ms | computed |
| b143, position | 3,041 | 1,421 | 27.4 ms | 5.8 ms | 25.4 ms | computed |
| b106, position | 2,231 | 909 | 22.7 ms | 5.6 ms | 32.1 ms | computed |
| b124, position | 1,950 | 1,019 | 11.4 ms | 6.4 ms | 11.3 ms | computed |

Summed over the corpus without the `dot_dag` graphs, the runs take 448 ms on C's functions and
108 ms as the selector chooses. Of the 111 runs with at least 64 nodes there, 32 are slower on the
computed path than on C's functions, by at most 0.07 ms each: they have few pivots, and the build is
most of their cost.

**Shapes the corpus does not have.** `par_K_L`, repeated for j in 0..K, is `a -> b{j}
[minlen=L]; a -> c{j}; c{j} -> b{j};`, with long edges tied by construction. The ratchet holds
`dot_par_10_500`, because a first version of this path, which climbed from every tied candidate, took
1.22 s on it against 0.32 s on C's functions, and 12.5 s against 0.29 s on `par_20_500` (§4.1). Whole
dot runs as shipped, user and system seconds on a gnu release build without PGO, the faster of two:

| graph | before this path | as shipped | C's functions only | × before | × C |
| :-- | --: | --: | --: | --: | --: |
| `g11005_grid` (54-node grid, one `minlen=1000`) | 0.153 | 0.101 | 0.116 | 1.51 | 1.15 |
| `par_10_500` | 0.341 | 0.204 | 0.317 | 1.67 | 1.55 |
| `par_20_500` | 0.344 | 0.184 | 0.286 | 1.88 | 1.56 |
| `dot_components_1000` | 0.058 | 0.044 | 0.065 | 1.31 | 1.46 |
| `grid_20x8_1_1000` | 2.948 | 2.534 | 3.048 | 1.16 | 1.20 |
| `grid_15x15_3_300` | 4.399 | 3.818 | 4.692 | 1.15 | 1.23 |
| `ladder_100_20` | 0.053 | 0.064 | 0.131 | 0.83 | 2.05 |
| `grid_nsl` (12×12 grid, `nslimit=3`) | 0.031 | 0.032 | 0.039 | 0.96 | 1.24 |

Both shapes slower than before this path are small and were on stored cut sets, which suit them:
`ladder_100_20` has 4,302 nodes in its position run, and takes 96.3 ms on C's functions, 31.8 ms
computed and 20.4 ms stored. The other way round, grids with long edges pass `worth_it` and are far
worse stored than computed. On `grid_20x8_1_1000` (16,436 nodes) C's functions take 3,084 ms, and
computed takes 886 ms. Stored takes 10,741 ms with the budget lifted, so as shipped they reach the
budget and hand over to C's functions, as they did before the computed path existed. So those stay
as slow as they were, with the memory bound holding. Both are limits of an estimate from a sample.

The memory estimate still matters on the graphs it turned away before there was a computed path.
A DAG whose edges span up to 400 nodes runs network simplex 2.85× slower on stored cut sets than on
C's functions, at +15% peak RSS; a reviewer's DAG with `samehead`/`sametail`/`constraint=false` had
a position graph of 140,100 nodes whose cut sets reached 110 MB. Such graphs now compute their cuts.

The selector is still an estimate from a sample: a graph unlike these could be slower on the path
it picks, though never larger than the bounds in this section and §9. §9 bounds a query against
C's. The output is the same on any path.

## 8. How it is checked

- **Check mode** (`cfg(test)` in the layout crate, or `--features graphviz-pure-layout/ns-check`
  anywhere): the cut-set path runs on every graph that has a pivot, whatever the selector says,
  stored and computed in turn, and without the pool budget unless a test sets one. Next to it, C's
  `rerank`, `invalidate_path` and `dfs_range` keep the real ranks and `lim`/`low` current. At
  every pivot and every `LR_balance` query it asserts:
  - that `cover[e]` is exactly the set of non-tree edges crossing e's cut, with the right bits;
  - that its signed weights plus weight(e) are e's cut value;
  - that every candidate slack is C's;
  - that the entering edge is C's `enter_edge`;
  - that the tree path and LCA are C's `treeupdate` walks (same edges, same signs);
  - that the re-hung parent pointers are what `dfs_range` assigns;
  - that after every pivot every slack is C's;
  - that every rank written at the end is C's;
  - on the computed path, after every pivot, that the links are the tree of the parent pointers,
    that each size is C's `lim - low + 1`, and that the storage has not changed.

  CI runs the workspace tests in this mode.
- **Property test** (`common/ns/cut/tests.rs`): thousands of random DAGs (random root, parallel
  edges, every balance mode, several `searchsize`s, runs cut short by `maxiter`) ranked by C's
  functions alone and by the cut-set path, stored and computed. It compares the result, every rank, parent edge,
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
- **Hand-overs as shipped.** `a_fallback_renumbers_the_tree_it_hands_to_c` turns off the check
  mode's hooks, so `lim`/`low` and the ranks go stale as they do in a release build, and compares
  with C's functions on broken cut sets, on clean runs and on wrapped slacks.
  `lr_balance_hands_over_when_its_walks_pass_the_allowance` does the same at allowances that run out
  at once, part way, or never, and counts the hand-overs, which output cannot show.
- **Ties.** `ties_on_long_chains_stay_c_s_choice` builds chains under one cut, each ending in an edge
  of the same slack, and `ties_agree_at_any_budget_for_the_climbs` uses random DAGs. Both run with
  budgets that send every tie to `first_tied`, none of them, or some part way through a query, and
  assert that both ways of settling a tie were taken.
- **NS attributes.** `ns_attributes_reach_the_pivots` runs `searchsize`, `nslimit`, `nslimit1` and
  `group` end to end, in the normal tests and in CI's `ns-check` job, and asserts that the last
  three change the output, so a run cut short or a merged group reaches the pivots. The corpus has
  no file with these attributes.
- **The cross-check has teeth.** `the_cross_check_sees_a_computed_cut_that_misses_nodes` breaks
  the links and asserts that check mode panics. Mutants of `computed.rs`, of the tie rule, of the
  hand-over, of the selector and of the adapter (reported in the pull request) are killed by these
  tests, but for three that cannot change the output.
- The fidelity ratchets of `cargo test --workspace` run with the selector as shipped.

## 9. Computed cut sets

Storing `cover[t]` for every tree edge costs `l·m` entries and an edit of about |cover[e]| × |f's
path| per pivot. On dot's mid-size graphs, whose tree cycles are long, that does not fit the memory
bound. But `enter_edge` and `shift` only ever ask about the one edge `e`, and §2 says what they
need is a set defined by the tree alone. So it can be computed when asked for.

**The computation.** Removing `e` splits the tree into S, the subtree under e's child `v`, and the
rest. Let W be whichever is smaller: S if `size(v)·2 ≤ n`, else the rest. Walk W, marking each node.
Then for each node of W and each edge at it, in either direction, take the edge if its other end is
unmarked, unless it is `e`.

> **Lemma 3.** This lists exactly the non-tree edges crossing e's cut, each once, with the right
> bit.
>
> *Proof.* An edge has one end marked exactly when it has one end on each side of the cut. A tree
> edge other than `e` has both ends on one side, because removing `e` leaves those two parts
> connected by the remaining tree edges. So after leaving out `e`, the edges with one end in W are
> the non-tree edges crossing the cut. Such an edge has one end in W, so it is read once, at that
> end. Its tail is in S exactly when (this end is its tail) = (W is S). It runs against `e` when it
> goes from head(e)'s side to tail(e)'s side, so `against` is (tail in S) ≠ (tail(e) in S), and
> tail(e) is in S exactly when `v` = tail(e). ∎

The result is (I1) for `e`, so §4.1's choice and §4.2's `shift` read it as they read a stored set.
The order differs, and §4.1 does not depend on it: the minimum slack does not depend on order, and
ties go through §4.1, which reads the tree.

**What it keeps.** Parent pointers are as in §3. Beside them, per node: its parent, its children as
a linked list, and the size of its subtree. These describe the same rooted tree, so a pivot updates
them along the same paths (`move_links`):

- nodes from `v`'s parent up to the LCA lose |S|;
- nodes from f's end outside S up to the LCA gain |S|;
- on the path from f's end `x` inside S up to `v`, which turns over, each node's new subtree is S
  minus what was below it on the path, and `x` gets all of S.

No other subtree changes: moving S changes the descendants only of the nodes on f's tree cycle.
The children lists' order means nothing: they are only walked to reach a set. C's `ND_tree_in` and
`ND_tree_out` stay as in (I4), in C's order. The sizes only choose which side to walk, and either
side gives the same result by Lemma 3, so a wrong size could cost time and nothing else.

**Staleness.** A computed set belongs to one tree: `cut_of` names its tree slot, and it is cleared
when the tree changes. If `member` is asked about any other slot it answers no, which is §5's
fallback.

**Cost against C.** For one query, with D the summed degree of W and C the edges C's scan reads:

| step | cost | what bounds it |
| :-- | :-- | :-- |
| compute the cut | D | W is at most S, so it covers no more nodes than C's full scan |
| find the minimum | at most D | the cut has at most D entries |
| climb between ties | at most 1,024 parent steps, plus the walk's list | `TIE_CLIMBS` |
| settle a tie past that | at most D to mark, then at most C | Lemma 1a: the walk stays within C's visit |

So a query costs at most 3D + C + 1,024 steps. When C's scan runs to the end (a minimum above 0), it
visits all of S, and W is no larger. D then reads both edge lists of no more nodes than C's scan
reads one list of, so D is a constant factor of C: about 2 where nodes have as many in-edges as
out-edges, which makes the query about 7C + 1,024 steps. It is not a fixed factor on every graph,
since a node's two lists can be of very different lengths.

That is not a bound on every query. When a slack is 0, C's scan stops at the first such candidate,
and its query can be far cheaper than D. In `LR_balance`, §4.3 bounds the whole phase: at most
8.5n nodes beyond what C must visit. A pivot has no such limit. It also renumbers below the LCA, which
the computed path never does, but that work is pruned and can be small, so a run of many slack-0
pivots on a tree whose cuts have large smaller sides is the shape where the computed path could
lose to C.
Of the 183 runs measured for §7 that the selector sends to this path, corpus and adversarial shapes
together, the worst is 0.13 ms slower than C's functions. That is a measurement and not a proof.

**Why not climb only, or scan only.** Climbs alone took 18.3 s on `g11005_grid`, which takes 0.15 s
on stored cut sets without this rule. C's scan alone took up to 20% longer on large DAGs on stored
cut sets, where a query reads few entries and the walk to the first tied edge can cover much of S.

**Cost and memory.** A query costs about |W| times the mean degree. On b29 the mean |W| is 407,
against a mean |S| of 853: 11% of the queries have S larger than half the graph, and those hold 69% of
all the |S|. The storage is fixed
when the build ends and no pivot changes it: 20 bytes per node for the links, 8 for where its edges
are, 4 for its mark and 4 for the walk's list, 16 per edge for its incident records, and room for
one cut set of at most the non-tree edges, beside §7's 16 bytes per edge and 20 per node. That is
about 750 KB on b29, whose whole layout has an RSS of 21 MB. A store over `COMPUTED_KEEP_WORDS`
(2¹² words, 16 KB) is released when its run ends, so it is gone before the layout's peak in the
splines: the peak heap rises 2.8% on b143, the most on the perf ratchet.

## 10. Local numbers in dot's adapter

`dotgen/ns.rs` copies a run's graph into arrays numbered for that run: nodes by their place in
`GD_nlist`, edges by their place in the in-lists. It also copies the in-lists and out-lists out of
`NodeInfo` into one flat array each, and writes everything back when `rank2` returns. Walks then touch
a few small arrays instead of a large struct, a heap list and arrays sized for the whole layout.

This cannot change a decision. Every list keeps its order, `GD_nlist` included, and the code in
`common/ns.rs` and `common/ns/cut.rs` uses a node or edge number only to index, or to test whether
two are the same. It never orders them. The one value that can name something outside the run is
`ND_par` from an earlier run, on a node this run has not yet reached. C overwrites it unread. It is
kept as `FOREIGN`, which equals no edge of this run, and it is written back unchanged if the run
ended before overwriting it.

**What this assumes.** A run's node list is closed under its edges: every edge in the in-list or
out-list of a listed node has both ends in the list. The copy needs this and C does not. C would
follow such an edge to a node outside the run and read that node's own fields, while the copy would
read whatever local number an earlier run left for it. A `debug_assert!` ("NS node list must be
closed under every incident edge") states it in debug builds and in the tests. A release build does
not test it, on the grounds that dot builds these lists by collecting a component or a cluster with
its edges. None of the 12,272 inputs of the review's comparison broke it, and if one did, the output
would differ from C's without an error.
