//! Correctly rounded `cbrt`, CORE-MATH's `src/binary64/cbrt/cbrt.c` (Alexei Sibidanov,
//! Copyright (c) 2021-2022 Alexei Sibidanov, MIT licence; <https://core-math.gitlabpages.inria.fr/>)
//! as the `libm` crate 0.2.16 ports it (`src/math/cbrt.rs`, Trevor Gross, MIT), cut down to
//! rounding to nearest, the only mode Rust has.
//!
//! The `libm` crate is the reference: this file is its `cbrt_round(x, Round::Nearest)` with the
//! same operations on the same operands in the same order. The one change is how each fused
//! multiply-add is reached. On x86-64 the crate calls its `fma` through a pointer chosen at run
//! time from the CPU features, so every call is an indirect call that spills the live vector
//! registers; this file calls [`crate::math::fma`], which is the `vfmadd` instruction on
//! x86-64-v3 and the exact emulated fma elsewhere. Both are correctly rounded fused
//! multiply-adds, so they return the same bits for the same operands, and so does this
//! function. `math::tests` compares it with `libm::cbrt` bit for bit, on every target.
//!
//! It is `math::cbrt` only where `fma` is a target feature. Without it, `math::fma` is the
//! emulated fma, slower than the hardware `fma` the crate's dispatch finds on most CPUs (3% more
//! instructions on dot's routing-heavy graphs), so `math::cbrt` stays `libm::cbrt` there.
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
    clippy::unreadable_literal,
    clippy::many_single_char_names,
    clippy::similar_names
)]

use crate::math::fma;

const SIG_BITS: u32 = 52;
const SIG_MASK: u64 = (1 << SIG_BITS) - 1;
const SIGN_MASK: u64 = 1 << 63;
const EXP_SAT: u32 = 0x7ff;

/// `f64::from_bits`, for the hexadecimal constants (written beside each one).
const fn bits(b: u64) -> f64 {
    f64::from_bits(b)
}

/// 1, 2^(1/3), 2^(2/3).
const ESCALE: [f64; 3] = [
    1.0,
    bits(0x3ff428a2f98d728b), // 0x1.428a2f98d728bp+0
    bits(0x3ff965fea53d6e3d), // 0x1.965fea53d6e3dp+0
];

/// `c0 + c1 x + c2 x^2 + c3 x^3` approximates x^(1/3) on [1, 2] with error < 9.2e-5.
const C: [f64; 4] = [
    bits(0x3fe1b0babccfef9c), // 0x1.1b0babccfef9cp-1
    bits(0x3fe2c9a3e94d1da5), // 0x1.2c9a3e94d1da5p-1
    bits(0xbfc4dc30b1a1ddba), // -0x1.4dc30b1a1ddbap-3
    bits(0x3f97a8d3e4ec9b07), // 0x1.7a8d3e4ec9b07p-6
];

const U0: f64 = bits(0x3fd5555555555555); // 0x1.5555555555555p-2
const U1: f64 = bits(0x3fcc71c71c71c71c); // 0x1.c71c71c71c71cp-3

const RSC: [f64; 6] = [1.0, -1.0, 0.5, -0.5, 0.25, -0.25];

/// `off[Round::Nearest]`.
const OFF: f64 = bits(0x3ca0000000000000); // 0x1p-53
const P52: f64 = bits(0x3cb0000000000000); // 0x1p-52
const P75: f64 = bits(0x3b40000000000000); // 0x1p-75
const P98: f64 = bits(0x39d0000000000000); // 0x1p-98
const P60: f64 = bits(0x3c30000000000000); // 0x1p-60

