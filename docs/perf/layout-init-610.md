# Measured shape initialization (#610)

`LGraph` and dot's node initialization now borrow the source node and consume the already
measured polygon vertices. They no longer clone `NodeData` just to override width/height,
reconstruct a polygon that will immediately be discarded, or clone the measured vertices.
The standalone `NodeShape::new` API remains available; record/epsf initialization keeps the
same constructor path with explicit measured dimensions. Shape names are copied once.
`AttrMap`, graph storage and the stable facade stay unchanged. This is independent of #623.

C provenance: `lib/common/utils.c:common_init_node`, `lib/common/shapes.c:poly_init`,
`point_init`, `record_init`, `epsf_init`, and `lib/dotgen/dotinit.c:dot_init_node`.
This applies T9 allocation avoidance to the layout-initialization stage.
Fidelity class **none**. Node/edge/label initialization and all diagnostic order are unchanged.
The floating-point argument for each retained shape field is:

| retained field | same source and operations |
| :-- | :-- |
| width, height | measured width/height multiplied by 72, as after the former temporary-node override |
| outline width/height | measured outline width/height multiplied by 72, as in `apply_geometry` |
| polygon coordinates | the same measured vector that `apply_geometry` cloned, now moved without touching any coordinate |
| orientation, distortion, skew, regularity, sides/peripheries | the same measured polygon fields that replaced reconstructed fields |
| pen width, fixed-shape flag, inside function | the same source attributes and descriptor lookup |
| record/epsf fields | the former constructor and `apply_geometry`, with identical measured dimensions |

Only the discarded polygon reconstruction is omitted; no arithmetic producing a retained
float is reordered, split, fused or changed. No unsafe code or dependency is added.

A differential unit test covers eight shape families and three periphery settings with
nontrivial sizing, orientation, distortion and skew. It compares the former construction
path with the new one and checks that the measured vertex buffer is transferred, not copied.
All workspace fidelity baselines pass unchanged.

