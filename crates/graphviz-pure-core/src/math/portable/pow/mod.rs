//! Correctly rounded `pow`, a Rust port of CORE-MATH's `src/binary64/pow/pow.c`
//! (Tom Hubrecht and Paul Zimmermann, Copyright (c) 2022-2025 CERN and Inria, MIT licence;
//! <https://core-math.gitlabpages.inria.fr/>), with its headers `pow.h`, `dint.h` and
//! `qint.h`. The port follows the C line by line and is checked bit for bit against it
//! (see `docs/porting/libm.md`). It only supports rounding to nearest (the only mode Rust
//! has) and drops the floating-point flag and `errno` handling, which do not change results.
//!
//! The algorithm is that of "Towards a correctly-rounded and fast power function in binary64
//! arithmetic" (Hubrecht, Jeannerod, Zimmermann, ARITH 2023): a double-double first phase, a
//! 128-bit second phase with an exact/midpoint case test, and a 256-bit third phase.
//!
//! MIT licence of the original:
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this
//! software and associated documentation files (the "Software"), to deal in the Software
//! without restriction, including without limitation the rights to use, copy, modify, merge,
//! publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
//! to whom the Software is furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all copies or
//! substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
//! INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
//! PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
//! FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//! DEALINGS IN THE SOFTWARE.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    clippy::excessive_precision
)]

mod dint;
mod qint;
mod tables;

use dint::{
    Dint, LOG2, LOG2_INV, M_ONE, add_dint, add_dint_11, cmp_dint_abs, dint_fromd, dint_toi,
    mul_dint, mul_dint_11, mul_dint_21, mul_dint_int64,
};
use qint::{
    LOG2_INV_Q, LOG2_Q, M_ONE_Q, Qint, add_qint, add_qint_22, mul_qint, mul_qint_2, mul_qint_11,
    mul_qint_21, mul_qint_22, mul_qint_31, mul_qint_33, mul_qint_41, qint_fromd, qint_toi,
};
use tables::{
    INVERSE, INVERSE_2_1, INVERSE_2_2, INVERSE_3_1, INVERSE_3_2, LOG_INV, LOG_INV_2_1, LOG_INV_2_2,
    LOG_INV_3_1, LOG_INV_3_2, P_1, P_2, P_3, Q_1, Q_2, Q_3, T1, T1_2, T1_3, T2, T2_2, T2_3,
};

/// `x = 2^e * m` with `m` the 53-bit significand (`e` unbiased; subnormals keep `e = -1023`).
fn fast_extract(x: f64) -> (i64, u64) {
    let u = x.to_bits();
    let e = ((u >> 52) & 0x7ff) as i64;
    let m = (u & (!0u64 >> 12)) + if e != 0 { 1u64 << 52 } else { 0 };
    (e - 0x3ff, m)
}

fn is_signaling(x: f64) -> bool {
    x.to_bits() & (1u64 << 51) == 0
}

#[inline]
fn fma(a: f64, b: f64, c: f64) -> f64 {
    crate::math::fma(a, b, c)
}

/// `hi + lo = a + b`, assuming `|a| >= |b|`.
#[inline]
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let hi = a + b;
    let e = hi - a;
    (hi, b - e)
}

/// `a + (bh + bl)`, assuming `|a| >= |bh|`.
#[inline]
fn fast_sum(a: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, lo) = fast_two_sum(a, bh);
    (hi, lo + bl)
}

/// `hi + lo = a * b` exactly.
#[inline]
fn a_mul(a: f64, b: f64) -> (f64, f64) {
    let hi = a * b;
    (hi, fma(a, b, -hi))
}

/// `a * (bh + bl)`.
#[inline]
fn s_mul(a: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, s) = a_mul(a, bh);
    (hi, fma(a, bl, s))
}

/// `(ah + al) * (bh + bl) - al * bl`.
#[inline]
fn d_mul(ah: f64, al: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (hi, s) = a_mul(ah, bh);
    let t = fma(al, bh, s);
    (hi, fma(ah, bl, t))
}

fn is_int(x: f64) -> bool {
    x == x.round_ties_even()
}

/// `(e, m)` with `m` odd and `|x| = 2^e * m`.
fn extract(x: f64) -> (i64, u64) {
    let u = x.to_bits();
    let mut e = ((u >> 52) & 0x7ff) as i64;
    let mut m = (u & (!0u64 >> 12)) + if e != 0 { 1u64 << 52 } else { 0 };
    let t = m.trailing_zeros();
    m >>= t;
    e = e + i64::from(t) - (0x433 - i64::from(e == 0));
    (e, m)
}

