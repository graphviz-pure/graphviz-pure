# Node geometry: font metadata and one-span records

Measured alone against master `f7da527e`, independently of pending #624/#627. The gains of separately measured PRs do not add exactly. The PR branches from master; no layout-init stack is required.

The shipped default will be x86-64-v3 (#605). The **PR effect** below compares two v3 musl release builds. The **vs 1.0** columns separately compare the shipped v1 musl reference `f89e21d9`; they include CPU-target and release-build differences and must not be attributed to this change. No PGO. Callgrind and dhat count the entire process. The 1.0 heap counter was built from `f89e21d9`; every reported stdout/stderr hash matches master and 1.0.

## Profile before choosing the change

Fresh Callgrind profiles use integration head `3b5a6e85`, with #624/#627 included, on an unstripped musl v3 release binary with frame pointers and line tables, no PGO.

| subtree | osage SVG Ir | patchwork plain Ir |
|---|---:|---:|
| process | 1,607,351,150 | 2,160,369,169 |
| node_geometry inclusive | 431,603,720 | 766,675,545 |
| make_text_label | 200,207,588 | 291,009,381 |
| make_simple_label_cs | 155,756,082 | 222,950,327 |
| textfont_intern | 62,175,523 | 79,012,815 |
| estimate_text_width_1pt (metrics lookup included) | 29,235,600 | 43,853,400 |
| node_geometry_measured self (includes inlined polygon/attrs) | 57,160,000 | 126,300,000 |
| strtod scan + to_f64 | 15,000,898 | 22,501,347 |
| label_l_text | 9,206,643 | 14,624,939 |

These are inclusive instruction counts, not a partition; do not add rows. Calls to make_simple_label_cs/font interning: 40,000 on osage, 60,000 on patchwork. Font interning spends 16.8M/23.9M instructions in malloc and 14.5M/20.8M in free. The old key path allocates strings before the replay return, and normal repeated keys create three name copies. Recording and replay clone a separate size vector even for one span. No AttrMap redesign is needed to remove these allocations. Metrics lookup is pure only for successful families; unknown-font warning paths must stay live.

The remaining label conversion/substitution path creates and discards buffers for ordinary single-line labels. It is a separate candidate after the font metadata change; polygon trigonometry is more prominent on patchwork, but a plain algebraic box shortcut would change rounding and is prohibited.

## What changes

Font keys and log keys share `Rc<str>` names/colors in private thread-local state. A one-entry cache owns the last name/color pair once, even when size or flags change. Replay returns before building any key. Each ordinary first measurement still executes the same `Cdt::insert` with the same comparator, flags, size bits and search history; it does not cache a dictionary answer. Cdt itself is unchanged.

Per-object recording uses Empty/One/Many storage; a single font size is inline, including replay. The Many transition reserves four entries, matching the old Vec's initial growth. Every recorded result and replay index stays in order.

A one-entry successful font-family LUT lookup cache avoids re-scanning aliases. Its key is the exact font-name bytes; size-dependent arithmetic and character-width summation still run. Unknown fonts keep the original lookup/warning path on every call, including quiet repeats. No shape or attribute warnings move.

No sizing float expression changes. Private TLS sharing changes neither facade auto traits nor Send/Sync requirements. The tests compare 60,000 dictionary requests bitwise against the old insertion path, including NaN payloads, infinities and signed zero, and cover nested recording/replay and all LUT aliases.

## Instructions

| input / format | master v3 Ir | after v3 Ir | PR effect | shipped 1.0 v1 Ir | instruction ratio vs 1.0 |
|---|---:|---:|---:|---:|---:|
| osage5000-svg | 717,593,133 | 702,114,700 | -2.16% | 722,687,070 | 1.029× |
| osage5000-plain | 714,082,077 | 698,745,054 | -2.15% | 719,032,834 | 1.029× |
| osage20000-svg | 3,027,082,731 | 2,967,381,771 | -1.97% | 3,047,065,891 | 1.027× |
| osage20000-plain | 3,026,542,790 | 2,967,188,746 | -1.96% | 3,045,891,971 | 1.027× |
| patchwork5000-svg | 666,957,330 | 642,096,949 | -3.73% | 698,792,779 | 1.088× |
| patchwork5000-plain | 649,861,496 | 625,188,412 | -3.80% | 681,983,148 | 1.091× |
| patchwork20000-svg | 2,865,109,473 | 2,769,767,539 | -3.33% | 2,991,789,235 | 1.080× |
| patchwork20000-plain | 2,824,716,042 | 2,710,958,873 | -4.03% | 2,952,081,938 | 1.089× |
| b29-svg | 4,123,701,670 | 4,121,914,051 | -0.04% | 7,807,324,410 | 1.894× |

## Allocations and peak heap

The measured 1.0 allocation/peak figures equal master on all nine cases. Bytes and peaks below are dhat values, not RSS.

| input / format | master / 1.0 allocations | after | Δ | master peak B | after peak B | master allocated B | after allocated B |
|---|---:|---:|---:|---:|---:|---:|---:|
| osage5000-svg | 894,992 | 859,994 | -34,998 | 21,787,503 | 21,627,396 | 73,609,290 | 73,134,205 |
| osage5000-plain | 877,396 | 842,398 | -34,998 | 18,584,444 | 18,424,337 | 67,065,834 | 66,590,749 |
| osage20000-svg | 3,603,171 | 3,463,173 | -139,998 | 87,382,234 | 86,742,127 | 295,483,621 | 293,583,536 |
| osage20000-plain | 3,526,921 | 3,386,923 | -139,998 | 74,500,398 | 73,860,291 | 269,020,986 | 267,120,901 |
| patchwork5000-svg | 871,641 | 821,643 | -49,998 | 22,403,017 | 22,242,910 | 77,128,605 | 76,503,520 |
| patchwork5000-plain | 824,343 | 774,345 | -49,998 | 18,925,094 | 18,764,987 | 69,803,654 | 69,178,569 |
| patchwork20000-svg | 3,521,804 | 3,321,806 | -199,998 | 89,862,700 | 89,222,593 | 309,831,872 | 307,331,787 |
| patchwork20000-plain | 3,332,957 | 3,132,959 | -199,998 | 75,862,351 | 75,222,244 | 280,297,568 | 277,797,483 |
| b29-svg | 349,194 | 345,151 | -4,043 | 15,660,792 | 15,648,290 | 149,507,006 | 149,468,422 |

## Informational wall time — noisy

Three-run medians; independent cases ran concurrently with profiling and builds. These columns are noise/context, not a claimed speedup. C is stock 16.1.0 with the pango-free LUT plugins.

| case | 1.0 ms | after ms | noisy ×1.0 | C ms | noisy ×C |
|---|---:|---:|---:|---:|---:|
| osage5000-svg | 141.3 | 140.0 | 1.01× | 57.2 | 0.41× |
| osage5000-plain | 135.1 | 137.1 | 0.99× | 49.1 | 0.36× |
| osage20000-svg | 547.7 | 540.9 | 1.01× | 225.5 | 0.42× |
| osage20000-plain | 582.9 | 541.1 | 1.08× | 194.0 | 0.36× |
| patchwork5000-svg | 131.3 | 135.6 | 0.97× | 67.6 | 0.50× |
| patchwork5000-plain | 132.8 | 124.9 | 1.06× | 49.6 | 0.40× |
| patchwork20000-svg | 552.1 | 531.0 | 1.04× | 270.2 | 0.51× |
| patchwork20000-plain | 553.8 | 538.2 | 1.03× | 197.9 | 0.37× |
| b29-svg | 1318.1 | 1028.3 | 1.28× | 1596.3 | 1.55× |

master start: `00:30:16 up 4 days, 21:12,  2 users,  load average: 9.00, 8.86, 10.52`; end: `00:33:09 up 4 days, 21:15,  2 users,  load average: 8.09, 8.73, 10.20`.

after start: `00:15:42 up 4 days, 20:57,  2 users,  load average: 5.66, 4.87, 13.86`; end: `00:18:33 up 4 days, 21:00,  2 users,  load average: 9.18, 6.61, 12.97`.

1.0 start: `00:18:33 up 4 days, 21:00,  2 users,  load average: 9.18, 6.61, 12.97`; end: `00:21:38 up 4 days, 21:03,  2 users,  load average: 5.76, 6.61, 11.85`.

C start: `00:18:09 up 4 days, 21:00,  2 users,  load average: 8.24, 6.23, 13.02`; end: `00:18:19 up 4 days, 21:00,  2 users,  load average: 8.66, 6.39, 13.00`.
