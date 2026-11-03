# `limit_boxes`: the retry takes only the new samples (#733)

When `routesplines_` cannot bound every box with the first `limitBoxes` pass, C doubles `delta`
and runs the whole pass again on the same boxes (up to 15 times). The port now takes only the
odd samples on each of these retries: the even ones are exactly the samples the pass before took,
and a box limit that already holds a sample does not move when the sample is taken again.
Fidelity class **none**; no float operation changes.

C provenance: `lib/common/routespl.c:limitBoxes` (the loop at `routesplines_`, `INIT_DELTA` 10,
`LOOP_TRIES` 15). Port: `common/routespl.rs`, `limit_boxes` (C's loop, unchanged for every other
caller, the last-resort call included) and `limit_boxes_samples` with `Samples::Odd`. Technique T9
(hoist work whose result is already known). No unsafe code, dependency or facade change.

The argument is proved in Lean 4 over FloatLib's binary64 model:
[`proofs/GvpProofs/Geom/LimitBoxes.lean`](../../proofs/GvpProofs/Geom/LimitBoxes.lean). The journal
entry in [`docs/research/lean-experiment/journal.md`](../research/lean-experiment/journal.md)
has what the proof cost and what it found.

## The argument

A pass with `delta` takes, for every cubic segment, the samples `si = 0, 1, .., num_div` with
`num_div = delta * boxn` and `t = si / num_div`, and for every box whose `y` range holds the
sample's `y` (within `.0001`) sets `ll.x = fmin(ll.x, x)` and `ur.x = fmax(ur.x, x)`. The loop
writes only `x`.

1. **`t` is a quotient of whole numbers.** `si` is `0.0` plus `k` times `1.0`, and a binary64 sum of
   whole numbers below 2^53 is exact, so `si` is exactly the number `k`; it is not accumulated
   rounding error. `delta` is `10.0 * 2^j` after `j` doublings and `num_div = delta * boxn` is the
   whole number `10 * 2^j * boxn`, exact while it stays below 2^53 (`boxn` below 5 * 10^10 for
   all 15 passes). So a sample's `t` is `fl(k / D)` for whole numbers `k <= D`.
2. **The even samples repeat, bit for bit.** After `delta` doubles, `D` becomes `2D`, and sample
   `2k` has `t = fl(2k / 2D)`. A binary64 quotient is the real quotient rounded once, and
   `2k / 2D = k / D` as reals, so `fl(2k / 2D)` is the same word as `fl(k / D)`: `0.0 / x` is `+0.0`
   on both sides, and a positive quotient of whole numbers below 2^53 cannot underflow. (The
   ticket expected a power-of-two scaling argument. None is needed: any common factor cancels
   before the one rounding.)
3. **A sample is a function of its `t`.** `sp` is copied from `pps[splinepi..splinepi + 4]` for
   every `si` and the twelve fused lerps read nothing else. The same `t` and segment give the same
   `(x, y)`, and the same box test.