/// Rounds `x` to 54 bits: `x ~ k * 2^G` (only used in [`exact_pow`]).
fn round_54(x: &Dint) -> (i64, i64) {
    let g = x.ex - 53;
    let k = ((x.hi >> 10) + ((x.hi >> 9) & 1)) as i64;
    (g, k)
}

/// `x * 2^e`.
fn pow2(x: f64, e: i64) -> f64 {
    let mut x = x;
    if e & 1 != 0 {
        x *= 2.0;
    }
    let e2 = f64::from_bits(((((e >> 1) + 0x3ff) as u64) & 0x7ff) << 52);
    (x * e2) * e2
}

/// Rounds `a`, in the subnormal range, to a double.
fn dint_tod_subnormal(a: &Dint) -> f64 {
    let ex = (-(1011 + a.ex)) as u64; // ex >= 12
    if ex >= 64 {
        let rb = a.hi >> 63;
        let sb = (a.hi << 1) | a.lo;
        let ret = if ex > 64 || rb == 0 || sb == 0 {
            0.0
        } else {
            f64::from_bits(1)
        };
        return if a.sgn != 0 { -ret } else { ret };
    }
    let mut hi = a.hi >> ex;
    let rb = (a.hi >> (ex - 1)) & 1;
    let sb = u64::from((a.hi << (65 - ex)) != 0 || a.lo != 0);
    hi += if sb != 0 { rb } else { hi & rb };
    f64::from_bits(hi | (a.sgn << 63))
}

/// Rounds `a` to a double.
fn dint_tod(a: &Dint) -> f64 {
    if a.ex < -1022 {
        return dint_tod_subnormal(a);
    }
    let mut r = f64::from_bits((a.hi >> 11) | (0x3ff << 52));
    let mut rd = 0.0;
    if (a.hi >> 10) & 1 != 0 {
        rd += f64::from_bits(0x3ca0_0000_0000_0000); // 2^-53
    }
    if a.hi & 0x3ff != 0 || a.lo != 0 {
        rd += f64::from_bits(0x3c90_0000_0000_0000); // 2^-54
    }
    if a.sgn != 0 {
        rd = -rd;
    }
    r = f64::from_bits(r.to_bits() | (a.sgn << 63));
    r += rd;
    let e;
    if a.ex > -1023 {
        if a.ex > 1023 {
            if a.ex == 1024 {
                r *= 2.0;
                e = f64::from_bits(0x7fe0_0000_0000_0000); // 2^1023
            } else {
                r = f64::MAX;
                e = f64::MAX;
            }
        } else {
            e = f64::from_bits((((a.ex + 1023) as u64) & 0x7ff) << 52);
        }
    } else if a.ex < -1074 {
        if a.ex == -1075 {
            r *= 0.5;
            e = f64::from_bits(1);
        } else {
            r = f64::from_bits(1);
            e = f64::from_bits(1);
        }
    } else {
        e = f64::from_bits(1u64 << (a.ex + 1074));
    }
    r * e
}

fn subnormalize_qint(a: &mut Qint) {
    if a.ex > -1023 {
        return;
    }
    let ex = (-(1011 + a.ex)) as u64;
    // Shifts by 64 or more are undefined in the C; the hardware it targets (and
    // `wrapping_sh*`) take the amount modulo 64.
    let mut hi = a.hh.wrapping_shr(ex as u32);
    let md = a.hh.wrapping_shr((ex - 1) as u32) & 1;
    let lo = u64::from(
        (a.hh & (!0u64).wrapping_shr(ex as u32)) != 0 || a.hl != 0 || a.lh != 0 || a.ll != 0,
    );
    hi += if lo != 0 { md } else { hi & md };
    a.hh = hi.wrapping_shl(ex as u32);
    a.hl = 0;
    a.lh = 0;
    a.ll = 0;
    if a.hh == 0 {
        a.ex += 1;
        a.hh = 1u64 << 63;
    }
}

