# Where C crashes: fixing panics at the cause

The port follows Graphviz's C function by function. Where the C has undefined behaviour (a
NULL dereference, an out-of-bounds read, a failed `assert`, an unbounded loop), the literal
translation becomes a Rust panic at the same spot: an `.unwrap()` on a pointer C never
checked, a `[i]` on an index C never bounds, an integer cast or add that overflows. The fuzzers
find these sites one at a time, and fixing them one at a time does not converge: in the
October 2026 p1 round, 14 single-site fixes turned up 8 new crashes next to them.

So a crash is fixed at its cause, not at the line that panics.

## Rules

- **The port must not panic, hang or run out of memory**, whatever C does. Where C produces
  output, the port matches it. Where C crashes, asserts or hangs, there is no output to match,
  and the port picks the smallest defensible behaviour: what C does on the nearest input it
  handles, or an error or warning and a clean exit. The comment names the C function and says
  that C crashes there.
- **Find the first point where the state goes wrong.** Before guarding the panicking line, ask
  which earlier step produced the bad state, and whether C reported it there (a warning such as
  `trouble in init_rank`, a failed return value it ignored). Fix it at that point, so every later
  site that depends on it is covered.
- **Group crashes by cause, not by panic site.** When filing or triaging, check whether open
  issues share the cause (the same diagnostic before the crash, the same input feature, the same
  function). One fix and one pull request closes the group.
- **Compare how C and Rust fail.** C keeps running on corrupt state and crashes later, or not at
  all; Rust stops at the first unchecked access. A Rust panic that C "survives" is usually C
  reading garbage, and the fix is still at the cause.
- **Report C bugs upstream.** If the cause is a bug in C (it crashes, hangs or reads out of
  bounds), label the issue `upstream-c` and note the C location, so it can be reported to
  Graphviz.
- **Port C's own checks, don't replace them.** Where C detects a bad state itself (an overflow
  check such as `sadd_overflow` or `INT_MAX / t < w`, an internal-invariant message such as
  `add_tree_edge: missing tree edge`) and stops or returns an error, the port does the same, with
  C's message. `wrapping_*` arithmetic is right only where C's arithmetic has no check of its own.
- **A limit C doesn't have needs a measurement.** A budget the port adds against a hang or
  running out of memory sits at or beyond the point where C stops being practical, measured in
  time and memory on the reference build, and its comment gives the measurement (#250: packing's
  cell budget refuses an input C packs in 1 s).

## Known causes

These causes account for most crashes found so far. A new crash in one of these areas is
fixed in that cause's shared mechanism, not with a new local guard.

