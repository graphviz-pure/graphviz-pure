# Single-attribute node statements without a temporary map (#610)

The parser holds the first statement attribute inline and transfers its owned key/value buffers to a
single node. It constructs an `AttrMap` when a second distinct key appears or when applying
attributes to an edge or node list. This removes the temporary statement map and final string copies
on the common one-node, one-attribute path. Empty statements, defaults, wider lists and their growth
policy keep their existing behavior. No parser scratch buffer is retained.

C counterparts: `lib/cgraph/grammar.y:optattr, appendattr, endnode` and `lib/cgraph/attr.c:agxset`.
Attribute declaration order still comes from `attr_order`; `endnode` binds that order before the
final insertion. Duplicate keys retain last-value-wins behavior and HTML flags, and map iteration
remains lexical. Concatenated values are trimmed before retention to match the old clone’s compact
capacity. Fidelity class **none**: no floating-point operations or source order change. Safe Rust,
static musl and its malloc, dependencies and the stable facade remain unchanged.

## Method

Measurements are the primary experiment of
[#782](https://github.com/graphviz-pure/graphviz-pure/pull/782), now promoted to production under
the owner’s 2026-11-04 decision. The product sources on master `8ed49c33` are identical to the
measured baseline `78a0cece`; only the #780 documentation merge lies between them. The applied Rust
source diff is SHA-256 `ffc264257dcd100852ebc07f41b66cff47e0a1ff76ed10a151bd57ddd2ab1268`, exactly
the measured candidate. The existing 40-round measurement is reused without selecting a new timing
window.

Baseline and head were independently built and trained by `scripts/build-pgo.py`: Rust 1.99.1, LLVM
23.1.1, `x86_64-unknown-linux-musl`, `-C target-cpu=x86-64-v3`, the same 276-command training list
and separate fresh profiles. Training metadata records the base commit because the candidate was an
uncommitted patch; the source-diff hash identifies the applied source. Binary hashes differ; control
is an identical baseline copy. All heavy work is serial at nice 15, with an 8 GB virtual-memory
limit; cargo uses cargo-slot with four jobs.

Three warmups per binary/case precede 40 rounds. The baseline, candidate, identical control and
shipped-shape v1 musl 1.0 reference (`f89e21d9`) are shuffled in every round with seed 6101134. A
fresh exec-isolated Python helper measures wall time with `perf_counter_ns`, and child user/kernel
time, minor/major faults and peak RSS with `wait4`. It checks that the child’s RSS exceeds the
helper’s current RSS so inherited driver high-water floors cannot mask small cases. Output is
discarded during timing; complete stdout and stderr equality is checked separately. Intervals
bootstrap paired log wall ratios 5,000 times. C is the stock pango-free C 16.1.0 reference,
separately measured five times without FMA preload. Full samples, hashes and build metadata are in
[data/parser-statement-attrs-610.json](data/parser-statement-attrs-610.json).

- Baseline/control SHA-256: `16478d5f1d96134b6c5ab9f8cd7ab8631a35db3ccfb36456c253312900d20a18`.
- Candidate SHA-256: `45679d07e5cc5d3b63ac9deddac772e83266d8909ba352461e8ce490fee4b2a3`.

Load at measurement start/end:

```text
20:45:40 up 1 day, 16:46,  0 users,  load average: 2.13, 1.58, 1.23
20:51:35 up 1 day, 16:52,  0 users,  load average: 1.37, 1.40, 1.27
```

## Wall time

Median milliseconds. Paired × is the geometric mean of within-round master/head ratios; the
identical control is master/copy. The 1.0 and C ratios use their median times.

| Case | Master ms | Head ms | Paired × [95%] | Control × | 1.0 ms | ×1.0 | C ms | ×C |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| osage 5k svg | 46.08 | 43.10 | 1.075 [1.056, 1.095] | 1.001 | 135.58 | 3.15 | 56.35 | 1.31 |
| osage 20k svg | 176.05 | 162.51 | 1.086 [1.075, 1.096] | 1.000 | 542.90 | 3.34 | 219.95 | 1.35 |
| osage 20k plain | 171.33 | 158.34 | 1.084 [1.071, 1.098] | 0.997 | 540.17 | 3.41 | 185.47 | 1.17 |
| patchwork 5k svg | 54.93 | 51.53 | 1.061 [1.048, 1.075] | 0.999 | 132.31 | 2.57 | 65.43 | 1.27 |
| patchwork 20k svg | 200.34 | 187.85 | 1.066 [1.058, 1.075] | 0.998 | 544.21 | 2.90 | 262.31 | 1.40 |
| patchwork 20k plain | 192.06 | 179.02 | 1.070 [1.060, 1.081] | 0.996 | 533.50 | 2.98 | 198.89 | 1.11 |
| dot 1k svg | 88.95 | 89.16 | 1.002 [0.990, 1.015] | 0.990 | 295.42 | 3.31 | 302.47 | 3.39 |
| twopi 20k svg | 272.54 | 272.24 | 1.003 [0.993, 1.014] | 1.002 | 903.08 | 3.32 | 668.25 | 2.45 |

The three headline cases give a paired 1.0817× gain for osage and 1.0657× for patchwork. Applying
these isolated gains to the owner’s 2.84× / 2.47× anchors projects **3.072× osage / 2.632×
patchwork**: osage clears the 3× floor, while patchwork remains 0.368× short. Fresh local
median-based ratios against 1.0 are 3.058× → 3.297× for osage and 2.629× → 2.809× for patchwork;
those use this timing window rather than replacing the owner’s anchors.

Dot’s median wall time increases 0.24%; its paired interval includes 1, as does twopi’s. No slowdown
or gain is established on those controls. Every observed median change is shown above.

## Peak RSS: master and 1.0

Median peak RSS in MiB. Positive master deltas are explicitly accepted by the owner; the project’s
[memory metric](1.x-10x.md#the-metric) compares RSS with 1.0. The patchwork 5k rise is real: +246
KiB in the median, and paired mean +246.0 KiB against the identical copy (95% interval +33.5 to
+451.1). Larger cases show additional RSS variation exposed by the control samples. No new allocator
or shipping-platform change is involved.

| Case | Master MiB | Head MiB | Δ master KiB | Head/master | 1.0 MiB | Head/1.0 |
| :-- | --: | --: | --: | --: | --: | --: |
| osage 5k svg | 18.980 | 18.980 | +0 | 1.0000 | 27.586 | 0.6880 |
| osage 20k svg | 59.744 | 59.863 | +122 | 1.0020 | 93.316 | 0.6415 |
| osage 20k plain | 49.320 | 49.324 | +4 | 1.0001 | 82.758 | 0.5960 |
| patchwork 5k svg | 19.674 | 19.914 | +246 | 1.0122 | 28.463 | 0.6997 |
| patchwork 20k svg | 63.199 | 63.412 | +218 | 1.0034 | 97.350 | 0.6514 |
| patchwork 20k plain | 50.834 | 51.016 | +186 | 1.0036 | 83.672 | 0.6097 |
| dot 1k svg | 16.344 | 16.344 | +0 | 1.0000 | 18.604 | 0.8785 |
| twopi 20k svg | 85.617 | 85.309 | -316 | 0.9964 | 121.984 | 0.6993 |

Across the three headline cases, engine geometric means of head/1.0 RSS are **0.6408× 1.0 for
osage** and **0.6526× for patchwork**, versus master’s 0.6403× / 0.6484×. Relative to master the
increases are 0.069% and 0.638%, respectively. Every measured case remains below 1.0 RSS; the
target-case maximum is 0.700×.

## CPU time and faults

Medians; arrows are master → head. Times are milliseconds, faults are minor faults. Major faults are
zero in all recorded samples. Independent medians need not sum to median wall time.

| Case | User ms | Kernel ms | Minor faults |
| :-- | --: | --: | --: |
| osage 5k svg | 28.34 → 27.29 | 16.52 → 15.66 | 4824 → 4673 |
| osage 20k svg | 119.86 → 113.65 | 55.71 → 47.74 | 13192 → 12602 |
| osage 20k plain | 120.15 → 111.37 | 50.09 → 49.33 | 11962 → 11339 |
| patchwork 5k svg | 36.04 → 31.01 | 19.47 → 20.07 | 5547 → 5401 |
| patchwork 20k svg | 134.74 → 130.96 | 64.04 → 56.07 | 14310 → 13673 |
| patchwork 20k plain | 133.31 → 126.30 | 59.95 → 53.89 | 12950 → 12333 |
| dot 1k svg | 68.21 → 65.12 | 20.27 → 23.87 | 5828 → 5815 |
| twopi 20k svg | 205.58 → 205.24 | 66.42 → 63.60 | 18544 → 18608 |

Separate three-repeat syscall traces show exactly **621 fewer mmap and 621 fewer munmap calls in
each 20k case**: osage SVG munmap 2,650 → 2,029, osage plain 2,645 → 2,024, patchwork SVG 3,589 →
2,968, patchwork plain 3,584 → 2,963. `mremap` and `brk` counts are unchanged. This follows the
traced temporary map group releases rather than relying on instruction cuts alone.

## Allocations and peak heap

Generic musl release builds under the existing dhat example; these deterministic heap measurements
are separate from shipping-malloc RSS. Before/after argument paths and executable names have equal
lengths.

| Case | Master allocations → head | Allocated bytes master → head | Peak heap B master → head |
| :-- | --: | --: | --: |
| osage 5k svg | 192,108 → 177,108 | 28,899,391 → 27,892,386 | 14,267,107 → 14,267,107 |
| osage 20k svg | 775,754 → 715,754 | 116,095,396 → 112,065,171 | 57,223,382 → 57,223,382 |
| osage 20k plain | 882,439 → 822,439 | 91,830,785 → 87,800,560 | 44,804,522 → 44,804,522 |
| patchwork 5k svg | 195,094 → 180,094 | 31,184,845 → 30,177,840 | 14,882,269 → 14,882,269 |
| patchwork 20k svg | 788,137 → 728,137 | 125,286,943 → 121,256,718 | 59,703,496 → 59,703,496 |
| patchwork 20k plain | 894,822 → 834,822 | 98,785,306 → 94,755,081 | 46,166,123 → 46,166,123 |

Full 19-case ratchet, fresh master → head:

| Case | Instructions master → head | Δ | Allocations master → head | Peak heap B master → head |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 114,966,131 → 115,259,983 | +0.26% | 129,392 → 129,392 | 2,857,357 → 2,857,357 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 99,130,835 → 99,418,491 | +0.29% | 88,828 → 88,828 | 2,099,381 → 2,099,381 |
| `synth/dot_components_1000.gv -Tsvg` | 347,841,628 → 347,948,159 | +0.03% | 184,052 → 184,052 | 9,415,246 → 9,415,246 |
| `synth/dot_dag_1000.gv -Tsvg` | 740,869,327 → 740,839,808 | -0.00% | 189,981 → 189,695 | 12,628,469 → 12,628,469 |
| `synth/dot_par_10_500.gv -Tsvg` | 1,417,559,644 → 1,417,562,060 | +0.00% | 124,344 → 124,344 | 59,808,867 → 59,808,867 |
| `synth/nop2_subgraph_10000.gv -Tcanon -Knop2` | 795,506,902 → 782,794,071 | -1.60% | 674,924 → 644,924 | 24,191,388 → 24,191,388 |
| `synth/twopi_pairs_1000.gv -Tsvg` | 623,171,072 → 622,181,744 | -0.16% | 107,414 → 107,414 | 14,795,950 → 14,795,950 |
| `synth/twopi_tree_5000.gv -Tsvg` | 410,550,394 → 411,005,104 | +0.11% | 321,541 → 321,541 | 21,212,116 → 21,212,116 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 25,881,402 → 25,897,137 | +0.06% | 16,179 → 16,179 | 839,754 → 839,754 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 2,635,271,658 → 2,635,346,966 | +0.00% | 178,264 → 178,264 | 1,063,653 → 1,063,653 |
| `tests/fixtures/unix.gv -Tpng` | 136,315,113 → 136,319,256 | +0.00% | 19,419 → 19,419 | 6,057,566 → 6,057,566 |
| `tests/fixtures/unix.gv -Tsvg` | 16,663,266 → 16,667,060 | +0.02% | 6,664 → 6,664 | 521,174 → 521,174 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,282,440,314 → 2,282,452,994 | +0.00% | 11,490 → 11,490 | 596,951 → 596,951 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 21,385,744 → 21,401,317 | +0.07% | 13,527 → 13,527 | 749,636 → 749,636 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 14,035,434 → 14,049,527 | +0.10% | 9,429 → 9,429 | 700,576 → 700,576 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 612,342,168 → 612,398,849 | +0.01% | 40,194 → 40,194 | 2,331,140 → 2,331,140 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 549,593,585 → 549,597,729 | +0.00% | 15,853 → 15,853 | 817,702 → 817,702 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 770,082,500 → 770,318,318 | +0.03% | 96,831 → 96,831 | 8,277,281 → 8,277,281 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 777,222,536 → 777,452,405 | +0.03% | 104,201 → 104,201 | 8,277,277 → 8,277,277 |

`ratchet-baseline.json` is unchanged. The coordinator will refresh the batch after merge.

## Verification

Parser tests cover defaults, HTML flags, duplicate keys across repeated brackets, multiple nodes,
edge attributes, wider maps, subgraphs, empty lists and compact retained concatenations. Full
workspace tests include layout, render, structure and the 2,595-case C e2e ratchet. All 2,595 stable
C cases match; the existing 14 unstable-C exclusions are unchanged. The shipping v3 musl PGO
binaries are additionally compared on the entire `.gv` corpus, including invalid inputs’ stderr and
exit status.

PGO master/head corpus comparison: 2,670 inputs identical, 0 different (SVG, diagnostics and exit
status). This correctness-only sweep recorded unpaired totals of 36.3 s master / 37.0 s head; those
totals are retained as diagnostics and are not used to claim a speedup.

Local gates pass: formatting; workspace/all-target Clippy with warnings denied; 44 parser unit and
16 syntax-error tests; panic sites; full workspace tests; the 19-case performance ratchet; and the
PGO CLI corpus comparison. Every cargo invocation used cargo-slot. No golden, panic-site, fidelity
or performance baseline is edited.

Refs #610.
