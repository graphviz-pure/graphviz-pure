# sfdp: fixed-dimension 2D tree construction (#644)

Technique T8/T9. Measured alone on #614, independently of #645. Gains from separate PRs do not add up exactly. No force-loop or public API changes.

The [integration profile and top-15 self functions](https://github.com/graphviz-pure/graphviz-pure/blob/7c59b1c2/docs/perf/sfdp-v3-query-639.md) attribute 11.91% / 12.35% of sampled user cycles to tree construction on gen_rnd2000 / sparse_5000, including 5.81% / 6.40% self in add_internal and 3.49% / 3.65% in child_for.

## Instructions and heap

Musl ordinary release, no PGO. v3 uses target-cpu=x86-64-v3; v1 has no target-cpu override. Master b35ebbe2 and parent #614 e176604b are separate builds. The 1.0 reference is the shipped f89e21d9 v1 musl artifact. Counts include the whole SVG process.

| CPU | case | master Ir | #614 Ir | after Ir | own Δ vs #614 | 1.0 Ir | ×1.0 instructions |
| :-- | :-- | --: | --: | --: | --: | --: | --: |
| v3 | rnd2000 | 2,934,885,984 | 2,902,844,131 | 2,758,323,019 | -4.98% | 8,213,496,943 | 2.98 |
| v3 | sparse5000 | 8,176,077,256 | 8,080,191,160 | 7,692,383,233 | -4.80% | 23,415,481,420 | 3.04 |
| v3 | grid20x20 | 372,253,745 | 369,486,413 | 350,226,784 | -5.21% | 975,845,464 | 2.79 |
| v1 | rnd2000 | 8,213,497,620 | 8,195,547,274 | 8,046,845,157 | -1.81% | 8,213,496,943 | 1.02 |
| v1 | sparse5000 | 23,415,458,020 | 23,357,675,279 | 22,958,345,694 | -1.71% | 23,415,481,420 | 1.02 |
| v1 | grid20x20 | 975,847,030 | 974,482,973 | 954,690,593 | -2.03% | 975,845,464 | 1.02 |

| case | allocations before → after | peak heap B before → after | bytes allocated before → after |
| :-- | :-- | :-- | :-- |
| rnd2000 | 410,278 → 410,278 | 13,285,549 → 13,285,549 | 42,702,103 → 42,702,103 |
| sparse5000 | 907,380 → 907,380 | 29,659,887 → 29,659,887 | 100,273,322 → 100,273,322 |
| grid20x20 | 97,132 → 97,132 | 3,066,410 → 3,066,410 | 9,567,407 → 9,567,407 |

The full 15-case v1 before/after ratchet passes. grid20x20 uses 974,616,154 → 954,826,998 instructions; allocations (97,132), bytes and peak heap are unchanged. The full ratchet uses a different fixed argv path from the targeted counts above, which accounts for its small process-startup difference. Baseline instructions must come from the CI artifact; no local instruction values are copied into the CI baseline.

## Native counters and wall noise

v3: seven rotating, interleaved rounds pinned to one allowed CPU; one warm-up per variant/input and no cargo command from this task during the pass. v1: five sequential samples per variant. All events have 100% coverage. Native counters and wall medians are informational on this shared machine; load reached 85+. Callgrind is the primary result.

| CPU | case | master cycles | #614 cycles | after cycles |
| :-- | :-- | --: | --: | --: |
| v3 | rnd2000 | 1,100,930,834 | 1,011,595,530 | 987,543,317 |
| v3 | sparse5000 | 3,201,967,511 | 3,100,582,812 | 3,061,057,423 |
| v3 | grid20x20 | 129,085,052 | 121,708,996 | 121,968,950 |
| v1 | rnd2000 | 2,598,947,304 | 3,764,832,498 | 3,554,956,542 |
| v1 | sparse5000 | 7,646,648,568 | 10,818,961,239 | 11,261,147,434 |
| v1 | grid20x20 | 304,871,029 | 467,266,577 | 340,421,592 |

The master cycle column is from the earlier sequential pass, not the paired #614/after pass. Do not infer an incremental gain from that column across different load windows. C and 1.0 wall times are the historical quiet baseline, not contemporaneous runs; every ratio in the following table is noise/context, not a release speed claim.

| case | quiet 1.0 ms | noisy v3 after ms | ×1.0 (noise) | quiet C ms | ×C (noise) |
| :-- | --: | --: | --: | --: | --: |
| rnd2000 | 822.2 | 337.2 | 2.44 | 660.8 | 1.96 |
| sparse5000 | 2354.9 | 998.7 | 2.36 | 1989.2 | 1.99 |
| grid20x20 | 102.9 | 45.6 | 2.26 | 75.2 | 1.65 |

Paired start: `2026-10-29T23:42:35.784598+00:00`; `23:42:35 up 4 days, 20:24,  2 users,  load average: 18.15, 64.07, 81.18`.
Paired end: `2026-10-29T23:43:19.233844+00:00`; `23:43:19 up 4 days, 20:25,  2 users,  load average: 12.87, 56.23, 77.72`.

## Operation/order argument

Public rebuild_from_point_list and add dispatch dimension 2 to private constant-dimension methods. DIM=0 retains all other dimensions. The complete build chain (cell allocation, quadrant selection, child creation, insertion and buffer appends) keeps its original body and ordering. New points are inserted before relocation of the previous point; newest-first leaf lists and quadrant slots are unchanged. All bounds/min/max operations, root widths, count increments, averaging FMA operands/divisors and child-width divisions are the original expressions in their original positions. The maximum-depth branch still increments n before averaging; it is not corrected or simplified.

No graph/point order or RNG calls change. Arena types, capacities, lazy reuse and the public API stay unchanged. No spring_electrical.rs modifications.

## Verification

A cfg(test)-only copy of the pre-specialization builder has only reference_ name prefixes changed; it does not call the new build chain. Tests compare every arena field: cell counts/links/width/weight bits; point ids/links/weight bits; and center, average, force, coordinate and bounds bits. They cover 1D/2D/3D, reused trees across dimension changes, 1/67/250 points, depths 0/1/4/17, duplicate/zero/tiny/large coordinates and weighted insertions, including point relocation and leaf lists.

All measured stdout/stderr hashes match #614, master and 1.0. The v3 workspace suite, new builder differential tests, fmt, workspace/all-target clippy, panic-site gate and full v1 before/after performance ratchet pass. No panic-site increase.

Closes #644. Stacked on #614.
