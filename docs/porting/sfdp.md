# sfdp port (Graphviz 16.1.0)

`layout=sfdp` (and `-Ksfdp`) now runs a function-by-function port of
`lib/sfdpgen` and the parts of `lib/sparse` it needs. It reuses the neato
port's shared infrastructure: `lib/pack`, `lib/common` (`LGraph`, splines,
postprocessing), neato's `spline_edges`, `removeOverlapWith` (adjust.c) and the
prism overlap removal (`overlap.c`, `call_tri.c`, `delaunay.c`). The old
stand-in (the `is_sfdp` branch of `force_circular::layout_fdp`) is removed.

Entry point: `graphviz_pure_layout::sfdpgen::sfdpinit::sfdp_layout(&mut Graph)`.

## How Homebrew's 16.1.0 is built

`libgvplugin_neato_layout.8.dylib` links `libgts-0.7.5` (no `triangle`, no
ANN). So `HAVE_GTS` and `SFDP` are defined:

- `graphAdjustMode(g, &am, "prism0")`: with no `overlap` attribute sfdp uses
  prism with 0 tries, i.e. only `remove_overlap`'s initial scaling
  (`overlap_scaling`, default -4: average edge length = 4 × average node
  size). `overlap=false` is prism with 1000 tries; `overlap=true` disables the
  scaling; other modes (`scale`, `voronoi`, ...) run after sfdp via
  `removeOverlapWith`.
- `smoothing=triangle|rng` are available (`late_smooth`).
- Delaunay triangulation is GTS (`delaunay_tri`, `UG_graph`).

## Fidelity

`gv-diff` against C 16.1.0 goldens (built-in text metrics,
`scripts/gen-golden.sh`):

| corpus | before (old approximation) | after |
|---|---|---|
| `tests/sfdp` (292 files at the start, now 297) | 70 exact, score 99.9/292 (34%), nodes 68/13022 | 293 exact, score 294.0/297 (99.0%) |
| main corpus, the two `layout=sfdp` files (`gallery_beautify_*`) | 0/7 nodes each | both exact |
| main corpus overall (508 files) | 250 exact, score 408.9 | 252 exact, score 410.7, no regressions |

Layouts are bit-identical to C, including multilevel coarsening, quadtree
Barnes–Hut forces, all smoothers and overlap removal, on graphs up to 12000
nodes (checked locally beyond the corpus: 5000-node random, 3000-node `dim=3`
octree, `quadtree=fast` with 12000 nodes, 7 packed components of 143 nodes,
`overlap=false` on 600 boxes). The remaining non-exact corpus files:

- `splines=ortho`: routed by `lib/ortho` (see `docs/porting/neato.md`);
  `overlap=ortho*` and `overlap=vpsc` are ported (opus5.5/neato-gaps).
- `label_scheme=4` with `overlap=false`: the penalty iteration is
  chaotic (C itself runs ~180 tries; scheme 2 diverges to 1e142 in C). C's
  Delaunay edges come from GTS's pointer-keyed hash tables, so the row order
  of the triangulation matrix (and therefore summation order in the CG
  solver) differs from the port by an ulp, which this case amplifies.
  Non-chaotic overlap removal is unaffected.
