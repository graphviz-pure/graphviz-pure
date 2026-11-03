# Exact circo edge-half recounts

C: `lib/circogen/blockpath.c:count_all_crossings`, `reduce`.
The retained bitset scan is `count_all_crossings_reference` in `blockpath.rs`.
The production counter replaces its enumeration with Fenwick suffix queries.
This is integer kernel restructuring (T8), with reused scratch (T9).

Proof of record: `proofs/GvpProofs/Circo/FenwickExtractedMain.lean` (Lean 4,
`scripts/proofs/lake.sh build`), theorem `count_indexed_eq_reference`. It is stated over the Lean
that Aeneas generates from the shipped `count_indexed` and `count_all_crossings_reference`
(`proofs/GvpProofs/Extracted/CircoCount/`): whenever the reference returns `ok` (its checked `i32`
additions did not overflow), `count_indexed` returns the same count, final orders and half set, and
the count is the exact crossing count. It reuses the simulation of
`proofs/GvpProofs/Circo/Fenwick.lean` (the model proof, #728) and the Fenwick lemmas of
`FenwickExtracted.lean`. Two proofs of record, with distinct scopes:

* `FenwickExtractedMain.count_indexed_eq_reference` (over the extracted Rust): runs where the
  checked reference succeeds, i.e. the exact crossing count is at most `i32::MAX`. The extraction
  carries the checked build's arithmetic (`debug_assertions` on), so it is silent above that bound.
* `Fenwick.countIndexed_wrapped` (over the hand model): the release build's wrapping `i32` total,
  which is what the kept/rejected move comparisons read. Counts above `i32::MAX` are reachable
  (a complete 403-node block already has about 2.17 × 10⁹ pairs), so this regime is shipped
  behaviour and the model proof remains its record.

The prose below is the informal reading of both; it states two hypotheses the prose only implies:
each node's adjacency slice lists every incident block edge exactly once, and every set half belongs
to a block edge. The runtime check (`circo-check`) stays: the extracted theorem covers 64-bit targets
(not wasm32), the checked build and `MUTATION = 0`, takes the caller's graph facts as hypotheses,
and trusts Aeneas's standard-library models (see "Model vs Rust" in `Fenwick.lean`'s header).

## Invariant and predicate

At the start of each **list occurrence**'s query phase, the bitset is exactly
the reference's set of `(edge id, half)` keys. The Fenwick tree contains one unit
at `EDGEORDER(e)` for **each set half**. Initially both are empty and every
block edge's order is zero. The list need not be a permutation: `LEAFONE` and
`LEAFTWO` paths can overlap, so a node can occur repeatedly. Opening orders
are positions in this list, 1 through L, not distinct-node ranks. Scratch is
therefore sized from the initial **list length**, not `subg.nnodes()`.
Production circo derived graphs are strict and loop-free: each incident edge
id occurs once per node, and a neighbour is always a different node. The counter
also handles self and parallel edges in synthetic tests.

Temporarily remove from the tree every set half of every incident edge. These
are exactly the halves whose head or tail equals the current node. The bitset
is unchanged. During all queries at this occurrence, the reference only erases
incident halves, which are already excluded. Consequently its qualifying set
is constant apart from the query threshold. Total tree weight minus its prefix
through `EDGEORDER(e)` counts exactly the strict `>` predicate, with neither
endpoint equal to the node. This argument also covers repeated occurrences.

Restore the removed units, then apply the reference erasures of exact half
keys, decrementing the tree **only when that bit was present**. At the other
endpoint the opposite key is always absent: only the first half was ever set,
and its order stays positive even after an erase, preventing any second opening.
The opened first half therefore stays until its opening endpoint repeats.
At a repeated occurrence of the opening endpoint, however, the same key **is
present**: both its bit and its tree unit must be removed. Repeated self loops
can erase their opened key too. The order stays positive after an erase, so
such an edge is not reopened. The second pass inserts only zero-order edges,
assigning this occurrence's position and adding its exact key. These operations
reestablish the invariant after every occurrence, whether an erase fired or not.
Edges outside the block cannot enter its initially empty set.

## Integers, state and decisions

A tree weight and every prefix is bounded by the number of half keys, hence
fits u64 on supported targets (the bitset itself must fit memory). Prefix
subtraction is nonnegative. Scratch is the old bitset plus L+1 u64 entries in
one allocation; it is reset before every recount. Indexed work is O((L+D) log L),
where D counts adjacency visits including repeated nodes,
plus clearing the existing arena-sized bitset, with no allocation during a
production recount or tentative move.

