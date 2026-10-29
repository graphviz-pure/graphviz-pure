# Render model allocation diet (#610)

Move measured label buffers into the render model, borrow source Bézier points, and share
immutable colour names across drawing-state copies. This removes the render stage's
second copies of label source strings, line strings and HTML trees, as well as per-segment
point allocations and repeated colour-name allocations. Shape-only geometry remains in
`RNode`; measured labels are released as soon as ownership passes to `TextLabel`.

This is the independent render-stage PR for #610 (T9 allocation avoidance), based on fresh
master rather than #623 or #624. `AttrMap`, source graph storage and the stable facade are
unchanged. C provenance: `lib/common/labels.c:make_label, make_simple_label`,
`lib/common/types.h:Agnodeinfo_t`, `lib/common/color.c:colorxlate`, and the existing
`emit.c` spline traversal functions. No unsafe code or new dependency.

Fidelity class **none**: the emitter sees the same strings, point slices, metadata and order.
Each label span retains `line.width`, `line.height`, the same `0.1 * line.font_size`, and the
same font construction. Label dimensions, space, font size, position and valign are copied
or moved unchanged. Root-label positioning retains the same `+16`, `+8`, division by two
and coordinate arithmetic on the same measured dimensions. All spline operations consume
the same source points in the same segment/point order; borrowing replaces only the copy.
Colour resolution and warnings still run in the same order; shared colour strings preserve
original spelling, list spelling and debug output. No retained floating-point operation is
reordered, removed, split or fused.