fn qint_tod(a: &mut Qint) -> f64 {
    subnormalize_qint(a);
    let mut r = f64::from_bits((a.hh >> 11) | (0x3ff << 52));
    let mut rd = 0.0;
    if a.hh & 0x400 != 0 {
        rd += f64::from_bits(0x3ca0_0000_0000_0000); // 2^-53
    }
    if a.hh & 0x3ff != 0 || a.hl != 0 || a.lh != 0 || a.ll != 0 {
        rd += f64::from_bits(0x3c90_0000_0000_0000); // 2^-54
    }
    if a.sgn != 0 {
        rd = -rd;
    }
    r = f64::from_bits(r.to_bits() | (a.sgn << 63));
    r += rd;
    let e;
    if a.ex > -1023 {
        if a.ex > 1023 {
            if a.ex == 1024 {
                r *= 2.0;
                e = f64::from_bits(0x7fe0_0000_0000_0000);
            } else {
                r = f64::MAX;
                e = f64::MAX;
            }
        } else {
            e = f64::from_bits((((a.ex + 1023) as u64) & 0x7ff) << 52);
        }
    } else if a.ex < -1074 {
        if a.ex == -1075 {
            r *= 0.5;
            e = f64::from_bits(1);
        } else {
            r = f64::from_bits(1);
            e = f64::from_bits(1);
        }
    } else {
        e = f64::from_bits(1u64 << (a.ex + 1074));
    }
    r * e
}

/* ---------------- polynomial approximations of exp(z) ---------------- */

/// `exp(z)` for `|z| < 2^-12.905` as `qh + ql` (relative error < 2^-64.90).
fn q_1(z: f64) -> (f64, f64) {
    let mut q = fma(Q_1[4], z, Q_1[3]);
    q = fma(q, z, Q_1[2]);
    let h0 = fma(q, z, Q_1[1]);
    let (h1, l1) = a_mul(z, h0);
    fast_sum(Q_1[0], h1, l1)
}

/// `exp(y)` for `|y| < 0.00016923`, relative error < 2^-122.29.
fn q_2(y: &Dint) -> Dint {
    let mut r = mul_dint_11(y, &Q_2[0]);
    r = add_dint_11(r, Q_2[1], r);
    r = mul_dint_11(y, &r);
    r = add_dint_11(r, Q_2[2], r);
    r = mul_dint_11(y, &r);
    r = add_dint(&Q_2[3], &r);
    for q in &Q_2[4..8] {
        r = mul_dint(y, &r);
        r = add_dint(q, &r);
    }
    r
}

/// `exp(y)` for `|y| < 0.00016923`, error < 2^-241.11.
fn q_3(y: &Qint) -> Qint {
    let mut r = mul_qint_11(y, &Q_3[0]);
    r = add_qint_22(&Q_3[1], &r);
    for q in &Q_3[2..7] {
        r = mul_qint_22(y, &r);
        r = add_qint_22(q, &r);
    }
    for q in &Q_3[7..12] {
        r = mul_qint_33(y, &r);
        r = add_qint(q, &r);
    }
    for q in &Q_3[12..15] {
        r = mul_qint(y, &r);
        r = add_qint(q, &r);
    }
    r
}

/* ---------------- polynomial approximations of log(1+x) ---------------- */

/// `log(1+z) - z` for `|z| <= 33*2^-13` as `ph + pl`.
fn p_1(z: f64) -> (f64, f64) {
    let (wh, wl) = a_mul(z, z);
    let t = fma(P_1[5], z, P_1[4]);
    let mut u = fma(P_1[3], z, P_1[2]);
    let mut v = fma(P_1[1], z, P_1[0]);
    u = fma(t, wh, u);
    v = fma(u, wh, v);
    u = v * wh;
    (-0.5 * wh, fma(u, z, -0.5 * wl))
}

/// `log(1+z)` for `|z| <= 2^-13` (low limb of `z` zero), relative error < 2^-124.82.
fn p_2(z: &Dint) -> Dint {
    let mut r = mul_dint_11(z, &P_2[0]);
    r = add_dint_11(r, P_2[1], r);
    r = mul_dint_11(z, &r);
    r = add_dint_11(r, P_2[2], r);
    r = mul_dint_11(z, &r);
    r = add_dint_11(r, P_2[3], r);
    r = mul_dint_11(z, &r);
    r = add_dint(&P_2[4], &r);
    for p in &P_2[5..9] {
        r = mul_dint_21(&r, z);
        r = add_dint(p, &r);
    }
    mul_dint_21(&r, z)
}

