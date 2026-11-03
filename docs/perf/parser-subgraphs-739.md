# Named-subgraph lookup (#739)

`Parser::open_subgraph` now uses a lookup-only B-tree keyed by parent, name text and HTML flag. Both lookup and insertion take O(log S) comparisons instead of scanning all earlier subgraphs. Name text borrows the input when the lexer can borrow it. Anonymous subgraphs never enter the index; empty names do. The index is reset for each graph.

C counterparts: `lib/cgraph/subg.c:agsubg` and `agfindsubg_by_id`. The graph and child vectors still assign IDs and define all iteration order. Appending a newly assigned ID also no longer scans the child vector: that ID cannot already be present. The reopen-default behavior from #720 is unchanged. No float operations, dependencies, public API or output ordering change.

## Method

Before: #720 head `ed47b765`; after: `8ae2f124`. Both use Rust 1.99.1, `x86_64-unknown-linux-musl`, `-Ctarget-cpu=x86-64-v3` and `scripts/build-pgo.py`, separately trained with fresh profiles on the same 276-command list. Builds, profiles and scratch outputs lived on the jobs volume. Builds used cargo-slot (4 jobs), and measurements used nice 15 with an 8 GB virtual-memory limit.

One warmup per binary/case, then 11 paired rounds alternating before/after/control and control/after/before. Control is a copy of the before binary. Times include process startup, with stdout discarded. The 1.0 reference is the v1 musl 1.0 release-candidate binary; C is the installed C 16.1.0 reference with its pango-free plugins (stock C math, no preload). Reference columns are three-run medians, informational and not paired.

Binary SHA-256:

- Before/control: `29d0d20fec3e05890419fe65a3baa48a34ec599b136f90eb82dd444743019a77`
- After: `3ab0d5b8a2ab0f9abaa01f682e00793fdaa694c20de4f1e8d13f7596965eec55`

Load at start/end:

```text
04:07:12 up 8 min,  0 users,  load average: 20.97, 18.29, 9.02
04:08:56 up 10 min,  0 users,  load average: 22.31, 19.62, 10.49
```

## Target wall time

Times are median milliseconds; paired ratios are medians of per-round before/after and before/control ratios. The identical-binary control exposes substantial shared-machine noise, especially patchwork SVG.

| Input / format | Before ms | After ms | Paired × | Identical control × | 1.0 ms | ×1.0 | C ms | ×C |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| osage_clustered_20000.gv / svg | 255.47 | 231.68 | 1.093 | 0.994 | 796.78 | 3.44 | 264.11 | 1.14 |
| osage_clustered_20000.gv / plain | 308.90 | 282.20 | 1.043 | 0.979 | 927.32 | 3.29 | 440.71 | 1.56 |
| patchwork_clustered_20000.gv / svg | 364.69 | 339.91 | 1.155 | 1.083 | 653.05 | 1.92 | 364.56 | 1.07 |
| patchwork_clustered_20000.gv / plain | 412.97 | 345.55 | 1.121 | 1.018 | 926.31 | 2.68 | 398.41 | 1.15 |

## Target instructions (v3 + PGO)

Valgrind Callgrind, same SVG commands and binaries as above. These local counts are diagnostic; ratchet baseline refreshes use CI only.

| Engine / 20k SVG | Before | After | Δ |
| :-- | --: | --: | --: |
| osage | 1,133,068,130 | 962,318,610 | -15.07% |
| patchwork | 1,270,083,569 | 1,099,422,205 | -13.44% |

## Other paths

All quick-suite cases were measured with the same paired/control protocol. Positive wall Δ means slower; sub-millisecond changes and control shifts should not be read as reliable gains or regressions. This includes every observed slowdown.

