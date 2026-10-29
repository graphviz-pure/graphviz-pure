# Profile-guided release binaries (T14, #608)

PGO reduces the geometric mean of executed instructions by **11.03%** on an
engine-balanced set of **eight held-out source inputs**, with no training overlap.
The initial 15 musl x86-64-v3 ratchet cases improved **8.80%**, but **13 of those
15 source inputs appear in training**; that figure describes a mostly in-sample
set, not an estimate for arbitrary unseen inputs. The two sets have different
sizes and engine weights. Every case in both sets improves instructions, with
identical allocation counts, allocated bytes and peak heap. The held-out result
also exceeds the 5% deterministic adoption threshold. All six default native
release binaries use PGO; v1 compat, launcher, WASM and `cargo install` stay plain.

This is build configuration only: no source floating-point operation, operand,
rounding, tie-break or iteration order changes, and no fast-math flag is enabled.
Fidelity class: **none**. No product dependency is added; rustup's
`llvm-tools` component supplies `llvm-profdata`.

## Measurement method

Initial measurements on 2026-10-29 (UTC), at base commit `a693a9589474ddd9876dd14c93ac693c90f47dc1`, stacked
on #605, with Rust 1.99.1 / LLVM 23.1.1, release `opt-level=3`, thin LTO,
one codegen unit and the static `x86_64-unknown-linux-musl` target. Both builds
use `-C target-cpu=x86-64-v3`; PGO adds only `-Cprofile-generate` for training
and `-Cprofile-use` for the final build. The initial diagnostic build also uses
`-Cllvm-args=-pgo-warn-missing-function`. The base executable is the exact #605
CLI; the 1.0 reference is the v1 musl build of `f89e21d9`.

The fixed [training list](../../scripts/bench/sets/pgo.tsv) has **276 commands**:
47 input/format pairs from `full.tsv` below roughly one second in the preceding
v3 probe, all 20 public format names on one input from each of the eight engines,
and 80 sampled corpus SVGs (duplicates removed). The sample uses SHA-256 of the
relative input path modulo 32 equal to zero, input size below 12 KB, and a probe
that succeeds in under one second. Selection happened once; CI reads the frozen
list and applies no time-based selection. Training runs serially in list order,
with fixed synthetic seeds and a separate numbered raw profile per command.
The first training run took 47.0 seconds at load about 133.

Instructions use Valgrind 3.24.0 Callgrind on the 15 cases in `quick.tsv` and
`ratchet-extra.tsv`, with output discarded. The laid b29 inputs are generated
once by plain v3 and shared by every build. Heap statistics use the matching
musl `heap_counts` example and identical argument paths. C is stock Graphviz
16.1.0 with the pango-free plugin directory; it uses glibc and has no PNG plugin.
C is a performance reference, while committed goldens define fidelity.

## Initial ratchet: mostly training inputs

13/15 source inputs in this table occur in the training list (the two laid b29
inputs do not). The geomean of PGO/plain-v3 instructions is **0.9120**
(8.80% fewer instructions on this mostly in-sample set).
Counts include process start-up,
parsing, layout, rendering and musl allocation work. They are not wall-time ratios.
The [raw table](data/pgo-instructions.tsv) also retains total allocated bytes.