/// `log(1+z)` for `|z| <= 2^-13` (upper limb of `z` only), relative error < 2^-252.66.
fn p_3(z: &Qint) -> Qint {
    let mut r = mul_qint_11(&P_3[0], z);
    r = add_qint_22(&P_3[1], &r);
    for p in &P_3[2..4] {
        r = mul_qint_11(&r, z);
        r = add_qint_22(p, &r);
    }
    for p in &P_3[4..8] {
        r = mul_qint_21(&r, z);
        r = add_qint_22(p, &r);
    }
    for p in &P_3[8..14] {
        r = mul_qint_31(&r, z);
        r = add_qint(p, &r);
    }
    for p in &P_3[14..18] {
        r = mul_qint_41(&r, z);
        r = add_qint(p, &r);
    }
    mul_qint_41(&r, z)
}

/// `log(x)` as `h + l` for positive finite `x`; the flag is the C's `cancel`.
fn log_1(x: f64) -> (f64, f64, usize) {
    let xu = x.to_bits();
    let mut m = xu & (!0u64 >> 12);
    let mut e = ((xu >> 52) & 0x7ff) as i64;
    let tu;
    if e != 0 {
        tu = m | (0x3ff << 52);
        m += 1u64 << 52;
        e -= 0x3ff;
    } else {
        let k = m.leading_zeros() - 11;
        e = -0x3fe - i64::from(k);
        m <<= k;
        tu = m | (0x3ff << 52);
    }
    let mut t = f64::from_bits(tu);
    let c = usize::from(m >= 0x16a09e667f3bcd);
    const CY: [f64; 2] = [1.0, 0.5];
    const CM: [u32; 2] = [44, 45];
    e += c as i64;
    let big_e = e as f64;
    let i = (m >> CM[c]) as usize;
    t *= CY[c];
    let r = INVERSE[i - 181];
    let l1 = LOG_INV[i - 181][0];
    let l2 = LOG_INV[i - 181][1];
    let z = fma(r, t, -1.0);
    const LOG2_H: f64 = f64::from_bits(0x3fe6_2e42_fefa_3800); // 0x1.62e42fefa38p-1
    const LOG2_L: f64 = f64::from_bits(0x3d2e_f357_93c7_6730); // 0x1.ef35793c7673p-45
    let th = fma(big_e, LOG2_H, l1);
    let tl = fma(big_e, LOG2_L, l2);
    let (mut h, mut l) = fast_sum(th, z, tl);
    let (ph, pl) = p_1(z);
    (h, l) = fast_sum(h, ph, l + pl);
    if e == 0 && l.abs() > h.abs() * f64::from_bits(0x3e70_0000_0000_0000) {
        // 2^-24
        (h, l) = fast_two_sum(h, l);
        return (h, l, 1);
    }
    (h, l, 0)
}

/// `log(x)`, relative error < 2^-122.88.
fn log_2(x: &Dint) -> Dint {
    let mut x = *x;
    let mut e = x.ex;
    let i = if x.hi > 0xb504_f333_f9de_6484 {
        e += 1;
        (x.hi >> (63 + 1 - 7)) as u16 as usize
    } else {
        (x.hi >> (63 - 7)) as u16 as usize
    };
    x.ex -= e;
    let mut z = mul_dint_11(&x, &INVERSE_2_1[i - 90]);
    let j = (z.hi >> (63 - 13 - z.ex)) as u16 as usize;
    z = mul_dint_11(&z, &INVERSE_2_2[j - 8128]);
    z = add_dint(&M_ONE, &z);
    let mut r = mul_dint_int64(&LOG2, e);
    let mut p = p_2(&z);
    p = add_dint(&LOG_INV_2_2[j - 8128], &p);
    p = add_dint(&LOG_INV_2_1[i - 90], &p);
    r = add_dint(&p, &r);
    r
}

/// `log(x)`, relative error < 2^-250.74.
fn log_3(x: &Qint) -> Qint {
    let mut x = *x;
    let mut e = x.ex;
    let i = if x.hh > 0xb504_f333_f9de_6484 {
        e += 1;
        (x.hh >> (63 + 1 - 7)) as u16 as usize
    } else {
        (x.hh >> (63 - 7)) as u16 as usize
    };
    x.ex -= e;
    let mut z = mul_qint(&x, &INVERSE_3_1[i - 90]);
    let j = (z.hh >> (63 - 13 - z.ex)) as u16 as usize;
    z = mul_qint(&z, &INVERSE_3_2[j - 8128]);
    z = add_qint(&M_ONE_Q, &z);
    let mut r = mul_qint_2(e, &LOG2_Q);
    let mut p = p_3(&z);
    p = add_qint(&LOG_INV_3_2[j - 8128], &p);
    p = add_qint(&LOG_INV_3_1[i - 90], &p);
    r = add_qint(&p, &r);
    r
}

