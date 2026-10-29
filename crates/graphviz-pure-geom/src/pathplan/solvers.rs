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
