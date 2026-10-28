# circo and twopi ports (Graphviz 16.1.0)

`layout=circo` / `-Kcirco` and `layout=twopi` / `-Ktwopi` now run
function-by-function ports of `lib/circogen` and `lib/twopigen`. The old
approximations (`force_circular::layout_circo`, `layout_twopi`) are removed.

Entry points: `graphviz_pure_layout::circogen::circularinit::circo_layout(&mut Graph)` and
`graphviz_pure_layout::twopigen::twopiinit::twopi_layout(&mut Graph)`.

Both reuse the shared neato-family infrastructure (`common::LGraph`, `pack`,
`neatogen::adjust::adjust_nodes`, `neatogen::neatosplines::spline_edges`,
`common::postproc::gv_postprocess`, `cutil::qsort_by`) as C does.

## Fidelity

Re-measured on 2026-10-28 with the circo and twopi commands in
`scripts/update-baselines.sh`, without `--update`, against the `*.portable.json` baselines:
**circo 282 / 282 exact; twopi 281 / 289 exact**, with no regressions.
All stable inputs in both corpora match byte for byte in `gv-diff e2e`.

Remaining differences:

- twopi `splines=true` on the arrows graphs (8 files): `i`, `Z` and `I` are
  exactly vertically aligned, and which side the pathplan router passes an
  obstacle vertex on is a floating-point tie. C does not even break it the same
  way from run to run: the same C binary gives 3–4 different outputs over 20
  runs, so there is no single C answer to match (issue
  `tests/fuzz-open/032-twopi-arrows-c-nondeterministic-routing`).

## Module ↔ C file map

### `graphviz_pure_layout::twopigen`

| Rust | C |
|---|---|
| `circle::circle_layout` | `circleLayout` |
| `circle::{init_layout, is_leaf, set_n_steps_to_leaf, find_center_node}` | `initLayout`, `isLeaf`, `setNStepsToLeaf`, `findCenterNode` |
| `circle::{set_n_steps_to_center, set_parent_nodes, set_subtree_size}` | `setNStepsToCenter` (BFS, skips `weight="0"`), `setParentNodes`, `setSubtreeSize` |
| `circle::{set_subtree_spans, set_child_subtree_spans, set_positions, set_child_positions}` | same names |
| `circle::{get_rankseps, set_absolute_pos}` | `getRankseps` (`ranksep` list, `MIN_RANKSEP`), `setAbsolutePos` |
| `circle::RData` | `rdata` (`ND_alg`) |
| `twopiinit::twopi_layout` | `twopi_layout`, `twopi_init_graph`, `findRootNode` |

### `graphviz_pure_layout::circogen`

| Rust | C |
|---|---|
| `circularinit::circo_layout` | `circo_layout`, `circo_init_graph` |
| `circularinit::circo_layout_main` | `circoLayout` |
| `circularinit::{derived_graph, component}` | `circomps`, `makeDerivedNode` |
| `circularinit::copy_posns`, `pack_derived` | `copyPosns`, `packSubgraphs` on the derived graph |
| `circularinit::{derived_src, derived_lgraph, adjust_derived}` | `adjustNodes(sg)` on the derived graph's components |
| `circular::circular_layout` | `circularLayout` |
| `circular::{init_graph_attrs, create_one_block}` | `initGraphAttrs`, `createOneBlock` |
| `circular::{CData, Circ}` | `cdata` (`ND_alg`), `circ_state`, derived `ND_pos` |
| `blocktree::{dfs, find_blocks, create_blocktree}` | same names (`addNode`, `makeBlock` inlined) |
| `blockpath::layout_block` | `layout_block` |
| `blockpath::{clone_graph, get_list, cmp_degree, find_pair_edges, remove_pair_edges}` | same names (`deglist` is the qsorted list) |
| `blockpath::{spanning_tree, dfs, find_longest_path, measure_distance}` | same names |
| `blockpath::{place_residual_nodes, place_node, block_graph, largest_nodesize}` | same names |
| `blockpath::{count_all_crossings, reduce, reduce_edge_crossings}` | same names |
| `circpos::{circ_pos, do_block, position, position_children, get_info, set_info, get_rotation, apply_delta}` | `circPos`, `doBlock`, … |
| `block`, `nodelist`, `edgelist` | `block.c`, `nodelist.c`, `edgelist.c` |
| `graph::{CGraph, EdgeArena}` | the cgraph operations circo uses (see below) |

`deglist.c` no longer exists in 16.1.0; its job is the qsorted `deglist_t` in
`blockpath.c`.

