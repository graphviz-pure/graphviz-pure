# Conservative spline-side rejection (#634)

Skip a barrier-side cubic solve only when a conservative bound proves that the existing
floating-point side-parameter filter cannot accept any root. A strict control-point extent
check is an inexpensive gate for calculating that bound; it never rejects on its own.
Coefficients and x/y root arrays now share scratch across a curve's barrier sides, and bounds
are calculated lazily once per coordinate. All scratch stays on the stack.

C provenance: `lib/pathplan/route.c:splineisinside`, `splineintersectsline`, `points2coeff`,
`horner`, `addroot`; `lib/pathplan/solvers.c:solve3` is unchanged. Techniques T8 bit-identical
kernel restructuring and T9 scratch reuse. The C-mirror functions retain their branch and
loop structure; surviving calls retain every original floating-point expression, root order
and barrier order. No unsafe code, dependency or facade change.

## Why the rejection preserves the bytes

Fidelity class **none**. A real-geometry hull argument alone would be insufficient because
`solve3` has `1e-7` coefficient tolerances and approximate roots. This rejection instead
bounds the **rounded computation that C actually uses**:

1. For each coordinate, compute the same `points2coeff` values used by the original Horner side-parameter check. Start with the highest coefficient as both bounds. At each Horner step, for any floating `t` in `[0,1]`, multiplication maps `[lo, hi]` into `[min(0,lo), max(0,hi)]`. At those endpoints, multiplication by zero or one is exact. Thus the two rounded additions `min(0,lo) + c` and `max(0,hi) + c` enclose C's rounded fused multiply-add. IEEE rounding is monotone; these are bounds on the computed float, not an estimate of the real polynomial. Non-finite coefficients or intermediate bounds disable rejection.
2. Map both bounds through the same `(value - origin) / delta` that C uses for `sv`. Subtraction and division are monotone, with reversed order for a negative delta. Reject only when **both** mapped endpoints are strictly below zero or strictly above one. Rounding onto an endpoint, including underflow to signed zero, prevents rejection. Zero/non-finite deltas and non-finite origins fall through.
3. Every candidate root that C considers is in `[0,1]`, so no candidate can pass that filter after a rejection, regardless of the accuracy of `solve3`. The original helper therefore returns either zero accepted roots or its sentinel 4; `splineisinside` continues in either case. Point barriers, which do not use this filter, always fall through.
4. Every surviving call executes the original coefficient construction, `solve3`, `horner`, `addroot` and intersection checks in their original order. Buffer references replace local arrays without changing stored values or arithmetic. No retained output float is reordered, split or fused differently.

The bounds helpers are Rust-only additions with no C counterparts. The original scalar
route implementation is copied into a test-only reference module; it uses the live `solve3`
because `solvers.rs` is unchanged by this PR. Tests compare
all four output-root bit patterns, the returned root count, and final routing decisions;
reused scratch is deliberately filled with stale NaNs/infinities to detect unwritten reads.
The independent bound test includes strict endpoint-rounding and signed-zero cases.

This is measured alone against master, without the pending wave-1 PRs. The profile's projected
post-wave percentages motivate the task; the tables below report this independent change.

## Performance

Musl release builds on SVG, measured alone against saved master `b35ebbe2`. The supplied 1.0 v1 artifact is `f89e21d9`; its Rust/Cargo sources are identical to this master. The matched v3 master also serves as a 1.0-source v3 rebuild. The separate supplied-artifact column includes build/startup variation; the matched-master column isolates this PR. C is stock 16.1.0 with pango-free plugins and platform math, for context rather than the fidelity oracle. Before/after executable names have equal lengths and input paths match.

### v1 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `gv16_graphs_b29_d1e4db` | 7,807,487,264 → 7,492,507,064 | -4.03% | 7,807,486,537 | -4.03% | 5,380,426,783 |
| `gv16_graphs_b143_5416de` | 2,015,884,062 → 1,952,408,832 | -3.15% | 2,015,886,815 | -3.15% | 1,128,444,308 |
| `gv16_graphs_b106_471493` | 1,763,786,485 → 1,673,127,638 | -5.14% | 1,763,786,745 | -5.14% | 998,946,088 |
| `dot_dag_1000` | 1,575,333,523 → 1,551,093,930 | -1.54% | 1,575,333,310 | -1.54% | 1,144,115,926 |

### v3 instructions

| input | matched master → after | Δ vs master | supplied 1.0 v1 | Δ vs 1.0 v1 | C |
| :-- | --: | --: | --: | --: | --: |
| `gv16_graphs_b29_d1e4db` | 4,123,859,876 → 4,042,899,069 | -1.96% | 7,807,486,537 | -48.22% | 5,380,426,783 |
| `gv16_graphs_b143_5416de` | 1,214,262,315 → 1,199,113,550 | -1.25% | 2,015,886,815 | -40.52% | 1,128,444,308 |
| `gv16_graphs_b106_471493` | 875,736,627 → 853,318,738 | -2.56% | 1,763,786,745 | -51.62% | 998,946,088 |
| `dot_dag_1000` | 1,007,328,084 → 1,002,377,742 | -0.49% | 1,575,333,310 | -36.37% | 1,144,115,926 |

