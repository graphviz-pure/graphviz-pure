# Exact circo edge-half recounts

C: `lib/circogen/blockpath.c:count_all_crossings`, `reduce`.
The retained bitset scan is `count_all_crossings_reference` in `blockpath.rs`.
The production counter replaces its enumeration with Fenwick suffix queries.
This is integer kernel restructuring (T8), with reused scratch (T9).

## Invariant and predicate

At the start of each **list occurrence**'s query phase, the bitset is exactly
the reference's set of `(edge id, half)` keys. The Fenwick tree contains one unit
at `EDGEORDER(e)` for **each set half**. Initially both are empty and every
block edge's order is zero. The list need not be a permutation: `LEAFONE` and
`LEAFTWO` paths can overlap, so a node can occur repeatedly. Opening orders
are positions in this list, 1 through L, not distinct-node ranks. Scratch is
therefore sized from the initial **list length**, not `subg.nnodes()`.
Each incident edge id occurs once in a node's adjacency, including a self
edge. Separate parallel edge ids remain separate entries.

Temporarily remove from the tree every set half of every incident edge. These
are exactly the halves whose head or tail equals the current node. The bitset
is unchanged. During all queries at this occurrence, the reference only erases
incident halves, which are already excluded. Consequently its qualifying set
is constant apart from the query threshold. Total tree weight minus its prefix
through `EDGEORDER(e)` counts exactly the strict `>` predicate, with neither
endpoint equal to the node. This argument also covers repeated occurrences.

Restore the removed units, then apply the reference erasures of exact half
keys, decrementing the tree **only when that bit was present**. At the other
endpoint the opposite key usually is absent, so the opened first half stays.
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

The initial list contains every block node at least once. A move removes at
most one occurrence and reinserts at most one. A node's multiplicity cannot
exceed its initial multiplicity (or one, if previously removed by a self move),
so the list cannot outgrow its initial length. Independently of this argument,
`list.len() >= tree.len()` declines to the reference rather than querying a
missing top slot. A production list of length at most 16 also uses the reference:
that avoids index maintenance on tiny blocks. Check builds still exercise the
index on these lists, so the sweep and mutation tests cover that implementation.

Every count consumed by `reduce` is therefore the same integer. Its node,
edge and before/after iteration orders, strict `<` acceptance, equal-count
rejection, restore, zero exit and ten-pass limit are unchanged. Induction on
these counts gives the same tentative moves, accepted moves and final list.

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
