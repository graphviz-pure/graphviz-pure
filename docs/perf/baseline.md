# Performance and memory baseline (0.2.2)

Where graphviz-pure spends its time and memory, how that compares with C Graphviz 16.1.0, and
which functions to work on first. This is the starting point for the post-1.0 work on speed and
memory, which must keep every output byte-identical. Nothing here was optimised: the numbers are
for `master` at 0.2.2 (`f9c2fd1d`). The raw data is in [`data/`](data/), and
`scripts/bench/` regenerates it ([Reproducing](#reproducing)).

## Headlines

- **dot, fdp and circo are already faster than C**: dot is 0.5–0.9× C's time (geometric mean
  0.70×), fdp is 0.4–0.6× and circo 0.1–0.3× on large inputs.
- **neato is 4.7× slower than C and sfdp 2.9× slower**, at the same iteration counts. Most of
  the gap is one thing: on x86-64 every `f32::mul_add`/`f64::mul_add` is a call into
  compiler-builtins' software `fma`/`fmaf`, because the x86-64 baseline target has no FMA
  instruction. Building with `-C target-feature=+fma` makes neato 4.9× faster (to C's speed),
  sfdp 2.3×, fdp 2.0–2.1× and spline routing 2.1×, and the SVG output stays byte-identical on all
  2,603 corpus inputs it was checked on (every input but the six slow `badvoro` copies). This is hotspot 1 below. aarch64 (Apple Silicon, Linux arm64)
  has FMA in its baseline, so this applies to x86-64 only, which includes the WASM build.
- **`-Tplain` is quadratic in graph size**: `root_holds_plain` scans the whole graph for every
  label it writes. osage on 20,000 nodes takes 8.9 s with `-Tplain` and 0.44 s with `-Tsvg`;
  C takes 0.2 s for both. This is the worst scaling found.
- **Memory**: the port's peak RSS is 1.0–3.1× C's. The ratio grows with size on graphs with
  many nodes (twopi tree with 20k nodes: 142 MiB against C's 45 MiB; sfdp 20k: 168 MiB against
  57 MiB). The port's floor for an empty graph is 6.6 MB, against C's 4.0 MB.
- **Scaling** follows C's: dot ~n^2.2 (network simplex for x positions), neato ~n^2.1, fdp
  ~n^2.0, circo ~n^2.9 (C ~n^3.4), sfdp/twopi/osage/patchwork ~n^1.0–1.2.

## Method

**Machine.** AMD Ryzen 9 5900XT (16 cores, 28 visible to the VM, no SMT), 86 GB RAM, Debian
13 (Linux 6.12.113 cloud kernel), glibc 2.41, Python 3.13.5, rustc 1.99.1, node 24.22.1. Four
other agents shared the machine, so the load average was 3–15 during the runs (each row of
`data/full-0.2.2.json` records it). Nothing was pinned to a core. Every benchmark run is
single-threaded and sequential, so the noise comes from shared caches, memory bandwidth and
frequency changes ([Noise](#noise)).

**Port build.** `cargo build --release -p graphviz-pure-cli`, which uses the release profile in
`.cargo/config.toml`: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. `panic` keeps the
default (`unwind`), and there is no `target-cpu` or `target-feature`, so this is the x86-64-v1
baseline that release binaries also use. The profile is in `.cargo/config.toml`, not in
`Cargo.toml`'s `[profile.release]`, so `cargo install graphviz-pure-cli` from crates.io builds
with Cargo's defaults instead (no LTO, 16 codegen units). The CLI binary is 13.4 MB, not stripped.

**C build.** `/opt/graphviz-16.1.0/bin/dot`, run with `GVBINDIR=/opt/graphviz-16.1.0/lutplugins`:
the pango-free plugin directory of the reference setup (only the core, dot and neato layout
plugins), so C measures text with the same built-in font metric tables as the port
([libm.md](../porting/libm.md)). C runs with the system glibc libm, not
`scripts/dot-portable-libm.sh`. The preload would add the port's slower math to C's time, and
neither side's timing is meant to be about libm. This Linux build of C compiles `a*b+c` as a
multiply and an add, while the macOS arm64 build that wrote the goldens contracts it to an FMA,
so on a minority of fdp, neato and sfdp inputs C lays out a slightly different graph. The
iteration counts were the same where checked (neato on `neato_sparse_500`: 180 stress iterations
in both; sfdp's multilevel parameters matched), so the ratios compare equal work. C has no cairo
in this setup, so there is no C `-Tpng`.

**Inputs.** `scripts/bench/sets/full.tsv` lists every case with the formats and run counts used.
They come from:
- a survey of every corpus input (`bench.py survey`, 2,609 inputs, port and C, one run each;
  [`data/survey-0.2.2.tsv`](data/survey-0.2.2.tsv)). From it the set takes the slowest inputs of
  each engine and a typical small one (`tests/fixtures/unix.gv`);
- synthetic scaling graphs from `scripts/bench/synth.py`, which are deterministic (a fixed LCG):
  `dot_dag_N` (a DAG, edges to the next 40 nodes, ~1.5 N edges), `*_sparse_N` (a random
  spanning tree plus N/2 edges), `twopi_tree_N` and `osage/patchwork_clustered_N` (nested
  clusters of about 20 nodes). circo, neato and fdp stop at 400, 2,000 and 1,000 nodes:
  `circo_sparse_1000` takes the port 133 s and C more than 8 minutes.

**Timing.** `bench.py run` runs each case once to warm up, then 5 times (once for the inputs
over about 15 s, the third column of the set). It reports the median wall time and its
min–max, and also the CPU time. Wall time is measured around the process, so it includes
process start-up (3.7 ms for both port and C on a two-node graph) and writing the output to
`/dev/null`.

**Memory.** Peak RSS comes from `/usr/bin/time`'s `%M`. `wait4`'s `ru_maxrss` from Python is not
usable here: a child forked from Python starts at Python's own RSS (about 23 MB), and Linux keeps
that high-water mark across `exec`, so every process smaller than Python reads as 23 MB.

**Allocations.** These were counted with a scratch crate outside the repository. It `include!`s
the CLI's `main.rs` and wraps the system allocator in a counting `#[global_allocator]`
([recipe](#allocation-counts-recipe)). The crate needs `unsafe` (`GlobalAlloc`), so it is not
committed.

**Profiles.** `perf record -F 1999 --call-graph fp` on a release build with
`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` and `RUSTFLAGS="-C force-frame-pointers=yes"`
(same optimisation settings). Callers of malloc were attributed with a uprobe on glibc's
`malloc` (`perf probe -x libc.so.6 malloc`, then `perf record -e probe_libc:malloc --call-graph fp`).
`kernel.perf_event_paranoid` was 3, so `perf` ran under sudo.

## Results

Port and C, `-Tsvg` unless marked, median of 5 (or of the runs given). Ratios below 1 mean the
port is faster or smaller. Full table with min/max and CPU time:
[`data/full-0.2.2.tsv`](data/full-0.2.2.tsv). The `+fma` build's numbers are in
[`data/fma-whatif-0.2.2.json`](data/fma-whatif-0.2.2.json).

### By engine

Geometric mean of port/C over the `-Tsvg` cases of each engine in `full.tsv`:

| engine | cases | time port/C | range | peak RSS port/C | range |
| :-- | --: | --: | :-- | --: | :-- |
| dot | 9 | **0.70** | 0.48–0.93 | 1.59 | 1.30–2.10 |
| neato | 9 | **4.70** | 3.13–5.31 | 1.20 | 1.08–1.44 |
| fdp | 7 | **0.55** | 0.38–2.10 | 1.24 | 1.11–1.37 |
| sfdp | 8 | **2.90** | 2.51–4.62 | 1.69 | 1.12–2.97 |
| circo | 7 | **0.33** | 0.09–2.20 | 1.01 | 0.74–1.11 |
| twopi | 6 | **1.56** | 0.96–2.16 | 1.55 | 1.03–3.14 |
| osage | 5 | **1.46** | 1.02–1.96 | 1.24 | 1.07–1.52 |
| patchwork | 5 | **1.25** | 0.93–1.73 | 1.22 | 1.04–1.52 |

Across the whole corpus (the survey, one run each, 6 in parallel), total time port/C by
directory: dot `wild` 0.84, `fixtures` 1.10, neato 3.48, sfdp 2.16, fdp 0.45, circo 0.21, twopi
1.06, osage 0.74, patchwork 0.76. Most corpus inputs (2,327 of 2,609) take the port under 10 ms.
At that size the median port/C ratio is 1.03 for dot but about 0.73 for every other engine.
The reason is start-up: C's `neato_layout` plugin, which serves all the non-dot engines, links
22 shared libraries where the dot plugin links 12. A two-node neato graph takes C 5.0 ms and the
port 3.7 ms, while a two-node dot graph takes both 3.9 ms.

### Corpus inputs

| input | -T | port ms | port MiB | C ms | C MiB | time ×C | RSS ×C |
| :-- | :-- | --: | --: | --: | --: | --: | --: |
| fixtures/unix.gv | svg | 6.9 | 6.9 | 7.4 | 4.2 | 0.93 | 1.65 |
| fixtures/unix.gv | plain | 7.8 | 7.0 | 5.6 | 4.2 | 1.41 | 1.68 |
| fixtures/unix.gv | png | 29.2 | 19.5 | | | | |
| wild/gv16_graphs_b29 | svg | 948 | 20.1 | 1,674 | 15.4 | 0.57 | 1.30 |
| wild/gv16_graphs_b29 | plain | 1,019 | 20.2 | 1,714 | 15.4 | 0.59 | 1.31 |
| wild/gv16_graphs_b29 | png | 2,319 | 192.7 | | | | |
| wild/gv16_graphs_b143 | svg | 225 | 16.5 | 259 | 9.4 | 0.87 | 1.76 |
| wild/gv16_graphs_b143 | png | 1,148 | 219.9 | | | | |
| wild/gv16_graphs_b106 | svg | 186 | 16.3 | 236 | 7.8 | 0.79 | 2.10 |
| wild/gv16_graphs_badvoro | svg | 589 | 21.7 | 745 | 14.1 | 0.79 | 1.53 |
| neato/badvoro (1 run) | svg | 20,037 | 16.3 | 3,803 | 13.6 | 5.27 | 1.20 |
| neato/hw3_vpsc_random200 | svg | 413 | 8.1 | 85 | 7.1 | 4.85 | 1.14 |
| neato/hw3_vpsc_random200 | png | 522 | 41.1 | | | | |
| neato/hw2_random_120 | svg | 224 | 6.8 | 50 | 6.2 | 4.48 | 1.10 |
| neato/fx_NaN | svg | 102 | 7.1 | 24 | 6.5 | 4.28 | 1.08 |
| fdp/badvoro (1 run) | svg | 46,554 | 18.9 | 108,675 | 14.5 | 0.43 | 1.30 |
| fdp/gv16_graphs_viewport | svg | 970 | 8.7 | 1,630 | 7.0 | 0.60 | 1.25 |
| fdp/gv16_undirected_ngk10_4 | svg | 241 | 8.4 | 115 | 7.6 | 2.10 | 1.11 |
| sfdp/gen_rnd2000 | svg | 1,872 | 20.6 | 674 | 11.8 | 2.78 | 1.74 |
| sfdp/gen_rnd2000 | png | 1,944 | 32.1 | | | | |
| sfdp/gen_tree9 | svg | 624 | 13.2 | 225 | 8.9 | 2.77 | 1.48 |
| sfdp/attr_quadtree_none | svg | 205 | 7.1 | 44 | 6.3 | 4.62 | 1.12 |
| circo/badvoro (1 run) | svg | 14,282 | 15.5 | 77,101 | 21.0 | 0.19 | 0.74 |
| circo/linux_x86_dotsplines_dot1 | svg | 236 | 7.0 | 107 | 7.2 | 2.20 | 0.98 |
| circo/gv16_directed_sdh | svg | 127 | 7.4 | 630 | 6.8 | 0.20 | 1.08 |
| twopi/gv16_undirected_ngk10_4 | svg | 225 | 8.2 | 104 | 7.6 | 2.16 | 1.08 |
| twopi/fx_arrows | svg | 89 | 7.3 | 45 | 7.0 | 1.96 | 1.04 |
| osage/gv16_graphs_b51 | svg | 8.5 | 7.0 | 8.3 | 6.6 | 1.02 | 1.07 |
| patchwork/gv16_graphs_b51 | svg | 7.2 | 6.8 | 7.7 | 6.5 | 0.93 | 1.04 |

The engine cases above 2× C (fdp and twopi on `ngk10_4`, circo on `dotsplines_dot1`,
twopi's arrows) all have `splines=true` or many arrowheads, so most of their time is in spline
routing and its `mul_add` calls (hotspots 1 and 9).

### Render cost by output format

`-Tsvg` and `-Tplain` cost the same on dot's graphs, so layout dominates there. `-Tpng` adds
1.4 s and 172 MiB on b29, and 0.9 s and 203 MiB on b143. That cost goes to the resvg/tiny-skia
rasterizer (41% of the b29 PNG profile) and to zlib compression of the pixels (miniz_oxide
`compress_inner`, 25%), and the memory to the full-size pixmap. `-Tplain` scales quadratically
on large graphs (hotspot 2):

| graph | -Tsvg | -Tplain | C -Tplain |
| :-- | --: | --: | --: |
| osage_clustered 1k / 5k / 20k | 21 / 96 / 436 ms | 20 / 570 / 8,880 ms | – / – / 200 ms |
| twopi_tree 1k / 5k / 20k | 25 / 128 / 646 ms | 30 / 470 / 4,437 ms | – / – / 722 ms |

### Scaling

Synthetic graphs, `-Tsvg`, wall time (MiB peak RSS). The exponent is the log-log slope between
the smallest and largest size.

| graph | sizes | port | C | exponent port / C |
| :-- | :-- | :-- | :-- | :-- |
| dot_dag | 1k / 5k / 20k | 0.23 s (18) / 4.34 s (71) / 150 s (238) | 0.32 s (11) / 9.06 s (42) / 241 s (159) | 2.16 / 2.21 |
| neato_sparse | 250 / 500 / 1k / 2k | 0.82 / 3.79 / 17.1 / 70.2 s (7–30) | 0.16 / 0.74 / 3.28 / 13.2 s (6–24) | 2.14 / 2.12 |
| fdp_sparse | 250 / 500 / 1k | 1.75 / 7.25 / 28.7 s (8–16) | 3.78 / 18.0 / 76.0 s (6–12) | 2.02 / 2.16 |
| sfdp_sparse | 1k / 5k / 20k | 0.81 s (14) / 5.51 s (45) / 28.5 s (168) | 0.29 s (8) / 2.04 s (19) / 11.4 s (56) | 1.19 / 1.22 |
| circo_sparse | 100 / 200 / 400 | 0.09 / 0.98 / 4.75 s (7–9) | 0.45 / 3.20 / 52.6 s (6–8) | 2.87 / 3.44 |
| twopi_tree | 1k / 5k / 20k | 25 ms (12) / 128 ms (39) / 646 ms (142) | 18 ms (7) / 100 ms (15) / 672 ms (45) | 1.08 / 1.20 |
| osage_clustered | 1k / 5k / 20k | 21 ms (10) / 96 ms (29) / 436 ms (99) | 15 ms (8) / 57 ms (20) / 223 ms (65) | 1.00 / 0.91 |
| patchwork_clustered | 1k / 5k / 20k | 23 ms (10) / 106 ms (30) / 464 ms (103) | 18 ms (9) / 71 ms (21) / 268 ms (68) | 1.01 / 0.91 |

Time grows at the same rate in the port as in C, so neither has a worse algorithm, apart from
`-Tplain`. Memory grows faster in the port: from 1k to 20k nodes the RSS gap to C grows from
1.6× to 3.1× on twopi and from 1.7× to 3.0× on sfdp. That points to per-node and per-element
overhead (hotspots 4, 13 and 15), not to an algorithm.

### Allocations

Counted with the scratch counting allocator (one run, `-Tsvg`; the counts are deterministic):

| input | allocs | reallocs | bytes allocated | peak live | allocs per node |
| :-- | --: | --: | --: | --: | --: |
| fixtures/unix.gv | 12,776 | 1,478 | 1.4 MB | 0.57 MB | ~310 |
| wild/b29 | 290,860 | 58,110 | 93 MB | 15.7 MB | |
| synth/dot_dag_5000 | 1,903,081 | 289,875 | 471 MB | 72.6 MB | 381 |
| neato/hw3_vpsc_random200 | 45,567 | 3,933 | 5.0 MB | 1.4 MB | 228 |
| synth/neato_sparse_500 | 90,203 | 7,984 | 12 MB | 3.6 MB | 180 |
| fdp/viewport | 453,928 | 53,363 | 24 MB | 2.3 MB | |
| sfdp/gen_rnd2000 | 4,032,213 | 21,946 | 215 MB | 13.3 MB | 2,016 |
| synth/sfdp_sparse_5000 | 10,359,807 | 49,649 | 588 MB | 31.5 MB | 2,072 |
| synth/circo_sparse_200 | 2,174,243 | 165,857 | 118 MB | 1.6 MB | 10,871 |
| twopi/ngk10_4 | 20,993 | 4,555 | 7.1 MB | 1.8 MB | |
| synth/twopi_tree_20000 | 5,319,456 | 319,938 | 332 MB | 117.9 MB | 266 |
| synth/osage_clustered_20000 | 3,162,760 | 440,405 | 234 MB | 87.4 MB | 158 |
| synth/patchwork_clustered_20000 | 3,372,336 | 149,462 | 248 MB | 89.9 MB | 169 |

The time spent in malloc and free is small (0.4% of dot_dag_5000, 2% of sfdp, 10–11% of
osage and patchwork at 20k nodes, where layout is cheap). The allocation counts matter mainly as
a stable signal for a ratchet and as a sign of per-element `Vec`s. Peak live heap per node is
4.4–5.9 KB at 20k nodes for twopi, osage and patchwork. Where the allocations come from (malloc
uprobe, inclusive): sfdp's are 85% `QuadTree::add_internal`; circo's 98% `count_all_crossings`;
twopi's 38% `set_n_steps_to_leaf`; 11% of dot's are `cnum::fmt_f` for SVG numbers; and in osage the
largest inclusive shares are the parser (32%), `node_geometry`/`make_text_label` (17%),
`LGraph::new` (15%) and render `Model::build` (14%).

### WASM

| artefact | size |
| :-- | --: |
| `graphviz_pure_wasm.wasm` (npm/web build: `opt-level=z`, fat LTO, before wasm-bindgen; no wasm-opt) | 3.81 MB (0.99 MB gzip -9) |
| `graphviz-pure.wasm` (wasm32-wasip1 CLI, release profile, includes the PNG rasterizer) | 11.2 MB |

wasm-bindgen-cli is not installed on this machine, so the first size is the module before
wasm-bindgen, which strips it and adds the JS glue. The size of the published `_bg.wasm` was not
measured. The WASI CLI under node 24's built-in `node:wasi` (median of 3, including about 90 ms
to start node and compile the module), with output identical to native:

| input | native | wasm (node) | ×native |
| :-- | --: | --: | --: |
| fixtures/unix.gv | 6.9 ms | 100 ms | (start-up) |
| wild/b29 | 0.95 s | 1.40 s | 1.5 |
| synth/dot_dag_5000 | 4.34 s | 4.87 s | 1.1 |
| neato/hw3_vpsc_random200 | 0.41 s | 0.48 s | 1.2 |
| fdp/viewport | 0.97 s | 4.73 s | 4.9 |
| sfdp/gen_rnd2000 | 1.87 s | 6.50 s | 3.5 |

wasm32 has no deterministic FMA instruction (relaxed-simd's `relaxed_madd` may or may not fuse),
so the WASM build keeps the software `fma` whatever happens on native. fdp and sfdp are a
further 3.5–5× slower than native there. A faster software `fma` (hotspot 1, option c) is
the only fix for WASM.

## Hotspots

Ranked by how much time they would save across the engines. Percentages are of the profile's
samples on the named input: *self* is the function's own time and *incl.* includes its callees.
"Float order" says whether the suggested fix could change any floating-point operation or its
order, and so the bytes: **none** means it is integer-only or computes the same IEEE operations
in the same order, **low** means only memory layout or control flow changes, and **high** means the
fix itself reorders arithmetic.

| # | where | cost | why it is hot | first guess at the fix | float order |
| --: | :-- | :-- | :-- | :-- | :-- |
| 1 | every `f32::mul_add`/`f64::mul_add` (≈30 files; hottest in `neatogen/matrix_ops.rs:39`, `fdpgen/xlayout.rs`, `geom/pathplan/visibility.rs`, `core/math/portable/pow`, `hypot`) | neato 48% self in `fmaf_with_fma` + 3% `fmaf`; fdp 19% in `fma_with_fma`/`fma`; twopi routing 14%; sfdp inside `pow`; dot 2% | The port uses `mul_add` wherever the macOS arm64 C build contracts `a*b+c` into an FMA, which is how it matches C byte for byte. The x86-64 baseline has no FMA instruction, so every call is an out-of-line libcall with a CPU-feature dispatch, and the loop around it cannot be vectorised. | (a) Build the x86-64 release binaries with `-C target-feature=+fma` (or `target-cpu=x86-64-v3`). Measured: neato 4.8–5.0×, sfdp 2.3×, fdp 2.0–2.1×, twopi routing 2.1×, circo with splines 2.8× faster, dot 1.0–1.1×, 2,603 corpus SVGs identical (all but the six `badvoro` copies). It drops CPUs without FMA (Intel before Haswell, 2013; AMD before Piledriver, 2012), so either ship a second baseline build or (b) choose at run time with `is_x86_feature_detected!("fma")` and `#[target_feature(enable = "fma")]` copies of the hot kernels. Calling those needs `unsafe`, which the workspace forbids, or a dependency such as `multiversion`. (c) For WASM, a faster software `fma` than compiler-builtins'. | **none**: FMA is correctly rounded, so hardware and software give the same bits |
| 2 | `render/src/emit/output.rs:277` `root_holds_plain` | 84% incl. of `-Tplain` on osage 20k (8.9 s against 0.44 s for SVG; C 0.2 s); O(n²) | It answers C's "is this string already interned in the root graph" (the refcount behind `agstrdup` in `canon`). It answers by scanning every node, edge and attribute map of the graph for each label written: one linear scan per label. | Build a `HashSet<&str>` of the graph's strings once per output, or keep the interned-string set the parser already has. | **none** (no floats) |
| 3 | `neatogen/matrix_ops.rs:39` `right_mult_with_vector_ff` (from `conjgrad::conjugate_gradient_mkernel` and `stress::stress_majorization_kd_mkernel`) | neato 42% self, 90% incl.; still 84% self with `+fma` | Packed upper-triangular `f32` matrix × vector, O(n²) per conjugate-gradient step, the same as C. Each `j` updates both `res` (a running sum) and `result[j]`. | First hotspot 1. Then: the `result[j] = fma(packed, v_i, result[j])` updates are independent across `j`, so they can be SIMD (by LLVM once `fma` is an instruction, or by hand). The `res` sum is sequential and must stay so. | **low** for the `result[j]` lanes; **high** if `res` is reassociated |
| 4 | `sparse/quad_tree.rs:148` `add_internal`, `:225` `get_supernodes_internal` | sfdp 13–15% self (28% with `+fma`); 85% of sfdp's 10 M allocations | Each `Cell` holds `center`, `average` and `force` as `Vec<f64>` of length `dim` (2), and each leaf a `coord: Vec<f64>`. The tree is rebuilt for every iteration of every level, as in C, so that is millions of 16-byte heap blocks. `get_supernodes` also pushes into growing `Vec`s. | `[f64; 2]` (or a const-generic `dim`) inline, and cells and leaves in flat arenas that are reused between iterations. | **none** |
| 5 | `sfdpgen/spring_electrical.rs:471, 513, 526` `powf(dist, 1 - p)` | sfdp 44% self in `math::portable::pow` (36% with `+fma`) | The repulsive force calls `pow` for each pair inside the per-dimension `k` loop, so twice per pair in 2-D, exactly as C's `spring_electrical.c` does. CORE-MATH `pow` is correctly rounded and comparatively slow. | Hoist `powf(dist, 1 - p)` out of the `k` loop (it is the same value) to halve the calls, and use hardware FMA inside `pow` (hotspot 1). Special-casing `p` is not possible: the default `p = -1.0001234` is not an integer. | **none** (same call, same arguments, fewer times) |
| 6 | `common/ns.rs:1351` `dfs_range_impl`, `:456` `enter_edge`, `:863` `rerank` (network simplex), from `dotgen/position.rs` `dot_position` | dot_dag_5000: 35% + 33% + 15% self (84% incl. under `dot_position`); b29: 27% + 16% + 11% | The x-coordinate network simplex on the auxiliary graph, O(V·E)-ish and the reason dot scales as n^2.2, the same as C. Every `low`/`lim`/`par`/`tree_out` access goes through the `NsGraph` trait into dot's `NodeInfo`, a 52-field struct with 12 `Vec`s. So each node's tree fields sit on their own cache lines, and each tree edge list is a separate heap `Vec`. | Struct-of-arrays scratch for the NS fields (`low`, `lim`, `par`, `cutvalue`) and tree edges in one CSR array, built when NS starts. Then a cheaper `enter_edge` search, as long as it picks the same edge C picks. | **none** (integers), as long as the edge visiting order is kept |
| 7 | `circogen/blockpath.rs:292` `count_all_crossings` (from `reduce`) and `circogen/graph.rs:96` `CGraph::edges` | circo 68–83% self; 98% of circo's allocations (10,900 per node on sparse_200) | `reduce` recounts every crossing after each tentative node move, O(n·E·open) per pass, as in C. Each count calls `subg.edges(n)` for every node, which collects a fresh `Vec` from the map-based `CGraph`, and `edgelist` add/remove are linear. The port is already 3–11× faster than C here. | Build the block's adjacency once (CSR) and reuse it for every count, use a reused buffer or an indexed set for `open`, and do an incremental crossing delta for a move. | **none** (integers) |
| 8 | `fdpgen/xlayout.rs:279` `fdp_x_layout` (`adjust`, `do_rep`) | fdp 74% self, 89% incl. (90% with `+fma`) | Overlap removal repels all pairs, O(n²) per iteration, with `math::hypot` (portable, built on `mul_add`) and `mul_add` in `add_disp`. C does the same (no grid in `xlayout.c`). | Hotspot 1 first (2.1×). Then make the inner loop branch-light, with positions in flat `[f64; 2]` arrays instead of per-node structs. A spatial grid for the repulsion would change which pairs are summed and in what order. | **low** for layout changes; **high** for a grid |
| 9 | `geom/pathplan/visibility.rs:133` `pt_vis`, `:92` `visibility`; `neatogen/routing.rs:161` `Router::new` (`splines=true` in neato, fdp, twopi, circo) | twopi ngk10_4: 45% + 25% self, 97% incl. under `spline_edges`; 14% in `fma` | The visibility graph of the obstacle polygons is O(V³) (every pair of vertices tested against every polygon side), as in C's `visibility.c`. `pt_vis` returns a fresh `Vec<f64>` per call. | Hotspot 1 (2.1×). Then reuse the `pt_vis` buffer, and keep obstacle vertices contiguous for the inner `in_cone`/`clear` tests. | **none** for buffers and layout |
| 10 | `dotgen/mincross.rs:471` `mincross`, `:453` `transpose`, `:371/387` `in_cross`/`out_cross`, `left2right` | b29: 23% incl. (mincross 7% self, in/out_cross 10%, left2right 4%) | Crossing counts that walk each node's edge `Vec`s through the fat `NodeInfo`, and `left2right` checks cluster membership per pair. | Struct-of-arrays positions and orders for the rank being swept, and a precomputed cluster id per node for `left2right`. | **none** (integers) |
| 11 | `twopigen/circle.rs:48` `set_n_steps_to_leaf` | twopi_tree_20000: 15% self, 28% incl.; 38% of its allocations | A recursive relaxation from every leaf, O(n·depth), as in C's `setNStepsToLeaf`. Each call collects `g.all_edges(sg, n)` into a new `Vec` before recursing. | Iterate with an explicit stack and a reused edge buffer (or borrow the adjacency without collecting). | **none** (integers) |
| 12 | `-Tpng`: `raster/src/lib.rs:150` `encode_png` (miniz_oxide `compress_inner`), resvg/tiny-skia `fill_path`/`stroke_path` | b29 PNG: rasterize 41% incl., deflate 25% self; RSS 193 MiB against 20 MiB for SVG | It rasterizes the whole page into one pixmap and then deflates it at the encoder's default level. | A faster deflate level, or encoding row by row to cut memory. Either changes the PNG bytes, which the cross-platform check compares (C has no comparable `-Tpng`), so it needs a refresh of the PNG references. | n/a for layout; **changes PNG bytes** |
| 13 | `core/src/cnum.rs:263` `fmt_f`, used by `render/src/svg_engine.rs:329` `SvgEngine::dbl` and by `diag::fmt_f` | 11% of dot_dag_5000's allocations; Dragon4 fallback (`format_exact`) 3.6% of osage 20k | A `format!("{v:.prec$}")` per number, which allocates a `String` that is then appended. Exact formatting falls back to Dragon4 for many values. | Write into the output buffer with `write!` (same formatting, no allocation). A specialised fixed-precision formatter would have to be checked digit for digit. | **none** for `write!`; **low** for a new formatter (bytes, not floats) |
| 14 | `layout/src/osage/mod.rs:254` `trace_node` (and `trace_bb`, `reposition`'s `trace`) | 5.8% incl. of osage 20k | The verbose trace message, including two float formats, is built with `format!` for every node even when `-v` is off. C only formats under `if (Verbose)`. | Check the verbosity before formatting. | **none** |
| 15 | Per-node memory: parser (`Parser::body`/`attr_brackets`), `LGraph::new`/`new_unported`, `geom::node_size::node_geometry` + `font::make_text_label`, render `emit::model::Model::build` | at 20k nodes: peak live heap 87–118 MB (4.4–5.9 KB per node), RSS 1.5–3.1× C's; in osage 20k they hold the largest inclusive shares of allocations (parser 32%, `node_geometry` 17%, `LGraph::new` 15%, `Model::build` 14%) | Each stage builds its own copy of the graph: the parsed graph with string-keyed attribute maps, the layout graph, the measured labels, and the render model. Each copy has per-node `String`s and `Vec`s. C keeps one `Agraph_t` with records attached to it. | Intern attribute names and values once (symbol ids), share the parsed graph instead of copying it into `LGraph` and `Model`, and free stages as soon as the next one is built. | **none** |
| 16 | Start-up floor | 6.6 MB RSS against C's 4.0 MB; 3.7 ms both | Static tables (font metrics, colour names) and the 13.4 MB binary. | Strip the release binaries; build the large tables lazily. | **none** |

Smaller items found on the way: `math::cbrt` is 2.8% of b29 (in `solve3`, the cubic solver of
spline fitting), `pathplan::triang::isdiagonal` 3.7% of b29, and `routespl::limit_boxes` 2%.

## Noise

The quick set (`sets/quick.tsv`), run three times in a row, 5 runs each, load average 2.7:

| input | median wall ms, 3 sessions | spread between sessions | max spread within a session |
| :-- | :-- | --: | --: |
| fixtures/unix.gv svg | 5.4 / 6.0 / 6.6 | 20.7% | 61% |
| fixtures/unix.gv png | 26.6 / 25.9 / 27.3 | 5.3% | 16% |
| wild/b143 | 221.4 / 218.4 / 219.8 | 1.4% | 7% |
| synth/dot_dag_1000 | 215.5 / 220.8 / 218.2 | 2.4% | 6% |
| neato/hw2_random_120 | 224.9 / 227.7 / 224.0 | 1.6% | 8% |
| fdp/b69 | 129.1 / 128.0 / 124.0 | 4.1% | 15% |
| sfdp/gen_grid20x20 | 209.7 / 211.1 / 209.0 | 1.0% | 4% |
| circo/sdh | 129.0 / 129.6 / 126.8 | 2.2% | 16% |
| twopi/fx_arrows | 87.8 / 87.5 / 86.5 | 1.6% | 5% |
| synth/twopi_tree_5000 | 117.8 / 116.8 / 117.7 | 0.8% | 20% |
| osage/b51 | 8.3 / 9.8 / 8.4 | 17.3% | 15% |
| patchwork/b51 | 7.3 / 7.9 / 7.3 | 9.2% | 40% |

Medians over 100 ms agree within 1–4% between sessions on this shared machine. Under 10 ms
the process start-up and scheduling dominate and the spread is 10–20%, so short inputs measure
start-up, not layout. Peak RSS varies by about 1% (allocator and ASLR). Allocation counts do not
vary at all. In the full run, a single slow outlier of up to 1.8× (dot_dag_5000: 4.08–7.88 s)
appears in a few cases while other agents were busy, which is why the tables use medians.

## Reproducing

```sh
CARGO_BUILD_JOBS=6 cargo build --release -p graphviz-pure-cli
python3 -I scripts/bench/synth.py --out target/bench/synth
python3 -I scripts/bench/bench.py run --set scripts/bench/sets/full.tsv \
    --c-dot /opt/graphviz-16.1.0/bin/dot --c-gvbindir /opt/graphviz-16.1.0/lutplugins \
    --out target/bench/full.json            # about 70 minutes with C, mostly the 1-run cases
python3 -I scripts/bench/bench.py survey --c-dot /opt/graphviz-16.1.0/bin/dot \
    --c-gvbindir /opt/graphviz-16.1.0/lutplugins --out target/bench/survey.tsv
python3 -I scripts/bench/bench.py compare docs/perf/data/full-0.2.2.json target/bench/full.json
```

`run` writes the JSON (with the machine, version, date and load), a TSV next to it, and prints
the Markdown table. `--only STRING` runs a subset; `--runs N` caps the runs. The harness needs
Python 3 and `/usr/bin/time` (GNU time, for peak RSS; without it RSS falls back to `wait4`,
which is wrong below Python's own size, see [Method](#method)).

### Profiles

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only RUSTFLAGS="-C force-frame-pointers=yes" \
  CARGO_TARGET_DIR=target/prof cargo build --release -p graphviz-pure-cli
perf record -F 1999 --call-graph fp -o out.data -- target/prof/release/graphviz-pure -Tsvg IN.gv >/dev/null
perf report -i out.data --no-children --sort symbol      # self time
perf report -i out.data --children --sort symbol -g none # inclusive, for the phase split
```

`perf` 6.x does not demangle Rust's v0 symbols; `rustfilt` does, or read the identifiers out of
`_RNv...` by eye. For the `+fma` comparison, the same build with
`RUSTFLAGS="-C target-feature=+fma"`.

### Allocation counts recipe

A crate outside the repository (it needs `unsafe`, which the workspace forbids). It reuses the
CLI's `main.rs` and wraps the system allocator:

```sh
R=$PWD; D=/tmp/gvp-alloc-count; mkdir -p $D/src
sed 's/^fn main() -> ExitCode {/fn cli_main() -> ExitCode {/; s/\benv!(/::core::env!(/g' \
  crates/graphviz-pure-cli/src/main.rs > $D/src/cli_main.rs
cp Cargo.lock $D/
```

`$D/Cargo.toml`: a `[package]`, an empty `[workspace]`, path dependencies on `$R/crates/graphviz-pure`,
`-core`, `-parser` and `-raster`, the root `Cargo.toml`'s `[patch.crates-io]` with paths under
`$R/vendor/`, and the `.cargo/config.toml` `[profile.release]`. `$D/src/main.rs`:

```rust
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size() as u64, Relaxed);
        let live = LIVE.fetch_add(l.size() as u64, Relaxed) + l.size() as u64;
        PEAK.fetch_max(live, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as u64, Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    // realloc: count it, and adjust BYTES/LIVE/PEAK by the size difference.
}
#[global_allocator]
static A: Counting = Counting;
include!("cli_main.rs");
fn main() -> std::process::ExitCode {
    let code = cli_main();
    eprintln!("allocs={} bytes={} peak_live={}", ALLOCS.load(Relaxed), BYTES.load(Relaxed), PEAK.load(Relaxed));
    code
}
```

## A CI perf ratchet (proposal)

Since added as [ratchet.md](ratchet.md) (#530): options 1 and 2 below, with Callgrind for the
instruction counts and dhat for the allocations. The proposal as written, with options from most
to least robust on shared GitHub runners:

1. **Allocation counts** for `sets/quick.tsv`. They are deterministic, so the ratchet can be
   exact: fail when any count goes up, and refresh the committed counts when they go down.
   This needs the counting allocator in a crate the workspace builds, which needs an `unsafe`
   exemption like `graphviz-pure-portable-libm`'s, or a dev-only dependency such as `dhat`, whose testing mode
   reports heap statistics a test can assert on. Either is a decision for the
   maintainer. It catches the regressions behind hotspots 2, 4, 7, 11, 13 and 15.
2. **Instruction counts** (`perf stat -e instructions:u`) for the same set. They vary far less
   than time between runs (not measured here). GitHub's hosted runners usually do not allow `perf_event_open`, so this would need a
   self-hosted runner, or `valgrind --tool=cachegrind` (slow, but exact and allowed).
3. **CPU time with a wide tolerance**:
   `bench.py run --set scripts/bench/sets/quick.tsv --runs 5`, then `bench.py compare BASE.json
   NEW.json --tolerance 1.25 --fail` against a baseline measured on the same runner in the same
   job (build `master` and the PR, run both, interleaved). Comparing with a number committed
   from another machine does not work. The quick set takes about 10 s per build. A 25%
   tolerance only catches large regressions, such as the 5× and 20× effects above.

Whichever is chosen, the quick set should keep one input per engine in the 50–500 ms range, where
the spread is 1–4%, plus `-Tplain` and `-Tpng` on one graph each.
