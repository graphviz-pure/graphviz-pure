//! `lib/pathplan/solvers.c`: real roots of polynomials up to degree 3.
//! A return value of 4 means "infinitely many roots".

use graphviz_pure_core::math;

const EPS: f64 = 1e-7;

fn aeq0(x: f64) -> bool {
    x < EPS && x > -EPS
}

/// `solve3(coeff, roots)`: `coeff[3] t^3 + ... + coeff[0] = 0`.
pub fn solve3(coeff: &[f64; 4], roots: &mut [f64; 3]) -> usize {
    use std::f64::consts::PI;
    let (a, b, c, d) = (coeff[3], coeff[2], coeff[1], coeff[0]);
    if aeq0(a) {
        return solve2(&[coeff[0], coeff[1], coeff[2]], roots);
    }
    let b_over_3a = b / (3.0 * a);
    let c_over_a = c / a;
    let d_over_a = d / a;
    let mut p = b_over_3a * b_over_3a;
    let q = math::fma(2.0 * b_over_3a, p, -(b_over_3a * c_over_a)) + d_over_a;
    p = c_over_a / 3.0 - p;
    let disc = math::fma(q, q, 4.0 * p * p * p);
    let rootn;
    if disc < 0.0 {
        let r = 0.5 * math::fma(q, q, -disc).sqrt();
        let theta = math::atan2((-disc).sqrt(), -q);
        let temp = 2.0 * math::cbrt(r);
        roots[0] = temp * math::cos(theta / 3.0);
        roots[1] = temp * math::cos((theta + PI + PI) / 3.0);
        roots[2] = temp * math::cos((theta - PI - PI) / 3.0);
        rootn = 3;
    } else {
        let alpha = 0.5 * (disc.sqrt() - q);
        let beta = -q - alpha;
        roots[0] = math::cbrt(alpha) + math::cbrt(beta);
        if disc > 0.0 {
            rootn = 1;
        } else {
            roots[1] = -0.5 * roots[0];
            roots[2] = roots[1];
            rootn = 3;
        }
    }
    for r in roots.iter_mut().take(rootn) {
        *r -= b_over_3a;
    }
    rootn
}

fn solve2(coeff: &[f64; 3], roots: &mut [f64; 3]) -> usize {
    let (a, b, c) = (coeff[2], coeff[1], coeff[0]);
    if aeq0(a) {
        return solve1(&[coeff[0], coeff[1]], roots);
    }
    let b_over_2a = b / (2.0 * a);
    let c_over_a = c / a;
    let disc = math::fma(b_over_2a, b_over_2a, -c_over_a);
    if disc < 0.0 {
        0
    } else if disc > 0.0 {
        roots[0] = -b_over_2a + disc.sqrt();
        roots[1] = math::fma(-2.0, b_over_2a, -roots[0]);
        2
    } else {
        roots[0] = -b_over_2a;
        1
    }
}

fn solve1(coeff: &[f64; 2], roots: &mut [f64; 3]) -> usize {
    let (a, b) = (coeff[1], coeff[0]);
    if aeq0(a) {
        return if aeq0(b) { 4 } else { 0 };
    }
    roots[0] = -b / a;
    1
}

/// Rust-only helper; no C counterpart. Bounds `cbrt(x)` from `x`'s exponent alone: for a
/// normal `x` with `2^e <= |x| < 2^(e+1)` and `k = floor(e / 3)`, every faithfully rounded
/// `cbrt(x)` lies in `[2^k, 2^(k+1)]` (negated for negative `x`), and `cbrt(±0)` is `±0`.
/// `None` for subnormals, infinities and NaN (`docs/perf/solve3-unit.md`, Lemma 2).
fn cbrt_bracket(x: f64) -> Option<(f64, f64)> {
    if x == 0.0 {
        return Some((0.0, 0.0));
    }
    if !x.is_normal() {
        return None;
    }
    // The biased exponent of a normal number is in 1..=2046, so `e` is in -1022..=1023,
    // `k` in -341..=341, and both powers of two are normal.
    let biased = ((x.to_bits() >> 52) & 0x7ff) as i32;
    let k = biased.wrapping_sub(1023).wrapping_div_euclid(3);
    let pow2 = |k: i32| f64::from_bits(u64::from(k.wrapping_add(1023).unsigned_abs()) << 52);
    let high = if mutant() == Mutant::NarrowBracket {
        pow2(k)
    } else {
        pow2(k.wrapping_add(1))
    };
    let (lo, hi) = (pow2(k), high);
    Some(if x > 0.0 { (lo, hi) } else { (-hi, -lo) })
}

/// Rust-only helper; no C counterpart. Whether every real number in `[lo, hi]` is outside
/// `[0, 1]`. NaN bounds are never outside.
fn outside_unit(lo: f64, hi: f64) -> bool {
    hi < 0.0 || lo > 1.0
}

