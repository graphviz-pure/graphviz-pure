//! `solve3_unit` against `solve3`, and the tests that break its bound on purpose.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Uniform in [lo, hi).
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
    /// `10^u`, `u` uniform in [lo, hi).
    fn magnitude(&mut self, lo: f64, hi: f64) -> f64 {
        math::powf(10.0, self.uniform(lo, hi))
    }
    fn sign(&mut self) -> f64 {
        if self.next() & 1 == 0 { 1.0 } else { -1.0 }
    }
}

/// `scale * (t - r) * (t^2 - 2 re t + re^2 + im^2)`, rounded coefficient by coefficient.
fn from_roots(scale: f64, r: f64, re: f64, m: f64) -> [f64; 4] {
    // (t - r)(t^2 + pt + q) with p = -2 re, q = re^2 + m (m = im^2, or -d^2 for real roots)
    let (p, q) = (-2.0 * re, re * re + m);
    [-r * q * scale, (q - r * p) * scale, (p - r) * scale, scale]
}

/// `points2coeff(v0, v1, v2, v3)` of `route.rs` with `x0` taken off `s[0]`: what
/// `splineintersectsline` hands to the solver for a vertical side.
fn bezier(v: [f64; 4], x0: f64) -> [f64; 4] {
    let [v0, v1, v2, v3] = v;
    [
        v0 - x0,
        3.0 * (v1 - v0),
        math::fma(-6.0f64, v1, math::fma(3.0f64, v0, 3.0 * v2)),
        math::fma(3.0f64, v1, v3) - math::fma(3.0f64, v2, v0),
    ]
}

/// Cubics of every kind the router meets and some it does not: one real root and a complex
/// pair anywhere near [0, 1], three real roots, double and near-double roots at 0 and 1, and
/// Bézier coordinates against a side. Every call goes through the check.
fn sweep(count: usize) {
    let mut r = Rng(0x5eed_0fc0_be5c_a1d7);
    let mut roots = [0.0; 3];
    for i in 0..count {
        let scale = r.sign() * r.magnitude(-6.0, 6.0);
        let coeff = match i % 6 {
            0 => {
                let im2 = r.magnitude(-12.0, 1.0);
                from_roots(scale, r.uniform(-3.0, 4.0), r.uniform(-3.0, 4.0), im2)
            }
            1 => {
                let d = r.uniform(0.0, 2.0);
                from_roots(scale, r.uniform(-3.0, 4.0), r.uniform(-3.0, 4.0), -d * d)
            }
            2 => {
                // a double or near-double root at 0 or 1
                let at = if r.next() & 1 == 0 { 0.0 } else { 1.0 };
                let eps = r.sign() * r.magnitude(-17.0, -3.0);
                let m = if r.next() & 1 == 0 { eps * eps } else { 0.0 };
                from_roots(scale, r.uniform(-3.0, 4.0), at + eps, m)
            }
            3 => {
                // a root just inside or outside an end of [0, 1]
                let at = if r.next() & 1 == 0 { 0.0 } else { 1.0 };
                let near = at + r.sign() * r.magnitude(-17.0, -1.0);
                from_roots(scale, near, r.uniform(-3.0, 4.0), r.magnitude(-6.0, 1.0))
            }
            _ => {
                let w = r.magnitude(-1.0, 4.0);
                let v = std::array::from_fn(|_| r.uniform(-w, w));
                bezier(v, r.uniform(-2.0 * w, 2.0 * w))
            }
        };
        solve3_unit(&coeff, &mut roots);
    }
}

fn skips() -> u64 {
    SKIPS.with(std::cell::Cell::get)
}

#[test]
fn cbrt_bracket_holds_at_every_exponent() {
    let mut r = Rng(0x00b0_a7d5);
    for biased in 1..=2046u64 {
        let mantissas = [0, (1 << 52) - 1, 1, (1 << 51)]
            .into_iter()
            .chain((0..60).map(|_| r.next() >> 12));
        for m in mantissas {
            for sign in [0, 1u64 << 63] {
                let x = f64::from_bits(sign | (biased << 52) | m);
                let (lo, hi) = cbrt_bracket(x).expect("normal");
                let y = math::cbrt(x);
                assert!(
                    lo <= y && y <= hi,
                    "{x:e}: cbrt {y:e} outside [{lo:e}, {hi:e}]"
                );
                assert!(
                    hi <= 2.0 * lo || (lo <= 0.0 && 2.0 * hi >= lo),
                    "{x:e}: too wide"
                );
            }
        }
    }
    assert_eq!(cbrt_bracket(0.0), Some((0.0, 0.0)));
    assert_eq!(cbrt_bracket(-0.0), Some((0.0, 0.0)));
    for x in [
        f64::from_bits(1),
        -f64::MIN_POSITIVE / 2.0,
        f64::INFINITY,
        f64::NAN,
    ] {
        assert_eq!(cbrt_bracket(x), None, "{x:e}");
    }
}

#[test]
fn solve3_unit_matches_solve3_and_skips() {
    let before = skips();
    sweep(600_000);
    let skipped = skips() - before;
    assert!(skipped > 100_000, "the bound should skip often: {skipped}");
}

/// The double root of `(t + 2)(t - 1)^2`, exactly 1, is the one `NoDoubleRoot` forgets.
#[test]
fn solve3_unit_keeps_an_exact_double_root_at_one() {
    let mut roots = [0.0; 3];
    assert_eq!(solve3_unit(&[2.0, -3.0, 0.0, 1.0], &mut roots), 3);
    assert_eq!(roots, [-2.0, 1.0, 1.0]);
}

fn with_mutant(m: Mutant, f: impl FnOnce()) {
    MUTANT.with(|c| c.set(m));
    let reset = scopeguard(|| MUTANT.with(|c| c.set(Mutant::None)));
    f();
    drop(reset);
}

/// Runs `f` when dropped, panicking or not.
fn scopeguard(f: impl FnOnce()) -> impl Drop {
    struct Guard<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Guard<F> {
        fn drop(&mut self) {
            if let Some(f) = self.0.take() {
                f();
            }
        }
    }
    Guard(Some(f))
}

#[test]
#[should_panic(expected = "solve3_unit: skipped a root in [0, 1]")]
fn the_check_catches_a_bracket_too_narrow() {
    with_mutant(Mutant::NarrowBracket, || sweep(600_000));
}

#[test]
#[should_panic(expected = "solve3_unit: skipped a root in [0, 1]")]
fn the_check_catches_a_bound_without_the_shift() {
    with_mutant(Mutant::NoShift, || sweep(600_000));
}

#[test]
#[should_panic(expected = "solve3_unit: skipped a root in [0, 1]")]
fn the_check_catches_a_forgotten_double_root() {
    with_mutant(Mutant::NoDoubleRoot, || {
        solve3_unit(&[2.0, -3.0, 0.0, 1.0], &mut [0.0; 3]);
    });
}