- `label_scheme=2` with `overlap=false` on a two-neighbour label (#493) is
  the same chaos, and the edge order is the only difference: the port
  matches C bit for bit for 33 of the ~390 tries, then `call_tri`
  (`overlap.c`) returns the same entries in another row order (GTS hands
  `delaunay_tri` the triangle as `2-0 2-1 1-0`; the port's `delaunay_edges`
  sorts `0-1 0-2 1-2`), which shifts one CG sum by an ulp. The coordinates
  blow up to 1e141 on both sides of that point; C then recovers and stops
  after ~390 tries, the port stops at 338 without recovering. Replaying
  C's GTS edge list call by call makes the port's output byte-identical.
  The order is still not reproducible: it comes from the face order of
  GTS's incremental flips plus `gts_surface_foreach_edge`'s pointer-keyed
  face hash (`src/surface.c`), and within one run GTS returns five
  different orders or orientations for the same three points. Unlike
  `smoothing=rng`, C's own result here is stable run to run.
- A label with three or more neighbours (#494, #496) is not a separate
  gap. Without the penalty iteration (default `overlap`, `prism0`) the port
  is byte-identical to C, multilevel coarsening of the 4-node graph
  included, and with `overlap=false` it enters the first overlap-removal
  try with C's exact coordinates. The ulp drift and the scheme-4 offset
  reported there came from C builds that are not the reference (x86-64
  without fused multiply-adds, stock libm; see "Comparing on Linux x86-64"
  below). From try 1 on, C's own result changes from run to run: on Linux
  `delaunay_tri` returns 4 different edge orders for the same 4 points
  across 40 runs, and each order gives a different try-1 smoothing. Replaying C's
  edge lists, the port matches C bit for bit for tries 0–174. At try 175
  the coordinates are about 1e76, the in-circle determinant against GTS's
  enclosing triangle (100 times wider) overflows, and GTS's flip-based
  insertion and the port's Bowyer–Watson then disagree on the edge set
  (5 and 6 edges). Like #493, the result depends on GTS's internal order
  and is not reproducible from the points.

### Comparing on Linux x86-64

The reference build is macOS arm64 (`docs/porting/libm.md`). A C build
on x86-64 Linux reproduces it for sfdp only with all three of:

- fused multiply-adds: `CFLAGS='-O2 -mfma'`. clang contracts the same
  expressions as on arm64, while the baseline x86-64 build fuses nothing;
- the port's math functions: `scripts/dot-portable-libm.sh`;
- Apple's `rand`, `srand` and `qsort`: `graphviz-pure-portable-libm` built
  with `--features apple-libc` (glibc's `rand` is another generator).

With all three, C matches 296 of the 297 `tests/golden/sfdp` goldens
(the stock x86-64 build with the port's math and Apple's libc matches 44
of the first 120). The other is
`wild_gv16_linux_x86_neatosplines_neato1_448ced`. GTS's edge order still
varies from run to run on Linux, so the chaotic penalty cases above
cannot be compared there.

Floating point follows clang's `-ffp-contract=on` on arm64 as in the neato
port: `a*b + c` is `fma(a, b, c)` (`mul_add`), `c - a*b` is `fma(-a, b, c)`,
the left product is fused in `a*b + c*d`, divisions and comparisons are never
fused. `pow`, `sqrt`, `cos`, `sin` go to the same libm.

### Random numbers

sfdp uses libc `rand()` only (`cutil::CRand`, threaded through `LGraph::crand`):

- `drand()` = `rand() / RAND_MAX` (`lib/sparse/general.c`).
- Each spring–electrical embedding with `random_start` calls
  `srand(ctrl->random_seed)` (default 123, `start=N` overrides it).
- `gv_permutation` (`lib/util/random.c`, ported as `cutil::gv_permutation`)
  in the heaviest-edge matching runs *before* that `srand`, so the first
  component's coarsening continues the process's initial `rand()` state
  (seed 1) and later components continue from the previous component's
  draws (prolongation noise, smoother perturbations).

### Validation

- `cargo run --release -p graphviz-pure-diff -- --golden tests/golden/sfdp --baseline tests/golden/sfdp-baseline.json tests/sfdp`
  (ratchet test: `crates/graphviz-pure-diff/tests/sfdp_fidelity.rs`).
- Corpus: `tests/sfdp/*.gv` from `scripts/gen-sfdp-corpus.py`: fixtures and
  undirected wild graphs with `layout=sfdp` forced; grids up to 20×20 and
  8×40, binary/ternary trees up to 1023 nodes, connected random graphs of
  100–2000 nodes, preferential-attachment (power-law, `p = -1.8`) trees of
  300/1000 nodes; attribute cases for `K`, `repulsiveforce`, `levels`,
  `smoothing` (all but `rng`), `quadtree`, `beautify`, `rotation`, `overlap`
  modes (`prism`, `prismN`, `false`, `true`, `scale`, ...), `overlap_scaling`,
  `overlap_shrink`, `sep`, `start`, `dim=3`, `splines`, `label_scheme`,
  pack/packmode, size/ratio. Goldens are 3 MB (`gen_rnd2000` is the largest).
- Excluded: `smoothing=rng` (C output changes run to run: `UG_graph` walks a
  GTS hash table keyed by pointers); `label_scheme=1|3` with `overlap=false`
  (C 16.1.0 segfaults: `x00` is NULL in `StressMajorizationSmoother_smooth`).
  Scheme 1 with no distinct neighbour pair now uses ordinary overlap removal.
- Instrumentable C copy: `~/src/gv-ref/build-sfdp` (`run.sh`; rebuild with
  `make -C src/lib/neatogen && make -C src/plugin/neato_layout` and copy the
  dylib into `plugins/`). Shared libraries come from `build-neato/inst`.

## Module ↔ C file map

### `graphviz_pure_layout::sfdpgen`

| Rust | C |
|---|---|
| `sfdpinit` | `sfdpinit.c`: `sfdp_layout`, `sfdp_init_graph`, `sfdpLayout`, `tuneControl`, `late_smooth`, `late_quadtree_scheme` (`getPos` is all zeros because `random_start` is always set) |
| `spring_electrical` | `spring_electrical.c`: `spring_electrical_control_new` (`Default`), `oned_optimizer_*`, `average_edge_length`, `update_step`, `beautify_leaves`, `set_leaves`, `spring_electrical_embedding`, `_fast`, `_slow`, `spring_electrical_spring_embedding`, `interpolate_coord`, `prolongate`, `power_law_graph`, `pcp_rotate`, `rotate`, `attach_edge_label_coordinates`, `shorting_edge_label_nodes`, `multilevel_spring_electrical_embedding` |
| `multilevel` | `Multilevel.c`: `Multilevel_new`, `Multilevel_establish`, `Multilevel_coarsen`, `Multilevel_coarsen_internal`, `maximal_independent_edge_set_heavest_edge_pernode_supernodes_first` (levels are a `Vec`, finest first) |
| `post_process` | `post_process.c`: `ideal_distance_matrix`, `StressMajorizationSmoother2_new`, `SparseStressMajorizationSmoother_new`, `StressMajorizationSmoother_smooth` (`smooth`/`smooth_with`, incl. `SM_SCHEME_NORMAL_ELABEL`), `get_edge_label_matrix`, `TriangleSmoother_new`, `SpringSmoother_new`/`_smooth`, `post_process_smoothing`; `sparse_solve.c` (`SparseMatrix_solve`, CG with diagonal preconditioner) |
| `stress_model` | `stress_model.c` (`stress_model`; not called by `sfdp_layout`) |

### `graphviz_pure_layout::sparse`

| Rust | C |
|---|---|
| `mod.rs` (from the neato port) | `SparseMatrix.c` basics, `general.c` (`distance`, `drand`, ...) |
| `matrix_ext` | `SparseMatrix_multiply`, `_multiply3`, `_divide_row_by_degree`, `_has_diagonal`, `_decompose_to_supervariables`; `from_csr` replaces `SparseMatrix_new` + in-place fill |
| `quad_tree` | `QuadTree.c`: `QuadTree_new`, `_new_from_point_list`, `_add` (with C's off-by-one average at the last level), `_get_supernodes`, `_get_repulsive_force` (cell–cell interaction and push-down), `point_distance`. Cells are an arena; each cell's point list is a `Vec` walked newest-first like C's prepend-linked list |

### Additions to shared modules

- `cutil`: `gv_random`, `gv_permutation` (`lib/util/random.c`).
- `neatogen::overlap`: `make_matrix` (`makeMatrix`), `get_sizes` (`getSizes`),
  now also used by `fdp_adjust`; `call_tri` is public; `remove_overlap_el`
  carries `edge_labeling_scheme`/constraint nodes and the penalty-free second
  pass (`remove_overlap` delegates to it with no constraints).
- `neatogen::delaunay`: `delaunay_edges` (raw edges, empty when collinear;
  `delaunay_tri` adds the collinear chain), `ug_graph` (`UG_graph`).
- `neatogen::call_tri`: `call_tri2`.
- `neatogen::neatoinit::set_seed` is public.

## Gaps

- GTS edge order (see Fidelity) cannot be reproduced; it only shows in
  chaotic cases and in `smoothing=rng`, which is not reproducible in C either.
- `label_scheme=1|3` with prism tries: C crashes; the port skips the missing
  right-hand side instead. Scheme 1 with no distinct neighbour pair now uses
  ordinary overlap removal.
- `start=random` without a number seeds C from the clock; the port uses 1.
- `smoothing=avg_dist`, `graph_dist` and `power_dist` with an isolated node
  (nothing adjacent to it; a self-edge does not count). C's
  `StressMajorizationSmoother2_new` asserts `nz > 0` (`post_process.c`) and
  writes nothing. The port keeps that node's unsmoothed position, the same
  drawing as `smoothing=none`. A component whose nodes all have a neighbour
  is still smoothed.
- Verbose (`-v`) diagnostics, `spring_electrical_control_print` included, are C's, except
  those under `DEBUG_PRINT` and `TIME` (not defined in a release build of C either).
