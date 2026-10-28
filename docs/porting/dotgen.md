# dotgen port (`crates/graphviz-pure-layout/src/dotgen/`)

Faithful, function-by-function port of Graphviz 16.1.0 `lib/dotgen` plus
`lib/common/ns.c` and the parts of `lib/common` that dot uses. It is the `dot`
engine (`crates/graphviz-pure-layout/src/lib.rs`); the old approximate engine (`dot.rs`,
`GV_DOT_LEGACY`) and its helpers (graphviz-pure-geom's old spline router and node sizing,
`force_circular.rs`) are deleted.

Functions keep their C names (snake_case) so the two can be read side by side.
C file-scope statics live as fields on `Dot` (`cmark`, `min_quit`, `max_iter`,
`remincross`, `weak_id`, ...).

## Module map

| Rust module | C source | Status |
|---|---|---|
| `types.rs` | `common/types.h`, `common/const.h` | `Agraphinfo_t`/`Agnodeinfo_t`/`Agedgeinfo_t`, `rank_t`, `port`, `textlabel_t`, `bezier` |
| `cg.rs` | `lib/cgraph` (subset) | iteration order, `agdelete` (node, recursive into subgraphs), `agsubedge`, `agsubnode`, strict `agedge`, `agsubg`, `agdegree` |
| `init.rs` | `dotinit.c` (`dot_init_subg`, `dot_init_node_edge`), `common/input.c` (`graph_init`, `do_graph_label`), `common/utils.c` (`common_init_edge`) | ported; node and label sizes come from `graphviz_pure_geom::node_geometry` / `edge_label_geometry` / `graph_label_geometry`; ports via the shared `common::ports::chk_port`; nodes carry a `common::shapes::NodeShape` for `insidefn` clipping |
| `utils.rs` | `common/utils.c` (`late_int`, `late_double`, `mapBool`, `UF_*`, `is_a_cluster`), `class1.c` (`nonconstraint_edge`), `util/gv_math.h` (`scale_clamp`) | ported |
| `qsort.rs` | Apple Libc `qsort` | thin wrapper over the single implementation `crate::cutil::qsort_by` (qsort is unstable; tie order leaks into `TB_balance`, `medians`, `restore_best`, `do_ordering_node`, `edgecmp`). `qsort_vectors.txt` holds recorded system permutations |
| `fastgr.rs` | `fastgr.c` | ported (all functions) |
| `class1.rs` | `class1.c` | ported |
| `acyclic.rs` | `acyclic.c` | ported (dfs iterative, same visiting order) |
| `decomp.rs` | `decomp.c` | ported (incl. the `Cmark` static) |
| `rank.rs` | `rank.c` | `dot1_rank` (collapse_sets/clusters, rank=same/min/max/source/sink, minmax_edges, expand_ranksets, cleanup1) and `dot2_rank` (newrank: compile_samerank/nodes/edges/clusters, strong/weak, break_cycles, connect_components, readout_levels) ported |
| `ns.rs` | `common/ns.c` | binding of the generic `common::ns` (shared with neato's `constraint.c`) to dot's fast graph via the `NsGraph` trait; full network simplex: init_rank, feasible_tree with STheap, leave/enter edge search order, update/treeupdate/invalidate_path/dfs_range, TB_balance (with `TBbalance=min|max`), LR_balance, `searchsize`, maxiter |
| `class2.rs` | `class2.c` (+ `ports_eq` from position.c) | ported |
| `cluster.rs` | `cluster.c` | ported (expand_cluster, map_path, interclexp, merge_ranks, build_skeleton, install_cluster, mark_clusters, mark_lowclusters) |
| `mincross.rs` | `mincross.c` | ported: ordered components, build_ranks (BFS, backwards walk for clusters), install_in_rank, transpose, reorder, medians, flat_mval, flat_breakcycles/flat_search/flat_rev, flat_reorder/postorder, mincross_clust recursion, remincross, `mclimit`, `ordering=in/out` (graph and node), rec_save/reset_vlists, fillRanks (newrank), checkLabelOrder/fixLabelOrder |
| `position.rs` | `position.c` | ported: set_ycoords (ranksep, `equally`, cluster ht1/ht2, clust_ht, adjustRanks/adjustSimple for flipped labels), expand_leaves (no-op in C), create_aux_edges (make_LR_constraints incl. self-loop space and flat labels, make_edge_pairs, pos_clusters, compress_graph), connectGraph, set_xcoords, dot_compute_bb/rec_bb, set_aspect (ratio fill/compress/expand/value/auto), make_lrvn, contain_nodes |
| `flat.rs` | `flat.c` | ported (flat_edges, flat_node, flat_limits, make_vn_slot, abomination, checkFlatAdjacent) |
| `sameport.rs` | `sameport.c` | ported (`shape_clip` is the shared one) |
| `conc.rs` | `conc.c` | ported: `dot_concentrate`, `mergevirtual`, `rebuild_vlists`, `infuse`, up/down candidates |
| `dotsplines.rs` | `dotsplines.c` | ported entirely (see "Phase 2" below) |
| `compound.rs` | `compound.c` | ported: `dot_compoundEdges`, `makeCompoundEdge`, `splineIntersectf`, `boxIntersectf`, cluster map |
| `splinegraph.rs` | glue | `SplineGraph`/`PortGraph`/`XLabelGraph`/`OrthoGraph` for `Dot` (the `ND_*`/`ED_*` view the shared `lib/common` and `lib/ortho` code uses), `record_path` (`pboxfn`), `arrow_flags` with `ED_conc_opp_flag` |
| `postproc.rs` | `common/postproc.c` (`gv_postprocess`, `place_graph_label`, `place_flip_graph_label`, `place_root_label`, `translate_drawing`, `map_edge`, `translate_bb`, `map_point`), `dotinit.c` (`removeFill`) | ported; `addXLabels` is the shared `common::xlabels::add_xlabels` |
| `pack.rs` | `dotinit.c` (`doDot`'s pack branch, `initSubg`, `attachPos`, `resetCoord`, `copyCluster`, `copyClusterInfo`) | ported, see "Phase 3" |
| `output.rs` | `common/output.c` (`attach_attrs_and_arrows`, the parts the IR needs) | write-back into `graphviz_pure_core::Graph`: positions, splines with `sflag`/`eflag`, `lp`/`head_lp`/`tail_lp`/`xlp` with C's emission rules, graph/cluster label positions (`GraphLayout::label_pos`) |

Shared code outside `dotgen/` that dot uses (one implementation per C function):

| Module | C source |
|---|---|
| `common/splines.rs` | `common/splines.c`: `clip_and_install`, `arrow_clip`, `bezier_clip`, `shape_clip(0)`, `new_spline`, `beginpath`/`endpath`, `conc_slope`, `getsplinepoints`, `makeSelfEdge` + `selfRight/Left/Top/Bottom`, `place_portlabel`, `makePortLabels`, `edgeMidpoint`, `updateBB`; `emit.c` `update_bb_bz` |
| `common/routespl.rs` | `common/routespl.c`: `routesplines_`, `checkpath` (incl. C's stale-tail quirk after removing degenerate boxes), `limitBoxes`, `overlap`, `simpleSplineRoute`, `makeStraightEdges`, `bend`; `path`/`pathend_t` |
| `common/ports.rs` | `common/shapes.c`: `chkPort`, `poly_port`, `record_port`, `compassPort`, `compassPoint`, `closestSide`, `resolvePort`, `invflip_side/angle`; `geom.c` `cwrotatepf`/`ccwrotatepf` |
| `common/shapes.rs` | `insidefn`s incl. `poly_inside`'s `scalex/scaley` and `fixedsize=shape` (`polyBB`) |
| `common/arrows.rs` | `arrows.c` incl. `arrowOrthoClip` |
| `common/xlabels.rs` | `postproc.c` `addXLabels` + `lib/label` |
| `ortho/` | `lib/ortho` (all files) |
| `cdt.rs` | `lib/cdt` ordered set (`dttree.c`, `dtflatten.c`, `dtrestore.c`) |
| `pack/cccomps.rs` | `lib/pack/ccomps.c` `cccomps` (`deriveGraph`, `deriveClusters`, `unionNodes`, `subGInduce`/`projectG`) on the parsed graph |
| `pack/mod.rs` | `lib/pack/pack.c`, generic over `PackGraph` (neato-family `LGraph` components and dot's `DotPack`) |

### Data model

* Arenas on `Dot`: `nodes`, `edges`, `graphs`, `labels`, plus `bufs` (storage of
  `rank_t.v`/`av`). Real nodes/edges keep their input index; virtual ones are
  appended. Graph 0 is the root, then input subgraphs in creation order; dot's
  private cgraphs (newrank `Xg`, `checkLabelOrder`'s `lg`, `_new_rank`) are
  extra graphs in the same arena.
* `Rank { vbuf, voff, n, avbuf, an, ... }`: in C cluster rank arrays alias windows
  of the root arrays (`GD_rank(subg)[r].v = GD_rank(root)[r].v + pos`), and the
  root window moves per component (`init_mccomp`). `rv(g, r, i)` reads
  `bufs[vbuf][voff + i]`. `GD_rank(g)[r]` is stored at index `r + 1` because
  `abomination` creates rank -1.
* cgraph iteration order: nodes by sequence (= index); **out-edges of a node by
  (head sequence, edge sequence) and in-edges by (tail sequence, edge sequence)**
  (`agedgeseqcmpf`), subgraphs by creation order. `NodeInfo.cg_out/cg_in` hold
  these orders; subgraph membership is a `BTreeSet` per graph.
* Only the root has `rankdir` (`GD_flip(subg)` is false in C) and `nodesep`
  (`GD_nodesep(subg)` is 0, so cluster virtual nodes are not widened).
* `free_matrix` in `cleanup2` leaves a dangling non-NULL pointer that
  `flat_edges` tests; the port keeps the matrix to preserve that truthiness.

## Phase 2: edge routing (done)

`dot_splines_` is a direct port: edges are collected per rank (normal nodes and
virtual merge points: `ND_out`, `ND_flat_out`, `ND_other`; `rw`/`mval` swap),
classified with `setflags`, sorted with `edgecmp` through the Apple `qsort`, then
grouped into equivalence classes (`getmainedge` + ports, `portcmp` on
`makefwdedge` copies). C's temporary edge copies (`makefwdedge`, the
`fwdedgea/b` pairs of `make_regular_edge`) are pushed on the edge arena and
truncated when `dot_splines_` returns.

* Regular edges: `make_regular_edge` with `maximal_bbox`/`neighbor`/
  `pathscross`/`cl_bound`/`rank_box`, straight-segment mode (`straight_len`,
  `straight_path`), `completeregularpath`/`adjustregularpath` (size_t wrap
  arithmetic kept), `recover_slack`/`resize_vn`, multi-edge spreading by
  `Multisep`, `splines=line` (`makeLineEdge`) and `polyline`.
* Flat edges: `make_flat_edge`, `make_flat_labeled_edge`,
  `make_flat_bottom_edges`, `makeFlatEnd`/`makeBottomFlatEnd`, `makeSimpleFlat`,
  `makeSimpleFlatLabels` (`edgelblcmpfn`), and `make_flat_adj_edges` with ports,
  which lays the two nodes out in a nested `Dot` on a cloned graph (rankdir
  turned, `setState`'s ignored attributes stripped, `rank=source` subgraph,
  heavy edge `weight=10000`) and copies the splines back with `transformf`.
* Self loops (`makeSelfEdge`), `splines=curved` (`makeStraightEdges` with the
  cycle centroid), `splines=none|""`, `splines=ortho` (`lib/ortho`, with
  `setEdgeLabelPos`), edge labels (`place_vnlabel`, flat label vnodes), port
  labels (`place_portlabel` when `labelangle`/`labeldistance` exist, else
  `addXLabels`), `edge_normalize`/`swap_spline`.
* `conc.c` (from `dot_position`) and `compound.c` (after splines) run where C
  runs them. `GD_bb` grows with every installed Bézier (`update_bb_bz`) and
  label, so the translation offsets of phase 1 are gone.

Validation beyond the corpus: `splines=true|line|polyline|curved|none|false` ×
`rankdir=TB|LR|BT|RL`, `concentrate=true` and `splines=ortho` over
`graphs/directed`, `graphs/undirected` and `tests/graphs` of the 16.1.0 sources
match C (pos/lp/head_lp/tail_lp/xlp within 2pt) except where C itself exceeds a
60 s limit (curved and ortho on the largest graphs) and `b100.gv`, where C's
`%.5g` output rounding of coordinates above 10^5 is the only difference.

## Known deviations / unported

* `make_flat_adj_edges` with records returns without splines, as C (it only
  warns).
* `record_path` when no top-level field contains the port: C returns `side`
  with stale boxes; the port falls back to the node box.
* `boxIntersectf` failure (C asserts) returns the segment start.
* `dot_concentrate` failures (C aborts the layout with "concentrate=true may not
  work correctly") are ignored.
* `aspect.c` is not ported (16.1.0's `setAspect` only warns).
* Pack branch: a node that belongs to two non-nested clusters (C reports an
  error) is laid out once per component that holds it; C's components share
  the node record, so stale state from the first layout leaks into the
  second. `wild_GraphViz-Samples_complex` with `-Gpack=true` differs.
* `agcontains` of virtual nodes is false (they have ID 0 in C), as in C.
* With `concentrate=true`, `arrow_flags` takes the arrowheads of an opposing
  edge from `agfindedge`. When there are several such edges, which one it finds
  depends on the shape of cgraph's splay tree (`Dot::agfindedge_root`), and that
  shape depends on the node ids, which are the addresses of the node names. The
  port takes the ids to grow in the order the nodes were created. C often
  reuses freed blocks, so its order differs from that in most graphs, and
  sometimes from one run to the next. It only matters for a node with
  in-edges from two tails or more. An edge statement's own `key` names the edge
  (not `edge [key=…]` or `-Ekey=…`, and not a key starting with `%`), whose id is
  the address of the key string, so named edges come after anonymous ones; among
  themselves they are also taken in creation order. A `%` key that is used again
  takes the id of its first edge in C; the port gives each edge a new one. In #289's sweeps (13,000
  generated concentrate inputs), 28 outputs still differ for this reason alone,
  and on 15 more C's own output varies from run to run.

## Validation

These are historical measurements from the original dotgen port; see the
[README](../../README.md#fidelity-against-c-graphviz-1610) for current ratchets.

* `cargo run --release -p graphviz-pure-diff -- --baseline tests/golden/baseline.portable.json`.
  At the switch to dotgen: nodes ok 73.5% (old engine 14.7%), edges 22%, bb 84%.
  After phase 2: exact 499/508, nodes ok 99.1%, edges ok 99.6%, bb ok 98.2%,
  clusters 98.9%, score 98.8%. The dot files left are the four `pack` files;
  the others are non-dot engines (fdp, osage).
* State dumps: `GVDUMP=1 graphviz-pure -Tdot f.gv` prints rank (`R`), mincross order
  (`M r: ...`), aux graph (`A idx name r= o= t= rw= lw=: head/minlen/weight`),
  final x (`P`), TB_balance (`TBPRE`, `TBORDER`, `TB`), decompose (`SC`) and label
  vnodes (`LV`). An instrumented C build with the same dumps lives in
  `~/src/gv-ref/build` (configure + make of the 16.1.0 sources with the
  `GVDUMP` prints in `dotinit.c`, `ns.c`, `decomp.c`, `position.c`, `class2.c`):
  run it with `GVDUMP=1 GVBINDIR=~/src/gv-ref/build/inst/lib/graphviz
  ~/src/gv-ref/build/inst/bin/dot -Tdot f.gv` and `diff` the two stderr outputs.
* Spline dumps (`GVDUMP=1`, both sides, `%.6f`): `RS boxes` (after `checkpath`),
  `RS ends` (start/end point, theta, constrained), `RS poly`, `RS sp`
  (`Pshortestpath`), `RS out` (`Proutespline`/`make_polyline`), in
  `routesplines_`. Ortho: `OE` (sorted edges), `OR` (routes), `OAS`
  (segment → channel), `OCG np|x` (channel graphs), `OPE`/`OSP`
  (parallel-segment decisions), `OT` (tracks). The C side is in
  `~/src/gv-ref/build/src/lib/common/routespl.c` and `lib/ortho/ortho.c`; the
  instrumented build now has expat (HTML labels).
* Note: C's `-Gphase=2` `order` attributes do not reflect the final in-rank
  order; compare `M` dumps instead.

## Phase 3: pack, legacy removal, validation (done)

`doDot` (`pack.rs`): with `pack` or `packmode` set, `cccomps` splits the graph
into components that keep top-level clusters whole. The derived graph finds
its nodes by name, as `agnode(dg, name, 1)` does, so a node named like a
cluster shares (and takes over) the cluster's derived node; `unionNodes` then
drops the cluster's other nodes, which can turn the graph into one component
(`b145`, `fdp.gv`). With one component, or a `ratio`, dot lays the root out
as usual. Otherwise each component becomes a standalone `graphviz_pure_core::Graph`
(`CComp::to_graph`: root graph attributes, induced edges, the projected
subgraphs with their attributes) laid out by its own `Dot` with
`Dot::new_component`, which reproduces what C sees through `agroot`:

* `initSubg`: rankdir, nodesep and ranksep from the root, a zeroed drawing
  (no `size`, no `ranksep=equally`), no root label;
* `GD_has_labels(g->root)` holds the root's flags so far (its graph label
  and the components initialised before), while the component's own record
  has none, so `edgelabel_ranks` does not double ranks;
* `build_ranks` walks backwards (`g != agroot(g)`) at the component root;
* `Dot::root_input` answers `agattr(agroot(g), ...)` declaration checks over
  the whole graph.

The node and edge records are copied into the root's `Dot` (C shares them),
each component's cluster records are kept apart (`PClust`, the projected
subgraphs) and `lib/pack` packs through the `PackGraph` trait (`DotPack`:
`ND_pos` from `attachPos`, `compute_bb` per component, `l_clust` with dot's
`ND_clust`, array modes, `sortv`). `packSubgraphs` sets the root box,
`resetCoord` and `copyClusterInfo` (later components win) follow, and
postprocessing runs with `EDGE_TYPE(root) = EDGETYPE_NONE` (the root never
had `setEdgeType`), so edge labels and xlabels stay unplaced as in C: the
output marks them (`label_unplaced`, the `set` flag of `EdgeLayout::head_label`/`tail_label`) and
the renderers do not draw them.

Other fixes found on the way: command-line `-G/-N/-E` attributes are `fixed`
at the root (`global_def`), so root statements cannot override them (parser);
`lib/ortho`'s search graph keeps `initSEdges`' 6 adjacency slots per node,
whose overflow into the next node C relies on; its point dictionaries use the
cdt splay tree; `edge_attr_declared` is cached (it made splines quadratic).

### Validation (phase 3)

Historical phase-3 measurements follow; the current ratchets are in
[README](../../README.md#fidelity-against-c-graphviz-1610).

* Main corpus: 499/508 -> 508/508 exact before the additions; with the six
  `tests/wild/phase3_*` cases (pack/packmode, records/samehead, neato ortho
  components) 514/514. `-Tsvg` byte identity (`gv-diff e2e`): main 514/514.
* The main corpus with `-Gpack=true`, `-Gpackmode=clust`, `-Gpackmode=node`
  and `-Gpackmode=array_t3`: 507, 508, 507, 507 of 508 exact (the misses are
  `complex`, see deviations).
* `scripts/dot-broad-validate.sh c|port` lays out the 379 graphs of
  `graphs/**`, `tests/graphs/**` and `tests/regression_tests/**` with C and
  the port (60 s cap) as is and with `-Grankdir=LR`, `-Gsplines=ortho`,
  `-Gconcentrate=true`, `-Gnewrank=true`; `gv-diff pairs C_DIR PORT_DIR`
  compares them. Exact: 374/379 in every variant at first; the five misses were the Latin-1
  inputs, exact (379/379) since the charset work (`render.md`, Charset). With
  `-Gsplines=ortho`, C needs more than 60 s on `b100`, `b103` and `b104`; given
  longer, its output equals the port's byte for byte, so ortho is 379/379 too.
* Timing (min of 3 process runs, `-Tsvg`, shared machine): b100 4.5 s (C) /
  4.7 s (port), b104 4.6/4.6, b103 0.73/0.69, b29 0.36/0.27, random DAGs of
  2000 and 8000 nodes 0.56/0.46 and 24.5/20.0, 40x40 grid 0.12/0.08.
  `splines=ortho`: root 3.6/0.36, badvoro 20.3/0.80, b100 >300/20.1.

## Handoff

* `pack` with nodes in two non-nested clusters would need the components to
  share node records (C lays such nodes out repeatedly).
