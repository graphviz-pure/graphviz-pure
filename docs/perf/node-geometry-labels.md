# Node geometry: carry label buffers through sizing

Measured alone against master `f7da527e`, independently of #624/#627 and the font-metadata PR #651. Separately measured gains do not add exactly. This is an independent branch from master.

The PR effect compares v3 musl release builds, without PGO. The separate vs 1.0 columns use shipped v1 musl `f89e21d9`; their ratios include the CPU target and build differences. The 1.0 heap counter was built from that exact source commit. All nine stdout/stderr pairs equal master and 1.0.

## Profile and change

Fresh Callgrind profiles of integration `3b5a6e85` (including #624/#627) showed `make_text_label` at 200.2M instructions on osage clustered 20k SVG and 291.0M on patchwork 20k plain. `make_simple_label_cs` accounts for 155.8M/223.0M; the main-label text clone is another 9.2M/14.6M. These are inclusive costs, not additive buckets. The font-metadata costs are handled independently by #651.

- Default `\N` labels reuse the resolved node-name buffer (or make one exact-size copy), after the same agnameof name snapshots.
- Entity-free, raw-byte-free UTF-8/Big5 text moves its substituted string into the label source. `htmlEntityUTF8` entry bookkeeping still runs at the same point. Latin-1, entities and raw bytes keep the old conversion.
- An ordinary single ASCII line goes directly to the unchanged `storeline`; both output vectors reserve one entry. The old scanner remains for escapes, literal newlines and non-ASCII text.
- Node xlabels and edge secondary labels borrow the already-made main label text; record context borrows the raw record string. The public internal helper returning a String keeps its return type.

Every line reaches the same `storeline`, font interning and width/height functions with identical bytes, size and order. Width accumulation, height sums, padding and all polygon expressions stay untouched. Warnings keep the same conversion/font/shape call sites. The differential test contains the old substitution and byte scanner, exercising 3,000 arbitrary-byte labels plus adversarial escapes/entities in UTF-8, Latin-1 and Big5, with ordinary, signed-zero and NaN sizes. It compares every layout float bitwise, as well as text, source and justification.

## Instructions

| input / format | master v3 Ir | after v3 Ir | PR effect | shipped 1.0 v1 Ir | instruction ratio vs 1.0 |
|---|---:|---:|---:|---:|---:|
| osage5000-svg | 717,593,133 | 690,024,744 | -3.84% | 722,687,070 | 1.047× |
| osage5000-plain | 714,082,077 | 680,924,832 | -4.64% | 719,032,834 | 1.056× |
| osage20000-svg | 3,027,082,731 | 2,917,347,796 | -3.63% | 3,047,065,891 | 1.044× |
| osage20000-plain | 3,026,542,790 | 2,932,780,296 | -3.10% | 3,045,891,971 | 1.039× |
| patchwork5000-svg | 666,957,330 | 642,723,330 | -3.63% | 698,792,779 | 1.087× |
| patchwork5000-plain | 649,861,496 | 633,405,273 | -2.53% | 681,983,148 | 1.077× |
| patchwork20000-svg | 2,865,109,473 | 2,773,700,675 | -3.19% | 2,991,789,235 | 1.079× |
| patchwork20000-plain | 2,824,716,042 | 2,668,172,540 | -5.54% | 2,952,081,938 | 1.106× |
| b29-svg | 4,123,701,670 | 4,122,739,431 | -0.02% | 7,807,324,410 | 1.894× |

## Allocations and peak heap

The measured 1.0 allocation/peak figures equal master on all nine cases. Bytes below are dhat values, not RSS.

| input / format | master / 1.0 allocations | after | Δ | master peak B | after peak B | master allocated B | after allocated B |
|---|---:|---:|---:|---:|---:|---:|---:|
| osage5000-svg | 894,992 | 865,012 | -29,980 | 21,787,503 | 20,811,393 | 73,609,290 | 71,509,450 |
| osage5000-plain | 877,396 | 847,416 | -29,980 | 18,584,444 | 17,608,334 | 67,065,834 | 64,965,994 |
| osage20000-svg | 3,603,171 | 3,483,191 | -119,980 | 87,382,234 | 83,491,124 | 295,483,621 | 287,083,781 |
| osage20000-plain | 3,526,921 | 3,406,941 | -119,980 | 74,500,398 | 70,609,288 | 269,020,986 | 260,621,146 |
| patchwork5000-svg | 871,641 | 826,671 | -44,970 | 22,403,017 | 21,426,907 | 77,128,605 | 73,978,845 |
| patchwork5000-plain | 824,343 | 779,373 | -44,970 | 18,925,094 | 17,948,984 | 69,803,654 | 66,653,894 |
| patchwork20000-svg | 3,521,804 | 3,341,834 | -179,970 | 89,862,700 | 85,971,590 | 309,831,872 | 297,232,112 |
| patchwork20000-plain | 3,332,957 | 3,152,987 | -179,970 | 75,862,351 | 71,971,241 | 280,297,568 | 267,697,808 |
| b29-svg | 349,194 | 346,670 | -2,524 | 15,660,792 | 15,660,792 | 149,507,006 | 149,307,651 |

## Informational wall time — noisy

Three-run medians; independent cases ran alongside profiling/builds. These columns are noise/context, not a claimed speedup. C is stock 16.1.0 with pango-free LUT plugins.

| case | 1.0 ms | after ms | noisy ×1.0 | C ms | noisy ×C |
|---|---:|---:|---:|---:|---:|
| osage5000-svg | 141.3 | 152.0 | 0.93× | 57.2 | 0.38× |
| osage5000-plain | 135.1 | 129.1 | 1.05× | 49.1 | 0.38× |
| osage20000-svg | 547.7 | 493.7 | 1.11× | 225.5 | 0.46× |
| osage20000-plain | 582.9 | 519.0 | 1.12× | 194.0 | 0.37× |
| patchwork5000-svg | 131.3 | 143.8 | 0.91× | 67.6 | 0.47× |
| patchwork5000-plain | 132.8 | 134.0 | 0.99× | 49.6 | 0.37× |
| patchwork20000-svg | 552.1 | 513.4 | 1.08× | 270.2 | 0.53× |
| patchwork20000-plain | 553.8 | 539.7 | 1.03× | 197.9 | 0.37× |
| b29-svg | 1318.1 | 1033.3 | 1.28× | 1596.3 | 1.54× |

master start: `00:30:16 up 4 days, 21:12,  2 users,  load average: 9.00, 8.86, 10.52`; end: `00:33:09 up 4 days, 21:15,  2 users,  load average: 8.09, 8.73, 10.20`.

after start: `00:36:46 up 4 days, 21:18,  2 users,  load average: 4.82, 6.69, 9.05`; end: `00:39:41 up 4 days, 21:21,  2 users,  load average: 9.09, 8.08, 9.20`.

1.0 start: `00:18:33 up 4 days, 21:00,  2 users,  load average: 9.18, 6.61, 12.97`; end: `00:21:38 up 4 days, 21:03,  2 users,  load average: 5.76, 6.61, 11.85`.

C start: `00:18:09 up 4 days, 21:00,  2 users,  load average: 8.24, 6.23, 13.02`; end: `00:18:19 up 4 days, 21:00,  2 users,  load average: 8.66, 6.39, 13.00`.