4. **A limit that holds a sample ignores it.** Take the box limits after the pass at `delta`. Each
   even sample of the next pass was in that pass; a limit `c` after folding in `x` satisfies
   "`x` is not below `c`" and keeps satisfying it as `c` only moves down (up for `ur.x`). So
   `fmin(c, x) = c` when the sample comes round again, whenever `c` and `x` are not zeros of
   opposite sign. The Lean theorem `limit_boxes_retry_exact` is this step for one box, for any
   strict order on the limits (binary64's `<`, NaN included).
5. **Zeros.** `fmin(-0.0, +0.0)` may return either, chosen by the code generator, so a limit that is
   a zero could in principle flip its sign when the sample comes round again, and the odd-only
   retry would not flip it. `may_skip_even_samples` therefore takes the retry only when no box
   limit equals zero (`ll.x` or `ur.x`, either sign). That is enough: a tie between a limit `c`
   and a retaken sample `e` needs `e` to be a zero, and `c` was at or beyond `e` when the retry
   began, so `c` was below zero (it was not a zero) and only moves further down (up for `ur.x`),
   never to a zero. Every other tie is two equal nonzero floats, which `fmin` and `fmax` return
   the same bits for whichever they pick. This step is argued here and covered by the check
   below; it is not in Lean.
6. **NaN.** The Lean step keeps a NaN limit on a tie, where Rust's `f64::min(NaN, x)` returns `x`,
   so the two agree only when no limit is NaN. That holds on every retry: the limits start as
   numbers (`routesplines_` initialises them from the boxes), a NaN sample fails the `y` box test
   and is never folded in, and `min`/`max` of a number and a number is a number. The straight-spline
   path can store a NaN coordinate, but it never reaches the retry loop. Like (5), this invariant
   is argued here, not in Lean.
7. **Domain.** Step 1's exactness needs `10 * 2^j * boxn < 2^53`, i.e. `boxn` below about
   5.5 * 10^10 with `LOOP_TRIES = 15`. Rust does not check this; it is an assumption of the proof.
   `boxn` counts the rank boxes of one edge, so it is far below the bound on any input that fits
   in memory, but the bound is the proof's, not the code's.

The order of the samples a box sees changes (the odd ones now come without the even ones between),
and the order does not matter: each box ends at the least and the greatest of the samples it
accepts (4), and no limit is a zero (5).

## Every output float keeps its operations

The floats `limit_boxes` produces are the box limits `ll.x` and `ur.x`. Each is `fmin`/`fmax` of
the samples `sp[0].x` that pass the `y` test, folded in sample order. Each sample's `sp[0].x` is
the twelve `fma(t, b - a, a)` of the unchanged loop body, on the `t = si / num_div` of the same
`si` as before. The retry computes the odd samples with exactly those operations and skips the
even ones; the box limits it ends with are bitwise those of the full pass (4, 5).

## What stays

- C's loop stays: `limit_boxes` is `limit_boxes_samples` with `Samples::All`, used by the first
  pass and by the last-resort call on the polyline (a different spline, so nothing to reuse).
- Where a box limit is a zero, the whole retry runs, as in C.

## How it is checked

**Check mode** (`--features graphviz-pure-layout/routing-check`, and `cfg(test)`): the retry loop
also runs C's full pass on a copy of the boxes and asserts every `ll.x` and `ur.x` equal to the
bit after every retry.

**Tests:** `odd_samples_only_retry_matches_full_passes` (3,000 random box lists and splines whose
whole-number coordinates produce zero limits, five doublings each, bitwise) and
`a_zero_limit_of_either_sign_keeps_the_even_samples`.

The Lean proof does not retire `routing-check`: the Rust is not extracted from it (Aeneas has no
floats), so the check still guards the model-to-Rust step.

## Performance

How often it matters: retries are rare. Over the corpus (`tests/{fixtures,wild,render}` with dot,
and `tests/{twopi,circo,neato,fdp}` with `splines=true`; 9,652 spline routes) 94 passes were
retries (the sum of the table's rows), in three inputs, all dot (`gv16_graphs_b57_957a04`, `gv16_graphs_b51_33bcb0`,
`gv16_graphs_b106_471493`); none had a zero limit. The twopi/circo/fdp `ngk10_4` rows never retry,
so this change does not touch them.

### Instructions (Callgrind, musl release, x86-64-v1; deterministic)

Before is #706's tip (`bac1128f`), after adds this change; allocations are identical on every
ratchet case (+0), and the ratchet gate passes ("No regression"; its Δ column compares with the
committed baseline, which also holds #706's own gains, so the table below is the direct pair).

| input | #706 | after | Δ | retry passes |
| :-- | --: | --: | --: | --: |
| `wild/gv16_graphs_b57_957a04.gv` | 43,183,145 | 36,900,336 | -14.55% | 60 |
| `wild/gv16_graphs_b51_33bcb0.gv` | 70,374,727 | 66,930,969 | -4.89% | 31 |
| `wild/gv16_graphs_b106_471493.gv` | 840,186,865 | 837,373,295 | -0.33% | 3 |
| `fixtures/unix.gv` | 17,386,288 | 17,391,950 | +0.03% | 0 |
| `twopi/wild_gv16_undirected_ngk10_4_793340.gv` | 1,314,260,676 | 1,314,345,356 | +0.006% | 0 |

The small rise where nothing retries is the new loop bounds and the zero scan, which only a
retry executes; it is far under the ratchet's 2% gate. The ratchet cases are not retry inputs, so
`ratchet-baseline.json` is not refreshed.

### Wall time (informational)

Musl release builds, nine runs per row, median ms, back to back; `uptime` load average was 17
at the start of the second pair: the machine was busy, differences under 10% are noise. 1.0 is the
1.0.0 release candidate (`f89e21d9`); C is stock 16.1.0.

| input | 1.0 | #706 | after | ×1.0 | C | ×C |
| :-- | --: | --: | --: | --: | --: | --: |
| twopi `ngk10_4` (`splines=true`) | 867.6 | 143.2 | 138.1 | 6.3 | 117.3 | 1.18 |
| circo `fx_arrows` (`splines=true`) | 374.7 | 76.7 | 72.0 | 5.2 | 54.9 | 1.31 |
| fdp `ngk10_4` (`splines=true`) | 965.5 | 169.6 | 162.3 | 5.9 | 125.5 | 1.29 |
| dot `gv16_graphs_b57_957a04` | 20.7 | 16.8 | 12.6 | 1.6 | 10.8 | 1.17 |
| dot `gv16_graphs_b51_33bcb0` | 26.9 | 21.9 | 20.7 | 1.3 | 19.8 | 1.05 |

The twopi, circo and fdp rows never retry, so their ×1.0 comes from #706 and earlier work; this
change does not move them. Only the two dot rows run retries, and their wall times sit within the
noise of the 15% and 5% instruction savings.

### Verification

`cargo test --workspace`: 34 suites, 805 tests passed, 0 failed (the gv-diff layout ratchet, the
seven-format render ratchet and `e2e_svg_identity_does_not_regress` among them; the e2e baseline
is unchanged). `cargo test -p graphviz-pure-layout --features routing-check limit_boxes`: the
new check-mode assertion and four tests pass. `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings` and `scripts/panic-sites.sh` clean. `scripts/proofs/lake.sh build`:
success, no `sorry`; `#print axioms` on the main theorems lists only `propext`,
`Classical.choice`, `Quot.sound`.
