# Share completed small polygon arrays (#610)

Patchwork forces box shapes. Their ordinary polygon and pen-width outline contain eight points,
previously retained in one heap vector per node in both layout and render. The earlier last-free
traces identify 715 layout-node and 750 render-node group-release triggers on patchwork 20k SVG,
versus only 39 at patchwork driver cleanup and 29 in treemap construction. These triggers identify
the last free before group release, not exclusive ownership of every byte in a mapping. The arrays,
rather than treemap arithmetic, are the target of this change.

Build four- and eight-point arrays on the stack, perform every existing geometry calculation, then
share a completed array only when its length and every x/y `to_bits()` value match a cached array.
This is storage interning after calculation: no rounded keys, label cache, geometry shortcut or
skipped floating-point work. Thirty-two fixed FIFO slots per thread bound retention independently of
graph size. Other lengths retain the existing checked vector allocation and vertex budget. Mutable
access uses `Arc::make_mut`; resizing to another length makes an owned vector, preserving
independent node mutation. The stable facade, dependencies, static musl and its malloc are
unchanged.

C counterparts: `lib/common/shapes.c:poly_init, point_init` and `polygon_t.vertices`. Layout still
transfers the measured vertex storage into `NodeShape`. Arrays on the other-length checked Vec path
retain their allocation identity. Debug formatting retains the original slice representation.
Fidelity class **none**: point order and all floating-point operations remain unchanged; sharing
requires bit equality, including signed zero and NaN payloads.

A persistent inline-array prototype removed the same patchwork mapping churn, but expanded the
per-node vertex holder and measured a dot 1k SVG regression: paired 0.979× [0.962, 0.993], identical
control 1.002×. The shared-array representation keeps the payload outside each node and avoids that
measured slowdown in the final run. A one-entry cache was insufficient for alternating node sizes;
fixed 32-slot storage bounds the cache without a per-graph map. These exploratory measurements
explain the storage tradeoff; all tables below describe only the final four/eight-point candidate.

## Method

Base is the exact approved #783 source `c52db39f`, measured using its saved v3 musl PGO binary. #783
merged as master `3686e399`; `git diff c52db39f 3686e399 -- crates Cargo.toml Cargo.lock` is empty,
so the measured production source matches master. The base build metadata records its original
pre-commit base; the source equivalence and binary SHA were verified when shipping #783. Candidate
is a separately trained fresh `scripts/build-pgo.py` build: Rust 1.99.1, LLVM 23.1.1,
`x86_64-unknown-linux-musl`, `-C target-cpu=x86-64-v3`, same 276-command training list. Candidate
source diff SHA-256: `e4214bd1497a60748fd490fdc1adb7e45993ccdf4b175a9757e1133e1902f820`.

Three warmups per binary/case, then 40 paired rounds with base, candidate, identical base copy and
shipped-shape musl v1 1.0 reference (`f89e21d9`) independently shuffled, seed 6101134. Patchwork
runs first, followed by osage, dot and twopi controls. All heavy work is serial, nice 15, under an 8
GB virtual-memory limit; cargo uses cargo-slot with four jobs. A fresh exec-isolated Python helper
measures child wall time with `perf_counter_ns`, and user/kernel CPU, peak RSS and faults with
`wait4`. It checks child RSS exceeds the helper’s RSS to reject inherited driver floors. Output is
discarded during timing; stdout hashes and full stderr are compared separately. Paired intervals
bootstrap log wall ratios 5,000 times.

- Base/control SHA-256: `45679d07e5cc5d3b63ac9deddac772e83266d8909ba352461e8ce490fee4b2a3`.
- Candidate SHA-256: `448d2df81e709cf187687ce93440dc9834aa99ba91345ecbf7525ab1911e6b3e`.

Load at measurement start/end:

```text
22:58:34 up 1 day, 18:59,  0 users,  load average: 2.44, 1.52, 1.37
23:04:21 up 1 day, 19:05,  0 users,  load average: 1.14, 1.26, 1.29
```

Full samples, hashes, build metadata and deterministic counts:
[data/shared-polygon-vertices-610.json](data/shared-polygon-vertices-610.json).

## Wall time

Medians in milliseconds. Paired × is the geometric mean of within-round base/head ratios; control is
base/identical copy. The 1.0 ratios use median times from the same rounds. C is the stock pango-free
C 16.1.0 reference, five separate runs, without FMA preload.

| Case | Base ms | Head ms | Paired × [95%] | Control × | 1.0 ms | ×1.0 | C ms | ×C |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| patchwork 5k svg | 51.41 | 49.28 | 1.048 [1.036, 1.059] | 0.991 | 131.52 | 2.67 | 65.77 | 1.33 |
| patchwork 20k svg | 187.90 | 171.83 | 1.095 [1.088, 1.103] | 0.997 | 545.48 | 3.17 | 263.55 | 1.53 |
| patchwork 20k plain | 180.44 | 162.71 | 1.105 [1.094, 1.116] | 1.006 | 533.94 | 3.28 | 197.56 | 1.21 |
| osage 5k svg | 42.89 | 42.91 | 1.009 [0.994, 1.028] | 1.006 | 132.52 | 3.09 | 57.06 | 1.33 |
| osage 20k svg | 163.09 | 159.91 | 1.020 [1.010, 1.030] | 1.014 | 552.76 | 3.46 | 221.25 | 1.38 |
| osage 20k plain | 158.84 | 155.24 | 1.018 [1.009, 1.026] | 1.006 | 540.55 | 3.48 | 189.04 | 1.22 |
| dot 1k svg | 88.78 | 87.92 | 1.010 [1.000, 1.020] | 1.000 | 291.34 | 3.31 | 300.20 | 3.41 |
| twopi 20k svg | 268.02 | 262.38 | 1.020 [1.007, 1.034] | 0.999 | 902.22 | 3.44 | 694.57 | 2.65 |

Patchwork’s three headline cases give **1.0826×** paired gain, with identical control 0.9982×.
Applying it to the owner’s 2.63× anchor projects **2.847×**, leaving 5.4% additional speedup to
reach 3×. Osage’s gain is 1.0157×, projecting 3.118× from 3.07×. Fresh local 1.0 median ratios are
patchwork 2.801× → 3.029× and osage 3.291× → 3.337×; these use this measurement window rather than
replacing the owner’s anchors.

Dot and twopi have no measured wall regression. Osage’s small gain is less conclusive: its identical
copy itself measures 1.014× on 20k SVG. Twopi kernel time rises 68.05 → 71.90 ms while total wall
time falls. Every observed control slowdown is retained in the table, including ratios below 1. The
paired intervals and identical copy expose shared-machine noise.

## Peak RSS against base and 1.0

Median MiB. The project’s memory metric compares against 1.0; changes against the #783/master
predecessor are also stated plainly.

| Case | Base MiB | Head MiB | Δ base KiB | Head/base | 1.0 MiB | Head/1.0 |
| :-- | --: | --: | --: | --: | --: | --: |
| patchwork 5k svg | 19.955 | 19.270 | -702 | 0.9656 | 28.477 | 0.6767 |
| patchwork 20k svg | 63.355 | 61.252 | -2154 | 0.9668 | 97.311 | 0.6294 |
| patchwork 20k plain | 51.566 | 48.133 | -3516 | 0.9334 | 83.672 | 0.5753 |
| osage 5k svg | 19.010 | 18.852 | -162 | 0.9917 | 27.557 | 0.6841 |
| osage 20k svg | 59.941 | 58.318 | -1662 | 0.9729 | 93.215 | 0.6256 |
| osage 20k plain | 49.324 | 47.730 | -1632 | 0.9677 | 82.758 | 0.5767 |
| dot 1k svg | 16.340 | 16.082 | -264 | 0.9842 | 18.498 | 0.8694 |
| twopi 20k svg | 85.707 | 83.928 | -1822 | 0.9792 | 121.811 | 0.6890 |