The reference adds only positive units to an i32 total. Release addition is
modulo 2^32; `add_crossings` adds the same number modulo 2^32. In checked
builds it asserts before the nonnegative mathematical sum exceeds i32::MAX,
exactly when repeated reference increments would panic. For lists larger than
i32::MAX the retained reference scan runs instead,
preserving the opening-order counter's overflow behavior too. Indexed lists
have positive opening orders that fit i32. No float is changed.
The final edge orders and half bitset are the reference's. The index is private.

The initial list contains every block node at least once. In production the
derived graph is strict and loop-free, so `curnode` differs from `neighbor`.
A move removes one occurrence of `curnode` and reinserts it next to a different
node that is still present. Every multiplicity, and hence list length, stays
unchanged. Independently of this argument,
`list.len() >= tree.len()` declines to the reference rather than querying a
missing top slot. A production list of length at most 16 also uses the reference:
that avoids index maintenance on tiny blocks. Check builds still exercise the
index on these lists, so the sweep and mutation tests cover that implementation.

Every count consumed by `reduce` is therefore the same integer. Its node,
edge and before/after iteration orders, strict `<` acceptance, equal-count
rejection, restore, zero exit and ten-pass limit are unchanged. Induction on
these counts gives the same tentative moves, accepted moves and final list.

## Incremental delta per tentative move (#730)

`reduce` tries two slots next to every neighbour of every node and, until #730, recounted
the whole list after each try. For a list that is a permutation of a strict loop-free block
(every production block except the duplicate-list case above) the reference count has a closed
form. With `d(z)` the neighbours of `z` earlier in the list, `fd(z)` the later ones, and `span(z)`
the edges with one end before and one after `z`:

    count = sum_z fd(z) * span(z) - sum_z C(d(z), 2)

Equivalently it is the number of ordered edge pairs `(a,b)`, `(a',b')` (positions, `a < b`,
`a' < b'`) with `a < a' < b` and `b' != b`. Nested chords count as well as crossing ones: this is
what C's loop counts, not a chord-crossing number. `span(z)` is a difference of two prefix sums
of `fd` and `d` over positions, so the count is a function of the degree sequence of the list.

Cut `x` out of the list. Let `fd`, `d` be the degrees in the graph without `x`, over the ranks of
the list without `x`, and `FP`, `DP` their prefix sums. Putting `x` back before the nodes of rank
`c` adds `phi(c)` to the count of the list without `x`, a constant independent of `c`:

    phi(c) = (q - below) * (FP[c] - DP[c]) - C(below, 2) + sum over neighbours y of x of T(y)
    T(y)   = FP[r] - FP[c] - (DP[r+1] - DP[r])                       if r >= c
           = FP[c] - FP[r+1] + (FP[r] - DP[r+1]) + (index of y)      if r < c      (r = rank of y)

`q` is `x`'s degree, `below` the number of its neighbours of rank `< c`, and the index of `y` the
number of neighbours of lower rank. A tentative move of `x` before or after neighbour `n` is the
cut `c = rank(n) + j`, so its delta is `phi(c_new) - phi(c_old)`: `O(deg x)` from the neighbour
ranks and the prefix arrays. The arrays are built once per node, `O(n + m)`, because they do not
depend on where `x` sits; an accepted move changes only `x`'s own cut. The list is touched only
when a move is accepted.

The issue text expected the delta to depend on the moved node's edges alone. It does not: the
delta of an edge `(x, y)` counts every other edge that starts between the old and the new slot,
which is a range sum over the whole list. The prefix arrays carry that sum.

Exactness and wrapping. `reduce` compares counts as `i32`. The recount adds queries with
`wrap_crossings`, so its result is the true count modulo 2^32. `apply_delta` returns
`old.wrapping_add(delta as i32)`, and `Z/2^32` is a group, so it is the same `i32` whenever `old`
is. Checked builds also assert that the mathematical total stays in `[0, i32::MAX]`, which is
exactly where the recount's `add_crossings` panics. The decisions in `reduce` (strict `<`,
restore, zero exit, ten passes) therefore see the same integers.

