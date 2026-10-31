# Exact circo edge-half recounts

C: `lib/circogen/blockpath.c:count_all_crossings`, `reduce`.
The retained bitset scan is `count_all_crossings_reference` in `blockpath.rs`.
The production counter replaces its enumeration with Fenwick suffix queries.
This is integer kernel restructuring (T8), with reused scratch (T9).

## Invariant and predicate

At the start of each node's query phase, the bitset is exactly the reference
set of `(edge id, half)` keys. The Fenwick tree contains one unit at
`EDGEORDER(e)` for **each set half**. Initially both are empty and every block
edge's order is zero. The list is a permutation of the block nodes; its orders
are 1 through n. Each incident edge id occurs once in that node's adjacency,
including a self edge. Separate parallel edge ids remain separate entries.

Temporarily remove from the tree every set half of every incident edge. These
are exactly the halves whose head or tail equals the current node. The bitset
is unchanged. During all queries at this node, the reference only erases
incident halves, which are already excluded. Consequently its qualifying set
is constant apart from the query threshold. Total tree weight minus its prefix
through `EDGEORDER(e)` counts exactly the strict `>` predicate, with neither
endpoint equal to the node. No chord-intersection interpretation is used.

Restore the removed units, then perform the reference's erasures of the exact
half keys, decrementing only if the bit was present. Open previously unordered
edges in the same second pass, setting their order and adding their exact key.
This reestablishes the invariant. In particular, closing the other half does
**not** erase the first half: that key continues contributing at later nodes.
Loops open once and their endpoint exclusions are handled identically. Edges
outside this block retain their orders and cannot enter its initially empty set.

## Integers, state and decisions

A tree weight and every prefix is bounded by the number of half keys, hence
fits u64 on supported targets (the bitset itself must fit memory). Prefix
subtraction is nonnegative. Scratch is the old bitset plus n+1 u64 entries in
one allocation; it is reset before every recount. Indexed work is O((n+m) log n),
plus clearing the existing arena-sized bitset, with no allocation during a
recount or tentative move.

The reference adds only positive units to an i32 total. Release addition is
modulo 2^32; `add_crossings` adds the same number modulo 2^32. In checked
builds it asserts before the nonnegative mathematical sum exceeds i32::MAX,
exactly when repeated reference increments would panic. For lists larger than
i32::MAX the retained reference scan runs instead,
preserving the opening-order counter's overflow behavior too. Indexed lists
have positive opening orders that fit i32. No float is changed.
The final edge orders and half bitset are the reference's. The index is private.

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
node, 4,000 generated multigraph permutations with loops and parallel edges,
and checked/release overflow boundary tests. Two deliberate mutations run
through the same checking wrapper: dropping endpoint exclusions, and erasing
the retained first half instead of the closing half. Each must fail at the
per-count assertion, before any move can consume the wrong count.

Check builds also compare every individual suffix query with direct enumeration
of the current half bitset and endpoint predicate, before adding it. Comparing
only the final total could hide compensating query errors. The checking wrapper
also compares final orders and half membership with the retained reference scan.

All indexed arena/adjacency/half accesses are checked. A missing entry hands the
recount back to the reference, which resets its orders and half-set before use.
The private Fenwick traversal uses checked slice access. Its wrapping u64/usize
operations are exact under the invariant and the allocated-size bounds: no
weight exceeds the number of stored halves, and indices are at most n+1 with
n <= i32::MAX. They introduce no new overflow panic sites.

CI enables `circo-check` in the existing mincross check-mode job, so every
corpus recount is checked there as well as the unit-test recounts.
