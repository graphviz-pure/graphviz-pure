# Routing: `solve3` once per cubic, fewer allocations

`Proutespline` tests each candidate Bézier against every barrier side. For each side,
`splineintersectsline` builds a cubic's coefficients and calls `solve3`. Sides that lie on one
line give bit-identical coefficient arrays. A box corridor is made of such sides: the boxes share
left, right, top and bottom coordinates. So `solve3` solves the same cubic again and again. This
page shows that answering a repeat from a cache gives **the same bits as the call**. It also lists
the allocations the routing path no longer makes. No floating-point operation is added, removed or
reordered on any path that reaches the output.

Issue #625; measurements are in its pull request.

## 1. The `solve3` memo

On dot's routing-heavy graphs, profiled on the x86-64-v3 PGO build, `Proutespline` takes 7–22% of
the run and `solve3` with its `cbrt`, `atan2` and `cos` takes most of that:

| graph | `dot_splines` | `Proutespline` | `solve3` (inclusive) | `solve3` calls | same coefficients as an earlier call in the same `splineisinside` |
| :-- | --: | --: | --: | --: | --: |
| b29 | 33.4% | 19.6% | 15.5% | 369,048 | 111,725 (30%) |
| b143 | 29.1% | 18.4% | 13.7% | 120,519 | 29,551 (25%) |
| b124 | 19.6% | 9.8% | 7.5% | 26,772 | 8,692 (32%) |
| b106 | 34.5% | 21.6% | 16.9% | 110,518 | 25,594 (23%) |
| `dot_dag_1000` | 18.3% | 7.4% | 5.5% | 58,586 | 10,959 (19%) |

(Shares of all samples of the whole process, parse and render included, from
`perf record --call-graph fp` on a frame-pointer build with the same PGO profile.)

> **Lemma.** `solve3(coeff, roots)` returns `n` and writes `roots[..n]` (nothing when `n` is 4)
> as a function of the bits of `coeff` alone.

`solve3`, `solve2` and `solve1` read nothing but `coeff`, and globals not at all. They write
`roots[i]` before they read it back (the final `roots[i] -= b_over_3a`). The math functions they
call (`sqrt`, `cbrt`, `atan2`, `cos`, `fma`) are pure functions of their arguments. With the
`math-trace` feature on, a memo hit logs fewer calls; that feature only records arguments.

`splineintersectsline` reads `roots[..n]` only (`take(n)`, and nothing when `n` is 4). So:

> **Theorem.** Replacing a `solve3` call by the `n` and `roots` of an earlier call whose `coeff` had
> the same bits changes no value that `splineintersectsline` reads.

`Solve3Memo` is a direct-mapped table of 64 slots, keyed by the four coefficient bit patterns.
A key that is not in the table, because the slot is empty or holds another key, calls `solve3`
and then fills the slot. So the table decides only whether `solve3` runs, never what a caller
reads. Keys compare as bits, so `-0.0` and `0.0` are different keys, and so are NaNs with
different payloads. One table lives on the stack per `Proutespline` call (4 KiB, no heap
allocation), and keys from earlier `splineisinside` calls of the same route stay valid. In
practice the table catches nearly all repeats: 99,595 hits on b29, 29,830 on b143, 9,531 on
b124, 23,549 on b106, 11,117 on `dot_dag_1000`.

**Check mode** (`cfg(test)` or the `pathplan-check` feature, which `routing-check` turns on in
CI): every hit calls `solve3` again and asserts that `n` and the bits of `roots[..n]` match.
`solve3_memo_check_catches_a_wrong_slot` corrupts the table and asserts that the check fires.
`solve3_memo_hits_on_shared_lines_and_keeps_every_decision` routes random curves through
corridors of shared lines, sharing one scratch across all of them. It asserts over 10,000 hits,
and that every decision equals the pre-#634 reference `splineisinside`.

## 2. Allocations

Each change keeps every value and order; only capacities and the time of allocation change.

- `Pshortestpath`: the deduplicated points go straight into the polygon array, with no index list
  in between (`pn + 2` is enough capacity). The triangles get the `n - 2` a triangulation makes,
  so they never grow. The output path is counted along its links first, then filled once.
- `Proutespline`: each fitted piece adds three points. The pieces split the route at interior
  points, so there are at most `len - 1` of them, and `3·len - 2` points hold any result.
- `routesplines_`: the endpoint text of the `-v` message ("spline … is horizontal") was formatted
  for every edge. It is now formatted inside `gv_info!`, which evaluates its arguments only under
  `-v`.
- `update_bb_bz`: the explicit stack is allocated only if the segment sticks out of the bounding
  box. The first step of the loop was to pop the segment and stop when it does not.
- dot's `completeregularpath`: the path's boxes are reserved once, for every box it may add.

On b143 (`-Tsvg`) the routing path's 16,581 allocations become 10,181. Over the whole run
(valgrind `dhat`, gnu build) that is 104,238 → 97,838, and 24.6 MB → 24.1 MB allocated. Peak heap
is unchanged.

## 3. What was ruled out

- **Skipping `solve3` when the curve stays on one side of the line.** About 75% of calls return
  no root in `[0, 1]`. But `solve3` is approximate: its `1e-7` tolerances drop leading
  coefficients, and its root formulas cancel. A spurious root in `[0, 1]` with `sv` in `[0, 1]`
  makes C reject the curve. Ruling that out needs a bound on how far rounding moves each root, and
  that bound is not proved.
- **Skipping `atan2` and `cos` when all three roots of the `disc < 0` branch fall outside
  `[0, 1]`.** The bound `|temp·cos(·)| ≤ temp` needs `|cos| ≤ 1` from the `libm` crate. That
  holds for faithful rounding, but it is not documented, and it would save at most about 1% on
  b29.
- **`limit_boxes`.** It already makes one evaluation and one binary search per C sample (#706).
  Skipping samples needs a convex-hull bound that also covers de Casteljau's rounding, for at most
  about 2.5% on b29.