Initial allocation evidence: [the stage-ranking report in #623](https://github.com/graphviz-pure/graphviz-pure/blob/e152b1f67ca16120a64bae75c992655df6e43a20/docs/perf/allocation-stages-610.md).
Raw measurements: [layout-init-610.json](data/layout-init-610.json).

## Performance

Musl x86-64-v1 release builds on the three 20,000-node inputs, `-Tsvg`. Before is saved
master `b35ebbe2`; the supplied 1.0 reference is `f89e21d9` (identical Rust sources).
Before/after heap counts use matching argument lengths and identical input paths.

| input | saved master allocations → after | Δ | allocated bytes before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,603,164 → 3,423,164 | -5.00% | 295,483,525 → 281,464,410 | 87,382,226 → 87,382,226 |
| `patchwork_clustered_20000` | 3,521,797 → 3,261,797 | -7.38% | 309,831,744 → 290,612,629 | 89,862,688 → 89,862,688 |
| `twopi_tree_20000` | 5,639,393 → 5,459,393 | -3.19% | 383,352,972 → 369,364,082 | 117,871,237 → 117,871,237 |

| input | saved master instructions | after | Δ vs master | 1.0 instructions | Δ vs 1.0 |
| :-- | --: | --: | --: | --: | --: |
| `osage_clustered_20000` | 3,046,843,008 | 2,965,431,309 | -2.67% | 3,091,213,389 | -4.07% |
| `patchwork_clustered_20000` | 2,993,812,312 | 2,813,244,582 | -6.03% | 2,990,044,942 | -5.91% |
| `twopi_tree_20000` | 4,457,593,205 | 4,379,403,152 | -1.75% | 4,453,943,115 | -1.67% |

Peak heap occurs later in the pipeline on all three inputs and stays unchanged. No peak-RSS
improvement is claimed for this stage. Instruction changes against the separately built 1.0
artifact include build/allocator-layout variation; the saved-master column isolates this PR.
All three SVG/stdout and stderr SHA-256 values match before/after.

Wall time is **shared-host noise**, not a speedup claim: medians of three runs, interleaving
1.0, after and stock C 16.1.0 (pango-free plugins) per sample. No wall-time ratios are inferred.

| input | 1.0 ms (noise) | after ms (noise) | C ms (noise) | 1.0 / after / C RSS KiB |
| :-- | --: | --: | --: | --: |
| `osage_clustered_20000` | 743 | 642 | 264 | 98,296 / 93,992 / 66,736 |
| `patchwork_clustered_20000` | 684 | 601 | 326 | 100,584 / 100,076 / 69,640 |
| `twopi_tree_20000` | 1,341 | 1,148 | 1,018 | 126,528 / 125,628 / 46,292 |

- `osage_clustered_20000`: `19:34:51 up 4 days, 16:16,  2 users,  load average: 10.18, 11.50, 15.54` → `19:36:27 up 4 days, 16:18,  2 users,  load average: 12.07, 12.06, 15.36`
- `patchwork_clustered_20000`: `19:34:51 up 4 days, 16:16,  2 users,  load average: 10.18, 11.50, 15.54` → `19:36:29 up 4 days, 16:18,  2 users,  load average: 11.66, 11.97, 15.31`
- `twopi_tree_20000`: `19:34:51 up 4 days, 16:16,  2 users,  load average: 10.18, 11.50, 15.54` → `19:37:03 up 4 days, 16:18,  2 users,  load average: 8.69, 11.22, 14.96`

### Full performance ratchet

| case | instructions before → after | Δ | allocations before → after | peak heap B before → after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,867,984 → 157,335,777 | -0.96% | 227,582 → 224,830 | 5,232,514 → 5,232,514 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,506,073 → 160,976,130 | -0.94% | 210,281 → 207,529 | 5,232,510 → 5,232,510 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,574,014,925 → 1,566,838,429 | -0.46% | 418,625 → 407,625 | 13,948,034 → 13,948,034 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,729,267 → 830,783,012 | -2.34% | 1,024,852 → 979,852 | 29,377,355 → 29,377,355 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,742,330 → 1,539,613,475 | -0.07% | 678,425 → 677,365 | 991,907 → 991,907 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,587,693 → 5,226,064,904 | -0.01% | 321,513 → 319,464 | 1,408,002 → 1,408,002 |
| `tests/fixtures/unix.gv -Tpng` | 147,008,600 → 146,928,800 | -0.05% | 26,276 → 25,989 | 7,460,290 → 7,460,290 |
| `tests/fixtures/unix.gv -Tsvg` | 27,585,866 → 27,503,281 | -0.30% | 14,253 → 13,966 | 569,138 → 569,138 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,966,755 → 2,532,563,505 | -0.02% | 26,433 → 25,593 | 798,646 → 798,646 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,833,294 → 41,192,454 | -1.53% | 42,661 → 41,641 | 983,906 → 983,906 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,209,721 → 30,554,656 | -2.10% | 36,848 → 35,828 | 908,287 → 908,287 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,524,285 → 974,884,936 | -0.17% | 97,178 → 93,578 | 3,066,531 → 3,066,531 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,951,923 → 4,304,398,943 | +0.01% | 22,546 → 22,196 | 909,658 → 909,658 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,044,621 → 2,031,287,743 | -0.09% | 245,032 → 242,053 | 8,293,745 → 8,293,745 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,750,972 → 2,013,997,313 | -0.09% | 250,586 → 247,607 | 8,293,741 → 8,293,741 |

No instruction increase above 2%, no allocation increase and no peak-heap increase above 5%
against the matched before run. The saved-master fdp b69 +612-allocation regression (#600) is pre-existing;
this PR makes no change to its cause. Only improved heap rows are refreshed; unrelated baselines and the CI-derived
instruction baselines are retained until CI provides instruction counts.