| case | 1.0 v1 instructions | plain v3 | PGO v3 | Δ vs v3 | C | allocations (both) | peak heap B (both) |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| dot unix SVG | 27,618,083 | 16,668,304 | 15,501,860 | -7.00% | 16,807,402 | 14,253 | 569,042 |
| dot unix PNG | 147,040,680 | 130,682,695 | 123,867,177 | -5.22% | — | 26,276 | 7,460,194 |
| dot b143 SVG | 2,015,790,098 | 1,214,171,835 | 1,031,425,241 | -15.05% | 1,128,444,020 | 250,586 | 8,293,645 |
| dot DAG 1000 | 1,575,110,574 | 1,007,112,569 | 917,682,594 | -8.88% | 1,144,115,638 | 418,625 | 13,947,938 |
| neato hw2 random 120 | 2,532,966,197 | 552,194,134 | 454,236,147 | -17.74% | 413,032,690 | 26,433 | 798,550 |
| fdp b69 | 5,226,771,525 | 718,489,269 | 609,637,304 | -15.15% | 2,515,083,659 | 321,513 | 1,407,906 |
| sfdp grid20x20 | 976,535,256 | 372,945,146 | 310,177,668 | -16.83% | 684,377,881 | 97,178 | 3,066,435 |
| circo sdh | 1,540,660,537 | 1,533,047,740 | 1,527,163,846 | -0.38% | 6,161,964,685 | 678,425 | 991,811 |
| twopi arrows | 4,303,944,457 | 349,157,985 | 324,699,070 | -7.01% | 537,591,042 | 22,546 | 909,562 |
| twopi tree5000 | 850,846,601 | 733,768,047 | 691,826,626 | -5.72% | 646,578,886 | 1,024,852 | 29,377,259 |
| osage b51 | 41,868,974 | 36,697,718 | 35,531,939 | -3.18% | 35,928,942 | 42,661 | 983,810 |
| patchwork b51 | 31,219,468 | 30,928,054 | 30,058,702 | -2.81% | 27,572,230 | 36,848 | 908,191 |
| dot b143 plain | 2,033,079,756 | 1,231,537,915 | 1,047,623,071 | -14.93% | 1,116,561,421 | 245,032 | 8,293,649 |
| parse b29 canon (-Knop2) | 158,763,475 | 157,141,860 | 149,616,944 | -4.79% | 86,037,675 | 227,582 | 5,232,418 |
| render b29 SVG (-Knop2) | 162,396,273 | 160,480,839 | 152,836,314 | -4.76% | 136,321,861 | 210,281 | 5,232,414 |