Headline engine geometric means of head/1.0 RSS: patchwork **0.6258×** (base 0.6551×), osage
**0.6273×** (base 0.6418×). These cover the measured engine headline cases, not a new full-61-case
release measurement.

## CPU, faults and syscalls

Median arrows are base → head. Times are milliseconds; faults are minor faults. Every recorded
major-fault sample is zero.

| Case | User ms | Kernel ms | Minor faults |
| :-- | --: | --: | --: |
| patchwork 5k svg | 31.38 → 32.26 | 19.62 → 16.04 | 5401 → 5038 |
| patchwork 20k svg | 127.33 → 128.95 | 59.61 → 43.24 | 13765 → 12516 |
| patchwork 20k plain | 130.41 → 123.06 | 51.91 → 39.76 | 12331 → 10896 |
| osage 5k svg | 26.79 → 27.00 | 16.17 → 15.67 | 4673 → 4575 |
| osage 20k svg | 112.34 → 114.37 | 48.58 → 47.31 | 12620 → 11878 |
| osage 20k plain | 111.98 → 114.30 | 43.88 → 39.78 | 11339 → 10649 |
| dot 1k svg | 65.11 → 66.68 | 23.83 → 21.76 | 5815 → 5704 |
| twopi 20k svg | 198.93 → 193.85 | 68.05 → 71.90 | 18602 → 17819 |

Separate three-repeat `strace -f -c` runs; medians below. These counts are not timing runs.

| Case | mmap base → head | munmap | mremap | brk |
| :-- | --: | --: | --: | --: |
| patchwork 20k svg | 2972 → 1543 | 2968 → 1539 | 27 → 27 | 120 → 113 |
| patchwork 20k plain | 2967 → 1538 | 2963 → 1534 | 22 → 22 | 120 → 113 |
| osage 20k svg | 2034 → 2090 | 2029 → 2086 | 22 → 22 | 107 → 99 |
| osage 20k plain | 2029 → 2085 | 2024 → 2081 | 17 → 17 | 107 → 99 |

Patchwork 20k removes exactly 1,429 mmap and 1,429 munmap calls in both formats. Osage instead adds
56 mmap / 57 munmap calls, with fewer brk calls; the grouping effect of musl is engine-specific. No
general claim that smaller allocation counts always reduce mappings is made.

## Allocations and peak heap

The existing dhat example and generic musl release ratchet measure allocation counts and peak live
heap separately from shipping-malloc RSS. Before/after heap-count executable names and argument
paths have equal lengths.

| Case | Allocations base → head | Allocated B base → head | Peak heap B base → head |
| :-- | --: | --: | --: |
| osage 5k svg | 177,108 → 167,118 | 27,892,388 → 27,253,188 | 14,267,109 → 13,947,909 |
| osage 20k svg | 715,754 → 675,764 | 112,065,173 → 109,505,973 | 57,223,384 → 55,944,184 |
| osage 20k plain | 822,439 → 782,449 | 87,800,562 → 85,241,362 | 44,804,524 → 43,525,324 |
| patchwork 5k svg | 180,094 → 170,106 | 30,177,842 → 28,899,570 | 14,882,271 → 14,243,999 |
| patchwork 20k svg | 728,137 → 688,149 | 121,256,720 → 116,138,448 | 59,703,498 → 57,145,226 |
| patchwork 20k plain | 834,822 → 794,834 | 94,755,083 → 89,636,811 | 46,166,125 → 43,607,853 |

Full 19-case ratchet:

| Case | Instructions base → head | Δ | Allocations base → head | Peak heap B base → head |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 115,259,983 → 115,427,714 | +0.15% | 129,392 → 129,377 | 2,857,357 → 2,858,109 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 99,418,491 → 99,593,881 | +0.18% | 88,828 → 88,813 | 2,099,381 → 2,100,133 |
| `synth/dot_components_1000.gv -Tsvg` | 347,948,159 → 348,785,266 | +0.24% | 184,052 → 178,055 | 9,415,246 → 9,287,486 |
| `synth/dot_dag_1000.gv -Tsvg` | 740,839,808 → 741,678,073 | +0.11% | 189,695 → 186,698 | 12,628,469 → 12,500,901 |
| `synth/dot_par_10_500.gv -Tsvg` | 1,417,562,060 → 1,417,566,969 | +0.00% | 124,344 → 124,282 | 59,808,867 → 59,807,603 |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 782,794,071 → 782,164,113 | -0.08% | 644,924 → 624,927 | 24,191,388 → 23,551,628 |
| `synth/twopi_pairs_1000.gv -Tsvg` | 622,181,744 → 622,655,029 | +0.08% | 107,414 → 103,415 | 14,795,950 → 14,668,030 |
| `synth/twopi_tree_5000.gv -Tsvg` | 411,005,104 → 410,689,277 | -0.08% | 321,541 → 311,542 | 21,212,116 → 20,892,196 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 25,897,137 → 25,940,744 | +0.17% | 16,179 → 16,108 | 839,754 → 835,450 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 2,635,346,966 → 2,635,494,242 | +0.01% | 178,264 → 177,884 | 1,063,653 → 1,042,341 |
| `tests/fixtures/unix.gv -Tpng` | 136,319,256 → 136,302,484 | -0.01% | 19,419 → 19,316 | 6,057,566 → 6,059,166 |
| `tests/fixtures/unix.gv -Tsvg` | 16,667,060 → 16,629,315 | -0.23% | 6,664 → 6,561 | 521,174 → 520,150 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,282,452,994 → 2,282,480,142 | +0.00% | 11,490 → 11,252 | 596,951 → 589,431 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 21,401,317 → 21,427,353 | +0.12% | 13,527 → 13,432 | 749,636 → 745,556 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 14,049,527 → 14,077,940 | +0.20% | 9,429 → 9,334 | 700,576 → 696,496 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 612,398,849 → 612,432,118 | +0.01% | 40,194 → 39,395 | 2,331,140 → 2,305,620 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 549,597,729 → 549,580,481 | -0.00% | 15,853 → 15,784 | 817,702 → 815,542 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 770,318,318 → 770,585,384 | +0.03% | 96,831 → 96,823 | 8,277,281 → 8,279,153 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 777,452,405 → 777,722,430 | +0.03% | 104,201 → 104,193 | 8,277,277 → 8,279,149 |

`ratchet-baseline.json` is unchanged; the coordinator refreshes gains in one batch.

## Verification

New tests cover exact sharing, signed zero and NaN-payload distinctions, mutation isolation,
distinct four/eight length keys, bounded FIFO eviction with live references, resizing and the
checked heap fallback. The existing measured-shape reconstruction and vertex-allocation transfer
test is unchanged in meaning and applies to both owned and shared arrays. All 48 geometry tests,
workspace Clippy, formatting and panic sites pass; panic-site counts do not rise. The full workspace
passes, including parser tests, layout/render/structural fidelity and 2,595/2,595 stable C e2e cases
(14 existing unstable-C exclusions unchanged). All 19 performance-ratchet cases pass. Against
matched #783 counts, every case allocates fewer blocks; the largest instruction increase is 0.241%,
and the largest peak-heap increase is 0.036%.

PGO corpus comparison: 2,670 inputs identical, 0 different (SVG bytes, diagnostics and exit status).
Correctness-sweep unpaired totals are 36.3 s base / 36.8 s head; they are diagnostic timings, not a
paired speed claim.

Refs #610; based on #783.
