# Lazy fdp fallback shapes (#626)

Construct fdp adapter fallback shapes only when the derived node has no existing shape.
An explicit match replaces eager `unwrap_or`, which allocated and then discarded a default
polygon even for real nodes whose original shape was present. The existing shape is still
cloned; missing shapes take the identical fallible constructor. Errors from an unused
fallback are no longer propagated.

The regression came from #590: `unwrap_or(NodeShape::new(..)?)` made the fallback eager. This restores lazy fallback construction. The run no longer aborts on an allocation failure in an unused fallback; C makes no such allocation.

C provenance: `lib/fdpgen/layout.c:mkDeriveNode`, adapted by `with_lgraph`.
Technique T9 allocation avoidance. Fidelity class **none**: source lookup, retained shape
fields, iteration, diagnostics and write-back are unchanged. The only omitted floating-point
work built the discarded fallback; no operation producing a retained output float is changed,
reordered, split or fused. No unsafe code, dependency or facade change.

The v1 instruction change is negligible. V3 instructions rise about 0.6–0.9% in this build despite the allocation reduction; the tables report this directly and no CPU speedup is claimed.

## Performance

Matched musl release builds, `-Tsvg`, before: master `b35ebbe2`. Both builds use the same input paths and equal-length executable paths. The supplied 1.0 v1 artifact (`f89e21d9`) is a separate reference. Master and that commit have identical Rust sources, so the matched v3 master build also serves as a 1.0-source v3 rebuild. C is stock 16.1.0 with pango-free plugins; its platform math is context, not the fidelity oracle.

### v1 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 48,482,948,570 → 48,482,161,573 | -0.00% | 48,482,946,118 | -0.00% | 13,868,762,713 |
| `wild_gv16_graphs_b69_4b5b48` | 5,226,733,518 → 5,226,382,370 | -0.01% | 5,226,780,296 | -0.01% | 2,515,084,000 |
| `fdp_sparse_250` | 86,372,584,838 → 86,371,624,432 | -0.00% | 86,372,583,812 | -0.00% | 25,624,258,178 |
| `fdp_sparse_500` | 367,103,940,899 → 367,101,059,261 | -0.00% | 367,103,942,831 | -0.00% | 123,579,157,863 |

| input | allocations before → after | Δ | cumulative bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 508,568 → 507,290 | -0.25% | 28,406,601 → 28,373,373 | 2,304,316 → 2,304,316 |
| `wild_gv16_graphs_b69_4b5b48` | 321,513 → 320,901 | -0.19% | 14,775,138 → 14,759,226 | 1,407,897 → 1,407,897 |
| `fdp_sparse_250` | 755,087 → 753,587 | -0.20% | 32,540,423 → 32,501,423 | 2,248,930 → 2,248,930 |
| `fdp_sparse_500` | 1,767,027 → 1,762,527 | -0.25% | 924,624,252 → 924,507,252 | 5,157,769 → 5,157,769 |

### v3 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 5,166,716,008 → 5,212,830,437 | +0.89% | 48,482,946,118 | -89.25% | 13,868,762,713 |
| `wild_gv16_graphs_b69_4b5b48` | 718,462,521 → 722,741,506 | +0.60% | 5,226,780,296 | -86.17% | 2,515,084,000 |
| `fdp_sparse_250` | 9,086,262,249 → 9,169,005,195 | +0.91% | 86,372,583,812 | -89.38% | 25,624,258,178 |
| `fdp_sparse_500` | 38,215,916,259 → 38,549,884,656 | +0.87% | 367,103,942,831 | -89.50% | 123,579,157,863 |

| input | allocations before → after | Δ | cumulative bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 508,568 → 507,290 | -0.25% | 28,406,601 → 28,373,373 | 2,304,316 → 2,304,316 |
| `wild_gv16_graphs_b69_4b5b48` | 321,513 → 320,901 | -0.19% | 14,775,138 → 14,759,226 | 1,407,897 → 1,407,897 |
| `fdp_sparse_250` | 755,087 → 753,587 | -0.20% | 32,540,423 → 32,501,423 | 2,248,930 → 2,248,930 |
| `fdp_sparse_500` | 1,767,027 → 1,762,527 | -0.25% | 924,624,252 → 924,507,252 | 5,157,769 → 5,157,769 |

### Native cycles and wall time

`sudo -n perf stat -x, -e cycles:u`, medians of three sequential runs per binary/input. These hardware cycles and wall times are informational on a shared host. No wall-time speedup is claimed. Each batch took its hardware-cycle samples before its own Callgrind phase; other batches and shared-host work may run concurrently.

| input | v1 master → after cycles | v3 master → after cycles | supplied 1.0 v1 / C cycles |
| :-- | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 14,540,011,336 → 14,300,296,995 | 1,588,938,737 → 1,583,292,206 | 14,580,998,657 / 5,774,058,743 |
| `wild_gv16_graphs_b69_4b5b48` | 2,001,329,353 → 1,694,813,941 | 326,082,122 → 249,535,732 | 1,860,960,375 / 1,015,565,487 |
| `fdp_sparse_250` | 32,122,144,427 → 30,339,712,837 | 2,867,626,824 → 2,898,546,542 | 29,886,785,873 / 12,177,159,284 |
| `fdp_sparse_500` | 131,161,775,150 → 130,577,810,894 | 12,242,018,271 → 12,450,831,725 | 129,282,352,345 / 57,487,952,592 |

| input | supplied 1.0 v1 ms (noise) | v1 after ms (noise) | v3 after ms (noise) | C ms (noise) |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 4582 | 4475 | 546 | 1843 |
| `wild_gv16_graphs_b69_4b5b48` | 655 | 565 | 120 | 362 |
| `fdp_sparse_250` | 9288 | 9412 | 948 | 3810 |
| `fdp_sparse_500` | 40506 | 40709 | 4186 | 17859 |

