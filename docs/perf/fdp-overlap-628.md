# Flat fdp overlap working arrays (#628)

Keep fdp overlap-removal positions and displacements in flat `[f64; 2]` arrays, with compact
immutable geometry and ordered CSR outgoing heads for each component. The repeated quadratic
force loop walks slices instead of scattered derived-node records, and no longer collects
an outgoing-edge vector for every node on every iteration. Scratch is sized to the component;
the zero-overlap early return remains allocation-free. Positions and displacements are copied
back together after the expansion loop.

C provenance: `lib/fdpgen/xlayout.c:overlap`, `cntOverlaps`, `doRep`, `applyAttr`, `adjust`,
`x_layout`; techniques T6 data layout, T7 compact adjacency and T9 allocation reuse. This branch starts independently
from master, without #626's fallback fix. No unsafe code, dependency or facade change.

Fidelity class **none**. Component/node/edge order, attraction after each outer node's
repulsion, pinned-node behavior, jitter RNG draws, diagnostics and all exit conditions stay
the same. The output-float argument is:

| retained quantity | identical operations and operands |
| :-- | :-- |
| half width/height | same `w / 2 + margin` or `w * margin / 2`, cached while dimensions/margins are immutable |
| radius | same `math::hypot(half_width, half_height)` result, cached for immutable geometry |
| pair delta/distance | same `q - p` coordinates and `math::hypot`; coincident/NaN distance draws jitter x then y in the same retry order |
| overlap | actual source positions, including after a force delta was jittered; same absolute differences, half-size sums and comparisons |
| repulsive force | same branch and division by `dist * dist`, then the same `dist > mlimit` zeroing |
| attractive force | same radius sum, `dist - din`, then `dout * dout / ((k + din) * dist)` |
| displacement | q x/y updates then p x/y updates, preserving each `delta.mul_add(force, old_disp)` and each sign; edge updates remain interleaved after each outer node |
| position | same `disp[0].mul_add(disp[0], disp[1] * disp[1])`, temperature comparison, square root and component-wise additions/scaling |
| cooling/tries | original parameter arithmetic, `k` increment and loop/early-return sequence |

No retained float is reassociated, split or fused differently. Matching position/displacement
suffixes are traversed with zipped `split_first`/`split_first_mut` iterators. The remaining
`unreachable!` states a component-construction invariant: `Comp::new` builds membership from
its sorted nodes, `Comp::remove` updates both, and `dg.outs` filters through the same unchanged
membership set, so every outgoing head must be found. `tests/panic-sites.tsv` has no
`unreachable!` column and does not count this site; its validity is justified independently
of that gate.

## Performance

Matched musl release builds, `-Tsvg`, before: master `b35ebbe2`. Both builds use the same input paths and equal-length executable paths. The supplied 1.0 v1 artifact (`f89e21d9`) is a separate reference. Master and that commit have identical Rust sources, so the matched v3 master build also serves as a 1.0-source v3 rebuild. C is stock 16.1.0 with pango-free plugins; its platform math is context, not the fidelity oracle.

### v1 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 48,482,948,570 → 47,537,670,307 | -1.95% | 48,482,946,118 | -1.95% | 13,868,762,713 |
| `wild_gv16_graphs_b69_4b5b48` | 5,226,733,518 → 5,042,941,255 | -3.52% | 5,226,780,296 | -3.52% | 2,515,084,000 |
| `fdp_sparse_250` | 86,372,584,838 → 84,655,882,791 | -1.99% | 86,372,583,812 | -1.99% | 25,624,258,178 |
| `fdp_sparse_500` | 367,103,940,899 → 361,372,020,466 | -1.56% | 367,103,942,831 | -1.56% | 123,579,157,863 |

| input | allocations before → after | Δ | cumulative bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 508,568 → 292,565 | -42.47% | 28,406,601 → 18,522,657 | 2,304,316 → 2,304,316 |
| `wild_gv16_graphs_b69_4b5b48` | 321,513 → 215,674 | -32.92% | 14,775,138 → 11,226,554 | 1,407,897 → 1,407,897 |
| `fdp_sparse_250` | 755,087 → 289,895 | -61.61% | 32,540,423 → 16,386,367 | 2,248,930 → 2,248,930 |
| `fdp_sparse_500` | 1,767,027 → 819,332 | -53.63% | 924,624,252 → 891,406,220 | 5,157,769 → 5,157,769 |