/// Rust-only alternative to [`solve3`] for `lib/pathplan/route.c:splineintersectsline`, which
/// reads only the roots in `[0, 1]`. It returns what `solve3(coeff, roots)` returns, except
/// that where `solve3` would return 1 or 3 roots all outside `[0, 1]`, it may return 0 without
/// computing them.
///
/// It runs `solve3`'s own operations, in its order, on the same operands. Only in the branch
/// with one real root (`disc >= 0`), before the two `cbrt` calls, it bounds every root
/// `solve3` would return from `alpha`, `beta` and `b_over_3a`, using [`cbrt_bracket`] and the
/// monotonicity of rounding. A bound outside `[0, 1]` returns 0 (`docs/perf/solve3-unit.md`).
/// [`solve3`] stays the reference: in check mode every call runs it too and compares.
pub fn solve3_unit(coeff: &[f64; 4], roots: &mut [f64; 3]) -> usize {
    let answer = solve3_unit_(coeff, roots);
    #[cfg(any(test, feature = "pathplan-check"))]
    {
        let mut reference = [f64::NAN; 3];
        let n = solve3(coeff, &mut reference);
        match answer {
            None => {
                assert!(
                    n != 4 && reference.iter().take(n).all(|t| !(0.0..=1.0).contains(t)),
                    "solve3_unit: skipped a root in [0, 1]: {coeff:?} gives {:?}",
                    reference.get(..n.min(3))
                );
                SKIPS.with(|s| s.set(s.get() + 1));
            }
            Some(got) => {
                let read = if n == 4 { 0 } else { n };
                assert!(
                    got == n
                        && roots
                            .iter()
                            .zip(&reference)
                            .take(read)
                            .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "solve3_unit: differs from solve3 for {coeff:?}: {got} {roots:?} vs {n} {reference:?}"
                );
            }
        }
    }
    answer.unwrap_or(0)
}

/// [`solve3_unit`] without its check: `None` where it skipped.
fn solve3_unit_(coeff: &[f64; 4], roots: &mut [f64; 3]) -> Option<usize> {
    use std::f64::consts::PI;
    let (a, b, c, d) = (coeff[3], coeff[2], coeff[1], coeff[0]);
    if aeq0(a) {
        return Some(solve2(&[coeff[0], coeff[1], coeff[2]], roots));
    }
    let b_over_3a = b / (3.0 * a);
    let c_over_a = c / a;
    let d_over_a = d / a;
    let mut p = b_over_3a * b_over_3a;
    let q = math::fma(2.0 * b_over_3a, p, -(b_over_3a * c_over_a)) + d_over_a;
    p = c_over_a / 3.0 - p;
    let disc = math::fma(q, q, 4.0 * p * p * p);
    let rootn;
    if disc < 0.0 {
        let r = 0.5 * math::fma(q, q, -disc).sqrt();
        let theta = math::atan2((-disc).sqrt(), -q);
        let temp = 2.0 * math::cbrt(r);
        roots[0] = temp * math::cos(theta / 3.0);
        roots[1] = temp * math::cos((theta + PI + PI) / 3.0);
        roots[2] = temp * math::cos((theta - PI - PI) / 3.0);
        rootn = 3;
    } else {
        let alpha = 0.5 * (disc.sqrt() - q);
        let beta = -q - alpha;
        // Lemma 3: every root below is in one of these intervals.
        if let (Some((alpha_lo, alpha_hi)), Some((beta_lo, beta_hi))) =
            (cbrt_bracket(alpha), cbrt_bracket(beta))
        {
            let (sum_lo, sum_hi) = (alpha_lo + beta_lo, alpha_hi + beta_hi);
            let shift = if mutant() == Mutant::NoShift {
                0.0
            } else {
                b_over_3a
            };
            let mut skip = outside_unit(sum_lo - shift, sum_hi - shift);
            // Both brackets exist, so `alpha` and `disc` are not NaN: not `disc > 0` is
            // `disc == 0`, the branch with the double root.
            if disc == 0.0 && mutant() != Mutant::NoDoubleRoot {
                skip = skip && outside_unit(-0.5 * sum_hi - shift, -0.5 * sum_lo - shift);
            }
            if skip {
                return None;
            }
        }
        roots[0] = math::cbrt(alpha) + math::cbrt(beta);
        if disc > 0.0 {
            rootn = 1;
        } else {
            roots[1] = -0.5 * roots[0];
            roots[2] = roots[1];
            rootn = 3;
        }
    }
    for r in roots.iter_mut().take(rootn) {
        *r -= b_over_3a;
    }
    Some(rootn)
}

/// A deliberate break of [`solve3_unit`]'s bound, for the tests that its check catches one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
enum Mutant {
    None,
    /// `cbrt_bracket`'s upper end at `2^k`, not `2^(k+1)`.
    NarrowBracket,
    /// The bound forgets the final `- b_over_3a`.
    NoShift,
    /// The double root of `disc == 0` is not bounded.
    NoDoubleRoot,
}

#[cfg(test)]
thread_local! {
    static MUTANT: std::cell::Cell<Mutant> = const { std::cell::Cell::new(Mutant::None) };
    /// Calls `solve3_unit` skipped and its check confirmed.
    static SKIPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(all(feature = "pathplan-check", not(test)))]
thread_local! {
    static SKIPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn mutant() -> Mutant {
    MUTANT.with(std::cell::Cell::get)
}

#[cfg(not(test))]
const fn mutant() -> Mutant {
    Mutant::None
}

#[cfg(test)]
mod tests;
