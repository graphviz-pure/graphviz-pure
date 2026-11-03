# dot position: `canreach` keeps what it found

Issue #735; measurements are in its pull request. Proof: `proofs/GvpProofs/Dot/CanReach.lean`.

## The loop

`make_lr_constraints` (`position.c`) walks each rank, left to right, and adds auxiliary edges. For
a node `u` that carries a flat edge's label (`ND_alg(u)`) it asks `canreach(tail, head)` twice,
and adds the edge that closes a cycle only when the answer is no. C's `go` is a depth-first
search over `ND_out` with nothing saved. The port searched with a fresh `HashSet` and a fresh stack
for every query.

## What it does now

`dotgen/reach.rs` keeps, for each source asked about, the set of nodes it has found and, for
each of them, how many of its out-edges it followed. A query follows only the out-edges appended
since the last one, from every member, into new members, and stops once the target is a member.
Membership is a bit per node instead of a hash.

The booleans are the same, whatever edges are added between two queries:

- `fast_edge` only pushes onto `out`, and nothing removes an edge while `make_lr_constraints` runs,
  so the edges a source followed are a prefix of the ones there are now;
- a source's set holds exactly what it reaches over the edges followed, and a node that is reachable
  stays reachable when edges are added, so closing the set under the new edges gives what a search
  from scratch finds (`resume_exact`);
- the search from scratch is reachability (`dfs_iff`).

The queries are asked in C's order; only the cost of answering changes.

## Check

Under `cfg(test)` and the `position-check` feature every answer is compared with the search from
scratch (`reach::canreach_fresh`), which is C's search. CI runs the feature in the mincross-check job.
`reach.rs` also tests random interleavings of edge additions and queries against it.

## Not covered

The set's memory is capped (`MAX_WORDS`): a set is charged its full possible size when admitted,
so growth after admission stays within the charge; past the cap a source without a set is
searched in one scratch set, emptied afterwards. (Review of the first version found the charge
was the size at admission, so sets that grew later exceeded the cap; a test now grows 8,192 sets
after admission and checks the words actually allocated.) The Lean model has no cap, and no `scanned` counters: it states
the closure the counters compute. The counters, the cap and the early exit are checked by the tests
above and the feature, not proved.