| Case | Before ms | After ms | Wall Δ | Paired × | Identical control × |
| :-- | --: | --: | --: | --: | --: |
| tests/fixtures/unix.gv / svg | 8.12 | 8.35 | +2.75% | 1.025 | 1.080 |
| tests/fixtures/unix.gv / png | 35.02 | 35.72 | +2.00% | 0.968 | 0.984 |
| tests/wild/gv16_graphs_b143_5416de.gv / svg | 126.23 | 130.21 | +3.15% | 1.009 | 1.043 |
| synth/dot_dag_1000.gv / svg | 156.60 | 157.26 | +0.42% | 1.010 | 1.005 |
| tests/neato/hw2_random_120.gv / svg | 32.43 | 32.55 | +0.37% | 0.964 | 0.946 |
| tests/fdp/wild_gv16_graphs_b69_4b5b48.gv / svg | 98.17 | 78.90 | -19.64% | 1.047 | 1.143 |
| tests/sfdp/gen_grid20x20.gv / svg | 35.30 | 35.93 | +1.81% | 0.998 | 0.987 |
| tests/circo/wild_gv16_directed_sdh_29b016.gv / svg | 24.09 | 23.19 | -3.75% | 1.017 | 1.042 |
| tests/twopi/fx_arrows.gv / svg | 45.78 | 42.73 | -6.67% | 1.070 | 1.067 |
| synth/twopi_tree_5000.gv / svg | 105.28 | 101.05 | -4.02% | 1.055 | 1.007 |
| tests/osage/wild_gv16_graphs_b51_33bcb0.gv / svg | 8.45 | 7.62 | -9.77% | 1.122 | 1.025 |
| tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv / svg | 6.99 | 6.71 | -3.92% | 0.979 | 1.038 |
| synth/osage_clustered_20000.gv / canon | 459.25 | 445.32 | -3.03% | 1.054 | 1.049 |

## Verification

Parser tests cover same names under distinct parents and at the root, reopening a parent, exact creation/child order, HTML/plain/empty names, anonymous subgraphs and owned concatenated token text. Existing tests cover local names and subgraph edge endpoints. Before/after SHA-256 output hashes match on both 20k inputs in SVG, plain, canon, DOT and JSON (10/10).

Full raw wall samples, PGO build metadata, instruction counts and output hashes: [data/parser-subgraphs-739.json](data/parser-subgraphs-739.json).

Local gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/panic-sites.sh` and `cargo test --workspace` all pass. Every cargo invocation used cargo-slot; full workspace tests include the unchanged fidelity ratchets.

## Local ratchet

All 17 cases complete normally. Instructions and peak heap pass; peak heap is unchanged in every case. The exact allocation gate fails on four cases: circo +4, osage +3, patchwork +3, and the single named-subgraph nop2 case +1. The largest relative increase is 0.031%. These are the lookup index’s B-tree nodes. This is the explicit tradeoff for logarithmic lookup; `perf-regression-accepted` marks it for coordinator review. No baseline has been refreshed from local counts. CI was still queued at handoff; refresh from its head artifact only.

Counts below compare the committed baseline with the local generic-musl ratchet build, not the separate v3+PGO pair. Local Valgrind/compiler instruction movement on unrelated paths must not be attributed to this change.

| Case | Baseline instructions | Head instructions | Δ | Baseline allocations | Head allocations | Δ |
| :-- | --: | --: | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 122,153,748 | 122,132,168 | -0.02% | 131,257 | 131,257 | +0 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 106,114,136 | 106,096,364 | -0.02% | 91,677 | 91,677 | +0 |
| `synth/dot_components_1000.gv -Tsvg` | 532,475,920 | 512,665,008 | -3.72% | 197,063 | 197,063 | +0 |
| `synth/dot_dag_1000.gv -Tsvg` | 935,360,867 | 917,310,746 | -1.93% | 206,165 | 206,165 | +0 |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 830,205,029 | 826,035,199 | -0.50% | 674,923 | 674,924 | +1 |
| `synth/twopi_tree_5000.gv -Tsvg` | 472,290,072 | 479,587,407 | +1.55% | 366,345 | 366,345 | +0 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 202,065,449 | 197,038,019 | -2.49% | 17,705 | 17,709 | +4 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 2,641,658,514 | 2,642,049,239 | +0.01% | 182,014 | 182,014 | +0 |
| `tests/fixtures/unix.gv -Tpng` | 138,430,686 | 137,766,231 | -0.48% | 19,911 | 19,911 | +0 |
| `tests/fixtures/unix.gv -Tsvg` | 18,845,855 | 18,182,963 | -3.52% | 7,385 | 7,385 | +0 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,265,401,487 | 2,283,497,437 | +0.80% | 12,663 | 12,663 | +0 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 22,738,230 | 22,738,421 | +0.00% | 14,339 | 14,342 | +3 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 14,985,925 | 15,004,868 | +0.13% | 9,884 | 9,887 | +3 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 618,829,934 | 617,954,632 | -0.14% | 46,516 | 46,516 | +0 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 2,070,691,238 | 2,070,632,272 | -0.00% | 16,206 | 16,206 | +0 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 1,071,869,954 | 1,064,166,292 | -0.72% | 99,958 | 99,958 | +0 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 1,079,961,939 | 1,072,254,421 | -0.71% | 108,210 | 108,210 | +0 |
