# Layout result ownership (#610)

Osage, patchwork and twopi borrow the parsed graph while building and running `LGraph`.
`into_layout_result` then moves only owned result fields out, ending the source borrow and
dropping adjacency/construction scratch before the original graph is written back. This
removes the whole-graph snapshots, without changing `AttrMap` or requiring attribute interning.
The borrowed write-back path used by other engines and the detached path share one traversal.

C provenance: `lib/osage/osageinit.c:osage_layout`, `lib/patchwork/patchworkinit.c:patchwork_layout`,
`lib/twopigen/twopiinit.c:twopi_layout`, and `lib/common/postproc.c:gv_postprocess`.
C stores results in its graph records in place; the port transfers its owned working fields.
This applies T9's allocation avoidance at the layout/result boundary.
Fidelity class **none**: all source attributes remain immutable until write-back, every node,
edge and cluster traversal is unchanged, and each retained float is calculated on the same
operands in the same order. Write-back's higher-dimension multiplications and spline/label
flags are unchanged. No float operation, tie-break, warning or new dependency is introduced.

The regression test drops the borrowed source graph before applying a detached result and
compares node/edge fields, dimensions, splines, arrows, labels and bounding boxes against the
existing borrowed write-back path. Full workspace fidelity tests pass unchanged.

See [the allocation-stage ranking](allocation-stages-610.md) and
[raw measurements](data/layout-results-610.json).

## Performance

Musl x86-64-v1 release builds on the three 20,000-node inputs, `-Tsvg`. Before is saved
master `b35ebbe2`; the supplied 1.0 reference is `f89e21d9` (identical Rust sources).
Before/after heap counts use matching argument lengths and identical input paths.
The saved-master column measures this PR’s own effect; “vs 1.0” is a separate comparison.
Each stage was measured alone against master; the gains from #623, #624 and #627 do not add up exactly.


| input | saved master allocations → after | Δ | allocated bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,603,164 → 3,442,669 | -4.45% | 295,483,525 → 274,592,933 | 87,382,226 → 87,382,226 |
| `patchwork_clustered_20000` | 3,521,797 → 3,321,297 | -5.69% | 309,831,744 → 288,767,208 | 89,862,688 → 89,862,688 |
| `twopi_tree_20000` | 5,639,393 → 5,496,283 | -2.54% | 383,352,972 → 358,882,477 | 117,871,237 → 117,871,237 |

| input | saved master instructions | after | Δ vs master | 1.0 instructions | Δ vs 1.0 |
| :-- | --: | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,046,843,008 | 3,022,812,538 | -0.79% | 3,091,213,389 | -2.21% |
| `patchwork_clustered_20000` | 2,993,812,312 | 2,903,779,406 | -3.01% | 2,990,044,942 | -2.89% |
| `twopi_tree_20000` | 4,457,593,205 | 4,385,041,795 | -1.63% | 4,453,943,115 | -1.55% |

Peak heap occurs later in the pipeline on all three inputs and stays unchanged. No peak-RSS
improvement is claimed for this stage. Instruction changes against the separately built 1.0
artifact include build/allocator-layout variation; the saved-master column isolates this PR.
All three SVG/stdout and stderr SHA-256 values match before/after.

Wall time is **shared-host noise**, not a speedup claim: medians of three runs, interleaving
1.0, after and stock C 16.1.0 (pango-free plugins) per sample. No wall-time ratios are inferred.

| input | 1.0 ms (noise) | after ms (noise) | C ms (noise) | 1.0 / after / C RSS KiB |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 1,128 | 1,142 | 539 | 95,160 / 97,552 / 66,696 |
| `patchwork_clustered_20000` | 1,133 | 1,053 | 620 | 97,940 / 100,052 / 69,676 |
| `twopi_tree_20000` | 2,379 | 2,289 | 2,207 | 125,296 / 126,184 / 46,300 |

- `osage_clustered_20000`: `19:13:50 up 4 days, 15:55,  2 users,  load average: 13.77, 15.59, 21.42` → `19:17:05 up 4 days, 15:58,  2 users,  load average: 25.89, 21.16, 22.55`
- `patchwork_clustered_20000`: `19:13:50 up 4 days, 15:55,  2 users,  load average: 13.77, 15.59, 21.42` → `19:17:11 up 4 days, 15:59,  2 users,  load average: 25.82, 21.23, 22.56`
- `twopi_tree_20000`: `19:13:50 up 4 days, 15:55,  2 users,  load average: 13.77, 15.59, 21.42` → `19:18:04 up 4 days, 15:59,  2 users,  load average: 22.34, 21.07, 22.43`

### Full performance ratchet

| case | instructions before → after | Δ | allocations before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,867,984 → 158,867,332 | -0.00% | 227,582 → 227,582 | 5,232,514 → 5,232,514 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,506,073 → 162,510,042 | +0.00% | 210,281 → 210,281 | 5,232,510 → 5,232,510 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,574,014,925 → 1,574,013,944 | -0.00% | 418,625 → 418,625 | 13,948,034 → 13,948,034 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,729,267 → 831,744,088 | -2.23% | 1,024,852 → 989,057 | 29,377,355 → 29,377,355 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,742,330 → 1,540,744,260 | +0.00% | 678,425 → 678,425 | 991,907 → 991,907 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,587,693 → 5,226,627,221 | +0.00% | 321,513 → 321,513 | 1,408,002 → 1,408,002 |
| `tests/fixtures/unix.gv -Tpng` | 147,008,600 → 147,007,441 | -0.00% | 26,276 → 26,276 | 7,460,290 → 7,460,290 |
| `tests/fixtures/unix.gv -Tsvg` | 27,585,866 → 27,586,160 | +0.00% | 14,253 → 14,253 | 569,138 → 569,138 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,966,755 → 2,532,966,871 | +0.00% | 26,433 → 26,433 | 798,646 → 798,646 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,833,294 → 41,261,674 | -1.37% | 42,661 → 41,116 | 983,906 → 983,906 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,209,721 → 30,713,028 | -1.59% | 36,848 → 35,303 | 908,287 → 908,287 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,524,285 → 976,525,530 | +0.00% | 97,178 → 97,178 | 3,066,531 → 3,066,531 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,951,923 → 4,303,917,409 | -0.00% | 22,546 → 21,772 | 909,658 → 836,085 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,044,621 → 2,033,042,768 | -0.00% | 245,032 → 245,032 | 8,293,745 → 8,293,745 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,750,972 → 2,015,750,386 | -0.00% | 250,586 → 250,586 | 8,293,741 → 8,293,741 |

No instruction increase above 2%, no allocation increase and no peak-heap increase above 5%
against the matched before run. The pre-existing fdp b69 +612 allocations (#600) remains
unchanged. Only improved heap rows are refreshed; unrelated baselines and the CI-derived
instruction baselines are retained until CI provides instruction counts.
