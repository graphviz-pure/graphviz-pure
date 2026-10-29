//! Exact software fused multiply-add (`RN(a * b + c)`): Boldo and Melquiond's emulated FMA
//! (*Emulation of a FMA and correctly-rounded sums: proved algorithms using rounding to odd*,
//! IEEE Transactions on Computers 57(4):462–471, 2008) inside an exponent window where no
//! intermediate overflows or underflows, with a zero-factor shortcut and `libm::fma` outside it.
//! See `docs/perf/wasm-fma.md`.

// Rounding to odd tests `e > 0.0 || e < 0.0` (which is false for both ±0.0 and NaN), and the
// fallback checks `a == 0.0 || b == 0.0` for the exact signed-zero product.
#![allow(clippy::double_comparisons, clippy::float_cmp)]

/// Veltkamp's splitting constant `2^27 + 1` for binary64.
const SPLIT: f64 = 134_217_729.0;

/// Knuth's TwoSum: `s + e == a + b` exactly and `s == RN(a + b)` when `a + b` does not overflow.
#[inline(always)]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// Veltkamp's split of `a` into two 26-bit halves `h + l == a` (`|a| < 2^996`).
#[inline(always)]
fn split(a: f64) -> (f64, f64) {
    let c = SPLIT * a;
    let h = c - (c - a);
    (h, a - h)
}

/// Round to odd of the exact sum `s + e` (a [`two_sum`] pair): when `e != 0` and `s` has an even
/// significand, step `s` one ulp towards `e`.
#[inline(always)]
fn odd(s: f64, e: f64) -> f64 {
    let b = s.to_bits();
    if (e > 0.0 || e < 0.0) && b & 1 == 0 {
        // `s != 0` (an inexact sum never rounds to zero); growing the bits grows the magnitude.
        f64::from_bits(if (e > 0.0) == (s > 0.0) { b + 1 } else { b - 1 })
    } else {
        s
    }
}

/// Whether `(a, b, c)` lies in the fast-path exponent window:
/// - `a`, `b` normal with `|a|, |b| < 2^996` (so `SPLIT * a` does not overflow);
/// - unbiased `ea + eb` in `[-900, 1018]` (so `a * b` and its Dekker error are normal, and the sum
///   with `c` stays below `2^1023`);
/// - `c == 0` or `2^-900 <= |c| < 2^1022` (so `two_sum(c, a * b)` neither overflows nor hits the
///   subnormal grid).
#[inline(always)]
pub(crate) fn in_window(a: f64, b: f64, c: f64) -> bool {
    let ea = ((a.to_bits() >> 52) & 0x7ff) as u32;
    let eb = ((b.to_bits() >> 52) & 0x7ff) as u32;
    let cb = c.to_bits() & !(1 << 63);
    let ec = (cb >> 52) as u32;
    let ab_ok = ea.wrapping_sub(1) < 2018 && eb.wrapping_sub(1) < 2018;
    let p_ok = (ea + eb).wrapping_sub(2046 - 900) <= 1918;
    let c_ok = cb == 0 || ec.wrapping_sub(123) <= 2044 - 123;
    ab_ok & p_ok & c_ok
}

/// Correctly rounded `RN(a * b + c)` for all binary64 inputs.
#[inline(always)]
#[must_use]
pub fn fma(a: f64, b: f64, c: f64) -> f64 {
    if in_window(a, b, c) {
        let p = a * b;
        let (ah, al) = split(a);
        let (bh, bl) = split(b);
        let ul = ((ah * bh - p) + ah * bl + al * bh) + al * bl;
        let (th, tl) = two_sum(c, p);
        let (vs, ve) = two_sum(tl, ul);
        th + odd(vs, ve)
    } else {
        fma_slow(a, b, c)
    }
}

#[cold]
#[inline(never)]
fn fma_slow(a: f64, b: f64, c: f64) -> f64 {
    // A zero factor with a finite other factor makes `a * b` an exact ±0, so the unfused
    // expression already computes `RN(a * b + c)` with IEEE's signed-zero rules.
    if (a == 0.0 || b == 0.0) && a.is_finite() && b.is_finite() {
        a * b + c
    } else {
        libm::fma(a, b, c)
    }
}