### Heap counts (identical on v1 and v3)

| input | allocations before = after | cumulative bytes before = after | peak heap B before = after |
| :-- | --: | --: | --: |
| `gv16_graphs_b29_d1e4db` | 349,187 | 149,506,939 | 15,660,799 |
| `gv16_graphs_b143_5416de` | 250,586 | 47,523,317 | 8,293,640 |
| `gv16_graphs_b106_471493` | 183,235 | 45,733,825 | 7,779,174 |
| `dot_dag_1000` | 418,625 | 78,051,913 | 13,947,933 |

### Hardware cycles and wall time

`sudo -n perf stat -x, -e cycles:u`, median of three sequential samples per binary/input. Hardware cycles and wall times are shared-host noise and are informational; no wall-time speedup is claimed. Each batch takes its cycle samples before its own Callgrind phase, while other shared-host work may run.

| input | v1 master → after cycles | v3 master → after cycles | supplied 1.0 v1 / C cycles |
| :-- | --: | --: | --: |
| `gv16_graphs_b29_d1e4db` | 4,303,628,449 → 4,075,912,864 | 3,449,969,277 → 3,469,530,432 | 4,236,743,450 / 5,378,223,098 |
| `gv16_graphs_b143_5416de` | 853,779,948 → 894,063,653 | 660,411,062 → 655,497,351 | 862,748,664 / 774,944,970 |
| `gv16_graphs_b106_471493` | 694,906,216 → 677,312,996 | 483,014,689 → 465,432,104 | 691,909,161 / 695,214,370 |
| `dot_dag_1000` | 830,292,765 → 881,380,613 | 714,513,323 → 729,878,080 | 845,622,954 / 988,440,612 |

| input | supplied 1.0 v1 ms (noise) | after v1 ms (noise) | after v3 ms (noise) | C ms (noise) |
| :-- | --: | --: | --: | --: |
| `gv16_graphs_b29_d1e4db` | 1407 | 1363 | 1192 | 1709 |
| `gv16_graphs_b143_5416de` | 332 | 347 | 270 | 282 |
| `gv16_graphs_b106_471493` | 281 | 275 | 209 | 256 |
| `dot_dag_1000` | 333 | 350 | 308 | 344 |

- before `gv16_graphs_b29_d1e4db`: `21:47:51 up 4 days, 18:29,  2 users,  load average: 5.42, 4.20, 4.97` → `21:48:14 up 4 days, 18:30,  2 users,  load average: 5.74, 4.36, 5.01`
- before `gv16_graphs_b143_5416de`: `21:48:14 up 4 days, 18:30,  2 users,  load average: 5.74, 4.36, 5.01` → `21:48:18 up 4 days, 18:30,  2 users,  load average: 5.92, 4.42, 5.02`
- before `gv16_graphs_b106_471493`: `21:48:18 up 4 days, 18:30,  2 users,  load average: 5.92, 4.42, 5.02` → `21:48:22 up 4 days, 18:30,  2 users,  load average: 6.09, 4.48, 5.04`
- before `dot_dag_1000`: `21:48:22 up 4 days, 18:30,  2 users,  load average: 6.09, 4.48, 5.04` → `21:48:27 up 4 days, 18:30,  2 users,  load average: 6.24, 4.53, 5.05`
- after `gv16_graphs_b29_d1e4db`: `21:54:16 up 4 days, 18:36,  2 users,  load average: 3.12, 4.78, 5.11` → `21:54:26 up 4 days, 18:36,  2 users,  load average: 4.43, 5.00, 5.18`
- after `gv16_graphs_b143_5416de`: `21:54:26 up 4 days, 18:36,  2 users,  load average: 4.43, 5.00, 5.18` → `21:54:29 up 4 days, 18:36,  2 users,  load average: 4.43, 5.00, 5.18`
- after `gv16_graphs_b106_471493`: `21:54:29 up 4 days, 18:36,  2 users,  load average: 4.43, 5.00, 5.18` → `21:54:31 up 4 days, 18:36,  2 users,  load average: 5.43, 5.20, 5.24`
- after `dot_dag_1000`: `21:54:31 up 4 days, 18:36,  2 users,  load average: 5.43, 5.20, 5.24` → `21:54:33 up 4 days, 18:36,  2 users,  load average: 5.43, 5.20, 5.24`

### Full v1 performance ratchet

