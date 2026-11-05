# Packing hash repeatability (#776)

The packing cell set now uses `BuildHasherDefault<DefaultHasher>`. This is technique T15
with the standard library and no new dependency. It makes both the set layout and the
cell iteration order repeatable within a given Rust build.

On `twopi_pairs_1000`, the five-run instruction spread falls from **0.21304% to
0.00098%** (about 217× smaller), and median instructions fall **2.48%**. The largest
remaining spread among all 19 cases is **0.01739%**, versus the unchanged **2%** gate.

Fidelity class: **ordering**. In `lib/pack/pack.c` terms, `genBox`/`genPoly` construct
the same cell sets and `fits` answers whether any translated cell is occupied. A failed
probe mutates nothing; a successful probe inserts every cell. Changing which collision
is discovered first cannot change the answer, the spiral candidate order, or the chosen
placement. `same_shape` ignores iteration order, and verbose cell dumps are sorted.
No floating-point expression changes.

## Method

Measured 2026-11-05 against master `234fc35f116bceca2c34579879a6e6893112ccf8`.
Both ratchet binaries use rustc 1.99.1 (58cf8f9c3 2026-10-13) and valgrind-3.24.0, static
`x86_64-unknown-linux-musl`, the pinned ratchet release profile, without PGO or v3 flags.
Each of the 19 cases ran five times through `ratchet.measure_case`, four workers maximum,
at nice 15 with an 8 GB virtual-memory limit per process. Relative binary/input paths
and the environment stayed the same. Each sample also hashes native stdout and stderr.
All 95 samples on each side have the same per-case output hashes, allocation counts,
allocated bytes and peak heap, and those values agree before/after.

**Spread = (maximum − minimum) / median × 100**, not a confidence interval.
Five samples are an observed range, not a guarantee for every future environment.
The earlier issue reported about 1.5%; this fresh-master sample observes a smaller
baseline spread. Other randomized lookups and startup work remain, so exact equality
of total instructions is not claimed. The +2% gate and committed baseline are unchanged.

[Raw samples, output hashes, memory counts and wall timings](data/pack-hash-repeatability.json).

Ratchet binary SHA-256:

- Before: `6ee6b734306953905ea6bee3dbf38bd81d9c99387056e8eb03c37acd22d94c26`
- After: `2358fb24a2976ab366e0fb98a5a65681b7d5f8a7c3edfbefad11dd9ffa235a58`

## Instruction spread on every ratchet case

| Case | Before min–max | Spread | After min–max | Spread |
| :-- | --: | --: | --: | --: |
| `tests/fixtures/unix.gv -Tsvg` | 16,578,833–16,579,131 | 0.00180% | 16,579,010–16,579,112 | 0.00062% |
| `tests/fixtures/unix.gv -Tpng` | 136,252,224–136,253,120 | 0.00066% | 136,252,359–136,253,201 | 0.00062% |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 777,140,741–777,145,289 | 0.00059% | 777,143,414–777,147,415 | 0.00051% |
| `synth/dot_dag_1000.gv -Tsvg` | 740,320,515–740,325,302 | 0.00065% | 740,316,912–740,325,618 | 0.00118% |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,282,478,639–2,282,479,668 | 0.00005% | 2,282,478,673–2,282,480,098 | 0.00006% |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 2,635,411,065–2,635,476,462 | 0.00248% | 2,635,383,335–2,635,384,185 | 0.00003% |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 612,428,548–612,430,756 | 0.00036% | 612,428,991–612,432,783 | 0.00062% |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 25,935,176–25,937,842 | 0.01028% | 25,935,467–25,939,978 | 0.01739% |
| `tests/twopi/fx_arrows.gv -Tsvg` | 549,573,314–549,574,060 | 0.00014% | 549,573,168–549,573,970 | 0.00015% |
| `synth/twopi_tree_5000.gv -Tsvg` | 410,683,340–410,699,256 | 0.00388% | 410,677,407–410,696,390 | 0.00462% |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 21,424,872–21,426,498 | 0.00759% | 21,425,442–21,426,026 | 0.00273% |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 14,076,602–14,077,133 | 0.00377% | 14,076,704–14,077,924 | 0.00867% |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 770,007,333–770,013,066 | 0.00074% | 770,004,356–770,011,256 | 0.00090% |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 115,426,529–115,428,739 | 0.00191% | 115,425,903–115,428,008 | 0.00182% |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 99,588,528–99,591,515 | 0.00300% | 99,589,219–99,595,184 | 0.00599% |
| `synth/dot_components_1000.gv -Tsvg` | 347,288,769–347,295,448 | 0.00192% | 347,278,224–347,295,169 | 0.00488% |
| `synth/dot_par_10_500.gv -Tsvg` | 1,415,667,897–1,415,668,280 | 0.00003% | 1,415,667,952–1,415,668,467 | 0.00004% |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 782,163,898–782,179,852 | 0.00204% | 782,165,242–782,182,527 | 0.00221% |
| `synth/twopi_pairs_1000.gv -Tsvg` | 620,738,543–622,061,941 | 0.21304% | 605,801,051–605,806,991 | 0.00098% |