The ordinary perf workflow continues to measure the plain v1 build, so its global
baseline is unchanged. The pre-existing fdp b69 allocation baseline discrepancy
([#600](https://github.com/graphviz-pure/graphviz-pure/issues/600)) appears equally
before and after PGO: 321,513 allocations in both fresh runs.

## Held-out measurement

This follow-up fixes one SVG case per engine **before** measuring. The first six
are `full.tsv` source inputs absent from `pgo.tsv` in every format. Osage and
patchwork use the largest remaining untrained corpus source in that engine.
[The fixed list](../../scripts/bench/sets/pgo-held-out.tsv) has **zero source-path
overlap** with training, and CI checks that separation. Training was not changed.
The product Rust source at `04792fc3` is unchanged from the initial measurement.
The held-out build/profile hashes are in [pgo-held-out-build.json](data/pgo-held-out-build.json).

The instruction geomean is **0.889672** (**11.03% fewer instructions**). All eight
cases improve; all eight have identical allocations, total bytes and peak heap.
This sample is larger and engine-balanced; it is not directly interchangeable
with the small, dot-heavy initial ratchet. Both sets pass the 5% primary-metric gate.

| engine / held-out input | plain v3 instructions | PGO | Δ | allocations (both) | peak heap B (both) |
| :-- | --: | --: | --: | --: | --: |
| dot: `wild/gv16_graphs_b29_d1e4db.gv` | 4,124,150,754 | 3,612,232,097 | -12.41% | 349,187 | 15,660,818 |
| neato: `synth/neato_sparse_500.gv` | 8,871,374,292 | 7,228,802,520 | -18.52% | 98,186 | 3,583,729 |
| fdp: `synth/fdp_sparse_250.gv` | 9,086,780,278 | 7,416,596,625 | -18.38% | 755,087 | 2,248,953 |
| sfdp: `synth/sfdp_sparse_5000.gv` | 8,177,093,400 | 6,688,522,597 | -18.20% | 907,512 | 29,504,022 |
| circo: `synth/circo_sparse_200.gv` | 9,410,304,320 | 9,205,172,267 | -2.18% | 2,340,099 | 1,572,219 |
| twopi: `synth/twopi_tree_20000.gv` | 3,980,676,854 | 3,728,207,455 | -6.34% | 5,639,393 | 117,871,269 |
| osage: `osage/wild_gv16_infosrc_plugins_1a797f.gv` | 34,198,153 | 32,422,924 | -5.19% | 44,190 | 1,089,252 |
| patchwork: `patchwork/wild_gv16_graphs_b15_8c212e.gv` | 12,121,727 | 11,495,837 | -5.16% | 18,543 | 646,407 |

Wall times are medians of five after a warm-up, with the four builds interleaved
in rotating order. The held-out PGO/plain-v3 wall geomean is **1.03474**
(**3.47% more wall time**) under load **29.03–38.38**. The tiny patchwork case
is 10.45 ms versus 19.72 ms and dominates that noisy ratio. We report it unchanged;
there is no held-out wall-time speed claim. The deterministic instruction result
remains the adoption metric. Every sample, case uptime and the full source paths
are in [pgo-held-out.tsv](data/pgo-held-out.tsv).

| engine | plain v3 ms | PGO ms | Δ | 1.0 v1 ms | ×1.0 | C ms | ×C |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| dot | 1977.87 | 1875.45 | -5.18% | 2682.28 | 1.43 | 3636.28 | 1.94 |
| neato | 1322.11 | 1272.48 | -3.75% | 6202.15 | 4.87 | 1170.46 | 0.92 |
| fdp | 1573.54 | 1477.15 | -6.13% | 14329.71 | 9.70 | 6077.82 | 4.11 |
| sfdp | 1650.23 | 1668.57 | +1.11% | 4136.34 | 2.48 | 3673.04 | 2.20 |
| circo | 2674.94 | 2370.85 | -11.37% | 2224.37 | 0.94 | 4745.78 | 2.00 |
| twopi | 1673.71 | 1479.71 | -11.59% | 1820.63 | 1.23 | 1627.66 | 1.10 |
| osage | 32.83 | 33.68 | +2.59% | 32.36 | 0.96 | 31.96 | 0.95 |
| patchwork | 10.45 | 19.72 | +88.72% | 32.07 | 1.63 | 15.89 | 0.81 |

```text
18:34:36 up 4 days, 15:16,  2 users,  load average: 30.27, 30.97, 37.44
18:41:46 up 4 days, 15:23,  2 users,  load average: 37.73, 34.63, 36.51
```

## Initial wall time (informational)

The 60 `full.tsv` input/format pairs below roughly five seconds in the preceding
plain-v3 probe were timed afresh. Every selected case has **five measured runs**
of each build, after one warm-up. The order of the four builds rotates each round;
PNG has three builds because the C installation has no PNG plugin. This overrides
`full.tsv`'s reduced run counts. Every selected case is reported, including regressions.

The geomean PGO/plain-v3 wall ratio is **0.9601** (4.0% less wall time). Individual
cases move in both directions under heavy scheduling noise. Adoption uses the
**primary deterministic metric**, instruction counts (8.8% here and 11.03% on the held-out set), exceeding
the 5% gate. These loaded wall times are not release speed promises.
The one-minute load ranged from **29.26 to 123.12** while timing:

```text
17:52:35 up 4 days, 14:34,  2 users,  load average: 44.82, 75.20, 86.68
18:15:13 up 4 days, 14:57,  2 users,  load average: 41.59, 42.81, 57.24
```

Per-engine ratios are geometric means of the selected cases; C excludes PNG.
PGO/plain-v3 below 1 means PGO took less time; reference/PGO above 1 means PGO
was faster than that reference in this loaded run.

| engine | cases | PGO/plain-v3 wall | 1.0 v1/PGO | C/PGO (non-PNG) |
| :-- | --: | --: | --: | --: |
| dot | 14 | 1.015 | 1.21× | 1.13× |
| neato | 8 | 1.005 | 4.28× | 0.98× |
| fdp | 5 | 0.900 | 8.04× | 2.86× |
| sfdp | 9 | 0.882 | 3.20× | 2.03× |
| circo | 5 | 1.028 | 1.90× | 1.90× |
| twopi | 7 | 0.971 | 2.58× | 0.97× |
| osage | 6 | 0.959 | 1.05× | 0.53× |
| patchwork | 6 | 0.889 | 1.19× | 0.62× |

The [raw wall data](data/pgo-wall.tsv) contains all five samples for every build,
per-case median ratios and start/end `uptime`. Times below are medians in ms;
×1.0 is v1/PGO and ×C is C/PGO.

| input | -T | plain v3 ms | PGO ms | 1.0 v1 ms | ×1.0 | C ms | ×C |
| :-- | :-- | --: | --: | --: | --: | --: | --: |
| `fixtures/unix.gv` | svg | 32.1 | 32.1 | 32.3 | 1.00 | 15.9 | 0.49 |
| `fixtures/unix.gv` | plain | 31.9 | 32.0 | 33.6 | 1.05 | 19.2 | 0.60 |
| `fixtures/unix.gv` | png | 119.7 | 114.1 | 116.7 | 1.02 | — | — |
| `wild/gv16_graphs_b29_d1e4db.gv` | svg | 1579.6 | 2071.8 | 2551.9 | 1.23 | 3613.3 | 1.74 |
| `wild/gv16_graphs_b29_d1e4db.gv` | plain | 2225.6 | 2030.2 | 2650.6 | 1.31 | 4051.3 | 2.00 |
| `wild/gv16_graphs_b29_d1e4db.gv` | png | 3545.0 | 3307.0 | 4166.0 | 1.26 | — | — |
| `wild/gv16_graphs_b143_5416de.gv` | svg | 418.2 | 369.3 | 469.8 | 1.27 | 467.6 | 1.27 |
| `wild/gv16_graphs_b143_5416de.gv` | plain | 417.1 | 365.2 | 517.8 | 1.42 | 421.8 | 1.15 |
| `wild/gv16_graphs_b143_5416de.gv` | png | 1849.8 | 1625.2 | 2027.2 | 1.25 | — | — |
| `wild/gv16_graphs_b106_471493.gv` | svg | 326.3 | 318.9 | 416.9 | 1.31 | 374.9 | 1.18 |
| `wild/gv16_graphs_badvoro_2e8cb8.gv` | svg | 929.7 | 850.1 | 1223.2 | 1.44 | 1221.0 | 1.44 |
| `wild/gv16_graphs_b124_4440e1.gv` | svg | 115.4 | 221.5 | 184.0 | 0.83 | 170.3 | 0.77 |
| `synth/dot_dag_1000.gv` | svg | 379.7 | 367.8 | 482.2 | 1.31 | 489.7 | 1.33 |
| `synth/dot_dag_1000.gv` | plain | 372.9 | 378.4 | 565.1 | 1.49 | 565.8 | 1.50 |
| `neato/hw3_vpsc_random200.gv` | svg | 166.2 | 168.9 | 767.6 | 4.54 | 172.9 | 1.02 |
| `neato/hw3_vpsc_random200.gv` | plain | 199.5 | 215.0 | 815.9 | 3.80 | 166.7 | 0.78 |
| `neato/hw3_vpsc_random200.gv` | png | 314.6 | 318.0 | 1266.6 | 3.98 | — | — |
| `neato/hw2_random_120.gv` | svg | 118.8 | 117.3 | 464.8 | 3.96 | 117.3 | 1.00 |
| `neato/fx_NaN.gv` | svg | 67.8 | 66.2 | 218.4 | 3.30 | 68.3 | 1.03 |
| `neato/wild_gv16_graphs_b69_4b5b48.gv` | svg | 36.5 | 36.0 | 164.3 | 4.56 | 34.9 | 0.97 |
| `synth/neato_sparse_250.gv` | svg | 267.5 | 265.3 | 1317.2 | 4.96 | 238.4 | 0.90 |
| `synth/neato_sparse_500.gv` | svg | 1070.5 | 1066.1 | 5926.8 | 5.56 | 1266.3 | 1.19 |
| `fdp/wild_gv16_graphs_viewport_651d5b.gv` | svg | 917.0 | 666.3 | 6684.2 | 10.03 | 2721.1 | 4.08 |
| `fdp/wild_gv16_graphs_viewport_651d5b.gv` | plain | 817.5 | 773.5 | 6695.0 | 8.66 | 2777.4 | 3.59 |
| `fdp/wild_gv16_undirected_ngk10_4_793340.gv` | svg | 218.3 | 217.7 | 1642.9 | 7.55 | 218.2 | 1.00 |
| `fdp/wild_gv16_graphs_b69_4b5b48.gv` | svg | 166.8 | 169.5 | 866.9 | 5.12 | 521.1 | 3.08 |
| `synth/fdp_sparse_250.gv` | svg | 1624.0 | 1373.1 | 13801.1 | 10.05 | 5842.8 | 4.26 |
| `sfdp/gen_rnd2000.gv` | svg | 624.6 | 565.8 | 1577.5 | 2.79 | 1285.3 | 2.27 |
| `sfdp/gen_rnd2000.gv` | plain | 888.1 | 668.7 | 1771.0 | 2.65 | 1428.6 | 2.14 |
| `sfdp/gen_rnd2000.gv` | png | 917.8 | 871.5 | 1721.7 | 1.98 | — | — |
| `sfdp/gen_tree9.gv` | svg | 418.9 | 321.0 | 2332.3 | 7.26 | 465.3 | 1.45 |
| `sfdp/gen_powerlaw1000.gv` | svg | 340.7 | 270.3 | 2222.4 | 8.22 | 421.2 | 1.56 |
| `sfdp/attr_quadtree_none.gv` | svg | 32.4 | 33.0 | 114.6 | 3.48 | 114.7 | 3.48 |
| `sfdp/gen_grid20x20.gv` | svg | 114.8 | 114.6 | 214.9 | 1.87 | 164.7 | 1.44 |
| `synth/sfdp_sparse_1000.gv` | svg | 267.5 | 269.2 | 620.9 | 2.31 | 566.6 | 2.10 |
| `synth/sfdp_sparse_5000.gv` | svg | 2331.7 | 1874.6 | 5096.4 | 2.72 | 4734.6 | 2.53 |
| `circo/wild_gv16_linux_x86_dotsplines_dot1_3910e3.gv` | svg | 222.9 | 266.9 | 2228.0 | 8.35 | 276.2 | 1.04 |
| `circo/wild_gv16_linux_x86_neatosplines_neato_c2c6f6.gv` | svg | 367.4 | 424.8 | 1588.2 | 3.74 | 520.7 | 1.23 |
| `circo/wild_gv16_directed_sdh_29b016.gv` | svg | 416.8 | 365.7 | 373.8 | 1.02 | 1224.1 | 3.35 |
| `synth/circo_sparse_100.gv` | svg | 316.3 | 315.3 | 271.5 | 0.86 | 832.2 | 2.64 |
| `synth/circo_sparse_200.gv` | svg | 2739.2 | 2600.2 | 2329.0 | 0.90 | 5808.8 | 2.23 |
| `twopi/wild_gv16_undirected_ngk10_4_793340.gv` | svg | 266.7 | 221.0 | 1576.7 | 7.13 | 269.6 | 1.22 |
| `twopi/fx_arrows.gv` | svg | 117.0 | 118.1 | 719.4 | 6.09 | 117.9 | 1.00 |
| `twopi/wild_gv16_linux_x86_arrows_dot_1ce23f.gv` | svg | 123.1 | 164.5 | 916.7 | 5.57 | 121.1 | 0.74 |
| `synth/twopi_tree_1000.gv` | svg | 65.1 | 72.8 | 117.8 | 1.62 | 64.7 | 0.89 |
| `synth/twopi_tree_5000.gv` | svg | 433.8 | 315.1 | 473.4 | 1.50 | 276.2 | 0.88 |
| `synth/twopi_tree_20000.gv` | svg | 2188.4 | 2189.0 | 2350.5 | 1.07 | 2179.1 | 1.00 |
| `synth/twopi_tree_20000.gv` | plain | 2495.4 | 2226.2 | 2677.5 | 1.20 | 2677.0 | 1.20 |
| `osage/wild_gv16_graphs_b51_33bcb0.gv` | svg | 32.9 | 32.9 | 34.4 | 1.04 | 32.8 | 1.00 |
| `osage/wild_gv16_graphs_b15_8c212e.gv` | svg | 66.8 | 65.0 | 66.6 | 1.02 | 33.9 | 0.52 |
| `synth/osage_clustered_1000.gv` | svg | 117.0 | 114.5 | 117.1 | 1.02 | 66.2 | 0.58 |
| `synth/osage_clustered_5000.gv` | svg | 316.5 | 321.9 | 365.5 | 1.14 | 168.2 | 0.52 |
| `synth/osage_clustered_20000.gv` | svg | 1501.6 | 1194.3 | 1284.6 | 1.08 | 515.9 | 0.43 |
| `synth/osage_clustered_20000.gv` | plain | 1290.6 | 1304.0 | 1321.7 | 1.01 | 422.4 | 0.32 |
| `patchwork/wild_gv16_graphs_b51_33bcb0.gv` | svg | 33.5 | 41.0 | 32.1 | 0.78 | 22.3 | 0.54 |
| `patchwork/wild_gv16_infosrc_plugins_1a797f.gv` | svg | 24.7 | 16.9 | 32.9 | 1.95 | 20.3 | 1.20 |
| `synth/patchwork_clustered_1000.gv` | svg | 123.7 | 77.5 | 122.7 | 1.58 | 47.8 | 0.62 |
| `synth/patchwork_clustered_5000.gv` | svg | 316.7 | 315.7 | 315.4 | 1.00 | 167.5 | 0.53 |
| `synth/patchwork_clustered_20000.gv` | svg | 1171.2 | 1088.0 | 1225.7 | 1.13 | 629.7 | 0.58 |
| `synth/patchwork_clustered_20000.gv` | plain | 1137.8 | 1157.5 | 1186.8 | 1.03 | 521.8 | 0.45 |

## Fidelity

- PGO release workspace tests: **656 passed, 0 failed, 1 ignored**, across 34 suites.
- Matched-libm e2e goldens: **2,595/2,595**, with the existing 14 unstable C cases skipped.
- Render ratchets: dot/json0/svg/svg_inline/xdot **587/587** each;
  plain/plain-ext **577/577** each; json **551/551**. No baseline changes.
- Final static PGO CLI versus plain v3: **2,667/2,667** corpus SVGs, including
  diagnostics and exit status. Invalid inputs must fail identically too.
- Release gallery smoke test: **30/30** byte-identical samples.

CI trains the musl PGO binary, runs the workspace fidelity ratchets with
`-Cprofile-use`, and compares the final musl CLI with plain v1 over every corpus
SVG. Every newly trained native release binary is also compared against its plain
reference over the whole SVG corpus before packaging. x86-64 reuses compat;
ARM64 builds a plain reference. The same gate runs in CI on Linux x86-64/ARM64,
macOS ARM64/Intel and Windows x86-64/ARM64. Gallery/wheel/container checks remain.

On x86-64 this release gate compares **PGO + v3** with **compat (v1, no PGO)**.
A mismatch therefore flags either PGO or CPU-level differences. Plain v3 versus
compat was already checked separately for #605; the release gate additionally
checks the actual newly trained artifact.

## Release builds and reproducibility

The helper trains and rebuilds each of the six default native targets on its own
runner, keeping the original CPU/runtime flags in both phases. It uses a new
profile directory, a numbered file per training command and an explicitly ordered
merge, and fails if any command or raw profile is missing. Compat and the launcher
remain plain builds to avoid another instrumented build/training pass per x86-64
job. No archive names, CPU requirements or install layouts change. Missing-function
profile warnings are enabled. Windows ARM64 uses bundled `rust-lld` for both
PGO phases to avoid MSVC link.exe padding LLVM profile section markers; the
first CI run detected the malformed profiles rather than shipping a fallback.

Before profile use, the build rejects a merged profile below 64 KiB, fewer than
1,000 recorded functions, a maximum function count below 10,000, or fewer than
one million total block executions. Normal training produces several MiB and
billions of block executions. These conservative floors catch empty or trivial
training even if a profile contains zero-count records for every function.

An independent run through the release helper reproduced the **8.80%** instruction
reduction; counts differ from the manual build by at most **0.0062%** per case.
The raw instruction table includes this replication column. The first helper run
took **252.3 seconds**, including **48.6 seconds** of training, under shared load.

**The same commit alone does not produce a guaranteed identical binary.** Two
fresh training runs, with the same source, toolchain, paths and fixed input list,
produced different profiles and binary hashes. The profile hashes started
`66c44396` and `1e214852`; the resulting binary hashes started `d2080751` and
`b888fceb`. Profiles can vary with randomized hash-table probing and platform
libraries even when rendered bytes are identical. Sorting the merge inputs
produced exactly the same merged bytes as each original merge, so input-file
ordering did not account for this difference. Full hashes and toolchain details
are in [pgo-builds.json](data/pgo-builds.json).

Release jobs retain the exact `merged.profdata` and `build-info.json` in an Actions
artifact named `pgo-TARGET`, for 90 days. The record includes source commit,
toolchain, base flags and training/profile/binary hashes. Save these artifacts for
longer-term reproduction; they are not permanent release assets. Reusing the exact
profile removes training variability, but identical compiler, linker, SDK, sources,
flags and build paths are still needed for a binary reproduction attempt.
Deterministic archive packaging continues to produce identical archives from the
same binary. Neither PGO nor its runtime instrumentation is part of `cargo install`.

## Release validation cost

Every target runs `scripts/check-release-pgo.py` on its actual fresh PGO binary.
It compares stdout, stderr and exit status for every corpus SVG, and rejects
crashes and an empty corpus. On x86-64, compat already exists, so the added
release time is the complete comparison. ARM64 adds a cold plain build as well.
The checker prints both components and records `additional_release_seconds` in
`pgo-fidelity.json`; release artifacts retain it beside the exact PGO profile.
Native CI uses the same helper, so per-target timings can be measured before merge.
[PR #618](https://github.com/graphviz-pure/graphviz-pure/pull/618) records the
initial runner measurements, including Windows/ARM. The local musl x86-64 gate
passed **2,667/2,667** inputs in **677.2 s** (reference 555.3 s, PGO 121.8 s),
adding 11.3 minutes while reusing compat. This is shared-machine wall time, not
a runner forecast. Future releases report their own actual timings; costs vary
with runner speed and corpus growth.

## Reproducing

Follow [rustc's PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html).
From this source checkout, using a new directory for each profile collection:

```sh
rustup component add llvm-tools
PGO_WORK=/jobs/work/gvp-pgo
mkdir -p "$PGO_WORK/tmp"
TMPDIR="$PGO_WORK/tmp" RUSTFLAGS="-C target-cpu=x86-64-v3" \
  python3 scripts/build-pgo.py --target x86_64-unknown-linux-musl \
    --target-dir "$PGO_WORK/target" --profiles "$PGO_WORK/profiles-run1"
python3 scripts/check-cpu-builds.py /path/to/plain-v3/graphviz-pure \
  "$PGO_WORK/target/x86_64-unknown-linux-musl/release/graphviz-pure"
RUSTFLAGS="-C target-cpu=x86-64-v3 -Cprofile-use=$PGO_WORK/profiles-run1/merged.profdata" \
  CARGO_TARGET_DIR="$PGO_WORK/target" TMPDIR="$PGO_WORK/tmp" \
  cargo test --workspace --release --locked
```

Use `--cargo-wrapper` when the development machine requires a build-slot wrapper.
For each instruction-table row, run the corresponding binary with `-TFORMAT`, its
listed flags and input under `valgrind --tool=callgrind --callgrind-out-file=/dev/null`.
`heap_counts` uses the same input/flags; synthetic inputs come from
`scripts/bench/synth.py`, and laid inputs are generated once by plain v3 `-Tdot`.
To rebuild from a retained profile, use the same release command with
`RUSTFLAGS="-C target-cpu=x86-64-v3 -Cprofile-use=/absolute/path/merged.profdata"`.
Keep the original static-CRT flag on Windows and omit the x86 CPU flag on ARM64.