| case | instructions before → after | Δ | allocations before = after | peak heap B before = after |
| :-- | --: | --: | --: | --: |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tcanon -Knop2` | 158,765,528 → 158,765,222 | -0.00% | 227,582 | 5,232,421 |
| `laid/tests/wild/gv16_graphs_b29_d1e4db.gv -Tsvg -Knop2` | 162,399,000 → 162,400,725 | +0.00% | 210,281 | 5,232,417 |
| `synth/dot_dag_1000.gv -Tsvg` | 1,575,332,160 → 1,551,088,760 | -1.54% | 418,625 | 13,947,933 |
| `synth/twopi_tree_5000.gv -Tsvg` | 850,833,840 → 850,855,217 | +0.00% | 1,024,852 | 29,377,254 |
| `tests/circo/wild_gv16_directed_sdh_29b016.gv -Tsvg` | 1,540,665,318 → 1,540,662,633 | -0.00% | 678,425 | 991,806 |
| `tests/fdp/wild_gv16_graphs_b69_4b5b48.gv -Tsvg` | 5,226,739,399 → 5,226,761,487 | +0.00% | 321,513 | 1,407,901 |
| `tests/fixtures/unix.gv -Tpng` | 147,049,395 → 146,768,157 | -0.19% | 26,276 | 7,460,189 |
| `tests/fixtures/unix.gv -Tsvg` | 27,624,531 → 27,342,339 | -1.02% | 14,253 | 569,037 |
| `tests/neato/hw2_random_120.gv -Tsvg` | 2,532,965,540 → 2,532,965,151 | -0.00% | 26,433 | 798,545 |
| `tests/osage/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 41,866,437 → 41,865,127 | -0.00% | 42,661 | 983,805 |
| `tests/patchwork/wild_gv16_graphs_b51_33bcb0.gv -Tsvg` | 31,217,582 → 31,217,886 | +0.00% | 36,848 | 908,186 |
| `tests/sfdp/gen_grid20x20.gv -Tsvg` | 976,538,650 → 976,537,407 | -0.00% | 97,178 | 3,066,430 |
| `tests/twopi/fx_arrows.gv -Tsvg` | 4,303,945,157 → 4,294,303,805 | -0.22% | 22,546 | 909,557 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tplain` | 2,033,175,218 → 1,969,698,595 | -3.12% | 245,032 | 8,293,644 |
| `tests/wild/gv16_graphs_b143_5416de.gv -Tsvg` | 2,015,885,377 → 1,952,406,391 | -3.15% | 250,586 | 8,293,640 |

No instruction increase over 2% and no allocation/peak-heap change against matched master. The only failure against the committed baseline is the pre-existing fdp +612 allocations (#600); #632 fixes that regression, but is intentionally absent from this independent branch. No baseline is weakened. Heap baselines need no update; CI-derived instruction baselines are retained until they can be refreshed from the CI artifact at the coordinator’s request. All four SVG/stdout and stderr hashes match before/after and between v1 and v3.

## Verification

- Workspace formatting and Clippy with all targets/warnings denied: pass.
- Panic-site ratchet: pass, unchanged; no new `unwrap`, `expect` or `unreachable!`.
- `cargo test --workspace`: pass with unchanged fidelity baselines.
- The four new geometry tests also pass with `RUSTFLAGS="-C target-cpu=x86-64-v3"`.
- 80,000 deterministic random segment/line pairs across scales `2^-900` through `2^900`, plus 840 adversarial pairs: tangencies, exact endpoints, repeated roots, degenerate cubics, signed zeros and subnormals. Old/new root counts and every output-root bit match; single-side and multi-side routing decisions match.
- 180,000 sampled Horner evaluations lie within the independently computed bounds. Explicit tests retain cases whose side parameter rounds to zero or one despite geometric separation.
- 260 fixed regressions that the old solver accepts despite all controls lying beyond the segment extent: 1–64 ulps beyond each near endpoint across both coordinate axes and mirrored signs, plus cancelling excursions. A temporary hull-only mutation fails the new test.
- Full performance ratchet meets all three limits against matched master; only the known #600 allocation failure remains against the committed baseline.
- All four target SVG/stdout and stderr SHA-256 values match before/after and between v1 and v3.

The old implementation's point-barrier/triple-root buffer overflow was reproduced separately
and filed as #635. It is not folded into this performance change; point barriers retain the
original path.

Closes #634. Refs #635.

## Changelog entry

- Dot spline routing skips proven barrier-side misses and reuses cubic-solver scratch, reducing end-to-end SVG instructions by 0.5–2.6% on x86-64-v3 builds and 1.5–5.1% on v1 musl builds (b29, b143, b106 and dot_dag_1000 versus master). Allocations and peak heap are unchanged.

Machine-readable measurements: [spline-cubic-634.json](data/spline-cubic-634.json).