| Cause | How it fails | Fix in the port |
|---|---|---|
| **dot keeps going after a failed ranking** | Network simplex prints `trouble in init_rank` and returns unnormalised ranks; C carries on and crashes later in mincross, position or splines (#114, #232, #233). | Stop the layout where a cluster's ranking fails, where the root's ranks start below 0 (#357), and where C's `install_in_rank` finds that the ranks do not fit the rank arrays: an error and exit status 1. A failed ranking of the root alone is not stopped; see [below](#dot-after-a-failed-ranking). |
| **dot's cluster membership** | A rank set or an earlier cluster takes a node from a cluster after the cluster was ranked (`x was already in a rankset, deleted from cluster G`). The cluster keeps its rank range, so it can have ranks with no node, or no nodes at all; C crashes later in `build_ranks`, `furthestnode`, `rank_box` (#114, #231). | Stop the layout where C dereferences an emptied cluster's NULL node list or a root rank's NULL first node, and where C's own assertions in `neighbor` and `flat_reorder` fail. Ranks with no node, which C often draws through, are kept; see [below](#dot-cluster-membership). |
| **Extreme coordinates** | NaN, infinity or huge pinned positions flow into every engine and overflow at the first integer cast or recursion (#191, #196, #201, #227, #228, #229, #239, #240). | Keep C's positions, which C lays out wherever it finishes; bound the algorithms whose limits depend on magnitude (see [below](#extreme-coordinates)). |
| **The ortho router** | Many internal invariants that C asserts or dereferences without checking (#186, #199, #202, #203). C falls back to straight edges when routing fails. | Ortho returns `Result` throughout; the entry point falls back to straight edges, as C does. |
| **C's own overflow checks dropped** | C checks an integer sum for overflow and exits with an error (`virtual_weight`, `x_cutval`); the port multiplied unchecked or wrapped, so it drew a layout from wrapped weights, or panicked in a build with overflow checks (#252). Unchecked `int` sums on huge `weight`, `minlen`, `ranksep`, `nodesep` or `margin` panicked the same way (#271). | Each check is ported with C's message as a layout error. Where C has no check the port wraps, and where the wrapped weights crash C (`update` with no entering edge) the port fails the layout; see [below](#cs-own-overflow-checks). |
| **`unwrap` and indexing on data C trusts** | The mechanism under all of the above. | The [panic-site ratchet](#the-panic-site-ratchet) maps the remaining sites and stops the count from growing; new code returns an error instead. |

The fix for each cause records here the issues it closed and the behaviour it chose. Narrower
causes, each confined to one function, have their own sections after these. Output that
depends on C's heap addresses without crashing is not covered here: VPSC's ties (#225) are in
`neato.md` ("Ties by address").

## dot after a failed ranking

Closed: #232, #233, #357 (and #114 earlier, by #208).

`trouble in init_rank` means the rank constraints contain a cycle, usually rank sets that
contradict each other (`rank=min` and `rank=max` on one node) or a node that is both in a
cluster and in a rank set. On its own it does not stop C, and it does not stop the port:
over 3000 generated inputs, C printed it on 182 and still drew 155 of them, the same way
on every run, and the port's output equals C's on all 155. Stopping the layout at the
message would turn those drawings into errors, so the port stops only where the failed
ranking leaves state that C cannot use:

- **A cluster's own ranking** (`collapse_cluster` → `dot1_rank`). The cluster's ranks
  normally start at 0, and `cluster_leader` asserts that some node is on rank 0
  (`rank.c:316`). After a failed ranking there may be none. The port fails the layout there
  (`cluster_leader: the ranking of cluster X failed: none of its nodes is on rank 0`), at
  any cluster depth (#233).
- **`install_in_rank`** (`mincross.c`), C's own check that the ranks fit the rank arrays
  built from them. The port now has all of C's checks, in C's order. Where C prints an
  error (lines 1158, 1179, 1184, 1190), the port prints the same error and abandons the
  layout as C does; the output is C's (#232, input A). Where C fails
  `assert(GD_rank(g)[r].n <= GD_rank(g)[r].an)` after writing past the array (line 1165),
  the port fails the layout before the write (`install_in_rank: rank R of G is full`)
  (#232, input B). Past the root's first component, C's line-1190 bound uses `>`, so one
  store can land one slot past the array unreported; the port grows the buffer there and
  goes on, as C does.

- **The root's ranks start below 0** (#357). Network simplex normalises each
  component's ranks to start at 0 in `scan_and_normalize`, but returns before it when
  `nslimit1` allows no iteration (`maxiter = nslimit1 × nodes` below 1, `ns.c:984`), and it
  prints nothing then. newrank's `readout_levels` shifts a single component's ranks only
  when they start above 0. From `init_mincross` on, C indexes its rank arrays with these
  ranks: `fill_ranks` and `allocate_ranks` write before the arrays, `make_chain` takes rank
  -1 for the label rank of an edge with no label (its "no label" value) and `label_vnode`
  reads the NULL label, and `build_skeleton` reads before a cluster's rank leaders. The port
  fails the layout at the start of `init_mincross` (`dot_mincross: the ranking of G stopped
  before normalising its ranks, which start at -1, below 0`). Ranks that start above 0
  are kept: C draws most of them. Measured on seeds 1–25000 of `fuzz-gen-ranks.py`, where
  ranking stopped early on 2,460 inputs: the root's ranks started below 0 on 51, and C
  crashed on all 51. They started above 0 on 126: C drew 27 and stopped cleanly on 9, the
  port's output is C's on all 36, and nothing changed there. On seeds 1–50000 the check
  changes 101 inputs, and C crashes or aborts on 97 of them. On the other 4, C exits 1
  (`gv_calloc` refusing a size computed from a negative rank, or `install_in_rank`'s own
  checks reading rank -1), and on the same input C can also crash: four runs of one of them
  gave SIGBUS, SIGSEGV twice, and line 1190's error with a drawing. What C does there
  depends on the memory before its arrays. The port exits 1 there, with no output.

Most of C's crashes after `trouble in init_rank` come from a related cause that is not in
the ranking itself: a cluster keeps the rank range of nodes that `mark_clusters` later
removes from it (`x was already in a rankset, deleted from cluster G`), so some of its
ranks have no node. `rec_reset_vlists` then follows a stale rank leader (C segfaults in
`furthestnode`, and on some inputs crashes or not depending on the run), and mincross can
leave a node unplaced. The port reaches the same state; `make_regular_edge`'s rank check
(#208) still stops the layout there, so it stays.

Not this cause, although they often follow the same inputs: #230 and #241 (`flat_node`
reads the rank above the top one when no rank was made for it), #231 (a cluster left with
no nodes), #234 (`conc_slope` on a one-sided merge node).

## dot cluster membership

Closed: #231 (with its `clust_ht` and draw-instead-of-crash variants), #358. #114's
remaining routes are this cause too.

dot ranks each cluster before the rest of the graph. A node can be taken from a cluster
after that: `mark_clusters` (at the parent's `class1`) deletes a cluster node that a later
rank set also claims, and newrank's `set_parent` registers a cluster before `node_induce`
deletes the nodes an earlier sibling cluster has. The cluster's rank range and rank
leaders still describe the nodes it had. C does not recompute them, and its drawing
depends on that: on a rank with no node left, `save_vlist` saves the root's node just right
of the cluster, and the cluster box reaches out to it. That output is C's on every run, so
the port keeps the stale range too. It stops only where C's behaviour cannot depend on
anything but a NULL pointer:

- **A cluster with no nodes left.** C removes one only among the root's clusters (the check
  at the top of `dot_mincross`, `removing empty cluster`). A nested one is kept: its rank
  leaders are installed in the parent's ranks, which can fail first with
  `install_in_rank`'s own `an = 0` error (line 1158, as in C), and otherwise
  `expand_cluster` → `build_ranks` dereferences its NULL node list (`mincross.c:1227`).
  The port fails the layout there:
  `expand_cluster: cluster X has no nodes left (a rank set or an earlier cluster took them)`.
  Dropping the cluster earlier would change the 60 inputs in the generated corpus where C
  stops cleanly at line 1158.

- **A root rank with no node** (#358). A rank that the stale range keeps in the root, with
  no node and no edge through it, has a NULL `v[0]`. `set_ycoords` reads `v[0]` of the
  bottom rank, and of each rank below a rank with nodes, without checking (`position.c:777`,
  `:783`, and `:802` and `:816` when the ranks are equally spaced), and C crashes. The port
  read 0 there and went on, so it drew a layout C never draws or panicked later (a self-loop
  reading the rank above, `dotsplines.rs`). It now fails the layout where C reads the NULL
  (`set_ycoords: rank R of G has no node`). The same state comes from `nslimit1` (#357)
  with ranks that start at 0 but have gaps. C crashed on all 30 inputs #358's probe
  found (seeds 1–25000), and on all 70 whose output the check changes in seeds 1–50000.
- **`neighbor`'s assertion.** C asserts that the node next to `v` in the root's rank has an
  order on the side the walk is going (`mincross.c:880`). It fails on a freed saved leader
  (below) whose stale order no longer matches the slot it was read from. Without the check,
  the port's `furthestnode` came back to a node it had visited and never ended (#358). The
  port fails the layout where the assertion fails. C asserts there, or, reading the freed
  node, crashes first.
- **`flat_reorder`'s assertions.** A cycle of constraining flat edges can be left on a rank:
  every node of it has a flat in-edge, so the topological sort in `temprank` never reaches
  it and holds fewer nodes than the rank. C's `LIST_GET` asserts (`mincross.c:1376`). The
  port used to stop copying there and leave the rank half rewritten, with some nodes in two
  slots, and `transpose` never ended (#358). It now fails the layout there, and at the
  function's two other assertions: `postorder`'s rank, and that an edge it reverses is not
  constraining (`mincross.c:1387`). That last one is reached without any membership change
  too, with newrank and `ordering=in` (C aborts there on every run).

Two later points look like better places to stop but are not, because C gets past them
on some inputs with output that is the same on every run:

- **A saved leader that was freed.** When the node `save_vlist` saves for a rank with no
  node is the rank leader of a cluster expanded later, `remove_rankleaders` frees it, and
  `rec_reset_vlists` reads it in `furthestnode`. That is a use after free. C segfaults on
  most such inputs (#114's repro), but on some it finishes with the same output on every
  run: the freed node's fields are still intact, or its memory has been reused by a live
  node. The port keeps the freed leader's fields, as macOS's allocator does, and goes on
  unless they break `neighbor`'s assertion (above);
  it matches C where those fields are what C reads, and still differs where the memory
  was reused (two generated inputs, where C draws and the port stops with
  `trouble in init_rank`, as before this change).
- **A node `merge2` drops.** `allocate_ranks` counts a slot per node and one spare per
  rank. A cluster rank with no node still gets a skeleton rank leader, which uses the spare;
  with two of them, a later component stores one slot past the rank array, which C's
  line-1190 check (`>`) lets through, and `merge2` cuts the rank at `an`, so the node there is
  never placed. C draws 7 generated inputs that pass through this, the same way on every
  run.

Where the unplaced node or the freed leader's stale state is read as a rank,
`make_regular_edge`'s rank check (#208) stops the layout, where C reads `GD_rank` out of
bounds in `rank_box`. Two inputs where C reads out of bounds there without crashing fail
in the port.

Measured against C and against the port before this change, on 33,686 inputs (Thread A's
corpora, #208's review hits and 4,000 new generated ones, none with `pack`): no output
changes on an input C draws or stops cleanly on, the same way on two runs. Only inputs
where C crashes change:
804 now stop at the `expand_cluster` error, of which the port drew 502 before, panicked on
54, and failed later on 248. The remaining panics on these corpora belong to #230 and
#234. With `pack=true`, where the port's components already differ from C's (#101), the
check also fires on some inputs where C stops cleanly at `install_in_rank` line 1158;
the port differed from C there before too.

## The panic-site ratchet

`scripts/panic-sites.sh` counts, for each source file of `graphviz-pure-layout` and
`graphviz-pure-geom`, the library code (not tests) that four clippy lints flag:

| Column | Lint | Site |
|---|---|---|
| unwrap | `clippy::unwrap_used` | `.unwrap()` |
| expect | `clippy::expect_used` | `.expect(..)` |
| index | `clippy::indexing_slicing` | `v[i]` or `v[a..b]` that can go out of bounds |
| arith | `clippy::arithmetic_side_effects` | integer `+ - * / % << >>` that can overflow or divide by zero |

The lints are passed to clippy for this one run; they are not turned on for the workspace,
because most of these sites are faithful C and correct for every input C handles. The counts are
checked in as `tests/panic-sites.tsv`. CI runs the script after clippy, and it fails when one
file's count of one kind goes up, or when a file that is not listed has any site. It also fails
when the count itself looks broken: a kind with no site at all, or a lint clippy does not know.
The lints are forced on, so an `allow` in the source does not hide a site. A count going down
passes; `scripts/panic-sites.sh --update` rewrites the baseline to lock it in. A pull request that
raises a count on purpose (a new port of C code that indexes the way C does, a new clippy release
that counts differently) refreshes the baseline with `--update` and says why.

When the count goes up, prefer what C does at that point: if C checks a bound or a NULL, port the
check; if C reads past the end or dereferences NULL, return an error from the cause.

At the start (2026-10-22) the two crates had 10,615 sites: 278 unwrap, 43 expect, 8,146 index and
2,148 arith. These ten files have the most, and are where to look for the next causes:

| File (under `crates/graphviz-pure-`) | unwrap | expect | index | arith | total |
|---|--:|--:|--:|--:|--:|
| `layout/src/dotgen/dotsplines.rs` | 32 | 5 | 478 | 61 | 576 |
| `layout/src/dotgen/mincross.rs` | 32 | 0 | 390 | 104 | 526 |
| `layout/src/dotgen/position.rs` | 33 | 0 | 302 | 39 | 374 |
| `layout/src/cutil.rs` | 1 | 0 | 185 | 179 | 365 |
| `layout/src/neatogen/digcola.rs` | 3 | 0 | 265 | 65 | 333 |
| `layout/src/neatogen/voronoi.rs` | 22 | 0 | 250 | 41 | 313 |
| `layout/src/ortho/trapezoid.rs` | 0 | 0 | 276 | 10 | 286 |
| `layout/src/dotgen/rank.rs` | 11 | 0 | 244 | 19 | 274 |
| `layout/src/neatogen/multispline.rs` | 4 | 0 | 179 | 81 | 264 |
| `layout/src/sfdpgen/spring_electrical.rs` | 0 | 3 | 154 | 103 | 260 |

The script prints the current table, most sites first. An unwrap is the cheapest to trace: it
names the value C assumed was there.

### What the unwraps assume

Grouped by the value C assumed was there, most of the unwraps in the three densest dot files fall
into four classes. Each class is one candidate cause: a fuzzer that targets the assumption finds its
crashes together. Counts are from master on 2026-10-22, when the whole baseline was 10,184 sites.

| Class | Sites | C assumes | Broken by |
|---|--:|---|---|
| `rv(…).unwrap()` (`GD_rank(g)[r].v[i]`) | 57 (mincross, position, dotsplines) | every rank has a node in that slot | ranks left empty by a failed root ranking, an emptied cluster (#231), a flat label on the top rank (#241) |
| `boxes.last().unwrap()` (`P->boxes[P->nbox - 1]`) | 14 (dotsplines) | `beginpath`/`endpath` made a box | ports and clipping on zero-size or shapeless nodes, samehead/sametail, self-loops |
| `to_orig.unwrap()`, `label.expect`, `rep.unwrap()` | 19 | a virtual edge chain leads back to its real edge | chains rewired by `class2`, `flat_rev`, `concentrate` (#234) |
| `.ln/.rn.unwrap()` (`GD_ln`, `GD_rn`) | 11 (position) | `make_lrvn` ran for every cluster | clusters emptied after ranking ([cluster membership](#dot-cluster-membership)) |

Two causes don't show up as unwraps. One is the arith column: integer attributes at extreme values
(`weight`, `minlen`, `nslimit`, `mclimit`, `searchsize`, `peripheries`, `sides`), the
[extreme coordinates](#extreme-coordinates) cause for attributes that are not positions. The other
is C's checks that the port dropped, as in #252. C prints a message where it notices a bad state,
so a C diagnostic with no counterpart in the port is where to look.

## The ortho router: one failure boundary

Closed: #186, #199, #202 and #203 (round 1, fixed one site at a time, now covered by the boundary), and #228 and #229.

`lib/ortho` asserts many of its invariants: `LIST_GET` bounds-checks every index, and `chanSearch` asserts its lookups. It trusts the rest without checking: raw `N_DAD` and `cells[]` pointers, and `int` distances. In the port, every function in `crates/graphviz-pure-layout/src/ortho/` returns `Result<_, Fail>` (`ortho/fail.rs`), and every index that comes from data goes through a checked accessor. `ortho_edges_of` (`orthoEdges`) handles each failure in one place, the way C handles the failure it does detect at that stage:

| Stage | C | Port |
|---|---|---|
| Building the maze (`mkMaze`: partition, trapezoidation, `markSmall`, `chkSgraph`) | Returns NULL where it detects a failure; elsewhere it asserts or segfaults | Behaves as if C had returned NULL: `orthoEdges` returns -1. dot abandons the layout and draws only the nodes (#203, #200). The neato-family callers draw straight edges. |
| Routing (`shortPath`, `convertSPtoRoute`, channels, tracks) | `goto orthofinish` where it detects a failure (heap overflow, incomparable segments, each with C's message); elsewhere it asserts, segfaults or loops | Behaves as `goto orthofinish`: no edge is routed and `orthoEdges` returns 0. A warning names the failure (`... - edges not routed orthogonally`). The routes are installed only once every edge has been routed. |

Inputs that C routes never reach a failure, so their output is unchanged.

- **#229:** pins 1e12 inches apart make `shortPath`'s distance `-(N_VAL + E_WT)` too large for an `int`. That is undefined behaviour in C, which then fails `PQcheck`. The port rejects any distance outside `(INT_MIN, 0]`. A positive distance would make C's `PQupheap` climb past its guard forever.
- **#228:** NaN pins leave no trapezoid inside the polygon, and C's `monotonate_trapezoids` reads one past the end of `tr`. The port fails the maze at that read.
- Loops that C would run forever on broken links are bounded by the size of the structure: stale parent chains, trapezoid walks and maze walks. The three recursions (`locate_endpoint`, `traverse_polygon` and `DFS_visit`) run on an explicit stack, in C's order.
- The boundary does not depend on positions being validated where they are parsed (the "Extreme coordinates" cause): it holds whatever the input.

## Flat edge labels on the top rank

Closed: #230, #241.

A labelled flat edge that is not adjacent to its head gets a label node on the rank above
(`flat_node`). On rank 0, `flat_edges` first makes a rank -1 for it (`abomination`), but C's
test looks only at `ND_flat_in`. A labelled edge that is not its class's representative
sits in `ND_other` instead: a parallel `a -> b` behind an unlabelled one (`class2`'s
`merge_oneway`), or a `b -> a` merged into `a -> b` by `flat_rev`. The `ND_other` loop
then calls `flat_node` for it, and C reads `GD_rank(g)[-1]` and segfaults. The port makes
rank -1 for those edges as well, so the edge is handled as it is on any lower rank: C
itself, on the same edges one rank down, places the label node and then fails to route
the edge (`lost a b edge`, exit 1). On rank 0 the port prints the same errors. When the
labelled edge is the representative, C makes rank -1 itself, and nothing changes.

`flat_edges` also assumes the top rank is rank 0, which it is once ranking has
normalised the ranks (`abomination` asserts it). After a failed ranking of the root
(`trouble in init_rank`, which does not stop the layout; see above), nothing normalised
them and the top rank is `GD_minrank(g)` > 0. C's ranks above it were never allocated, so
`flat_node` on it reads a NULL `v` and segfaults (#241). The port runs the test, and makes
the new rank, at the top rank. When the top rank is 0 this is C's code.

## Adjacent flat edges the aux layout leaves without a spline

Closed: #341.

`make_flat_adj_edges` (`lib/dotgen/dotsplines.c`) lays out flat edges between adjacent
nodes, where one has a port, as a separate graph with the rank direction turned, and copies
each edge's spline back with `auxbz = ED_spl(auxe)->list` (line 1260), unchecked. The aux
layout leaves an edge no spline in two ways: its routing fails (`Pshortestpath failed`,
`illegal values of prev`; the aux graph's `dotneato_postprocess` then reports `lost a b
edge`), which `pack` makes happen on some inputs C draws without it, or concentrate merges
it into another aux edge. C then dereferences the NULL spline and crashes, on every run.

The port stops the layout there, with an error and no output
(`make_flat_adj_edges: the layout of the adjacent flat edges gave edge a -> b no spline`).
Its stderr up to that line is C's up to the crash. Before, it skipped the edge and went on:
it reported the edge lost a second time (from the main graph's postprocess, which C never
reaches), or with concentrate drew the graph without that edge. Measured on 3,000 generated
inputs with adjacent flat edges, ports, labels, `concentrate`, `pack`, `rankdir` and
`splines`: C crashes on 214, and the stop fires on those 214 and on no other input. On the
other 2,786 the output and stderr are unchanged, and equal to C's. On the 214, the port
used to draw 185 concentrate inputs without the merged edge (174 with exit 0, 11 with exit 1
after a routing error), and on the other 29 (20 plain, 9 under `pack`) to report an edge
lost twice and exit 1.

So 174 concentrate inputs that drew with exit 0 now fail with exit 1. That is the trade-off.
The drawing they had is not one C makes on a nearby input: with the two edges of
`a -> b [tailport=ne]; a -> b` in the other order, C draws both, where the port dropped one.
Nor would giving the lost edge the spline of the aux edge it was merged into be C's: nothing
in C's copy loop copies one edge's spline to another. The port stops at C's dereference, as it does where C
dereferences the NULL nodes of an empty cluster or rank (#357, #358).

## clip_and_install's crossed walks

Closed: #359.

`clip_and_install` (`splines.c`) walks the spline's segments from the tail while their
points test inside the tail node, and from the head while they test inside the head node.
When every point tests inside (zero-size `shape=plain` nodes with `fixedsize=true` or an
empty label, ends at the same place after a failed ranking), the tail-side walks pass the
head-side ones. One segment apart (`start == end + 3`), C installs a one-point bezier,
which `bezier_bb` accepts, and draws it; the port does the same. Further apart, C's `size_t`
`end - start + 4` wraps, and C fails `bezier_bb`'s assertion (`emit.c:4082`), crashes where
the bezier is read (`map_edge`, `polylineMidpoint`), or, through `makeCompoundEdge`'s
`size - 1`, has `gv_calloc` refuse the size. The port installed an empty bezier and panicked
in emit, xlabels, the edge midpoint and compound, or drew an edge `pos` with no points. It
now keeps the one segment where the walks meet (`start = end`), the last state in which C
installs a whole segment, in every engine that uses `clip_and_install`. Over 40,000 `fuzz-gen-endpoints.py`
and 50,000 `fuzz-gen-ranks.py` inputs, plus hand-written ones in neato, fdp, circo and
osage, every input this changes is one C crashes on, except one: there `makeCompoundEdge`
gives `gv_calloc` the wrapped size and C exits 1 with no output, and the port draws.

## Record ports in no field: dot's uninitialised path ends

Closed: #360.

At the record end of a `samehead`/`sametail` edge, the port point can lie in no field of
the record. `record_path` (`shapes.c:3793`) then returns the side mask without writing a
box, so `beginpath` and `endpath` keep whatever `endp->boxes` and `endp->boxn` held. In
`make_regular_edge`, `make_flat_edge`, `make_flat_labeled_edge` and
`make_flat_bottom_edges` (`dotsplines.c:1713`, `:1515`, `:1326`, `:1432`) that is
`pathend_t tend, hend`, declared without an initialiser. In practice it holds what the
previous call of the same function left there, on every run and on both C 16.1.0 builds
measured (a packaged build and one built from source). The port keeps each function's last
`tend`/`hend` (`StaleEnds` in `dotsplines.rs`) and hands their boxes to `beginpath` and
`endpath` as `kept`. The shared code only installs boxes it is given; the reuse lives in
dot's routers.

How far that model reaches was measured, not assumed:

- **Other routers in between.** `make_regular_edge` and `make_flat_edge` are called from
  `dot_splines_`, `make_flat_labeled_edge` and `make_flat_bottom_edges` from inside
  `make_flat_edge`, one frame deeper, so their frames can overlap. `edgecmp` sorts self loops
  and flat edges before regular ones, so after the first `make_regular_edge` call no other
  router runs: none did on 52,000 generated inputs. Between flat calls, another flat
  router, and once the graph `make_flat_adj_edges` lays out, did run; where C drew, the
  port's one slot per function matched it every time.
- **Earlier graphs and pack components.** C does not reuse them. On 291 files of two
  graphs, where the second graph's first router call misses, carrying the first graph's
  ends matched C on no file where fresh slots did not; fresh slots matched on 8 where
  carrying did not. With `pack=true` and a simple first component, carrying matched no
  more drawings than fresh slots (21 each, of 42). Each layout the port runs separately
  (each graph, each `pack` component, the graph `make_flat_adj_edges` lays out) starts with
  no ends.

Where a function has no previous call, C reads unrelated stack memory. Its output then
differs between the two builds, and C often crashes or gives different results from run
to run. The port uses the node box there, as `beginpath` does when the shape has no port
boxes. Each miss in a row adds a box to the kept ones (at most 4 seen). C's array holds
20, and C writes past it only when the box after the miss is not empty. The port keeps
the 20 boxes when that box is empty, and fails the layout before the write otherwise
(`dot_splines: an edge end already holds the 20 boxes C has room for`). No input reached
that.

Measured on 40,000 `fuzz-gen-endpoints.py` inputs (seeds 1–40000), where the port reaches
this case on 213. On 5 of them C's output varies between runs. Of the other 208:

| C | Inputs | Port's output equals C's before | after |
|---|--:|--:|--:|
| draws | 74 | 5 | 70 |
| exits 1 | 98 | 13 | 94 |
| crashes | 36 | n/a | n/a |

The 4 drawings that still differ all reach the case in a function's first call. On 4,000
generated samehead/sametail × record × `rankdir` inputs, C draws 2,816 the same way on two
runs. The port matched C on 2,771 of them before and matches on 2,806 now. The 10 that
still differ also reach the case in the first call. No input that matched C before differs
now. The port draws or exits 1 on every input where C crashes, as before.

## Concentrated cluster edges

Closed: #234.

With `concentrate=true`, an edge into or out of a cluster whose ports differ from a
parallel edge's shares that edge's virtual chain (`interclrep`) but is routed on its own,
through `make_regular_edge`'s `hackflag` branch. That branch follows the chain as it was
before `dot_concentrate` rewired it, and it goes wrong in two ways:

- **The chain now ends at a concentrator.** The walk stops there, but C
  (`dotsplines.c:1845`) passes the concentrator's merge flag to `endpath` together with the
  original edge, whose head is the real node further on. `conc_slope` then averages the real
  node's in- and out-edges. If the node has both, C draws, and the port draws the same way.
  If one side is empty, C dereferences NULL. The port then ends the path with the real
  node's own merge flag (false), as for an edge that was not concentrated.
- **`mergevirtual` deleted the node the chain ends at.** In the up pass, the main edge's
  `ED_to_virt` chain can still end at a deleted node, which has no edges left, and C segfaults in `straight_len`.
  `mergevirtual` now records the node it merged each deleted node into, and the port routes
  the edge from there, as the chain's own edges are.

Both change output only where C crashes. (With `splines=line`, C survives the deleted-node
case because `makeLineEdge` never reads the head; the redirect changes nothing there either.)

## dot compound edges: an lhead clip with no arrow

Closed: #490.

`makeCompoundEdge` (`lib/dotgen/compound.c`) clips an `lhead` spline at the head cluster's box.
When the spline's first control point is already inside the box (`splines=curved` routing a
back edge, in a graph that also has the edge in the other direction), it rebuilds the spline from the tail
arrow's tip `bez->sp` and asserts `bez->sflag` (`compound.c:324`); with no tail arrow, `sp`
was never set. When the spline does not cross the box at all, it moves the head arrow's tip
and asserts `bez->eflag` (`compound.c:342`, `dir=back`). C aborts on both. The `ltail` branch
of the same function tests the flags instead: with no arrow it leaves the end as it was
(`if (!fixed)`, "something went wrong, use original tail"). The port does the same at the
head, so the edge is C's drawing of the same graph without `lhead`. Before, it bent the
spline toward (0, 0), the unset `sp`, so it began away from its tail node. `dir=both` is
unchanged and matches C.

## newrank with packmode=array: dot_rank's crash

Closed: #427.

`is_a_cluster(g)` (`lib/common/utils.c:697`) returns true for the cgraph root
(`g == g->root`), for a subgraph whose name starts with `cluster`, and when `g` has
`cluster=true`. In an ordinary `newrank=true` layout, `dot2_rank(g)` (`lib/dotgen/rank.c:1095`)
calls `compile_samerank(g, 0)` on the root, so `is_a_cluster(g)` is true at the top call: the
root sits at `GD_level(g) = 0` with `GD_parent(g) = NULL`, every top-level cluster gets
`GD_level = 1` and `GD_parent = g` (`set_parent` registers it in `GD_clust(g)`), and every
node outside a cluster gets `ND_clust(n) = g`. Every cluster chain and every node's
`ND_clust` therefore ends at `g`, and `dot_lca(c0, c1)` (`rank.c:708-717`) reads
`GD_level(c0)` and `GD_level(c1)` without checking for NULL.

When `doDot` (`lib/dotgen/dotinit.c:475-483`) lays out multiple `cccomps` components
(`pack` or `packmode`, `ncc > 1` and no `ratio`), each component `sg` is a `_cc_<i>`
subgraph of the root (`sg != sg->root`). Unless the root has `cluster=true`,
`is_a_cluster(sg)` is false. In `compile_samerank(sg, 0)` (`rank.c:636-670`), `clust` stays
NULL: each top-level cluster projection (`projectG`) gets `GD_level = 0` and
`GD_parent = NULL` instead of calling `set_parent`, and nodes outside any cluster keep
`ND_clust(n) = NULL`. In `compile_edges` (`rank.c:830`), any constraint edge whose ends are
not in the same cluster or rank-set representative calls `is_internal_to_cluster(e)` →
`dot_lca(ct, ch)`, which dereferences NULL (`GD_level(c0)` or `GD_level(c1)`) when one end
is in a cluster and the other is outside (`ct == NULL` or `ch == NULL`), or after one step
when the ends are in two different top-level clusters (`GD_parent` is NULL at `GD_level == 0`).
(`rank=same` is not needed to trigger it: the minimal form
`graph{packmode=array; newrank=true; subgraph cluster_c{a}; f0--g0; z--a}` crashes C at the
same dereference.)

The port keeps `is_a_cluster(0) == false` on a pack component (`dotgen/utils.rs`), so when
C does not reach `dot_lca` (every edge of the component has both ends in the same cluster or
outside any cluster) the component is laid out just as in C (including `GD_n_cluster(sg) == 0`,
no cluster box or margin). In `dot_lca` (`dotgen/rank.rs`), the port gives `None` a sentinel
level of `-1`, below `GD_level == 0`, so `None` plays the root cluster: any `Some(c)` at
level `>= 0` steps to its `parent` until both reach `None`, and `is_internal_to_cluster`
gives the answers C gives when the root is a cluster (normal `newrank` layout, where the
root is a level-0 cluster: `true` when one end is `None` so `dot_lca` returns `None == ct`
or `ch`, and `false` when the ends are in two different top-level clusters). The port keeps
this behaviour. Measured on 200 generated `pack` / `packmode` + `newrank=true` inputs with
disjoint clusters, rank sets and `compact=true`: C draws 138 and crashes in `dot_lca` on the
other 62; the port matches C byte for byte on all 138 and draws all 62 without panicking.

## dot's pack components

Closed: #100, #226, #312, #335, #475; #101 and #352 are not matched (below).

With `pack` or `packmode`, `doDot` (`lib/dotgen/dotinit.c`) splits the graph with `cccomps`
into subgraphs `_cc_<i>` of the root, each with clones of the root's clusters (`projectG`),
and lays each one out with `dotLayout(sg)`. The port lays each component out as a standalone
copy and copies the results back to the root. What follows from C's components being
subgraphs, the port now does too:

- **A component's box** (`compute_bb`, `pack.c:801`) walks the component's own out-edges, not
  the root's (#100).
- **Cluster edges.** dot's `node_induce` adds the edges between a cluster's nodes to the
  component's clone, which `agdelete` removes after packing. The root's clusters keep the
  edges they were declared with, so `-Tdot` writes the others at the root (#312).
- **A subgraph the input names `_cc_<i>`** (#335). `cccomps` makes component `i` with
  `agsubg(g, "_cc_<i>", 1)`, which returns a top-level subgraph of that name if the input has
  one. The component is then that subgraph: its subgraphs come first, before `subGInduce`'s
  projections, its own values are what the component's `dotLayout` reads with `agget(sg)`
  (`splines`, `phase`, `rank`, `ordering`, ...; `initSubg` copies `nodesep`, `ranksep` and
  `rankdir` from the root), and `doDot` deletes it after packing, so it is missing from
  the output (in every branch: several components, one, or a `ratio`). The port does the
  same. A node of that subgraph outside the component is in two components, which share
  its C record, as with #101 below, and is not matched. A cluster in it makes C fail an
  assertion in `mapClust` (`ccomps.c:366`); the port lays it out as a cluster of the
  component. With `newrank` and `cluster=true` on that subgraph, even an empty one, C aborts in
  `feasible_tree` (`ns.c:669`, #475; one component or a lone node is enough): there
  `is_a_cluster(sg)` is true, so `compile_samerank` makes the component a level-0 cluster,
  and network simplex finds no spanning tight tree. The step that breaks was not traced; it
  is not a shared field, since `ND_clust`, `ND_rep` and `ND_set` are separate members of
  `Agnodeinfo_t` (`lib/common/types.h`). The port draws. On the issue's inputs, and with
  `packmode=array`, `_cc_1`, a lone node or a `digraph`, its drawing is C's drawing of the
  same graph without `newrank`. If the subgraph already holds the nodes of the only component,
  C segfaults with or without `newrank`; the port draws that too.
- **A failed component.** When `dotLayout(sg)` fails (ortho cannot build its maze), `doDot`
  returns before `attachPos` and `agdelete`. The earlier components keep their positions, the
  `_cc_<i>` subgraphs and their clones stay in the output, and the root's box is what each
  component's `clip_and_install` grew it to (#226). Some of these inputs make C fail
  `bezier_bb`'s assertion (`emit.c:4082`) while drawing, for example with the failing
  component first; the port draws the nodes and exits 0.

**#101 is not matched**, nor #352, the same case: of 400 generated `pack` graphs with
clusters, the 135 where no node is in two non-nested clusters all match C, and every one that
differs shares C records between components. A node in two non-nested clusters is in two
components. In C both components share its one `Agnodeinfo_t`, and the edges between two shared nodes their
`Agedgeinfo_t`: the second layout starts from what the first left in those records, and
`new_spline` appends a second bezier to a shared edge. The port lays every component out on
fresh records, and the later component's results win. Matching C would mean laying the
components out in turn on one shared store and reproducing exactly which fields
`dot_init_node` and `dot_init_edge` do not reset, on inputs where C prints an error
(`node … belongs to two non-nested clusters`) but still draws and exits 0. On those inputs the
port prints the same errors and draws a different layout; where C stops
(`install_in_rank … _cc_2`, exit 1), the port can stop at another check (`expand_cluster`), also
with exit status 1. Where no node is in two non-nested clusters, the components share no node or
edge, and the port lays them out as C does.

## Extreme coordinates

The plan was to validate positions once, where they are parsed. Running C showed this would
break the first rule. C 16.1.0 and the port were run on pins at NaN, ±inf and 1e6–1e308 inches,
across neato, fdp, `-n` and `-n2`, with every `splines` mode, comparing `-Tdot`.

- **C produces output for most non-finite pins.** C reads `pos` with `sscanf("%lf")`, which
  accepts `nan` and `inf`. It then lays the whole component out at NaN (`pos="nan,nan"`), or
  prints `inf` for a finite pin that overflows once multiplied by 72. The port already
  matched those outputs byte for byte, in neato, `-n` and `-n2`. C crashes only on some
  routes: ortho asserts (#228), and `update_bb_bz` overflows its stack on #201's shape. Which
  route an input takes depends on `splines`, the components and the other pins, so no check at
  parse time can reject exactly the inputs C crashes on. Positions are therefore kept as C
  reads them, and the routes where C crashes are made safe downstream. #187's finiteness test
  in `update_bb_bz` covers #201; ortho's failure boundary covers #228 (see above).
- **Finite positions are laid out exactly by C up to 1e306 inches**, except in algorithms that
  have a limit of their own:
  - **Packing** (#174) was budgeted in 0.1.1, at 2^24 cells once `computeStep` saturates.
    That refused #250's input, which C packs in 1 s, so the budget is now 2^27 cells. Measured
    on the reference build with one long edge (#250's input, scaled): C takes 0.9 s and 1.1 GB
    at 3.4e7 cells, 3.9 s and 4.3 GB at 2^27, 10.5 s and 9.2 GB at 3.4e8, and 47 s and 18 GB
    at 1e9. The port takes 13 s and 6 GB at 2^27, with C's output. Past 2^27, where C needs more
    than 4 GB, the port skips packing with a warning. On 32-bit targets (wasm32, at most 4 GiB)
    the budget is 2^25 cells, about 1.5 GB in the port, which still covers #250. A non-finite pin gives a component an infinite
    box; where its width or height is NaN (`inf - inf`), `computeStep`'s discriminant is NaN
    and C fails `assert(d >= 0)` (#239). The port returns step 0 there, which takes C's own
    `stepSize <= 0` exit: packing is skipped with a warning.
  - **`update_bb_bz`** (#227) refines a curve until its control points are within 2pt of the
    chord, which takes about `sqrt(length)` splits at a given magnitude, and more as the
    coordinates' rounding nears 2pt: 1.3e5 for pins 1e10 inches apart, 5.7e7 at 1.5e15.
    Further out, 2pt is below the rounding and C never finishes. The count grows
    continuously, so the port uses a cutoff, as with packing: 2^28 splits per segment, about
    5 s of C's work. A segment under it is refined as in C. Past it, the remaining pieces
    are taken whole, and so is every later segment of that layout. Their control points
    enclose the curve, so the bb may be a little larger than C's, on a segment that takes C
    seconds. The budget does not bound the depth: at 1e300 points the pieces stop shrinking
    (a piece recurs two splits down) while `ptToLine2` overflows to NaN, and C recurses until its stack
    overflows (#269: `ratio` or `ranksep` at 1e300). The port refines on an explicit stack in
    C's order, and a piece deeper than C's stack holds ends the segment as the budget does.
  - **Path boxes `add_box` drops** (#270). Huge sizes or scales (`quantum` or `ratio` at
    1e300, cluster `fontsize` at 1e20) make some of an edge's boxes empty. `add_box` drops them,
    but `completeregularpath` still counts them, so C's `adjustregularpath` goes past `P->nbox`.
    Those slots are stale and nothing reads them before the next `add_box` writes them, so the
    port stops at the last box and draws what C draws. Where every box is dropped, C segfaults
    in `Pshortestpath`; the port's `checkpath` loses the edge.
  - **Integer conversions that C lets wrap.** Where C converts a far position to `int` and
    goes on, the port reproduces what the reference build does. fdp's `gridRepulse` (#196)
    saturates the cell index and wraps `i + 1`. neato's `initItem` for `overlap=*ortho*`
    (#240) converts `x` and `y` through 64 bits and keeps the low 32, so they wrap
    from about 3e6 inches, and its `int` arithmetic after that wraps too. The output stays
    C's.
  - **VPSC overlap removal** (`overlap=vpsc`). C's `Rectangle` and `Node` assert `x <= X`
    and `width < 1e40`, which a pin at NaN, infinity or 1e300 fails (1e300 is infinite as a
    `float`). The port skips overlap removal there with a warning. C's solver loop
    `while (fabs(lastcost - cost) > 0.0001)` ends on a NaN cost; the port's had the test
    inverted and never ended. It now ends where C's does.
  - **Ortho's integer path costs** overflow once coordinates span about 3e7 inches (INT_MAX
    points), and C fails `PQcheck` (#229). The ortho router's failure boundary rejects such
    distances (see above).
  - **Exterior labels** (`addXLabels`). `xlhdxload` asserts that each label area's centre
    fits an `int`, which an infinite or 400-digit `fontsize`/`labelfontsize` (or an object at
    infinity, #191) fails; past it, the placement slide never ends. The port stops there as
    `xlhdxload`'s own failure return does: no exterior label is placed, with a warning (#301).

Closed: #227, #239, #240, #250, #269, #270, #301.
The round-1 guards stay, because they match C and finite or NaN positions still reach them:
#206 (the Hilbert order and shift for a bb above 2^32 points, or an infinite one), #207 (the
wrapping grid cell index at 1e17), and #187's finiteness test in `update_bb_bz` (#173, #201).

## C's own overflow checks

Closed: #252, #271.

C checks two integer sums for overflow. Where a check fails, C prints an error and exits 1,
after writing the graphs before the failing one. The port fails the layout at the same point,
with C's message, and writes nothing more:

| C | Message | Port |
|---|---|---|
| `virtual_weight` (mincross.c:1725), `INT_MAX / t < ED_weight(e)` | `overflow when calculating virtual weight of edge` | `dotgen/mincross.rs` `virtual_weight` |
| `x_cutval` (ns.c:1060), `sadd_overflow` | `overflow when computing edge weight sum` | `common/ns.rs` `x_cutval`; `rank2` returns the error to every caller |

The stderr line has the port's usual prefix for a failed layout
(`Error: <file>: Layout computation error in dot: …`) where C prints `Error: …`.

**C's allocation checks.** `gv_calloc` (`lib/util/alloc.h:26`) checks the count of the array
it allocates for overflow, and reports a failed allocation. C writes both messages with
`fprintf(stderr, ...)`, with no `Error: ` prefix, and exits 1:

| C | Message | Port |
|---|---|---|
| `gv_calloc` (`alloc.h:28`), `SIZE_MAX / nmemb < size` | `integer overflow when trying to allocate N * M bytes` | `core/src/alloc.rs` `gv_calloc` |
| `gv_calloc` (`alloc.h:37`), `calloc` returns NULL | `out of memory when trying to allocate N bytes` | `core/src/alloc.rs` `gv_calloc` |

The port allocates the vertices of a node's peripheries with that check (`poly_init` and
`point_init` in `lib/common/shapes.c`, and the layout's own reconstruction of them), so an
extreme `peripheries` or `sides` ends the run with C's message and exit status 1, where the
port aborted the process (#583).

**Where C has no check, the port wraps.** Everywhere else, C's `int` arithmetic has undefined
behaviour on overflow, and the reference build wraps. That covers ns.c's arithmetic on
weights, cut values, ranks and minimum lengths, and dotgen's weight sums (`group`'s ×100,
merged parallel and virtual edges, `CL_BACK` cluster edges, newrank's merged constraint
edges, `contain_clustnodes`' +128). It also covers the sums on `minlen`, `ranksep` and
`nodesep` (`edgelabel_ranks`, `make_LR_constraints`, `make_edge_pairs`, #271). The port
wraps there too (`wrapping_*`). Before, a build with overflow checks panicked at these
sites, and the panics stopped some inputs before they reached C's two checks: `x_val`'s
`rv -= weight` runs before `x_cutval`'s sum. A comment at each dotgen site says so, and so
does the module note in `common/ns.rs`. One site doesn't wrap in the reference build.
`make_edge_pairs` passes `m + 1` to a `double` parameter, and clang computes it as
`(double)m + 1`, so `INT_MAX + 1` arrives as 2147483648 and `largeMinlen` reports it. The
port does the same.

**What wrapped weights do in C.** A wrapped sum can come back positive (three parallel
edges of weight 2e9). C draws those inputs, and so does the port, byte for byte. A sum that
wraps negative leaves a tree edge whose cut value is negative with no edge to replace it.
`enter_edge` returns NULL, and `rank2` passes it to `update`, which dereferences it
(`SLACK(f)`, ns.c:710) and crashes. Before, the port returned 2 there and went on, so it
drew from broken ranks or panicked later in class2 or mincross. Now it fails the layout at
that point (`network simplex: no edge can enter the tree for a tree edge with a negative
cut value`). All 14 of C's crashes that were traced in a debug build stopped there: 7 of
1500 generated inputs with extreme weights, and 7 hand-written ones. Other negative cut
values make network simplex cycle until `maxiter`, which is INT_MAX for ranking. Measured
at 1e5 to 7e7 iterations, that takes about 0.27 µs per iteration in C and 0.19 µs in the
port, so about 10 and 7 minutes, and with a lower `nslimit1` the two outputs are identical.
That is C's own limit, so the port adds none.

**Also reported now:** ns.c's internal errors, `add_tree_edge: missing tree edge`,
`add_tree_edge: empty outedge list` and `add_tree_edge: empty inedge list`,
`update: mismatched lca in treeupdates` and `invalidate_path: skipped over LCA`. The port
already returned where C does. Only the messages were missing. dot's `largeMinlen`
(position.c:64) is reported too: `Edge length … larger than maximum 2147483647 allowed`.
The port already clamped the edge as C does, and the error makes dot exit 1, as in C
(part of #262).

**Checked and left as they are:**

- `RectArea`'s `UINT64_MAX / dim < area` (label/rectangle.c:73). Both sides of an rtree
  rectangle must pass 2^32 points. Every engine has put the drawing's lower-left corner
  near the origin before `addXLabels`, so such a rectangle has an object centre past
  `INT_MAX`, and C fails `xlhdxload`'s assertion first. That assertion belongs to
  [extreme coordinates](#extreme-coordinates).
- Checks the port already has: `late_int`'s `rv > INT_MAX`, `scale_clamp`, dijkstra's
  `INT_MAX - prevClosestDist < 10`, and `make_aux_edge`'s clamp.
- Checks that cannot fail in the port: `Pobsopen`'s `n > INT_MAX`, and the `SIZE_MAX`
  checks in `gv_calloc`/`gv_recalloc`/`LIST`, which Rust's allocator makes.
- neato's overlap constraint graph (`constraint.c`) ranks with the same network simplex.
  Its edges weigh 1 or 100, so `x_cutval`'s sum cannot overflow for a graph that fits in
  memory, and the port ignores the error there.

## C's numeric idioms

Closed: #192, #194, #216, #272, #274, #292.

Some C idioms look like a Rust operation but differ at NaN, at the ends of the integer
ranges, or in how they print. A port of one of them uses the helper in
`graphviz_pure_core::cnum`, which behaves as the reference build (clang on arm64, the macOS
libc) does, not the Rust operation that looks the same:

| C | Differs from Rust's look-alike in | Helper |
|---|---|---|
| `MIN(a,b)`, `MAX(a,b)` (`arith.h` ternaries) | the result is `b` whenever the test is false, so a NaN `b` is kept; `f64::min`/`max` drop it. `fmin`/`fmax` are Rust's. | `c_min`, `c_max` |
| `is_exactly_zero`, `is_exactly_equal` (`gv_math.h`) | they compare bits: `-0.0` is not zero, and a NaN equals itself | `is_exactly_zero`, `is_exactly_equal` |
| `ROUND(f)`, `POINTS(f)` | the result is an `int` | `c_round` |
| `ROUND` in a pair that clang vectorizes | converts through 64 bits and keeps the low 32 | `c_round_low32` |
| `strtol`, `sscanf("%ld")` | saturates at `LONG_MIN`/`LONG_MAX`, reads a prefix after white space | `strtol` |
| `atoi`, `sscanf("%d")` | the saturated `long`, truncated to 32 bits | `atoi`, `scan_d` |
| `late_int`, `mapBool` (`utils.c`) | built on these | `late_int`, `map_bool` |
| `printf("%.Nf")`, `%g` | `nan`, not `NaN` | `fmt_f`, `fmt_g` |

**Float-to-int conversions.** arm64's `fcvtzs`/`fcvtzu` into a 32-bit register saturate and
give 0 for a NaN, as Rust's `as i32`/`as u32` do, so a plain `(int)d` is `d as i32`. The
port has to convert to C's type (`int`, not `i64`), and then do C's `int` arithmetic, which
wraps (fdp's `T_pass1 = T_unscaled * T_maxIters / 100` with `maxiter=INT_MAX` is 0 in C,
where the 64-bit product ran 2^31 iterations, #272). Clang converts through 64 bits only
where it vectorizes or widens. In the reference dylibs (`otool -tv`, looking for
`fcvtzs x`, `fcvtzs.2d` and `fcvtzu`) that is: `gvRenderJobs` (`init_job_pagination`'s
`job->width`/`height` and `setup_page`'s `pageBoundingBox`, #194), `cAdjust` (`initItem`,
#240), `colorxlate` and `attach_edge_colors`, plus three conversions to 64-bit unsigned
types (`RectArea`, `generateRandomOrdering`, `StressMajorizationSmoother_smooth`). A new
conversion site follows the default unless its function is on this list.

**C's font dictionary** (#274). C interns every label font in `gvc->textfont_dt`, a CDT
splay tree whose comparison (`textfont_comparf`) orders sizes with `<` and `>`. A NaN
`fontsize` compares equal to any size, so the label gets the font that the search reaches
first: the most recently used font of that name, or, when a NaN font was interned first,
later labels of that name get the NaN one. The port interns fonts in the same splay
tree (`core::cdt`, moved from layout), one per thread and run, at C's sites: `storeline`,
and for HTML labels `mkFont` (when the parser reads the token), `pushFont` and
`size_html_txt`. C measures each label once and keeps the interned font in each span;
the port measures some objects again (dot's node pre-pass, port lookups, patchwork, the
root label after `graph_init`) and the renderer measures them all. So the geometry
functions measure through `font::textfont_object` with the object's identity (`node:`,
`edge:` with its id, `graph:`, `root:`): the first measurement in a layout interns and
keeps the sizes, and later ones get the same sizes and leave the tree alone. Where the
port's first measurement comes in a different order from C's (fdp's nested cluster
labels, which C makes one level at a time in `deriveGraph`; C14), a NaN size can still
meet a different font. The render-time `_ldraw_` fonts of `emit.c` are not interned.

**Not this cause:** #249 (`overlap=prism` with pins near 1e7 inches). The divergence
starts in the prism smoother's first iteration and only at that magnitude, C's residual
there is 8.5e14, and the path has no conversion that differs from Rust, but its
`SparseMatrix_multiply_vector` and `vector_product` are FMA-contracted: C11.

## Subgraph nesting: bison's stack

The parser had a limit C doesn't have: `MAX_SUBGRAPH_DEPTH = 256` nested subgraphs (#118,
C21). C's own check, `opensubg`'s `SubgraphDepth >= YYMAXDEPTH/2` (5000), never fires,
because bison's state stack runs out first. It holds `YYMAXDEPTH` = 10000 states, and each
anonymous subgraph level takes three (`optsubghdr`, `$@1`, `{`), four with a statement
before it, as does each edge operator of one statement (`rcompound` is right-recursive).
So C 16.1.0 reads 3331 nested `{` and stops at 3332 with `memory exhausted in line N near
'<token>'`. There is no `graph : error` recovery then: `agread` returns the graph read so
far, which dot lays out and writes, and the exit status is 1.

The port (C6, #118) reads subgraph bodies without recursion and counts bison's states, so it
stops at the same token with the same message and renders the same partial graph. C's
next `agread` starts after the last line `fgets` read, whatever statement that leaves
unfinished, and so does the port's; C splits a line longer than flex's 8 KB read into
pieces, which the port does not, so after such a line only C goes on to a second error.
The layouts and writers still recurse once per subgraph level, as C's do on its 8 MB main
stack: at 3331 levels the release build needs 2 to 4 MB, more than Windows' 1 MB main
thread, so the CLI runs on a 64 MB thread (`DOT_STACK`). Measured on the reference build:
3331 nested anonymous
subgraphs take C 0.05 s and 34 MB, and the port 0.03 s and 14 MB, in every engine.
2498 nested clusters, each with a node, take C 0.3 to 13 s and up to 400 MB; the port
takes up to 35 s, and fdp needs 11.5 GB. That cost is the engines' (#323), not the parser's.

## Truncated styles: parse_style's static array

`parse_style` (`lib/common/emit.c`) keeps its result in a static array, `parse`, and its
strings in a static buffer, `ps_xb`. At the 64th item (`FUNLIMIT - 1`) it warns `truncating
style` and returns `parse` without filling it in (#268), so the result is what the previous
call left there:

- **No earlier call, or the previous one failed** (`nesting not allowed`, `unmatched`
  parenthesis, which set `parse[0] = NULL`): no items. The port gives the same.
- **The previous call returned items.** `parse` still holds its pointers, at that call's
  offsets in `ps_xb`. That call ended in `agxbuse`, which resets the buffer, so this call
  writes its own tokens from the start; once the buffer grows past its inline storage, or a
  heap `realloc` moves it, the old pointers point at freed memory. C then draws whatever is
  at those offsets, often printing `unsupported style <garbage>`.

The port does not model the second case. What C reads depends on the buffer's growth and on
the order of every `parse_style` call in the run, which the port's renderers don't reproduce
call for call. A truncated style gives no items in the port, C's result in the first case.

## Empty colour segments

Closed: #385.

`parseSegs` (`lib/common/emit.c:470`) splits a colour list with `tok(clrs, ":")`
(`lib/util/tokenize.h`), which, unlike `strtok`, yields an empty token for a leading `:` and
for a trailing one (a run of `:` inside the list is one separator). An empty token is a
segment with no colour; with no `;` weight it takes a share of the weight left unassigned.
The port dropped empty tokens; it now splits as `tok` does. That changes every consumer of
`parseSegs`:

- The wedges, the stripes and the pieces of a multicolour edge stop at the first segment with
  no colour, so `red;0.3:` draws a red piece or stripe 30% long, and `:red;0.3` draws no
  wedge. The port drew red over the whole width.
- A gradient (`findStopColor`) takes an empty second segment as no second colour, and its
  callers end the gradient in black: `bgcolor="red:"` and `fillcolor="red;0.3:"` on the graph,
  a node or an HTML cell draw C's red-to-black gradient. The port drew plain red. An empty
  first segment is no gradient, in C and the port.

Where the first segment of a multicolour edge (one with a `;`) is empty, C crashes:
`multicolor` sets the tail arrow's colour to that segment's NULL colour
(`LIST_FRONT(&segs)->color`), and the head arrow's to `endcolor`, which only a coloured
segment with a weight above 0 sets (`emit.c:2036-2045`); `gvrender_set_pencolor` then calls
`strchr` on the NULL. The head arrow alone crashes the same way where every coloured segment
before the first empty one has a weight that rounds to 0 (`red;0.000001:`).

The port draws an arrow only when its colour is set. Two fallbacks were possible:

- C's nearest input that it handles is the same edge with `dir=none`. C draws the coloured
  pieces of the spline before the first empty segment, none where it is the first, and no
  arrow: the edge is invisible, with no warning. The port draws exactly those drawing
  operations (the SVG and xdot `_draw_` of the edge are C's for `dir=none`); its layout is
  still the arrowed edge's, so `-Tdot`'s `pos` keeps the `e,…` end and the spline clipped
  for the arrow.
- `multicolor`'s failure path (#283), which draws the edge in black. C takes it only when
  `parseSegs` reports an error, which it does not here: the list is valid, and C's
  `multicolor` has already drawn the pieces before it reaches the arrows.

The port takes the first: it is what C draws for the edge in every respect but the arrow C
crashes on, and the colours the user gave are honoured. The second would draw a black edge
that no C input with this list draws.

Measured on 2,000 generated graphs with colour lists on nodes, clusters and edges (wedged,
striped, radial and filled styles, arrows in every direction): C draws 1,036, and the port's
SVG and stderr equal C's on all 1,036, against 251 (SVG) and 763 (stderr) before. C crashes
on the other 964, every one on this route: with `dir=none` on every edge, C draws all 964,
and the port's output on those copies equals C's. The port draws all 964, with no panic.

## Freed keys in EPSF_contents

`user_init` (`lib/common/psusershape.c:49`) keys `EPSF_contents`, cdt's splay tree, by the
pointer `safefile` returned (#380). For a name `safefile` returns as it was given (an
absolute path, or any name without `imagepath`), that is the node's `shapefile` string,
which `agclose` frees with its graph; the tree keeps the pointer, and the next
graph's lookups compare with that freed memory. That is undefined behaviour: C's result
depends on the platform's allocator, and on whether a later allocation reused the block.

The port models the reference build's allocator (macOS arm64), which clears freed small
blocks: a closed graph's key compares as the empty string, so it matches nothing and every
search passes it on the right. The choice is deterministic and the same on every platform; on
other platforms, or where the memory was reused, C reads other bytes. Measured on 147
generated files of two or three graphs with `shape=epsf` nodes and `imagepath`, against C on
the reference build:

| A closed graph's key | Files equal to C |
|---|---|
| reads as `""` (chosen) | 137 |
| keeps its text, never equal, ties below | 125 |
| keeps its text, never equal, ties above | 124 |
| greater than any key | 120 |
| keeps its text, and matches | 96 |

The other 10 depend on where the next graph's strings land in memory (C's output on some of
them changes from run to run), which the port does not model.

## parseFactor's uninitialised x, y

Closed: #372.

`parseFactor` (`lib/neatogen/adjust.c:1020`) reads `sep` and `esep` with
`if ((i = sscanf(s, "%lf,%lf", &x, &y)))`. On a value that is empty or blank after the
optional `+`, `sscanf` returns `EOF` (-1), which passes the test. `x` and `y` are then
uninitialised and `i != 1`, so C computes the margin from whatever their stack slots hold.

What they hold was measured, not assumed. In the compiled function (the packaged arm64
build), `x` and `y` live 0x38 and 0x40 bytes below the caller's frame. Every call follows
an `agget` call from the same caller (`sepFactor`, `esepFactor`), so the slots hold what
`agget` and its callees left there. In the build from source `parseFactor` is inlined, and
they hold what earlier code left in `sepFactor`'s frame. Read with a debugger on every
call, they are pointers (heap and stack addresses) or small integers, and their exact
values change from run to run with address randomisation. These have their top 12 bits
zero, so as a double each is a nonnegative subnormal (about 1e-314 here) or 0. Every use C
makes of the margin in a layout then gives what 0 gives:

- `1 + x / sepfact` is exactly 1.
- An additive margin is added to node sizes and coordinates (at least 0.01 inch), where a
  subnormal disappears.
- `fmin(dflt, x / sepfact)` gives the subnormal and `fmax` gives `dflt`. `-v` prints both as
  0 or as `dflt`.

On `sep=""`, `" "`, `"+"`, `" + "`, `esep=""`, `"+"`, the same combined with a valid other
value, and a `sep` declared only by a subgraph (so the root's value is `""`), the two
C 16.1.0 builds (a packaged build and one built from source) gave the same separation
values on every run. Across fdp, neato's overlap modes, splines and ortho, sfdp, circo and
twopi, C's output was the same on every run.

The values differ from call to call, and between the builds: in the packaged build, where
`parseFactor` is a function of its own, the pointers read in `esepFactor` and in
`sepFactor` called from `compoundEdges` differ by tens of bytes (`0x7c6d004c90` and
`0x7c6d004c60`, with the same `y`). In the build from source, where it is inlined into
`sepFactor`, they are `4` and a heap pointer, or a heap pointer and exactly 0. Only one
place in C compares two of them: `compoundEdges`'s `sep value … is smaller than esep` note
(`fdpgen/clusteredges.c:243`), reached when nodes touch. With `sep="+"` alone it tests
about `G <= G / 1.25`, false since the factor dwarfs the gaps: no note. With `esep="+"` too
it tests about `G <= G`, true where the `y` values are equal: a note. A model of 0 printed
the note in both cases.

So the port models the garbage as one fixed positive subnormal (`STALE_POINTER` in
`neatogen/adjust.rs`, `2^32` as bits, about 2.1e-314). It acts as 0 in layouts and gives C's
answer in that comparison. It is a model of undefined behaviour, measured on one build, the
packaged arm64 one the goldens come from. The layouts agree on both builds, but the note
does not: on 7 forms of it, the packaged build prints it for 3 (`+`/`+`, `+0`/`+`,
`+`/blank) and the build from source, which reads different garbage, for none. The port
follows the packaged build; another compiler or allocator can make C's garbage compare
otherwise. The port matches C, the packaged build and the one with the
port's math, on 9 `sep`/`esep` forms of that note in this PR and on 16 more in its review
(declaration order, `sep` only in a subgraph, `pack`, `"+,"`), each stable over two C runs.
`simpleScale`'s blank `scale` returns at its `ISZERO`, as C does. Two `f32` roundings of these
margins (`compoundEdges`'s note and `neatosplines.c:625`'s `some nodes with margin`) were
dropped with this: `expand_t` holds doubles (`adjust.h:41-44`).

Measured on 612 inputs: 12 `sep`/`esep` forms × 17 engines and overlap modes × 3 graphs. C
ran twice on each, and inputs where its two outputs differed were skipped. Over three
measurements (the third with the stale-pointer model, the first two with 0):

| Measurement | C varied | Port matched C, before | after | Still differ |
|---|--:|--:|--:|--:|
| first | 3 | 343 | 605 | 4 |
| second | 6 | 343 | 604 | 2 |
| third | 3 | 344 | 606 | 3 |

Every input that still differs is `mode=ipsep` with clusters. There C's layout also changes
between spellings of the same value (`sep="0"` and `sep="0,0"`), because VPSC breaks ties by
heap address (#225).

## voronoi newpos on an empty cell

Closed: #450.

`newpos` (`lib/neatogen/adjust.c`) places a site at the centroid of its Voronoi
cell: a fan of triangles from `verts[0]`, divided by their total area. The cell
is clipped to the window `chkBoundBox` computed before `rmEquality`. Three or
more nodes pinned on one point overlap, so adjustment runs, and `rmEquality`
then shifts each later site by half the sum of the polygon widths. That puts
every site but the first outside the window. `addCorners` (`adjust.c:355`)
gives a corner only when a site is strictly closer, so the first site takes all
four corners and the last gets none. The sweep, clipped to the same window,
records nothing for that site either. Its `n_verts` is 0 and `verts` is NULL.

That empty cell is C's state. The port's `addCorners` uses the same strict `<`
(`voronoi_pass` in `neatogen/voronoi.rs`); it does not drop a vertex C kept.
On the reference build, a breakpoint on the compare clang emits for this
function (inlined in `removeOverlapWith`) shows the first pass: `n_verts` 4, 4
and 0, the last `verts` pointer NULL, the sites already spread to
`0`, `0.86111111`, `1.7222222`. The four vertices of the first site are the
window corners. The second site's four are the sweep's clips of the window.
The third has no vertex.

C calls `newpos` anyway. The source reads `ip->verts[0]` with no `n_verts`
check (`adjust.c:331`). On an empty cell, that is a read through a NULL pointer,
undefined behaviour: `sortSites` (`adjust.c:189`) sets `verts` back to NULL at
the start of each pass, and a site the sweep never touches stays NULL. The
loop is `for (i = 1; i + 1 < n_verts; ++i)`, so it does not run for fewer than
3 vertices, the area stays 0, and the coordinates are `0/0`. The reference
build gives that NaN layout because clang sinks the `verts[0]` load past the
zero-trip loop (`cmp` against 3, then the division). An `-O0` or gcc build of
C would likely crash. The port writes the reference build's output: it loads
the anchor only when the cell has the three vertices the loop reads, and
divides the same way. Indexing `verts[0]` first panicked.

The NaN site does not end the adjustment. The first pass still leaves the other
two overlapping, so the next pass moves every site. Bisectors against the NaN
site write NaN vertices into those cells (`n_verts` 6, 6 and 4 on that pass),
and their centroids become NaN too. C exits 0. `-Tdot` is `pos="nan,nan"` and
`bb="0,0,-4.295e+09,-4.295e+09"` (the sentinel box `INT_MAX` / `-INT_MAX`,
shifted so its corner is the origin: `-4294967294` prints as `-4.295e+09`).
`-Tplain` is `nan` nodes on a graph `-5.9652e+07` square. fdp calls the same
`removeOverlapWith` and writes the same drawing. Two pinned nodes keep a cell
with area, so both draw them apart. An unpinned `pos` is placed by the model
before adjustment, and that drawing is finite.

## sameport's 0/0 direction

Closed: #396.

`sameport` (`lib/dotgen/sameport.c:110-124`) points the shared port of a `samehead` or
`sametail` group along the sum of the unit vectors from the node to the group's other ends,
and divides that sum by its length. Where the ends cancel out, as for two edges reaching a
node from the nodes straight above and below it, the sum is (0, 0) and the division is 0/0:
the port's position is NaN, and so are the path ends and splines of the group's edges. C does
not crash there. It writes the coordinates as `nan` (`-Tplain`, `-Tdot`'s `pos`) and leaves
the edges out of the SVG, on every run, and the port writes what C writes.

The only C test that reads these splines is `routesplines_`'s check for a spline that goes
straight across or straight up and down (`routespl.c:508-545`). It clears each flag on
`fabs(ps[0].y - ps[i].y) > FUDGE`, which is false for NaN, so a NaN spline counts as
straight: C bounds its boxes at the first point's x at once and prints no warning. The port
tests the same way (`is_straight` in `common/routespl.rs`). It used to test the negation,
`all(<= FUDGE)`, which is false for NaN, so it printed `Unable to reclaim box space` for each
such edge, and the polyline fallback could leave boxes unbounded, which moved every node
to x = 2.5e306 on some inputs. Measured on 3,000 generated inputs with such groups (46 crash C
for other reasons): the SVG equal to C on 2,938 before and 2,954 after, stderr on 2,869 before
and 2,954 after, which is all of them.

## polylineMidpoint's unreachable end

Closed: #435.

When nodes touch under `splines=polyline`, neato and circo fall back to straight-line
edges, and `clip_and_install` (lib/common/splines.c) can clip such an edge past itself:
the tail walk and the head walk both stop only at the last segment, so `start` passes
`end`. One segment apart (`start == end + 3`) C installs a one-point bezier; further
apart the `size_t` in `newspl->size = end - start + 4` (splines.c:312) wraps, and C
installs a bezier whose size is huge and whose list is never written. An edge label then
reaches `polylineMidpoint` (splines.c:1245), whose stride-3 segment loops find no
segment: on the one-point bezier both loops skip and C aborts at `UNREACHABLE()`
(splines.c:1277); on the wrapped one the length pass reads off the allocation (a bus
error), and without a label the emit pass's `assert(bz.size % 3 == 1)` in `bezier_bb`
(emit.c:4082) fails instead.

The loop cannot be stranded by rounding when a segment exists: before the last segment
the remaining half-distance is `d_last - total / 2 <= d_last` in exact arithmetic, and
the two passes disagree by O(n * 2^-53) of the total, so the last segment always
satisfies `d >= dist`. The fall-through needs a bezier with no complete segment — a
one-point list — or NaN lengths.

The port's `clip_and_install` keeps the one segment where the walks meet and installs
the same one-point bezier as C when they meet one segment apart; both are the #359 fix,
whose comment there records the choices. The wrapped size therefore never reaches the
midpoint walk, but the one-point bezier does, as in C: there the port returns the
spline's first point (`polyline_midpoint` in `common/splines.rs`), the only point of
the polyline, and draws the graph. That is the smallest defensible behaviour: C installs
and draws one-point beziers when there is no label, and for a one-point polyline every
candidate midpoint is that point.

## circo's derived nodes have no shape

With several components, circo runs `adjustNodes(sg)` on each component of its derived graph
(`circoLayout`, `lib/circogen/circularinit.c`), which sees the command-line attributes (#398).
`makeDerivedNode` gives a derived node sizes but no shape. With `-Goverlap=voronoi` or
`-Goverlap=oscale`, `makeInfo` calls `makePoly` (`lib/neatogen/poly.c`) on such a node:
`shapeOf` returns `SH_UNSET`, and the `default:` case dereferences `ND_shape(n)->name`, a NULL
pointer, so C segfaults. The port takes C's behaviour on the nearest input it handles, an
in-file `overlap` value, which never reaches the derived graph: it leaves the overlaps, but still
applies `normalize` and `scale`, which `removeOverlapWith` runs before it picks the mode
(`circularinit::adjust_derived`).

## sfdp: an edge label with no edge

Closed: #492.

`label_scheme` 3 and 4 treat a node whose name starts with `|edgelabel|` as an edge label that
sits between two neighbours. `shorting_edge_label_nodes` (`lib/sfdpgen/spring_electrical.c`)
removes those nodes and calls `LIST_FRONT` on the edges that remain. When none remain (the node
has no edge, or every node is an edge label), the list is empty and `gv_list_get_` asserts
(`list.c`). When an edge does remain but the label node itself has fewer than two neighbours,
`attach_edge_label_coordinates` asserts `len >= 2` and writes nothing. Schemes 1 and 2 do not
short. They reach `get_edge_label_matrix` (`lib/sfdpgen/post_process.c`), which reads `ja[ia[ii]]`
and `ja[ia[ii] + 1]` with no check. `ja` has exactly `nz` entries, so a last-row label with one
neighbour reads past the array. That read is undefined behaviour in C. C is deterministic only
when its allocation has padding after `ja`. With eight `ja` entries (`a--b; a--c; b--c; c--L`,
the label last) C writes coordinates around 1e140 and segfaults one run in three. With four
(`a--b; b--L`) the positions differ from run to run. The port chooses 0.

Scheme 1 skips a label whose two reads are equal (`if (jj == ll) continue`). When that is every
label, both reads are the port's 0, C still enters `StressMajorizationSmoother_smooth`, and C
segfaults on a NULL `x00`. The empty penalty diverges to NaN, so the port runs ordinary overlap
removal instead.

Scheme 2 always builds the right-hand side and divides by the neighbour count. A node with no
neighbours is skipped, so that division does not write NaN. The past-the-end read is 0, by the
choice above. On `overlap=false` with `a -- "|edgelabel|"` (the label last) that choice matches
the drawing C produced for that specific graph.

Schemes 3 and 4 do not reach `get_edge_label_matrix` on the graphs where C asserts. A
straight-line scheme whose label nodes do not all have two neighbours, or whose shortened graph
has no edge, is laid out as an ordinary sfdp graph: those nodes are not edge labels, which is
what C draws for the same graph with `label_scheme=0`. A label that does have two neighbours,
and a shortened graph that still has an edge, is unchanged.

## neato mode=sgd: pins with no path between them

Closed: #488.

`pccomps` (`lib/pack/ccomps.c`) puts every pinned node into one component, whether or not
there is a path between them. `sgd` (`lib/neatogen/sgd.c`) sizes its term array as if every
search from an unfixed node reached every node, and `dijkstra_sgd` (`dijkstra.c`) stops at
the first node still at `FLT_MAX`, so a pin with no path to the unfixed nodes gets no term
and C fails `assert(offset == n_terms)` (`sgd.c:175`). Without the assertion, the unwritten
terms would read as zeros (`gv_calloc`): `w_min` would be 0, so the annealing schedule has an
infinite first step, and each zero term pairs node 0 with itself at distance 0, a 0/0 that
writes NaN when node 0 is unfixed. The port keeps the terms the searches made, which is what
the sizing assumes, and the pins stay where they were put. One pin, or pins with no unfixed
node, draw as in C.

## fdp/sfdp overlap=portho: a separation above 0xFFFF

Closed: #489.

For two boxes that overlap in x, `mkNConstraintG` (`lib/neatogen/constraint.c`) stores the sum
of their half heights, in tenths of a point, as `ED_minlen`, after `assert(delta <= 0xFFFF)`
(`constraint.c:242`). The bound is 16 bits, but `ED_minlen` is an `int` (`lib/common/types.h`),
and this is the only `0xFFFF` check in the 16.1.0 sources. A node 182 inches tall next to a
default node is 65,700, and C aborts (`overlap=portho`, `porthoxy`, `porthoyx`, in fdp and
sfdp; also a large `sep`). `mkConstraintG`, which the `ortho` modes use, has no such check. The port
stores the full value, as the assignment under the assertion does. Past about 3e6 inches
`initItem`'s integer boxes wrap ([Extreme coordinates](#extreme-coordinates)).
