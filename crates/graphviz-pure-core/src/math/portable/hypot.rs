//! `hypot` as macOS arm64 computes it: `sqrt(fma(small, small, big * big))`, with exact
//! power-of-two scaling outside the range where the squares can overflow or underflow.
//!
//! Apple's source is not published; this formula was identified by comparing candidates with
//! the system `hypot` on the arguments the corpus produces and on random inputs (see
//! `docs/porting/libm.md`). It is not correctly rounded: it disagrees with the correctly
//! rounded result on about 16% of the corpus's calls, and Rust's wasm32 `hypot` is correctly
//! rounded, which is why the wasm build drifted.

/// `2^e` for `e` in the normal exponent range.
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

pub fn hypot(x: f64, y: f64) -> f64 {
    let (mut big, mut small) = (x.abs(), y.abs());
    if big.is_infinite() || small.is_infinite() {
        return f64::INFINITY;
    }
    if big.is_nan() || small.is_nan() {
        return x + y;
    }
    if big < small {
        std::mem::swap(&mut big, &mut small);
    }
    if small == 0.0 {
        return big;
    }
    // Squares of numbers in [2^-500, 2^500] stay normal, and scaling by a power of two
    // commutes with rounding there, so the scaled path gives the same bits wherever the
    // direct one does not overflow or underflow.
    if big < pow2(500) && small > pow2(-500) {
        return super::fma(small, small, big * big).sqrt();
    }
    // Scale `big` into [1, 2). `small` may become subnormal or zero; its square is then
    // below half an ulp of `big * big` and cannot change the sum.
    let e = ((big.to_bits() >> 52) as i32) - 1023;
    let e = if e == -1023 {
        // Subnormal `big`: take its exponent after normalizing.
        (((big * pow2(54)).to_bits() >> 52) as i32) - 1023 - 54
    } else {
        e
    };
    let (half, rest) = (e / 2, e - e / 2);
    let (b, s) = (
        big * pow2(-half) * pow2(-rest),
        small * pow2(-half) * pow2(-rest),
    );
    super::fma(s, s, b * b).sqrt() * pow2(half) * pow2(rest)
}