### v3 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 5,166,716,008 → 4,178,103,402 | -19.13% | 48,482,946,118 | -91.38% | 13,868,762,713 |
| `wild_gv16_graphs_b69_4b5b48` | 718,462,521 → 575,644,503 | -19.88% | 5,226,780,296 | -88.99% | 2,515,084,000 |
| `fdp_sparse_250` | 9,086,262,249 → 7,292,791,226 | -19.74% | 86,372,583,812 | -91.56% | 25,624,258,178 |
| `fdp_sparse_500` | 38,215,916,259 → 31,499,217,925 | -17.58% | 367,103,942,831 | -91.42% | 123,579,157,863 |

| input | allocations before → after | Δ | cumulative bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 508,568 → 292,565 | -42.47% | 28,406,601 → 18,522,657 | 2,304,316 → 2,304,316 |
| `wild_gv16_graphs_b69_4b5b48` | 321,513 → 215,674 | -32.92% | 14,775,138 → 11,226,554 | 1,407,897 → 1,407,897 |
| `fdp_sparse_250` | 755,087 → 289,895 | -61.61% | 32,540,423 → 16,386,367 | 2,248,930 → 2,248,930 |
| `fdp_sparse_500` | 1,767,027 → 819,332 | -53.63% | 924,624,252 → 891,406,220 | 5,157,769 → 5,157,769 |

### Native cycles and wall time

`sudo -n perf stat -x, -e cycles:u`, medians of three sequential runs per binary/input. These hardware cycles and wall times are informational on a shared host. No wall-time speedup is claimed. Each batch took its hardware-cycle samples before its own Callgrind phase; other batches and shared-host work may run concurrently.

| input | v1 master → after cycles | v3 master → after cycles | supplied 1.0 v1 / C cycles |
| :-- | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 14,540,011,336 → 13,756,952,829 | 1,588,938,737 → 1,351,706,972 | 14,580,998,657 / 5,774,058,743 |
| `wild_gv16_graphs_b69_4b5b48` | 2,001,329,353 → 1,602,615,568 | 326,082,122 → 205,049,194 | 1,860,960,375 / 1,015,565,487 |
| `fdp_sparse_250` | 32,122,144,427 → 29,385,000,581 | 2,867,626,824 → 2,414,670,748 | 29,886,785,873 / 12,177,159,284 |
| `fdp_sparse_500` | 131,161,775,150 → 126,416,948,635 | 12,242,018,271 → 10,435,380,654 | 129,282,352,345 / 57,487,952,592 |

| input | supplied 1.0 v1 ms (noise) | v1 after ms (noise) | v3 after ms (noise) | C ms (noise) |
| :-- | --: | --: | --: | --: |
| `wild_gv16_graphs_viewport_651d5b` | 4582 | 4342 | 474 | 1843 |
| `wild_gv16_graphs_b69_4b5b48` | 655 | 538 | 103 | 362 |
| `fdp_sparse_250` | 9288 | 9161 | 791 | 3810 |
| `fdp_sparse_500` | 40506 | 39463 | 3559 | 17859 |

- before `wild_gv16_graphs_viewport_651d5b`: `20:23:46 up 4 days, 17:05,  2 users,  load average: 7.82, 8.26, 8.21` → `20:24:31 up 4 days, 17:06,  2 users,  load average: 9.82, 8.70, 8.36`
- before `wild_gv16_graphs_b69_4b5b48`: `20:24:31 up 4 days, 17:06,  2 users,  load average: 9.82, 8.70, 8.36` → `20:24:38 up 4 days, 17:06,  2 users,  load average: 11.24, 9.04, 8.47`
- before `fdp_sparse_250`: `20:24:38 up 4 days, 17:06,  2 users,  load average: 11.24, 9.04, 8.47` → `20:26:15 up 4 days, 17:08,  2 users,  load average: 13.30, 10.40, 9.03`
- before `fdp_sparse_500`: `20:26:15 up 4 days, 17:08,  2 users,  load average: 13.30, 10.40, 9.03` → `20:33:15 up 4 days, 17:15,  2 users,  load average: 10.98, 11.40, 10.14`
- after `wild_gv16_graphs_viewport_651d5b`: `20:39:20 up 4 days, 17:21,  2 users,  load average: 11.05, 10.28, 9.98` → `20:39:40 up 4 days, 17:21,  2 users,  load average: 11.86, 10.52, 10.06`
- after `wild_gv16_graphs_b69_4b5b48`: `20:39:40 up 4 days, 17:21,  2 users,  load average: 11.86, 10.52, 10.06` → `20:39:43 up 4 days, 17:21,  2 users,  load average: 11.87, 10.55, 10.07`
- after `fdp_sparse_250`: `20:39:43 up 4 days, 17:21,  2 users,  load average: 11.87, 10.55, 10.07` → `20:40:22 up 4 days, 17:22,  2 users,  load average: 10.52, 10.41, 10.05`
- after `fdp_sparse_500`: `20:40:22 up 4 days, 17:22,  2 users,  load average: 10.52, 10.41, 10.05` → `20:43:16 up 4 days, 17:25,  2 users,  load average: 8.92, 9.83, 9.90`