- before `wild_gv16_graphs_viewport_651d5b`: `20:23:46 up 4 days, 17:05,  2 users,  load average: 7.82, 8.26, 8.21` → `20:24:31 up 4 days, 17:06,  2 users,  load average: 9.82, 8.70, 8.36`
- before `wild_gv16_graphs_b69_4b5b48`: `20:24:31 up 4 days, 17:06,  2 users,  load average: 9.82, 8.70, 8.36` → `20:24:38 up 4 days, 17:06,  2 users,  load average: 11.24, 9.04, 8.47`
- before `fdp_sparse_250`: `20:24:38 up 4 days, 17:06,  2 users,  load average: 11.24, 9.04, 8.47` → `20:26:15 up 4 days, 17:08,  2 users,  load average: 13.30, 10.40, 9.03`
- before `fdp_sparse_500`: `20:26:15 up 4 days, 17:08,  2 users,  load average: 13.30, 10.40, 9.03` → `20:33:15 up 4 days, 17:15,  2 users,  load average: 10.98, 11.40, 10.14`
- after `wild_gv16_graphs_viewport_651d5b`: `20:31:07 up 4 days, 17:13,  2 users,  load average: 13.27, 12.12, 10.18` → `20:31:27 up 4 days, 17:13,  2 users,  load average: 12.54, 12.03, 10.19`
- after `wild_gv16_graphs_b69_4b5b48`: `20:31:27 up 4 days, 17:13,  2 users,  load average: 12.54, 12.03, 10.19` → `20:31:29 up 4 days, 17:13,  2 users,  load average: 12.54, 12.03, 10.19`
- after `fdp_sparse_250`: `20:31:29 up 4 days, 17:13,  2 users,  load average: 12.54, 12.03, 10.19` → `20:32:11 up 4 days, 17:14,  2 users,  load average: 10.57, 11.59, 10.12`
- after `fdp_sparse_500`: `20:32:11 up 4 days, 17:14,  2 users,  load average: 10.57, 11.59, 10.12` → `20:35:11 up 4 days, 17:17,  2 users,  load average: 8.95, 10.70, 10.05`

### Full v1 performance ratchet

| case | instructions before → after | Δ | allocations before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,765,014 → 158,766,179 | +0.00% | 227,582 → 227,582 | 5,232,409 → 5,232,409 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,397,731 → 162,396,058 | -0.00% | 210,281 → 210,281 | 5,232,405 → 5,232,405 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,575,329,983 → 1,575,337,248 | +0.00% | 418,625 → 418,625 | 13,947,929 → 13,947,929 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,840,660 → 850,851,681 | +0.00% | 1,024,852 → 1,024,852 | 29,377,250 → 29,377,250 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,658,698 → 1,540,658,950 | +0.00% | 678,425 → 678,425 | 991,802 → 991,802 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,803,395 → 5,226,382,406 | -0.01% | 321,513 → 320,901 | 1,407,897 → 1,407,897 |
| `tests/fixtures/unix.gv -Tpng` | 147,049,978 → 147,049,312 | -0.00% | 26,276 → 26,276 | 7,460,185 → 7,460,185 |
| `tests/fixtures/unix.gv -Tsvg` | 27,625,230 → 27,624,362 | -0.00% | 14,253 → 14,253 | 569,033 → 569,033 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,965,984 → 2,532,964,919 | -0.00% | 26,433 → 26,433 | 798,541 → 798,541 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,867,497 → 41,865,221 | -0.01% | 42,661 → 42,661 | 983,801 → 983,801 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,217,954 → 31,218,539 | +0.00% | 36,848 → 36,848 | 908,182 → 908,182 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,533,752 → 976,539,973 | +0.00% | 97,178 → 97,178 | 3,066,426 → 3,066,426 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,946,085 → 4,303,945,201 | -0.00% | 22,546 → 22,546 | 909,553 → 909,553 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,182,738 → 2,033,172,652 | -0.00% | 245,032 → 245,032 | 8,293,640 → 8,293,640 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,883,259 → 2,015,882,851 | -0.00% | 250,586 → 250,586 | 8,293,636 → 8,293,636 |

No allocation increase, no instruction increase over 2%, and no peak-heap increase over 5% against matched master. The matched before run includes the pre-existing fdp +612 allocation excess (#600). The lazy fallback removes exactly those 612 allocations; the independent overlap PR is measured against master, including that excess. No unrelated baseline is weakened. Only improved heap baselines are refreshed, normalizing local argument-path bytes. CI-derived instruction baselines are retained until the coordinator requests a refresh from the CI artifact. All four SVG/stdout and stderr hashes match before/after and between v1 and v3.

## Verification

- Workspace formatting, Clippy with all targets/warnings denied, and panic-site ratchet: pass; panic counts unchanged.
- `cargo test --workspace`: pass, with unchanged fidelity baselines.
- New adapter test covers existing and missing shapes, three-dimensional coordinates and position write-back.
- Full v1 performance ratchet: pass against the committed baseline and matched master. The b69 count returns to the existing baseline; no heap baseline needs weakening or refreshing.
- Four target SVG/stdout and stderr hashes match before/after and between v1 and v3.

Closes #626. Fixes #600 (the matched b69 allocation excess disappears).

## Changelog entry

- fdp avoids reconstructing unused default shapes in derived-graph adapters, removing 612–4,500 allocations on viewport, b69 and sparse 250/500 inputs. Peak heap is unchanged. This fixes an allocation and unused-error path; no end-to-end CPU speedup is claimed.

Machine-readable measurements: [fdp-fallback-626.json](data/fdp-fallback-626.json).
