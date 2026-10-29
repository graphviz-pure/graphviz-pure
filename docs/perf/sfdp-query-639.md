# sfdp: fixed-dimension 2D supernode query (#639)

Technique T8. This PR is measured alone on #614, not on the full pending-head integration. Gains from separate PRs do not add up exactly. The integration profile, top 15 self functions and phase split are in [sfdp-v3-query-639.md](sfdp-v3-query-639.md).

## Instructions and heap

Static musl, ordinary release profile, no PGO. v3 is `-C target-cpu=x86-64-v3`; v1 has no target-cpu override. The 1.0 column is the shipped f89e21d9 v1 musl binary. Master is b35ebbe2; parent #614 is e176604b. Each number is a full SVG process under Callgrind, including parsing and rendering.

| CPU | case | master Ir | #614 Ir | after Ir | own Δ vs #614 | 1.0 Ir | ×1.0 instructions |
| :-- | :-- | --: | --: | --: | --: | --: | --: |
| v3 | rnd2000 | 2,934,885,984 | 2,902,844,131 | 2,462,695,090 | -15.16% | 8,213,496,943 | 3.34 |
| v3 | sparse5000 | 8,176,077,256 | 8,080,191,160 | 6,806,969,010 | -15.76% | 23,415,481,420 | 3.44 |
| v3 | grid20x20 | 372,253,745 | 369,486,413 | 325,330,006 | -11.95% | 975,845,464 | 3.00 |
| v1 | rnd2000 | 8,213,497,620 | 8,195,547,274 | 7,669,786,835 | -6.42% | 8,213,496,943 | 1.07 |
| v1 | sparse5000 | 23,415,458,020 | 23,357,675,279 | 21,837,794,307 | -6.51% | 23,415,481,420 | 1.07 |
| v1 | grid20x20 | 975,847,030 | 974,482,973 | 921,821,128 | -5.40% | 975,845,464 | 1.06 |

| case | allocations before → after | peak heap B before → after | bytes allocated before → after |
| :-- | :-- | :-- | :-- |
| rnd2000 | 410,278 → 410,278 | 13,285,549 → 13,285,549 | 42,702,103 → 42,702,103 |
| sparse5000 | 907,380 → 907,380 | 29,659,887 → 29,659,887 | 100,273,322 → 100,273,322 |
| grid20x20 | 97,132 → 97,132 | 3,066,410 → 3,066,410 | 9,567,407 → 9,567,407 |

The full 15-case v1 ratchet comparison against #614 passes: no allocation increases, no peak increases and no instruction regressions above 2%. grid20x20: 974,616,154 → 921,957,811 instructions, 97,132 allocations unchanged. The full ratchet uses a separate fixed argv path, so its small process-startup count differs from the target measurements above. Instruction baselines must be refreshed from CI rather than from local Valgrind counts.

## Native counters and wall observations

All hardware events had 100% coverage. v3 uses seven rotating, interleaved rounds pinned to one allowed CPU, with one warm-up per variant/input and no cargo build from this task during that pass. v1 records five sequential samples per variant; its time windows are noisier and are not used for speed claims. The machine was shared. C and 1.0 wall columns below are the quiet historical baseline, not contemporaneous runs. Ratios in this wall table are observations from different windows, not a claimed release speedup.

| CPU | case | master cycles | #614 cycles | after cycles |
| :-- | :-- | --: | --: | --: |
| v3 | rnd2000 | 1,636,246,325 | 1,452,833,586 | 1,355,249,927 |
| v3 | sparse5000 | 4,812,614,542 | 4,353,548,423 | 3,963,607,709 |
| v3 | grid20x20 | 200,196,621 | 176,699,167 | 167,923,445 |
| v1 | rnd2000 | 2,598,947,304 | 3,764,832,498 | 3,603,404,746 |
| v1 | sparse5000 | 7,646,648,568 | 10,818,961,239 | 10,583,064,390 |
| v1 | grid20x20 | 304,871,029 | 467,266,577 | 430,189,020 |

| case | quiet 1.0 ms | noisy v3 after ms | ×1.0 (noise) | quiet C ms | ×C (noise) |
| :-- | --: | --: | --: | --: | --: |
| rnd2000 | 822.2 | 562.7 | 1.46 | 660.8 | 1.17 |
| sparse5000 | 2354.9 | 1888.7 | 1.25 | 1989.2 | 1.05 |
| grid20x20 | 102.9 | 95.5 | 1.08 | 75.2 | 0.79 |

Paired v3 start: `2026-10-29T23:18:26.586972+00:00`; `23:18:26 up 4 days, 20:00,  2 users,  load average: 107.61, 141.03, 100.22`.
Paired v3 end: `2026-10-29T23:21:16.261280+00:00`; `23:21:16 up 4 days, 20:03,  2 users,  load average: 171.91, 151.20, 110.55`.

## Fidelity and verification

Only the private query receives a constant dimension (2), selected by the unchanged public get_supernodes wrapper. All other dimensions use the original runtime-dimensional walk. Point insertion, children, leaf lists, empty-child counts and output-buffer order are unchanged. point_distance still executes the same subtraction, serial FMA accumulation and sqrt on the same coordinates. No force-loop changes or signature changes; the parallel force-loop owner can call the same API.

The independent old collector compares count bits, center bits, weight bits and distance bits on weighted/duplicate points, depths 0/1/3/10, dimensions 1/2/3 and far/near/infinite Barnes–Hut parameters. Existing tests cover buffer-capacity reuse. Every measured stdout/stderr hash matches master and the 1.0 reference. cargo test --workspace (v3), the new unit tests (v1 and v3), clippy --workspace --all-targets -- -D warnings, fmt, panic-sites and the before/after full ratchet pass. No panic-site increase.

Closes #639. Stacked on #614.