/// `s * exp(rh + rl)` as `eh + el`, or NaNs to defer to the second phase.
fn exp_1(rh: f64, rl: f64, s: f64) -> (f64, f64) {
    const RHO0: f64 = f64::from_bits(0xc087_4910_ee4e_8a27); // -0x1.74910ee4e8a27p+9
    const RHO1: f64 = f64::from_bits(0xc084_83b8_cca4_21af); // -0x1.483b8cca421afp+9
    const RHO2: f64 = f64::from_bits(0x4086_2e42_e709_a95b); // 0x1.62e42e709a95bp+9
    const RHO3: f64 = f64::from_bits(0x4086_2e43_16ea_5df9); // 0x1.62e4316ea5df9p+9
    if rh.is_nan() || rh > RHO2 {
        if !rh.is_nan() && rh > RHO3 {
            return (f64::MAX * s, f64::MAX * s);
        }
        return (f64::NAN, f64::NAN);
    }
    if rh < RHO1 {
        if rh < RHO0 {
            return (0.0 * s, f64::from_bits(1) * (0.5 * s));
        }
        return (f64::NAN, f64::NAN);
    }
    const INVLOG2: f64 = f64::from_bits(0x40b7_1547_652b_82fe); // 0x1.71547652b82fep+12
    let k = (rh * INVLOG2).round_ties_even();
    const LOG2H: f64 = f64::from_bits(0x3f26_2e42_fefa_39ef); // 0x1.62e42fefa39efp-13
    const LOG2L: f64 = f64::from_bits(0x3bba_bc9e_3b39_803f); // 0x1.abc9e3b39803fp-68
    let zh = fma(LOG2H, -k, rh);
    let zl = fma(LOG2L, -k, rl);
    let big_k = k as i64;
    let m = (big_k >> 12) + 0x3ff;
    let i2 = ((big_k >> 6) & 0x3f) as usize;
    let i1 = (big_k & 0x3f) as usize;
    let (t1h, t1l, t2h, t2l) = (T1[i2][0], T1[i2][1], T2[i1][0], T2[i1][1]);
    let (eh, el) = d_mul(t2h, t2l, t1h, t1l);
    let (qh, ql) = q_1(zh + zl);
    let (eh, el) = d_mul(eh, el, qh, ql);
    let d = f64::from_bits((m as u64) << 52) * s;
    (eh * d, el * d)
}

/// `exp(x)` for `|x| < 744.45`, relative error < 2^-121.70.
fn exp_2(x: &Dint) -> Dint {
    if x.ex >= 10 {
        let mut r = *x;
        r.ex = if x.sgn == 1 { -1076 } else { 1025 };
        r.sgn = 0;
        return r;
    }
    let kk = mul_dint_11(x, &LOG2_INV);
    let k = dint_toi(&kk);
    let mut kk = mul_dint_int64(&LOG2, k);
    kk.ex -= 12;
    kk.sgn = u64::from(kk.sgn == 0);
    let y = add_dint(x, &kk);
    let m = k >> 12;
    let i2 = ((k >> 6) & 0x3f) as usize;
    let i1 = (k & 0x3f) as usize;
    let mut r = q_2(&y);
    r = mul_dint(&T1_2[i2], &r);
    r = mul_dint(&T2_2[i1], &r);
    r.ex += m;
    r
}

/// `exp(x)` for `|x| < 744.45`, relative error < 2^-241.10.
fn exp_3(x: &Qint) -> Qint {
    let kk = mul_qint_11(x, &LOG2_INV_Q);
    let k = qint_toi(&kk);
    let mut kk = mul_qint_2(k, &LOG2_Q);
    kk.ex -= 12;
    kk.sgn = u64::from(kk.sgn == 0);
    let y = add_qint(x, &kk);
    let m = k >> 12;
    let i2 = ((k >> 6) & 0x3f) as usize;
    let i1 = (k & 0x3f) as usize;
    let mut r = q_3(&y);
    r = mul_qint(&T1_3[i2], &r);
    r = mul_qint(&T2_3[i1], &r);
    r.ex += m;
    r
}