## Notes and deliberate modelling choices

- **Derived graph.** circo works on a strict, undirected, loop-free copy of
  the graph. `graph::CGraph` models it and every graph built from it (block
  subgraphs, the `_clone_` subgraph and root copy, the spanning tree) with
  cgraph's iteration order: nodes by sequence, `agfstout` by head sequence,
  `agfstedge` = out-edges then non-loop in-edges. Node ids are original node
  indices (derived and clone nodes are created in that order).
- **`cdata` union.** The union members are separate fields; each pass writes
  its fields before reading them, so the aliasing is unobservable.
- **Edge halves in `count_all_crossings`.** C keys the open-edge set by
  `Agedge_t *`; an edge opened from its tail (out-half) is never found when
  removed from its head (in-half), so closed edges stay open and keep being
  counted. `edgelist` keys by `(edge, half)` to reproduce the counts, which
  drive crossing reduction.
- **Pointer comparisons.** `find_pair_edges` counts each neighbour pair once
  via `n1 < n2` on addresses; any strict order gives the same count and
  deletes the same skeleton edge, so node ids are compared.
- **Packing.** C packs the derived graph's components. The port packs the
  original nodes with the adjacency temporarily reduced to the derived edges
  (first-seen orientation, no loops or duplicates), which gives the same
  polyomino cells.
- **Derived graph attributes.** With several components, C runs `adjustNodes(sg)` on each
  component of the derived graph. `agopen("derived")` copies cgraph's ProtoGraph, so that graph
  declares only the command-line attributes (`-Goverlap`, `-Gnormalize`, `-Gscale`, `-Gsep`, …),
  not the input's, and its nodes have the `lw`, `rw` and `ht` that `makeDerivedNode` copies but
  width and height 0. So `-Goverlap=false` overlaps the nodes, while in-file `overlap` does
  nothing there (#398). The port builds that graph (`derived_src`, `derived_lgraph`, from
  `Graph::protograph_root`) only when it has `overlap`, `normalize` or `scale`.
- **twopi `root`.** `root=""` or an unknown name sets `root` to the chosen
  centre in the output (also in the root attribute dictionary, as `agset`
  does); a declared node `root` attribute is set to `1` on chosen centres.
  The C crash on a node `root` attribute with isolated nodes is avoided in
  the corpus, not reproduced.
- **Messages.** C's "use of weight=0 creates disconnected component" error
  is not printed; positions stay at the origin as in C.

## Shared fixes made on this branch

- `graphviz-pure-geom` `poly_init`: the polygon vertex walk and `1 - SQR(r)` use the
  fused multiply-adds clang emits (checked against the libgvc disassembly);
  `ND_ht` was one ulp off for shapes like `invhouse`, moving pack cells.
- `neatogen::delaunay`: exact-sign `orient2d`/`incircle`
  (`neatogen::robust`: Shewchuk's float filter, exact big-integer fallback)
  and GTS's `gts_triangle_enclosing` triangle. The inexact Bowyer–Watson
  produced non-planar edge sets on co-circular circo/twopi positions, so
  prism overlap removal diverged.
- `common::shapes` `record_inside` clips against the record root field box,
  not `ND_ht` (which carries `record_init`'s `+1`).
- `LGraph::write_back` hands placed graph/cluster label positions to the
  renderer (fixes `lp` for `rankdir` graphs in all neato-family engines) and
  marks the graph so `-Tdot` does not apply dot's `mark_lowclusters` edge
  placement.

## Validation

- `cargo run --release -p graphviz-pure-diff -- --golden tests/golden/circo --baseline tests/golden/circo-baseline.json tests/circo`
  and the same for `twopi` (ratchet tests: `crates/graphviz-pure-diff/tests/circo_fidelity.rs`,
  `twopi_fidelity.rs`).
- Corpora: `scripts/gen-circo-corpus.py` and `scripts/gen-twopi-corpus.py`
  force `layout=` on the fixtures and undirected wild graphs and add
  hand-written cases: trees, cycles, complete graphs, biconnected blocks, cut
  vertices, chains of cycles, disconnected graphs with `pack`/`packmode`,
  `root` (graph and node), `ranksep` lists, `mindist`, `oneblock`, `overlap`,
  `splines`, weights (`weight=0` for twopi), loops and multi-edges.
  Goldens: `scripts/gen-golden.sh` writes `tests/golden/{circo,twopi}/`.
- An instrumentable C copy is in `~/src/gv-ref/build-circo` (`run.sh`; set
  `CIRCO_DBG=1` for block, crossing, pack and triangulation traces).