When the delta does not apply. `Delta::new` declines unless the list is a permutation of the
block's nodes and the block is strict and loop-free, with every edge endpoint in the list. Then
`reduce` runs the unchanged recount loop. Lists of 16 or fewer keep the recount scan in
production, as above. Any failed lookup inside a move (a `get` that returns `None`) falls back to
the recount for that move and for the rest of that node.

Oracle. `cfg(test)` and `circo-check` run the recount on every tentative move and assert it equals
the delta-derived count (`circo incremental crossing delta`), on lists of every length. The
recount's own indexed-versus-scan check still runs inside it. The recount also leaves
`EDGEORDER` and the half bitset as the scan does; the delta path does not touch them after the
initial count. Nothing reads them after `reduce_edge_crossings`: `blocktree` sets them earlier and
`count_all_crossings` resets them on entry.

Proof. `proofs/GvpProofs/Circo/CrossingDelta.lean` proves, over a model of the list as a position
function and the block as a symmetric irreflexive relation: the pair count equals the formula
above (`crossings_formula`); moving `x` gives `Kc + phi(c)` with `phi` as written here
(`crossings_move`); and the wrapped `i32` sum of old total and delta equals the wrapped recount
(`fast_eq_recount`, `wrap_foldl`). What the model leaves to tests: that C's loop on a
permutation computes the pair count (checked against the scan on every move in check mode, and by
the randomized tests), that `insert_nodelist` is the cut `rank(n) + j`, and that the prefix loops
in `Delta::prepare` compute `FP`, `DP`.

## Check mode and tests

`cfg(test)` and the `circo-check` layout feature run the retained bitset scan
on **every** recount and assert equality before `reduce` sees it. This includes
rejected moves. The reference resets edge orders itself, so the check starts
from the same inputs. The independent ordered half-set test also remains.

The tests include all permutations of sparse ids with a loop and isolated
node, 4,000 generated multigraph lists with repeated nodes, loops and parallel edges,
and checked/release overflow boundary tests. Four deliberate mutations run
through the same checking wrapper: dropping endpoint exclusions, and erasing
the retained first half instead of the closing half, erasing only the bit, and
erasing only the tree unit. Repeated-node fixtures force actual erasures. Each fails at the
per-count assertion, before any move can consume the wrong count.

Check builds also compare every individual suffix query with direct enumeration
of the current half bitset and endpoint predicate, before adding it. Comparing
only the final total could hide compensating query errors. The checking wrapper
also compares final orders and half membership with the retained reference scan.

All indexed arena/adjacency/half accesses are checked. A missing entry hands the
recount back to the reference, which resets its orders and half-set before use.
Fenwick updates reject zero and out-of-range keys before entering the loop;
prefix queries reject an absent top slot instead of skipping it. Suffix subtraction also declines on an inconsistent prefix. Weight arithmetic
wraps without changing valid counts: each weight and sum is bounded by the
number of allocated half keys. Thus a broken
index cannot hang at zero or silently truncate a prefix. Check-mode mismatch
messages print scalar counts, not arena-sized vectors.

CI enables `circo-check` in the existing mincross check-mode job, with a
20-minute timeout. A 3,000-seed simple-graph sweep in the CLI tests exercises
5..15 nodes, shuffled edge insertion and varying density, alongside the ordinary
corpus. A dedicated CLI regression pins the reviewed duplicate-list graph's
entire plain output to C Graphviz 16.1.0's bytes. Unit tests check actual erasure,
truncated capacity, zero keys, and all four mutations.

The fallback has a fault-injection test that declines after partial writes and
checks a nonzero reference count and final state. An explicit policy test uses
the production small-list selection and fails if the index is entered; check
policy still forces the index on the same input. The release wrapping kernel
is directly tested for overflow in debug CI too.

## C oracle limits on duplicate lists

Independent review compared 41,600 graphs with zero base/head differences,
including 200 duplicate-list cases. Of 5,376 C comparisons, 5,362 were
byte-identical; the other 14 aborted in C. `lib/circogen/circpos.c:315` sizes
`parents` by child count, but its loop at line 329 writes once per list
occurrence. Duplicate occurrences can therefore write past C's `parents` heap buffer
(14 of the 200 duplicate-list cases aborted under the portable libm preload). Those inputs
have no usable C oracle: their evidence is base equalling head, backed by the
per-query reference check, not a claim that C produced matching bytes.