/// `x^y` when it is exact or a midpoint (Algorithm detectRoundingBoundaryCase of Lauter and
/// Lefèvre), given an approximation `z` with relative error < 2^-117.
fn exact_pow(x: f64, y: f64, z: &Dint, exact: bool) -> Option<f64> {
    let s: i64 = if z.sgn != 0 { -1 } else { 1 };
    let (e, m) = extract(x);
    if m == 1 {
        let g = e as f64 * y;
        if is_int(g) {
            let r = if z.sgn != 0 { -1.0 } else { 1.0 };
            return Some(pow2(r, g as i64));
        }
        return None;
    }
    if !(0.0..=34.0).contains(&y) {
        return None;
    }
    let (f, n) = extract(y);
    if n > 34 || f < -5 {
        return None;
    }
    if f < 0 {
        if (e as u64) & (!0u64 >> (64 + f)) != 0 {
            return None;
        }
        let g = ((e >> -f) as u64).wrapping_mul(n) as i64;
        let (big_g, k) = round_54(z);
        if !exact {
            let cnt = (k as u64).leading_zeros();
            let mut d = Dint::new((k as u64) << cnt, 0, big_g + 63 - i64::from(cnt), 1 - z.sgn);
            d = add_dint(z, &d);
            d.ex += 116;
            if cmp_dint_abs(&d, z) >= 0 {
                return None;
            }
        }
        if big_g > g {
            return None;
        }
        let sh = (g - big_g) as u32;
        let ku = k as u64;
        if ku & !((!1u64).wrapping_shl(sh)) == 1u64.wrapping_shl(sh) {
            let r = (k.wrapping_shr(sh)).wrapping_mul(s) as f64;
            return Some(pow2(r, g));
        }
        return None;
    }
    let mut t = n << f;
    let mut k: i64 = 1;
    let mut mm = m;
    while t != 0 {
        if t & 1 != 0 {
            let p = i128::from(mm) * i128::from(k);
            if p > i128::from(i64::MAX) || p < i128::from(i64::MIN) {
                return None;
            }
            k = p as i64;
        }
        t >>= 1;
        if t != 0 {
            mm = mm.checked_mul(mm)?;
        }
    }
    if k >> 54 != 0 {
        return None;
    }
    let r = k.wrapping_mul(s) as f64;
    let g = e.wrapping_mul((n << f) as i64);
    Some(pow2(r, g))
}

/// Whether `x^y` is exact (and representable): the C uses it for flags and to skip one test.
fn is_exact(x: f64, y: f64) -> bool {
    let (vu, wu) = (x.to_bits(), y.to_bits());
    if (vu << 1) != 0x7fe0_0000_0000_0000 && (wu << 22) != 0 {
        return false;
    }
    if (vu << 1) == 0x7fe0_0000_0000_0000 {
        return true;
    }
    const XMAX: [u64; 34] = [
        0,
        0xffff_ffff_ffff_ffff,
        94_906_265,
        208_063,
        9741,
        1551,
        455,
        189,
        97,
        59,
        39,
        27,
        21,
        15,
        13,
        11,
        9,
        7,
        7,
        5,
        5,
        5,
        5,
        4,
        3,
        3,
        3,
        3,
        3,
        3,
        3,
        3,
        3,
        3,
    ];
    let split = |u: u64| {
        let mut m = u & 0xf_ffff_ffff_ffff;
        let mut e = ((u << 1) >> 53) as i64 - 0x433;
        if e >= -1074 {
            m |= 0x10_0000_0000_0000;
        } else {
            e += 1;
        }
        let t = m.trailing_zeros();
        (m >> t, e + i64::from(t))
    };
    if y >= 0.0 && is_int(y) {
        let (m, e) = split(vu);
        if y == 0.0 || y == 1.0 {
            return true;
        }
        if m == 1 {
            return -1074.0 <= y * e as f64 && y * (e as f64) < 1024.0;
        }
        if !(0.0..=33.0).contains(&y) {
            return false;
        }
        let y_int = y as i32;
        if m > XMAX[y_int as usize] {
            return false;
        }
        let t = 64 - i64::from(m.leading_zeros());
        let ez = e * i64::from(y_int) + t;
        if ez <= -1074 || 1024 < ez {
            return false;
        }
        return e * i64::from(y_int) >= -1074;
    }
    let (n, mut f) = split(wu);
    let (mut m, mut e) = split(vu);
    if y < 0.0 {
        if m != 1 {
            return false;
        }
        let ez: i64 = if f >= 0 {
            if f < 12 {
                (n.wrapping_neg().wrapping_mul(e as u64) << f) as i64
            } else {
                1024
            }
        } else {
            let t = (e as u64).trailing_zeros();
            if -f > i64::from(t) {
                return false;
            }
            ((-e >> (-f)) as u64).wrapping_mul(n) as i64
        };
        return (-1074..1024).contains(&ez);
    }
    while f != 0 {
        f += 1;
        if e & 1 != 0 {
            return false;
        }
        e /= 2;
        let dm = m as f64;
        let s = dm.sqrt().round();
        if s * s != dm {
            return false;
        }
        m = s as u64;
    }
    if m > 1 {
        if 33 < n {
            return false;
        }
        if m > XMAX[n as usize] {
            return false;
        }
    }
    let mut my = m;
    let mut n0 = n;
    while n0 > 1 {
        n0 -= 1;
        my = my.wrapping_mul(m);
    }
    let t = 64 - i64::from(my.leading_zeros());
    let en = e * i64::from(n as i32);
    -1074 <= en && en + t <= 1024
}

