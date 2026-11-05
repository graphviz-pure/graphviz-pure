# `solve3` without the cube roots no caller reads

`splineintersectsline` (`lib/pathplan/route.c`) calls `solve3` once per barrier side and keeps
only the roots in `[0, 1]`. On dot's routing-heavy graphs, about 70% of `solve3` calls return
no such root. Most of those have one real root (`disc >= 0`), and that root costs two `cbrt`
calls. `pathplan/solvers.rs:solve3_unit` runs `solve3`'s operations up to those `cbrt` calls.
There it bounds every root `solve3` would return, using only the exponents of `alpha` and
`beta`. When the bound lies outside `[0, 1]`, it returns 0 without computing them. This page
shows that **no caller can tell the difference**. The argument needs no error bound on
`solve3`: it bounds what `solve3` computes, not how close that is to the true roots.

`solve3` stays as C has it and is the reference. In check mode it runs beside every call (§5).

Issue #625; measurements are in its pull request.

## 1. What `solve3` computes

`solve3(coeff, roots)` with `a, b, c, d = coeff[3], coeff[2], coeff[1], coeff[0]`:

- if `|a| < 1e-7`, it returns `solve2`'s answer (0, 1, 2 or, through `solve1`, 4);
- otherwise it computes `b_over_3a`, `c_over_a`, `d_over_a`, `p`, `q` and
  `disc = fma(q, q, 4p³)`, then:
  - `disc < 0`: three roots `temp·cos(…)`, `rootn = 3`;
  - otherwise: `alpha = 0.5·(sqrt(disc) − q)`, `beta = −q − alpha`,
    `roots[0] = cbrt(alpha) + cbrt(beta)`, and
    - `disc > 0`: `rootn = 1`;
    - otherwise (`disc == 0`, or NaN): `roots[1] = roots[2] = −0.5·roots[0]`, `rootn = 3`;
- and finally `roots[i] −= b_over_3a` for each `i < rootn`.

Every operation is one IEEE operation rounded to nearest. Rust does not contract `x * y + z`,
so each `*`, `+` and `−` rounds on its own.

## 2. What the callers read

> **Lemma 1.** In each branch of `splineintersectsline`, a `solve3` result `n ∈ {1, 3}` whose
> roots all lie outside `[0, 1]` has the same effect as the result 0.

- **A side that is not vertical, and a vertical side** (`xc[1] != 0`, or `xc[1] == 0` and
  `yc[1] != 0`): `n == 4` returns 4. Otherwise only roots `tv` with `0 <= tv <= 1` go on to the
  `sv` test and `addroot`, so roots outside contribute nothing, and neither does `n = 0`.
- **A point barrier** (`xc[1] == yc[1] == 0`): `x` and `y` are solved separately. A count of 4
  selects the branch. Otherwise roots reach `addroot` alone or as equal pairs `a == b`, and
  `addroot` drops anything outside `[0, 1]`. A count of 0 drops the same roots, all of them.

In every branch the skipped result is never 4 (§3 skips only where `solve3` returns 1 or 3), so
the `== 4` tests take the same branch. `roots` is not read beyond the count, so the stale
entries a skip leaves are never read either. `splineintersectsline`'s return value and the
roots it adds are therefore unchanged, and so is every decision of `splineisinside`.

The `solve3` memo (`docs/perf/route-solve3-memo.md`) caches `solve3_unit`. Its argument needs
only a pure function of the coefficient bits, and `solve3_unit` is one.

## 3. The bound

> **Lemma 2 (`cbrt_bracket`).** Let `x` be normal with `2^e ≤ |x| < 2^(e+1)` and
> `k = ⌊e/3⌋`. Then any faithfully rounded `cbrt(x)` lies in `[2^k, 2^(k+1)]`, negated for
> negative `x`. `cbrt(±0) = ±0`.

`3k ≤ e` and `e + 1 ≤ 3k + 3`, so `2^(3k) ≤ |x| < 2^(3k+3)`, and the real cube root of `|x|`
is in `[2^k, 2^(k+1))`. `e` is in `−1022..=1023`, so `k` is in `−341..=341`, and both ends are
normal floats. A faithfully rounded result is one of the two floats around the real value, or
the value itself, so it is in `[2^k, 2^(k+1)]`. For negative `x` the real value is in
`(−2^(k+1), −2^k]`, and the same holds negated. `math::cbrt` is CORE-MATH's correctly rounded
`cbrt` on every target: the `libm` crate's, or its copy in `core/math/portable/cbrt.rs`, which
`math::tests` checks bit for bit against it. Correct rounding is faithful. The bracket returns
`None`, and nothing is skipped, for subnormal, infinite and NaN `x`.

`cbrt_bracket_holds_at_every_exponent` checks the bracket against `math::cbrt` on all 2,046
exponents, both signs, at the ends of each binade and at 60 random significands.