Maximum after spread across all cases: **0.01739%**.

## Ratchet medians and memory

Allocation and peak-heap counts below are identical in all five runs on each side.

| Case | Instructions before → after | Δ | Allocations before → after | Δ | Peak heap B before → after |
| :-- | --: | --: | --: | --: | --: |
| `tests/fixtures/unix.gv -Tsvg` | 16,578,904 → 16,579,056 | +0.001% | 6,561 → 6,561 | +0 | 520,150 → 520,150 |
| `tests/fixtures/unix.gv -Tpng` | 136,252,478 → 136,252,621 | +0.000% | 19,316 → 19,316 | +0 | 6,059,166 → 6,059,166 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 777,143,840 → 777,145,389 | +0.000% | 104,193 → 104,193 | +0 | 8,279,149 → 8,279,149 |
| `synth/dot_dag_1000.gv -Tsvg` | 740,322,170 → 740,322,077 | -0.000% | 186,698 → 186,698 | +0 | 12,500,901 → 12,500,901 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,282,479,085 → 2,282,478,990 | -0.000% | 11,252 → 11,252 | +0 | 589,431 → 589,431 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 2,635,432,517 → 2,635,384,007 | -0.002% | 177,884 → 177,884 | +0 | 1,042,341 → 1,042,341 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 612,430,097 → 612,430,181 | +0.000% | 39,395 → 39,395 | +0 | 2,305,620 → 2,305,620 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 25,935,953 → 25,936,162 | +0.001% | 16,108 → 16,108 | +0 | 835,450 → 835,450 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 549,573,809 → 549,573,338 | -0.000% | 15,784 → 15,784 | +0 | 815,542 → 815,542 |
| `synth/twopi_tree_5000.gv -Tsvg` | 410,685,584 → 410,689,628 | +0.001% | 311,542 → 311,542 | +0 | 20,892,196 → 20,892,196 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 21,425,455 → 21,425,897 | +0.002% | 13,432 → 13,432 | +0 | 745,556 → 745,556 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 14,076,855 → 14,077,159 | +0.002% | 9,334 → 9,334 | +0 | 696,496 → 696,496 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 770,008,830 → 770,008,241 | -0.000% | 96,823 → 96,823 | +0 | 8,279,153 → 8,279,153 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 115,428,338 → 115,426,993 | -0.001% | 129,377 → 129,377 | +0 | 2,858,109 → 2,858,109 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 99,590,012 → 99,590,829 | +0.001% | 88,813 → 88,813 | +0 | 2,100,133 → 2,100,133 |
| `synth/dot_components_1000.gv -Tsvg` | 347,289,713 → 347,288,771 | -0.000% | 178,055 → 178,055 | +0 | 9,287,486 → 9,287,486 |
| `synth/dot_par_10_500.gv -Tsvg` | 1,415,668,072 → 1,415,668,175 | +0.000% | 124,282 → 124,282 | +0 | 59,807,603 → 59,807,603 |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 782,171,902 → 782,173,570 | +0.000% | 624,927 → 624,927 | +0 | 23,551,628 → 23,551,628 |
| `synth/twopi_pairs_1000.gv -Tsvg` | 621,182,725 → 605,801,810 | -2.476% | 103,415 → 103,415 | +0 | 14,668,030 → 14,668,030 |

## Paired wall times (informational)

Separate musl binaries built with `scripts/build-pgo.py`, `-C target-cpu=x86-64-v3`,
fresh PGO training per revision. Nine before/after pairs alternate execution order;
each pair is followed by two runs of the identical after binary as a control.
The table reports median times and median per-pair ratios. All ratchet cases are
included, including paths this change does not optimize. Shared-machine timings
are informational; the control shows the scale of timing noise.

Load at start: `03:27:38 up 1 day, 23:28,  0 users,  load average: 9.74, 9.93, 6.59`. At end: `03:29:11 up 1 day, 23:30,  0 users,  load average: 5.13, 8.32, 6.34`.

PGO before SHA-256: `2af698e777509584906fc0eff1adb59eac120c36522fb3f3fe402b925ffe1365`.

PGO after SHA-256: `db4bc85a0cdcaa22eeb9f7e0f7fc12305d8ace0627ac9dd966d8baa82755d353`.