/// Correctly rounded `x^y` (C `pow`), rounding to nearest.
///
/// Not in the C: `y == 2` and `y == 1` return before the general path, which gives the same
/// bits. `x * x` is the correctly rounded square (one IEEE multiplication, overflow and the
/// subnormal range included) and `x` is exactly `x^1`; CORE-MATH's `pow`, which C Graphviz runs
/// too, is correctly rounded, so both agree with it, and its phase 1 already returns `x * x` and
/// `s * x` for these exponents when it cannot decide the rounding. NaN `x` stays on the general
/// path, which returns `x + x` (quieting a signaling NaN). sfdp's default `p = -1` makes every
/// repulsive force a `pow(dist, 2)`. `tests::fast_paths_match_general` checks the bits.
#[inline]
#[must_use]
pub fn pow(x: f64, y: f64) -> f64 {
    if !x.is_nan() {
        if y == 2.0 {
            return x * x;
        }
        if y == 1.0 {
            return x;
        }
    }
    pow_general(x, y)
}

/// CORE-MATH's `cr_pow`: [`pow`] without its fast paths.
fn pow_general(x: f64, y: f64) -> f64 {
    let mut s = 1.0;
    let x0 = x;
    let mut x = x;
    let xu = x.to_bits();
    let yu = y.to_bits();

    if xu >= 0x7ff0_0000_0000_0000 || yu >= 0x7ff0_0000_0000_0000 {
        if x.is_nan() {
            if y == 0.0 && !is_signaling(x) {
                return 1.0;
            }
            return x + x;
        }
        if y.is_nan() {
            if x == 1.0 && !is_signaling(y) {
                return 1.0;
            }
            return y + y;
        }
        match xu {
            0x7ff0_0000_0000_0000 => {
                if y == 0.0 {
                    return 1.0;
                }
                if y < 0.0 {
                    return 0.0;
                }
                if y > 0.0 {
                    return f64::INFINITY;
                }
            }
            0xfff0_0000_0000_0000 => {
                if yu == 0x7ff0_0000_0000_0000 {
                    return y;
                }
                if yu == 0xfff0_0000_0000_0000 {
                    return 0.0;
                }
                if is_int(y) && !is_int(y * 0.5) {
                    return if y < 0.0 { -0.0 } else { f64::NEG_INFINITY };
                }
                if y < 0.0 {
                    return 0.0;
                }
                if y > 0.0 {
                    return f64::INFINITY;
                }
            }
            _ => {}
        }
        match yu {
            0x7ff0_0000_0000_0000 => {
                if x == 0.0 {
                    return 0.0;
                }
                if x == -1.0 || x == 1.0 {
                    return 1.0;
                }
                if -1.0 < x && x < 1.0 {
                    return 0.0;
                }
                if x < -1.0 || 1.0 < x {
                    return f64::INFINITY;
                }
            }
            0xfff0_0000_0000_0000 => {
                if x == 0.0 {
                    return f64::INFINITY;
                }
                if x == -1.0 || x == 1.0 {
                    return 1.0;
                }
                if -1.0 < x && x < 1.0 {
                    return f64::INFINITY;
                }
                if x < -1.0 || 1.0 < x {
                    return 0.0;
                }
            }
            _ => {}
        }
    }

    if x <= 0.0 {
        if y == 0.0 {
            return 1.0;
        }
        match xu {
            0 => {
                if is_int(y) && !is_int(y * 0.5) {
                    return if y < 0.0 { f64::INFINITY } else { 0.0 };
                }
                return if y > 0.0 { 0.0 } else { f64::INFINITY };
            }
            0x8000_0000_0000_0000 => {
                if is_int(y) && !is_int(y * 0.5) {
                    return if y < 0.0 { f64::NEG_INFINITY } else { -0.0 };
                }
                return if y > 0.0 { 0.0 } else { f64::INFINITY };
            }
            _ => {}
        }
        if !is_int(y) {
            return f64::NAN;
        }
        let y_parity = if y.abs() >= f64::from_bits(0x4340_0000_0000_0000) {
            0 // |y| >= 2^53
        } else {
            (y as i64) & 1
        };
        s = if y_parity != 0 { -1.0 } else { 1.0 };
        x = -x;
    }

    // Phase 1 (Algorithm phase_1 of the ARITH 2023 paper).
    let (mut lh, mut ll, cancel) = log_1(x);
    let ey = (yu >> 52) & 0x7ff;
    if !(0x36..0x7f5).contains(&ey) {
        lh = f64::NAN;
        ll = f64::NAN;
    }
    let (rh, rl) = s_mul(y, lh, ll);
    let (res_h, res_l) = exp_1(rh, rl, s);
    const ERR: [f64; 2] = [
        f64::from_bits(0x3bf2_7000_0000_0000), // 0x1.27p-64
        f64::from_bits(0x3c55_7000_0000_0000), // 0x1.57p-58
    ];
    let res_min = res_h + fma(ERR[cancel], -res_h, res_l);
    let res_max = res_h + fma(ERR[cancel], res_h, res_l);
    let exact = is_exact(x, y);
    if res_min == res_max {
        return res_max;
    }
    if y == 1.0 {
        return s * x;
    }
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        return x.sqrt();
    }
    if y == 0.0 {
        return 1.0;
    }

    // Phase 2.
    let mut xd = dint_fromd(x);
    let yd = dint_fromd(y);
    xd.sgn = 0;
    let mut r = log_2(&xd);
    r = mul_dint_21(&r, &yd);
    r = exp_2(&r);
    if r.ex < -1075 {
        return 0.5 * (s * f64::from_bits(1));
    }
    let rd = if r.ex < -1022 {
        let ex = (-(1022 + r.ex)) as u64;
        let m = (r.lo >> (10 + ex)) | (r.hi << (54 - ex));
        m.wrapping_add(14) > 2 * 14
    } else {
        const ERR_BND_2: u64 = 28;
        let lo = (r.lo >> 10) | (r.hi << 54);
        lo.wrapping_add(ERR_BND_2) > 2 * ERR_BND_2
    };
    r.sgn = u64::from(s == -1.0);
    if rd {
        return dint_tod(&r);
    }
    if let Some(e) = exact_pow(x0, y, &r, exact) {
        return e;
    }

    // Phase 3 (hard-to-round cases).
    let mut qx = qint_fromd(x);
    let qy = qint_fromd(y);
    qx.sgn = 0;
    let mut qr = log_3(&qx);
    qr = mul_qint_41(&qr, &qy);
    let mut qz = exp_3(&qr);
    const ERR_BND_3: u64 = 60;
    let r1 = (qz.hh << 54) | (qz.hl >> 10);
    let r2 = (qz.hl << 54) | (qz.lh >> 10);
    let r3 = (qz.lh << 54) | (qz.ll >> 10);
    let rd = !((r1 == 0 && r2 == 0 && r3 <= ERR_BND_3)
        || (!r1 == 0 && !r2 == 0 && r3.wrapping_add(2 * ERR_BND_3) <= ERR_BND_3));
    if rd {
        qz.sgn = u64::from(s == -1.0);
        qz.ll &= !0u64 << 10;
        return qint_tod(&mut qz);
    }
    if qr.ex < -56 {
        // 1 +/- 2^-100 rounds to 1.
        return if qr.sgn == 0 {
            1.0 + f64::from_bits(0x39b0_0000_0000_0000)
        } else {
            1.0 - f64::from_bits(0x39b0_0000_0000_0000)
        };
    }
    // The C reports an unexpected worst case and exits here; none is known. Return the
    // 256-bit approximation rounded, which is correct unless it is one.
    qz.sgn = u64::from(s == -1.0);
    qint_tod(&mut qz)
}

#[cfg(test)]
mod tests;