**Monotone rounding.** For round-to-nearest, `u ≤ v` implies `fl(u) ≤ fl(v)`, also when the
result overflows to ±∞. Each bound below follows from this, applied to the exact operation on
the bracketing values.

> **Lemma 3.** In the branch `disc >= 0`, let `alpha` and `beta` be the values `solve3`
> computes. If both have a bracket, `[αl, αh]` and `[βl, βh]`, put `s_lo = fl(αl + βl)` and
> `s_hi = fl(αh + βh)`. Then every root `solve3` returns lies in
>
> - `[fl(s_lo − b_over_3a), fl(s_hi − b_over_3a)]`, and, when `disc` is not `> 0`, also
> - `[fl(fl(−0.5·s_hi) − b_over_3a), fl(fl(−0.5·s_lo) − b_over_3a)]`.

`solve3_unit` computes `alpha` and `beta` with `solve3`'s own operations on the same operands,
so they are the same floats. Let `A = cbrt(alpha)` and `B = cbrt(beta)`. By Lemma 2,
`αl + βl ≤ A + B ≤ αh + βh` exactly, so `s_lo ≤ fl(A + B) ≤ s_hi`: that is the bound on
`roots[0]` before the shift. Subtracting `b_over_3a` is monotone in its first operand, which
gives the first interval. For `disc == 0`, `−0.5·S` is decreasing in `S`, so
`fl(−0.5·s_hi) ≤ roots[1] = roots[2] ≤ fl(−0.5·s_lo)`, and the shift is monotone again. Under
Lemma 2, `|A|` and `|B|` are at most `2^342`, so `fl(A + B)` is finite. If `b_over_3a` is
infinite, both the root and the interval are the same infinity. If it is NaN, every comparison
below is false and nothing is skipped.

`solve3_unit` returns 0 when the first interval, and for `disc` not `> 0` also the second, is
outside `[0, 1]` (`hi < 0` or `lo > 1`; a NaN end never counts as outside).

> **Theorem.** `solve3_unit` either returns exactly what `solve3` returns, bit for bit, or
> returns 0 where `solve3` returns 1 or 3 roots all outside `[0, 1]`.

If it does not skip, it executes `solve3`'s operations on the same operands in the same order:
outside the skip block the two bodies are token for token the same. If it skips, it is in the
branch `disc >= 0`, where `solve3` returns 1 or 3 roots, and Lemma 3 puts each of them outside
`[0, 1]`. With Lemma 1, routing is unchanged.

The skip adds no floating-point operation to any value a caller reads: the bound's own
additions only decide whether to skip.

## 4. What it saves, and what it leaves

On b29 (after the memo), 269,453 `solve3` calls: 79,727 have a root in `[0, 1]`, 76,750 take
the three-root branch, and 192,599 the one-root branch, 178,636 of them with no root in range.
The exponent bracket skips 160,879, or 60% of all calls (b143 49,649 of 90,689; `dot_dag_1000`
26,288 of 47,469).

What is not skipped, and why:

- **The prefix.** The bound needs `solve3`'s own `alpha`, `beta` and `b_over_3a`, so the
  divisions, `disc` and `sqrt` still run. Bounding them from the coefficients would need the
  forward error of Cardano's formula, which is unbounded near multiple roots. This is the part
  of the "skip `solve3`" idea that stays unproved.
- **The three-root branch.** Its roots are `temp·cos(θ_k) − b_over_3a`. With `|cos| ≤ 1` they
  lie within `temp` of `−b_over_3a`, but that interval misses `[0, 1]` on only 4 of b29's
  10,987 calls of that branch with no root in range. It is not worth an assumption about
  `libm`'s `cos`.
- **A tighter bracket.** A bracket about 2% wide (16 significand buckets per binade, a table of
  verified constants) would skip about 176,000 on b29 instead of 160,879. That is 6% more of the
  calls, for a table and its proof.

## 5. How it is checked

Under `cfg(test)` or the `pathplan-check` feature (which `routing-check` turns on in CI over the
whole test suite), **every** `solve3_unit` call also runs `solve3`:

- a skip asserts that `solve3` returned neither 4 nor any root in `[0, 1]`;
- any other call asserts the same count and the same bits of `roots[..n]`.

Tests in `pathplan/solvers/tests.rs`:

- the bracket on every exponent (§3);
- `solve3_unit_matches_solve3_and_skips`: 600,000 cubics through the check. They cover one
  real root with a complex pair anywhere near `[0, 1]` (imaginary parts down to `1e-6`), three
  real roots, double and near-double roots at 0 and 1, roots within `1e-17` of either end,
  Bézier coordinates against a side, and leading coefficients from `1e-6` to `1e6`. It asserts
  more than 100,000 skips;
- `(t + 2)(t − 1)²`, whose exact double root 1 only the `disc == 0` interval keeps;
- three mutations, each of which the check must catch. The bracket's top end at `2^k`
  (`NarrowBracket`), the bound without `− b_over_3a` (`NoShift`), and no `disc == 0` interval
  (`NoDoubleRoot`) all make the check fail.