The initial full dhat stage rankings and top allocation sites are recorded in [#623's
measurement note](https://github.com/graphviz-pure/graphviz-pure/blob/e152b1f67ca16120a64bae75c992655df6e43a20/docs/perf/allocation-stages-610.md).
This completes the requested three stage PRs; their gains are independently measured
against master and should not be added as CPU speedups. Remaining known work includes
#536 attribute interning, #604 twopi traversal, #606 osage trace and #617 formatting.

## Performance

Musl x86-64-v1 release builds on the three 20,000-node inputs, `-Tsvg`. Before is saved
master `b35ebbe2`; the supplied 1.0 reference is `f89e21d9` (identical Rust sources).
Before/after heap counts use matching argument lengths and identical input paths.

| input | saved master allocations → after | Δ | allocated bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,603,164 → 3,369,782 | -6.48% | 295,483,525 → 280,242,373 | 87,382,226 → 67,044,446 |
| `patchwork_clustered_20000` | 3,521,797 → 3,288,415 | -6.63% | 309,831,744 → 294,590,592 | 89,862,688 → 69,524,908 |
| `twopi_tree_20000` | 5,639,393 → 5,199,397 | -7.80% | 383,352,972 → 366,215,319 | 117,871,237 → 98,464,870 |

| input | saved master instructions | after | Δ vs master | 1.0 instructions | Δ vs 1.0 |
| :-- | --: | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,046,843,008 | 2,923,802,535 | -4.04% | 3,091,213,389 | -5.42% |
| `patchwork_clustered_20000` | 2,993,812,312 | 2,896,256,756 | -3.26% | 2,990,044,942 | -3.14% |
| `twopi_tree_20000` | 4,457,593,205 | 4,331,103,320 | -2.84% | 4,453,943,115 | -2.76% |

Peak heap falls 16.5–23.3% because the render model no longer retains the measured label
buffers alongside their copies. Instruction changes against the separately built 1.0 artifact
include build/allocator-layout variation; the saved-master column isolates this PR.
All three SVG/stdout and stderr SHA-256 values match before/after.

Wall time is **shared-host noise**, not a speedup claim: medians of three runs, interleaving
1.0, after and stock C 16.1.0 (pango-free plugins) per sample. No wall-time ratios are inferred.

| input | 1.0 ms (noise) | after ms (noise) | C ms (noise) | 1.0 / after / C RSS KiB |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 604 | 516 | 235 | 94,388 / 73,496 / 66,500 |
| `patchwork_clustered_20000` | 581 | 527 | 270 | 100,120 / 76,684 / 69,560 |
| `twopi_tree_20000` | 938 | 917 | 774 | 125,668 / 106,700 / 46,164 |

- `osage_clustered_20000`: `20:01:26 up 4 days, 16:43,  2 users,  load average: 5.95, 4.71, 6.42` → `20:03:03 up 4 days, 16:44,  2 users,  load average: 12.64, 7.66, 7.33`
- `patchwork_clustered_20000`: `20:01:26 up 4 days, 16:43,  2 users,  load average: 5.95, 4.71, 6.42` → `20:03:08 up 4 days, 16:45,  2 users,  load average: 12.58, 7.73, 7.35`
- `twopi_tree_20000`: `20:01:26 up 4 days, 16:43,  2 users,  load average: 5.95, 4.71, 6.42` → `20:03:37 up 4 days, 16:45,  2 users,  load average: 11.08, 7.82, 7.39`

### Full performance ratchet

| case | instructions before → after | Δ | allocations before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,867,984 → 157,226,824 | -1.03% | 227,582 → 221,072 | 5,232,514 → 5,232,514 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,506,073 → 160,876,921 | -1.00% | 210,281 → 205,041 | 5,232,510 → 5,232,510 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,574,014,925 → 1,552,988,613 | -1.34% | 418,625 → 389,523 | 13,948,034 → 13,948,034 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,729,267 → 816,127,523 | -4.07% | 1,024,852 → 914,856 | 29,377,355 → 24,579,126 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,742,330 → 1,540,047,207 | -0.05% | 678,425 → 676,321 | 991,907 → 991,907 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,587,693 → 5,225,598,374 | -0.02% | 321,513 → 318,730 | 1,408,002 → 1,408,002 |
| `tests/fixtures/unix.gv -Tpng` | 147,008,600 → 146,981,757 | -0.02% | 26,276 → 26,063 | 7,460,290 → 7,460,290 |
| `tests/fixtures/unix.gv -Tsvg` | 27,585,866 → 27,281,270 | -1.10% | 14,253 → 13,208 | 569,138 → 569,138 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,966,755 → 2,531,631,279 | -0.05% | 26,433 → 23,507 | 798,646 → 665,952 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,833,294 → 41,024,264 | -1.93% | 42,661 → 40,200 | 983,906 → 818,607 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,209,721 → 30,983,309 | -0.73% | 36,848 → 35,441 | 908,287 → 759,735 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,524,285 → 971,097,308 | -0.56% | 97,178 → 84,772 | 3,066,531 → 2,658,197 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,951,923 → 4,303,682,260 | -0.01% | 22,546 → 21,709 | 909,658 → 909,658 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,044,621 → 2,031,200,444 | -0.09% | 245,032 → 238,718 | 8,293,745 → 8,293,745 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,750,972 → 2,014,226,235 | -0.08% | 250,586 → 245,278 | 8,293,741 → 8,293,741 |

No instruction increase above 2%, no allocation increase and no peak-heap increase above 5%
against the matched before run. The saved-master fdp b69 +612-allocation regression (#600) is pre-existing;
this PR makes no change to its cause. Only improved heap rows are refreshed; unrelated baselines and the CI-derived
instruction baselines are retained until CI provides instruction counts.

## Verification

- Formatting and workspace Clippy with all targets and warnings denied: pass.
- Panic-site ratchet: pass, unchanged.
- `cargo test --workspace`: pass; layout, render and e2e fidelity baselines unchanged.
- All 29 render unit tests pass. New tests compare owned and borrowed plain/HTML label conversions, verify transferred buffers, source Bézier pointer identity/order/flags, and shared colour storage/spelling/debug output.
- Full performance ratchet: pass against the existing committed baseline and no new regression against matched saved master. #600's cause is untouched; its allocation increase is covered by this larger reduction.
- All three 20k-node SVG/stdout and stderr SHA-256 values are unchanged.

Refs #610.

## Changelog entry

- SVG rendering reuses measured label buffers, borrows spline points and shares drawing-state colour names, removing 233k–440k allocations on 20,000-node osage, patchwork and twopi inputs. Peak heap falls 16.5–23.3% and matched musl x86-64-v1 builds execute 2.8–4.0% fewer instructions versus master (2.8–5.4% versus the supplied 1.0 artifact).

The machine-readable measurements are in [render-model-610.json](data/render-model-610.json).