/// The cube root of `x`, correctly rounded.
#[must_use]
#[cfg_attr(not(target_feature = "fma"), allow(dead_code))]
pub fn cbrt(x: f64) -> f64 {
    let hx: u64 = x.to_bits();
    let mut mant: u64 = hx & SIG_MASK;
    let sign: u64 = hx >> 63;

    let mut e: u32 = (hx >> SIG_BITS) as u32 & EXP_SAT;

    if ((e + 1) & EXP_SAT) < 2 {
        let ix: u64 = hx & !SIGN_MASK;

        // 0, inf, nan: x + x, as in the original.
        if e == EXP_SAT || ix == 0 {
            return x + x;
        }

        let nz = ix.leading_zeros() - 11; // subnormal
        mant <<= nz;
        mant &= SIG_MASK;
        e = e.wrapping_sub(nz - 1);
    }

    e = e.wrapping_add(3072);
    let cvt1: u64 = mant | (0x3ffu64 << 52);
    let mut cvt5: u64 = cvt1;

    let et: u32 = e / 3;
    let it: u32 = e % 3;

    // 2^(3k+it) <= x < 2^(3k+it+1), with 0 <= it <= 3
    cvt5 += u64::from(it) << SIG_BITS;
    cvt5 |= sign << 63;
    let zz: f64 = f64::from_bits(cvt5);

    // cbrt(x) = cbrt(zz)*2^(et-1365) where 1 <= zz < 8
    let mut isc: u64 = ESCALE[it as usize].to_bits();
    isc |= sign << 63;
    let cvt2: u64 = isc;
    let z: f64 = f64::from_bits(cvt1);

    // cbrt(zz) = cbrt(z)*isc, where isc encodes 1, 2^(1/3) or 2^(2/3), and 1 <= z < 2
    let r: f64 = 1.0 / z;
    let rr: f64 = r * RSC[((it as usize) << 1) | sign as usize];
    let z2: f64 = z * z;
    let c0: f64 = C[0] + z * C[1];
    let c2: f64 = C[2] + z * C[3];
    let mut y: f64 = c0 + z2 * c2;
    let mut y2: f64 = y * y;

    // y is an approximation of z^(1/3)
    let mut h: f64 = y2 * (y * r) - 1.0;

    // h determines the error between y and z^(1/3)
    y -= (h * y) * (U0 - U1 * h);

    // The correction y -= (h*y)*(u0 - u1*h) corresponds to a cubic variant of Newton's
    // method, with the function f(y) = 1-z/y^3.
    y *= f64::from_bits(cvt2);

    // Now y is an approximation of zz^(1/3), and rr an approximation of 1/zz. We now perform
    // another iteration of Newton-Raphson, this time with a linear approximation only.
    y2 = y * y;
    let mut y2l: f64 = fma(y, y, -y2);

    // y2 + y2l = y^2 exactly
    let mut y3: f64 = y2 * y;
    let mut y3l: f64 = fma(y, y2, -y3) + y * y2l;

    // y3 + y3l approximates y^3 with about 106 bits of accuracy
    h = ((y3 - zz) + y3l) * rr;
    let mut dy: f64 = h * (y * U0);

    // the approximation of zz^(1/3) is y - dy
    let mut y1: f64 = y - dy;
    dy = (y - y1) - dy;

    // the approximation of zz^(1/3) is now y1 + dy, where |dy| < 1/2 ulp(y)
    let mut ady: f64 = dy.abs();

    // ady0 is tiny when dy is near 1/2 ulp(1), or near 3/2 ulp(1).
    let mut ady0: f64 = (ady - OFF).abs();
    let mut ady1: f64 = (ady - (P52 + OFF)).abs();

    if ady0 < P75 || ady1 < P75 {
        y2 = y1 * y1;
        y2l = fma(y1, y1, -y2);
        y3 = y2 * y1;
        y3l = fma(y1, y2, -y3) + y1 * y2l;
        h = ((y3 - zz) + y3l) * rr;
        dy = h * (y1 * U0);
        y = y1 - dy;
        dy = (y1 - y) - dy;
        y1 = y;
        ady = dy.abs();
        ady0 = (ady - OFF).abs();
        ady1 = (ady - (P52 + OFF)).abs();

        if ady0 < P98 || ady1 < P98 {
            let azz: f64 = zz.abs();

            // ~ 0x1.79d15d0e8d59b80000000000000ffc3dp+0
            if azz == bits(0x4009b78223aa307c) {
                // 0x1.9b78223aa307cp+1
                y1 = bits(0x3ff79d15d0e8d59c).copysign(zz); // 0x1.79d15d0e8d59cp+0
            }

            // ~ 0x1.de87aa837820e80000000000001c0f08p+0
            if azz == bits(0x401a202bfc89ddff) {
                // 0x1.a202bfc89ddffp+2
                y1 = bits(0x3ffde87aa837820f).copysign(zz); // 0x1.de87aa837820fp+0
            }
        }
    }

    let mut cvt3: u64 = y1.to_bits();
    cvt3 = cvt3.wrapping_add(u64::from(et.wrapping_sub(342).wrapping_sub(1023)) << 52);
    let m0: u64 = cvt3 << 30;
    let m1 = m0 >> 63;

    if (m0 ^ m1) <= (1u64 << 30) {
        let mut cvt4: u64 = y1.to_bits();
        cvt4 = (cvt4 + (164 << 15)) & 0xffffffffffff0000u64;

        if ((f64::from_bits(cvt4) - y1) - dy).abs() < P60 || (zz).abs() == 1.0 {
            cvt3 = (cvt3 + (1u64 << 15)) & 0xffffffffffff0000u64;
        }
    }

    f64::from_bits(cvt3)
}