### Full v1 performance ratchet

| case | instructions before → after | Δ | allocations before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,765,014 → 158,766,047 | +0.00% | 227,582 → 227,582 | 5,232,409 → 5,232,409 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,397,731 → 162,398,977 | +0.00% | 210,281 → 210,281 | 5,232,405 → 5,232,405 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,575,329,983 → 1,575,338,454 | +0.00% | 418,625 → 418,625 | 13,947,929 → 13,947,929 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,840,660 → 850,842,498 | +0.00% | 1,024,852 → 1,024,852 | 29,377,250 → 29,377,250 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,658,698 → 1,540,659,380 | +0.00% | 678,425 → 678,425 | 991,802 → 991,802 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,803,395 → 5,042,927,258 | -3.52% | 321,513 → 215,674 | 1,407,897 → 1,407,897 |
| `tests/fixtures/unix.gv -Tpng` | 147,049,978 → 147,049,519 | -0.00% | 26,276 → 26,276 | 7,460,185 → 7,460,185 |
| `tests/fixtures/unix.gv -Tsvg` | 27,625,230 → 27,624,777 | -0.00% | 14,253 → 14,253 | 569,033 → 569,033 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,965,984 → 2,532,964,807 | -0.00% | 26,433 → 26,433 | 798,541 → 798,541 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,867,497 → 41,866,926 | -0.00% | 42,661 → 42,661 | 983,801 → 983,801 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,217,954 → 31,218,427 | +0.00% | 36,848 → 36,848 | 908,182 → 908,182 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,533,752 → 976,535,230 | +0.00% | 97,178 → 97,178 | 3,066,426 → 3,066,426 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,946,085 → 4,303,946,136 | +0.00% | 22,546 → 22,546 | 909,553 → 909,553 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,182,738 → 2,033,175,418 | -0.00% | 245,032 → 245,032 | 8,293,640 → 8,293,640 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,883,259 → 2,015,886,971 | +0.00% | 250,586 → 250,586 | 8,293,636 → 8,293,636 |

No allocation increase, no instruction increase over 2%, and no peak-heap increase over 5% against matched master. The matched before run includes the pre-existing fdp +612 allocation excess (#600). The lazy fallback removes exactly those 612 allocations; the independent overlap PR is measured against master, including that excess. No unrelated baseline is weakened. Only improved heap baselines are refreshed, normalizing local argument-path bytes. CI-derived instruction baselines are retained until the coordinator requests a refresh from the CI artifact. All four SVG/stdout and stderr hashes match before/after and between v1 and v3.

## Verification

- Workspace formatting and Clippy with all targets/warnings denied: pass.
- Panic-site ratchet: pass; `xlayout.rs` indexing sites fall 26 → 20 and arithmetic sites 9 → 8; baseline tightened.
- `cargo test --workspace`: pass, with unchanged fidelity baselines; the full suite also passes with `RUSTFLAGS="-C target-cpu=x86-64-v3"`.
- Differential test runs 24 combinations against the original scalar traversal and compares every position/displacement bit, returned overlap count and next RNG draw. Includes pinned nodes, coincident points, non-member heads, non-contiguous component IDs, backward heads, self edges, zero iterations, additive/scaled margins and finite/infinite force limits.
- A traced clustered run (`GVDBG=1 GVDBG2=1`) matches master stdout/stderr on both v1 and v3: 77,933 stderr bytes, including 528 repulsion and 151 attraction records.
- Full v1 performance ratchet: pass against the committed baseline and matched master.
- Four target SVG/stdout and stderr hashes match before/after and between v1 and v3.

Closes #628. This PR is measured alone against master; its gains and #626's do not add exactly.

## Changelog entry

- fdp overlap removal uses flat working arrays and reuses ordered adjacency, reducing end-to-end instructions by 17.6–19.9% on x86-64-v3 builds (viewport, b69, sparse 250/500) and removing 106k–948k allocations versus master. Peak heap is unchanged.

Machine-readable measurements: [fdp-overlap-628.json](data/fdp-overlap-628.json).