| Case | Before ms | After ms | Paired Δ | Identical-binary control Δ |
| :-- | --: | --: | --: | --: |
| `tests/fixtures/unix.gv -Tsvg` | 4.881 | 4.806 | +1.09% | -0.99% |
| `tests/fixtures/unix.gv -Tpng` | 29.414 | 30.010 | -0.07% | -0.49% |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 78.448 | 75.684 | -1.34% | -0.26% |
| `synth/dot_dag_1000.gv -Tsvg` | 89.976 | 89.074 | -1.13% | +0.31% |
| `tests/neato/hw2_random_120.gv -Tsvg` | 21.817 | 21.850 | -0.53% | -1.16% |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 63.153 | 63.048 | +0.22% | +0.35% |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 30.762 | 30.487 | -0.16% | +0.49% |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 6.556 | 6.609 | +1.34% | +2.20% |
| `tests/twopi/fx_arrows.gv -Tsvg` | 31.438 | 31.566 | +0.17% | -0.14% |
| `synth/twopi_tree_5000.gv -Tsvg` | 65.464 | 68.613 | +2.29% | -2.65% |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 6.381 | 6.375 | -0.19% | +1.49% |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 5.115 | 5.091 | -0.42% | -1.09% |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 74.473 | 75.408 | +1.48% | -0.54% |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 20.831 | 20.658 | -1.09% | +0.38% |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 18.528 | 18.621 | -1.90% | +0.41% |
| `synth/dot_components_1000.gv -Tsvg` | 48.264 | 48.194 | -0.15% | +1.83% |
| `synth/dot_par_10_500.gv -Tsvg` | 190.317 | 186.176 | -1.88% | +0.56% |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 112.295 | 112.202 | -2.24% | -0.28% |
| `synth/twopi_pairs_1000.gv -Tsvg` | 81.492 | 69.328 | -14.43% | +0.09% |

The largest measured slowdown on an untargeted path is `twopi_tree_5000` at +2.29%
paired wall time; its identical-binary control moves −2.65%, and its median
instruction count changes by less than 0.001%. Other measured slowdowns are
`b143 -Tplain` (+1.48%), circo (+1.34%), `unix -Tsvg` (+1.09%), fdp (+0.22%) and
`twopi/fx_arrows` (+0.17%). These measurements do not establish zero wall-time
regression; they report the shared-machine noise alongside each change.

The targeted packing inputs also have five reference runs against the 1.0 musl
binary (v1, no PGO) and C Graphviz 16.1.0 built with FMA, using its core font plugins
and system libm. These are timing references, not byte-identity oracles.

| Case | 1.0 ms | After ms | ×1.0 speed | C ms | ×C speed |
| :-- | --: | --: | --: | --: | --: |
| `synth/dot_components_1000.gv -Tsvg` | 140.397 | 48.194 | 2.91 | 111.238 | 2.31 |
| `synth/twopi_pairs_1000.gv -Tsvg` | 4029.202 | 69.328 | 58.12 | 6863.866 | 99.01 |

## Byte and regression checks

- The repeatability unit test fails with the old randomized set and passes with the fix.
- The packing/reference-search test also reverses cell order, covering fixed components,
  empty shapes, changing shapes/orientations, and wrapping coordinates.
- All 2,612 corpus inputs have identical stdout, stderr and exit status between
  the before/after v3/PGO CLIs with `-Tsvg`, including inputs outside the C-stable baseline.
- All 19 ratchet outputs are byte-identical across all five runs and both revisions.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `scripts/panic-sites.sh`, and `cargo test --workspace` pass. All cargo work uses the
  local cargo-slot wrapper; the full workspace suite includes the fidelity ratchets.
- `ratchet.py --from` the first after sample passes against the committed baseline
  without an acceptance override. Neither performance nor fidelity baselines change.

## Reproduction

Build each revision with the pinned `ratchet.py` profile, then run
`python3 -I scripts/bench/ratchet.py --no-build --out <sample.json>` five times,
keeping binary/input paths and environment constant. Compare per-case min/max and
medians. Generate synthetic/laid inputs through `ratchet.prepare_inputs`; hash stdout
and stderr from the same CLI command for byte comparisons. For wall measurements,
build each revision using `scripts/build-pgo.py --target x86_64-unknown-linux-musl`
with `RUSTFLAGS="-C target-cpu=x86-64-v3"`, fresh external profile directories and the
local cargo-slot wrapper; alternate the binaries and interleave identical-binary controls.
Keep builds, profiles and renders on the scratch volume. Do not refresh the baseline
from these local measurements; the coordinator batches CI baseline updates.
