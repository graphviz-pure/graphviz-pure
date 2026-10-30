//! C's numeric idioms as the reference build (Graphviz 16.1.0, clang on arm64, macOS libc)
//! evaluates them. Rust's own operations differ at NaN, at the ends of the integer ranges
//! and in formatting, so a port of one of these idioms uses the helper here, not the Rust
//! operation that looks the same:
//!
//! | C | Rust look-alike | Difference | Here |
//! |---|---|---|---|
//! | `MIN(a,b)`, `MAX(a,b)` (`arith.h` ternaries) | `f64::min`/`f64::max` | C's result is `b` when the test is false, so a NaN in `b`, or in `a` against any `b`, is kept or chosen by position; Rust drops a NaN | [`c_min`], [`c_max`] |
//! | `is_exactly_zero`, `is_exactly_equal` (`gv_math.h`) | `==` | C compares bits: `-0.0` is not zero, a NaN equals itself | [`is_exactly_zero`], [`is_exactly_equal`] |
//! | `ROUND(f)` (`arith.h`) | `f.round() as i64` | C converts to `int` | [`c_round`] |
//! | `ROUND(f)` vectorized by clang | | the conversion goes through 64 bits and keeps the low 32 | [`c_round_low32`] |
//! | `(int)d`, `(unsigned)d` | — | none: arm64 `fcvtzs`/`fcvtzu` saturate and give 0 for NaN, as Rust's `as` does; the port only has to convert to the C type | `d as i32`, `d as u32` |
//! | `strtol(s, &end, 10)` | `str::parse::<i64>` | C skips white space, reads the longest prefix and saturates at `LONG_MIN`/`LONG_MAX` | [`strtol`] |
//! | `atoi(s)` | — | `(int)strtol(s, NULL, 10)`: the saturated `long`, truncated to its low 32 bits | [`atoi`] |
//! | `sscanf(s, "%d", &i)` | — | as `atoi`, but tells whether there were digits | [`scan_d`] |
//! | `late_int` (`utils.c`) | — | built on `strtol` | [`late_int`] |
//! | `mapBool` (`utils.c`) | — | numbers go through `atoi` | [`map_bool`] |
//! | `printf("%.Nf")` | `format!("{:.N}")` | C prints `nan`, `inf`, `-inf`; Rust `NaN`, `inf`, `-inf` | [`fmt_f`] |
//! | `printf("%.Ng")` | — | | [`fmt_g`] |
//!
//! `int` arithmetic that C does not check wraps in the reference build; the port uses
//! `wrapping_*` there (see `docs/porting/crashes.md`, "C's own overflow checks").

use crate::math;
use std::fmt::Write as _;

/// C's `printf("%.{prec}g", v)` on the reference platform (macOS libc), as diagnostics and
/// renderers print numbers.
pub fn fmt_g(v: f64, prec: usize) -> String {
    let mut s = String::new();
    fmt_g_into(&mut s, v, prec);
    s
}

/// C's `printf("%.{prec}f", v)` on the macOS libc, written into `out`.
///
/// The digits are the ones [`fmt_f`] prints; this writes them into the caller's buffer
/// instead of allocating a `String` per number. Returns the writer's error; writing into a
/// `String` never fails.
///
/// The digits come from the fixed-point path ([`fixed_into_buf`]), which rounds the *exact*
/// value of the double, half to even, as `format!("{v:.prec$}")` does; a value outside the
/// path (a precision above [`FIXED_MAX_PREC`], or a value whose scaled integer does not fit
/// in a `u64`) goes to the generic formatter, which prints the same digits.
pub fn fmt_f_into<W: std::fmt::Write>(out: &mut W, v: f64, prec: usize) -> std::fmt::Result {
    if v.is_nan() {
        // C prints `nan` whatever the sign.
        return out.write_str("nan");
    }
    if v.is_infinite() {
        return out.write_str(if v < 0.0 { "-inf" } else { "inf" });
    }
    let mut buf = [0u8; FIXED_BUF];
    if let Some(start) = fixed_into_buf(&mut buf, v, prec) {
        // The buffer holds a sign, digits and a point: ASCII.
        let text = std::str::from_utf8(&buf[start..]).map_err(|_| std::fmt::Error)?;
        return out.write_str(text);
    }
    write!(out, "{v:.prec$}")
}

/// The most decimals the fixed-point path prints.
///
/// The `u128` the path scales with is not what bounds this: `m·10^prec` still fits it at
/// precision 22 (the mantissa is 53 bits and `10^22` is 74 bits, so the numerator is 127),
/// and it stops fitting at 23. The bound is what the writers need — the precisions they
/// print with are 0, 2, 3, 4 and 6 — and, for the value, the other limit: the rounded
/// scaled integer has to fit in a `u64`, which is what sends a large value to the generic
/// formatter.
const FIXED_MAX_PREC: usize = 6;

/// The buffer the fixed-point path writes into: a sign, the digits of the integer part (at
/// most 20), a point and the decimals (at most [`FIXED_MAX_PREC`]).
const FIXED_BUF: usize = 48;

/// The powers of ten the fixed-point path scales by.
const FIXED_POW10: [u128; FIXED_MAX_PREC + 1] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];

/// C's `printf("%.{prec}f", v)` for the values the fixed-point path covers, written into
/// `buf` backwards from its end; the text is `buf[start..]`.
///
/// The double is exactly `±m·2^e`, so the scaled value `|v|·10^prec` is the exact rational
/// `m·10^prec·2^e`: an integer when `e ≥ 0`, and `n/2^k` with `n = m·10^prec` when `e < 0`.
/// Rounding that rational to an integer is integer arithmetic — no floating point is
/// involved — and it is what `format!("{v:.prec$}")` does: the exact decimal expansion of the
/// double, rounded half to even. (A tie is a value whose scaled form ends in exactly a half,
/// which a double has at the odd multiples of `2^-3` for two decimals and of `2^-4` for
/// three.)
///
/// `None` when the value is outside the path: a precision above [`FIXED_MAX_PREC`], or a
/// value whose scaled integer does not fit in a `u64` (a large value, whose exact decimal
/// expansion the generic formatter prints).
fn fixed_into_buf(buf: &mut [u8; FIXED_BUF], v: f64, prec: usize) -> Option<usize> {
    if prec > FIXED_MAX_PREC {
        return None;
    }
    // The double's exact value: `±m·2^e`, with `m` the mantissa and `e` its exponent.
    let bits = v.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if biased == 0 {
        // A subnormal (zero too): the mantissa has no hidden bit.
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), biased - 1075)
    };
    // The scaled value: `m·10^prec·2^e`.
    let n = u128::from(m) * FIXED_POW10[prec];
    let q: u64 = if e >= 0 {
        // Already an integer: `n·2^e` must fit in a `u64`.
        if e > 63 {
            return None;
        }
        let shift = e as u32;
        if n > u128::from(u64::MAX >> shift) {
            return None;
        }
        (n << shift) as u64
    } else {
        let k = -e;
        if k >= 128 {
            // `n/2^k` is far below a half (the numerator is below `2^74`), so it rounds to
            // zero; a tie is impossible here.
            0
        } else {
            let d = 1u128 << k;
            let mut q = n >> k;
            let r = n & (d - 1);
            let twice = r << 1;
            // Half to even: a tie goes to the even neighbour.
            if twice > d || (twice == d && q & 1 == 1) {
                q += 1;
            }
            if q > u128::from(u64::MAX) {
                return None;
            }
            q as u64
        }
    };
    // The digits: the fraction, the point, the integer part and the sign, written backwards
    // from the end of the buffer.
    let pow = FIXED_POW10[prec] as u64;
    let mut start = buf.len();
    let mut frac = q % pow;
    for _ in 0..prec {
        start -= 1;
        buf[start] = b'0' + (frac % 10) as u8;
        frac /= 10;
    }
    if prec > 0 {
        start -= 1;
        buf[start] = b'.';
    }
    let mut int = q / pow;
    loop {
        start -= 1;
        buf[start] = b'0' + (int % 10) as u8;
        int /= 10;
        if int == 0 {
            break;
        }
    }
    if v.is_sign_negative() {
        // C prints the sign of a negative zero too.
        start -= 1;
        buf[start] = b'-';
    }
    Some(start)
}

/// C's `printf("%.{prec}g", v)` on the macOS libc, written into `out`.
///
/// The digits are the ones [`fmt_g`] prints; this writes them into the caller's buffer
/// instead of allocating a `String` per number. The trailing zeros of the mantissa go, so
/// the text is trimmed in place, which needs a `String` to trim.
pub fn fmt_g_into(out: &mut String, v: f64, prec: usize) {
    if v.is_nan() {
        // macOS libc prints `nan` for either sign.
        out.push_str("nan");
        return;
    }
    if v.is_infinite() {
        out.push_str(if v < 0.0 { "-inf" } else { "inf" });
        return;
    }
    let p = prec.max(1);
    if v == 0.0 {
        out.push_str(if v.is_sign_negative() { "-0" } else { "0" });
        return;
    }
    // The exponent after rounding to `p` significant digits. Rust's `LowerExp` writes it as
    // `d.dddde<exp>`; C writes the exponent with a sign and two digits. The exponent is read
    // as the code before this one read it: the text after the last `e`, and 0 when there is
    // no `e` or it does not parse. Rust's `LowerExp` always writes one, so the fallback is
    // the fixed form, as it was.
    let start = out.len();
    let _ = write!(out, "{:.*e}", p - 1, v);
    let epos = out[start..].rfind('e').map(|p| start + p);
    let exp: i32 = epos.and_then(|p| out[p + 1..].parse().ok()).unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        // The mantissa is what came before the `e`; with no `e` it is the whole text.
        let epos = epos.unwrap_or(out.len());
        out.truncate(epos);
        trim_zeros_from(out, start);
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        let _ = write!(out, "{:02}", exp.unsigned_abs());
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        out.truncate(start);
        let _ = write!(out, "{v:.decimals$}");
        trim_zeros_from(out, start);
    }
}

/// C's `%g`'s trimming: the trailing zeros after a decimal point, and the point itself.
/// Only a number with a decimal point is trimmed, as C's `agxbuf_trim_zeros` does.
fn trim_zeros_from(out: &mut String, start: usize) {
    if !out[start..].contains('.') {
        return;
    }
    let mut end = out.len();
    while end > start && out.as_bytes()[end - 1] == b'0' {
        end -= 1;
    }
    if end > start && out.as_bytes()[end - 1] == b'.' {
        end -= 1;
    }
    out.truncate(end);
}

/// C's `MIN(a,b)`: `a < b ? a : b`. With a NaN in either operand the test is false and the
/// result is `b`.
#[inline]
pub fn c_min<T: PartialOrd>(a: T, b: T) -> T {
    if a < b { a } else { b }
}

/// C's `MAX(a,b)`: `a > b ? a : b`. With a NaN in either operand the test is false and the
/// result is `b`.
#[inline]
pub fn c_max<T: PartialOrd>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

/// `is_exactly_equal(a, b)` (`lib/util/gv_math.h`): the bit patterns are the same
/// (`memcmp`), so `-0.0` differs from `0.0` and a NaN equals the same NaN.
#[inline]
pub fn is_exactly_equal(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

/// `is_exactly_zero(v)`: `is_exactly_equal(v, 0)`, false for `-0.0`.
#[inline]
pub fn is_exactly_zero(v: f64) -> bool {
    is_exactly_equal(v, 0.0)
}

/// C's `ROUND(f)`: `f >= 0 ? (int)(f + .5) : (int)(f - .5)`. The conversion saturates at
/// `INT_MIN`/`INT_MAX`, and a NaN gives 0, as arm64's `fcvtzs` does.
#[inline]
pub fn c_round(f: f64) -> i32 {
    if f >= 0.0 {
        (f + 0.5) as i32
    } else {
        (f - 0.5) as i32
    }
}

/// C's `ROUND(a * b)` as clang compiles it: the sign test reads the rounded product, and
/// `a * b + .5` / `a * b - .5` are fused into one fma each. `POINTS(x)` is
/// `c_round_mul(x, 72.0)`.
#[inline]
pub fn c_round_mul(a: f64, b: f64) -> i32 {
    if a * b >= 0.0 {
        math::fma(a, b, 0.5) as i32
    } else {
        math::fma(a, b, -0.5) as i32
    }
}

/// C's `ROUND(f)` where clang converts through 64 bits (`fcvtzs` into an `x` register, or
/// the vector `fcvtzs.2d`) and keeps the low 32 bits: the `i64` conversion saturates and a
/// NaN gives 0, then the result wraps. The reference build does this where it vectorizes
/// a pair of `ROUND`s, as in `init_job_pagination`'s `job->width`/`job->height`. Which
/// conversion a site gets is read from the reference build's disassembly.
#[inline]
pub fn c_round_low32(f: f64) -> i32 {
    let r = if f >= 0.0 { f + 0.5 } else { f - 0.5 };
    r as i64 as i32
}

/// C's `isspace` in the C locale.
pub fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `late_double` (`lib/common/utils.c`) on a value: `default` when it is unset, empty or
/// no number, `minimum` below it.
pub fn late_double(p: Option<&str>, default: f64, minimum: f64) -> f64 {
    match p
        .filter(|s| !s.is_empty())
        .and_then(crate::strtod::strtod_rest)
    {
        None => default,
        Some((v, _)) if v < minimum => minimum,
        Some((v, _)) => v,
    }
}

/// `GD_drawing(g)->dpi` as `graph_init` sets it: `late_double(g, dpi,
/// late_double(g, resolution, 0, 0), 0)`.
pub fn drawing_dpi(graph_attrs: &crate::AttrMap) -> f64 {
    let attr = |k: &str| graph_attrs.get(k).map(String::as_str);
    late_double(attr("dpi"), late_double(attr("resolution"), 0.0, 0.0), 0.0)
}

/// C's `strtol(s, &end, 10)`: the value and the number of bytes read (`end - s`), or
/// `(0, 0)` when there are no digits. Leading white space and a sign are skipped; a value
/// beyond the range of `long` gives `LONG_MAX`, or `LONG_MIN` with a minus sign.
pub fn strtol(s: &str) -> (i64, usize) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && is_c_space(b[i]) {
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let start = i;
    // The magnitude, accumulated past i64 so that `LONG_MIN` itself reads exactly.
    let mut v: u64 = 0;
    let mut over = false;
    while i < b.len() && b[i].is_ascii_digit() {
        match v
            .checked_mul(10)
            .and_then(|m| m.checked_add(u64::from(b[i] - b'0')))
        {
            Some(m) => v = m,
            None => over = true,
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    let limit = if neg {
        i64::MIN.unsigned_abs()
    } else {
        i64::MAX as u64
    };
    if over || v > limit {
        return (if neg { i64::MIN } else { i64::MAX }, i);
    }
    let v = if neg {
        0i64.wrapping_sub_unsigned(v)
    } else {
        v as i64
    };
    (v, i)
}

/// C's `strtoul(s, &end, base)` for `base` 10, or 0 (a `0x` prefix reads hexadecimal and a
/// leading `0` octal): the value and the number of bytes read, or `(0, 0)` when there are no
/// digits. Leading white space and a sign are skipped; a `-` negates the value modulo 2^64,
/// and a value beyond `ULONG_MAX` gives `ULONG_MAX`, whatever the sign.
pub fn strtoul(s: &str, base: u32) -> (u64, usize) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && is_c_space(b[i]) {
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut radix = if base == 0 { 10 } else { base };
    if base == 0 && b.get(i) == Some(&b'0') {
        if b.get(i + 1).is_some_and(|c| c | 0x20 == b'x')
            && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            radix = 16;
            i += 2;
        } else {
            radix = 8;
        }
    }
    let start = i;
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = b.get(i).and_then(|&c| char::from(c).to_digit(radix)) {
        match v
            .checked_mul(u64::from(radix))
            .and_then(|m| m.checked_add(u64::from(d)))
        {
            Some(m) => v = m,
            None => over = true,
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    if over {
        return (u64::MAX, i);
    }
    (if neg { v.wrapping_neg() } else { v }, i)
}

/// C's `atoi(s)`, which the macOS libc implements as `(int)strtol(s, NULL, 10)`: the low 32
/// bits of the saturated `long`. `"2147483648"` reads `INT_MIN`, `"4294967296"` 0, and a
/// number past `LONG_MAX` -1.
pub fn atoi(s: &str) -> i32 {
    strtol(s).0 as i32
}

/// One `sscanf` `%d` conversion at the start of `s`, or `None` when there are no digits.
/// The macOS libc converts with `strtol`'s saturation and stores the low 32 bits, as
/// [`atoi`] does.
pub fn scan_d(s: &str) -> Option<i32> {
    let (v, n) = strtol(s);
    (n > 0).then_some(v as i32)
}

/// `late_int` (`lib/common/utils.c`) on an attribute's value (`None` when the object has
/// no such attribute): the default for an absent or empty value, one with no digits or
/// one above `INT_MAX`; `minimum` for one below it.
pub fn late_int(p: Option<&str>, default: i32, minimum: i32) -> i32 {
    let Some(p) = p.filter(|p| !p.is_empty()) else {
        return default;
    };
    let (rv, used) = strtol(p);
    if used == 0 || rv > i64::from(i32::MAX) {
        return default;
    }
    if rv < i64::from(minimum) {
        minimum
    } else {
        rv as i32
    }
}

/// `mapBool(p, dflt)` (`lib/common/utils.c`): `false`/`no`, `true`/`yes` in any case, or a
/// number through `atoi`; otherwise `dflt`.
pub fn map_bool(p: Option<&str>, dflt: bool) -> bool {
    let Some(p) = p.filter(|p| !p.is_empty()) else {
        return dflt;
    };
    if p.eq_ignore_ascii_case("false") || p.eq_ignore_ascii_case("no") {
        return false;
    }
    if p.eq_ignore_ascii_case("true") || p.eq_ignore_ascii_case("yes") {
        return true;
    }
    if p.as_bytes()[0].is_ascii_digit() {
        return atoi(p) != 0;
    }
    dflt
}

/// `mapbool(p)`: `mapBool(p, false)`.
pub fn mapbool(p: Option<&str>) -> bool {
    map_bool(p, false)
}

/// C's `printf("%.{prec}f", v)` on the macOS libc: NaN prints `nan` whatever its sign,
/// infinities `inf` and `-inf`.
pub fn fmt_f(v: f64, prec: usize) -> String {
    let mut s = String::new();
    let _ = fmt_f_into(&mut s, v, prec);
    s
}

#[cfg(test)]
mod tests {
    /// The `fmt_f` the port printed before [`fmt_f_into`] existed, kept verbatim as the
    /// differential test's oracle: the same digits, each in its own `String`.
    fn fmt_f_oracle(v: f64, prec: usize) -> String {
        if v.is_nan() {
            "nan".to_string()
        } else if v.is_infinite() {
            if v > 0.0 { "inf" } else { "-inf" }.to_string()
        } else {
            format!("{v:.prec$}")
        }
    }

    /// The `%g` the port printed before [`fmt_g_into`] existed, kept verbatim as the
    /// differential test's oracle: the same digits, each in its own `String`.
    fn fmt_g_oracle(v: f64, prec: usize) -> String {
        fn trim(mut s: String) -> String {
            if s.contains('.') {
                while s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
            }
            s
        }
        if v.is_nan() {
            return "nan".to_string();
        }
        if v.is_infinite() {
            return if v < 0.0 { "-inf" } else { "inf" }.to_string();
        }
        let p = prec.max(1);
        if v == 0.0 {
            return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
        }
        let e_form = format!("{:.*e}", p - 1, v);
        let exp: i32 = e_form
            .rsplit_once('e')
            .and_then(|(_, e)| e.parse().ok())
            .unwrap_or(0);
        if exp < -4 || exp >= p as i32 {
            let (mant, _) = e_form.rsplit_once('e').unwrap_or((&e_form, "0"));
            let mant = trim(mant.to_string());
            let sign = if exp < 0 { '-' } else { '+' };
            format!("{mant}e{sign}{:02}", exp.abs())
        } else {
            let decimals = (p as i32 - 1 - exp).max(0) as usize;
            trim(format!("{v:.decimals$}"))
        }
    }

    /// The values whose scaled form is exactly a half, and their neighbours: a double's
    /// decimal expansion ends in 5 at the odd multiples of `1/2^(prec+1)` for `prec`
    /// decimals, and the doubles next to those are the values that round the other way.
    /// The precisions the writers use are 0, 2, 3, 4 and 6, so the ties are at the odd
    /// multiples of `1/2`, `1/8`, `1/16`, `1/32` and `1/128`.
    fn halfway_cases() -> Vec<f64> {
        let mut v = Vec::new();
        for k in 0..2_000 {
            for den in [2.0f64, 8.0, 16.0, 32.0, 128.0] {
                let h = (2 * k + 1) as f64 / den;
                v.push(h);
                v.push(-h);
                // The next double up and the next double down.
                v.push(f64::from_bits(h.to_bits() + 1));
                v.push(f64::from_bits(h.to_bits() - 1));
            }
        }
        v
    }

    /// The doubles the differential test walks: the edges, the powers of ten, the halfway
    /// cases and a deterministic spread of random ones.
    fn doubles() -> Vec<f64> {
        let mut v = vec![
            0.0,
            -0.0,
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            1.0,
            -1.0,
            0.005,
            -0.005,
            0.0001234,
            0.00001234,
            99999.5,
            123456.0,
            205.8,
            0.75,
            0.333333333,
        ];
        for e in -30i32..30 {
            let mut p = 1.0f64;
            for _ in 0..e.unsigned_abs() {
                p *= 10.0;
            }
            if e < 0 {
                p = 1.0 / p;
            }
            v.push(p);
            v.push(-p);
            v.push(p * 1.5);
            v.push(p * 9.9999);
        }
        // A deterministic spread of random doubles: a 64-bit xorshift, then the mantissa
        // and the exponent of a double out of the 64 bits.
        let mut s: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..20_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            v.push(f64::from_bits(s));
            v.push(f64::from_bits(s >> 11));
        }
        v.extend(halfway_cases());
        v
    }

    /// The prefixes a writer may find in the buffer: the text before the number must not be
    /// touched by the in-place trimming.
    const PREFIXES: [&str; 5] = ["0.", "10", "1.00", "-", " "];

    #[test]
    fn fmt_g_into_prints_what_the_old_fmt_g_printed() {
        for v in doubles() {
            for prec in 1..=17 {
                let want = fmt_g_oracle(v, prec);
                let mut got = String::new();
                fmt_g_into(&mut got, v, prec);
                assert_eq!(got, want, "fmt_g_into({v:?}, {prec})");
                assert_eq!(fmt_g(v, prec), want, "fmt_g({v:?}, {prec})");
            }
        }
    }

    #[test]
    fn fmt_f_into_prints_what_the_old_fmt_f_printed() {
        for v in doubles() {
            for prec in 0..=17 {
                let want = fmt_f_oracle(v, prec);
                let mut got = String::new();
                let _ = fmt_f_into(&mut got, v, prec);
                assert_eq!(got, want, "fmt_f_into({v:?}, {prec})");
                assert_eq!(fmt_f(v, prec), want, "fmt_f({v:?}, {prec})");
            }
        }
    }

    #[test]
    fn fmt_f_into_appends_after_a_prefix() {
        for v in doubles() {
            for prec in [0, 1, 2, 6, 17] {
                let want = fmt_f_oracle(v, prec);
                for prefix in PREFIXES {
                    let mut s = String::from(prefix);
                    let _ = fmt_f_into(&mut s, v, prec);
                    assert_eq!(
                        s,
                        format!("{prefix}{want}"),
                        "fmt_f_into({v:?}, {prec}) after {prefix:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn fmt_g_into_appends_after_a_prefix() {
        for v in doubles() {
            for prec in [1, 2, 6, 17] {
                let want = fmt_g_oracle(v, prec);
                for prefix in PREFIXES {
                    let mut s = String::from(prefix);
                    fmt_g_into(&mut s, v, prec);
                    assert_eq!(
                        s,
                        format!("{prefix}{want}"),
                        "fmt_g_into({v:?}, {prec}) after {prefix:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_fixed_point_path_covers_what_the_writers_print() {
        // Every precision the writers use goes through the path.
        let mut buf = [0u8; FIXED_BUF];
        for prec in 0..=FIXED_MAX_PREC {
            assert!(
                fixed_into_buf(&mut buf, 123.456, prec).is_some(),
                "prec {prec}"
            );
        }
        // A value whose scaled integer does not fit in a `u64`, and a precision above the
        // path's, go to the generic formatter.
        assert!(fixed_into_buf(&mut buf, 1e30, 2).is_none());
        assert!(fixed_into_buf(&mut buf, 1.5, FIXED_MAX_PREC + 1).is_none());
        // The digits of the halfway cases: half to even, and the sign of a negative zero.
        let start = fixed_into_buf(&mut buf, 0.125, 2).unwrap();
        assert_eq!(&buf[start..], b"0.12");
        let start = fixed_into_buf(&mut buf, 0.375, 2).unwrap();
        assert_eq!(&buf[start..], b"0.38");
        let start = fixed_into_buf(&mut buf, 0.0625, 3).unwrap();
        assert_eq!(&buf[start..], b"0.062");
        let start = fixed_into_buf(&mut buf, -0.0, 2).unwrap();
        assert_eq!(&buf[start..], b"-0.00");
        let start = fixed_into_buf(&mut buf, -0.004, 2).unwrap();
        assert_eq!(&buf[start..], b"-0.00");
        let start = fixed_into_buf(&mut buf, 2.5, 0).unwrap();
        assert_eq!(&buf[start..], b"2");
        let start = fixed_into_buf(&mut buf, 3.5, 0).unwrap();
        assert_eq!(&buf[start..], b"4");
    }

    /// The exhaustive check: every `f32` value, converted to `f64`, at the two precisions the
    /// writers use most. Four billion values, so it is `#[ignore]`d and run by hand:
    ///
    /// ```sh
    /// cargo test --release -p graphviz-pure-core -- --ignored every_f32
    /// ```
    ///
    /// The threads take the bit patterns in stripes rather than in ranges: the cost of a value
    /// grows with the number of digits it prints, and the bit patterns of the large values are
    /// all at the end of the space, so a range split would leave every thread with the same
    /// slow tail.
    #[test]
    #[ignore = "four billion values; run it by hand"]
    fn every_f32_matches_the_generic_formatter() {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        let total = u64::from(u32::MAX) + 1;
        let stride = u64::try_from(threads).unwrap_or(1);
        let start = std::time::Instant::now();
        let mut handles = Vec::new();
        for t in 0..threads {
            handles.push(std::thread::spawn(move || {
                let mut i = u64::try_from(t).unwrap_or(0);
                while i < total {
                    let v = f64::from(f32::from_bits(i as u32));
                    for prec in [2usize, 3] {
                        // C prints `nan` for a NaN of either sign; Rust prints `NaN`.
                        let want = if v.is_nan() {
                            "nan".to_string()
                        } else {
                            format!("{v:.prec$}")
                        };
                        let mut got = String::new();
                        let _ = fmt_f_into(&mut got, v, prec);
                        assert_eq!(got, want, "f32 bits {i} ({v:?}) at {prec}");
                    }
                    i += stride;
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        println!(
            "{} f32 values, two precisions each, in {:.1} s",
            total,
            start.elapsed().as_secs_f64()
        );
    }

    /// The randomized differential test over `f64`: a hundred million values with the full
    /// 53-bit mantissa, at every precision the writers use, against the generic formatter.
    ///
    /// The `f32` sweep above cannot cover them: an `f32` has 24 bits of mantissa, so every
    /// value it holds has 29 low mantissa bits that are zero, and it never reaches a long
    /// decimal expansion. This one does.
    ///
    /// It is `#[ignore]`d and run by hand:
    ///
    /// ```sh
    /// cargo test --release -p graphviz-pure-core -- --ignored every_f64
    /// ```
    #[test]
    #[ignore = "a hundred million values; run it by hand"]
    fn every_f64_matches_the_generic_formatter() {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(16));
        let total = 100_000_000u64;
        let stride = u64::try_from(threads).unwrap_or(1);
        let start = std::time::Instant::now();
        let mut handles = Vec::new();
        for t in 0..threads {
            handles.push(std::thread::spawn(move || {
                let mut rng = 0x2545_f491_4f6c_dd1du64
                    ^ (u64::try_from(t).unwrap_or(0)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                let mut i = u64::try_from(t).unwrap_or(0);
                while i < total {
                    let v = random_double(&mut rng);
                    for prec in [0usize, 2, 3, 4, 6] {
                        // C prints `nan` for a NaN of either sign; Rust prints `NaN`.
                        let want = if v.is_nan() {
                            "nan".to_string()
                        } else {
                            format!("{v:.prec$}")
                        };
                        let mut got = String::new();
                        let _ = fmt_f_into(&mut got, v, prec);
                        assert_eq!(got, want, "bits {:#x} ({v:?}) at {prec}", v.to_bits());
                    }
                    i += stride;
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        println!(
            "{} f64 values, five precisions each, in {:.1} s",
            total,
            start.elapsed().as_secs_f64()
        );
    }

    /// One value for the randomized differential test: the kinds where the fixed-point path
    /// and the generic formatter could disagree — a random bit pattern with the full
    /// 53-bit mantissa, a value on a decimal boundary, an exact tie, a neighbour of either,
    /// and the edge of the range where the scaled integer stops fitting in a `u64`.
    fn random_double(rng: &mut u64) -> f64 {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        let bits = *rng;
        // The precision the value is built for.
        let (pow10, pow5, pow2) = RANDOM_POWERS[(bits >> 3) as usize % RANDOM_POWERS.len()];
        match bits % 8 {
            // A random bit pattern: the full 53-bit mantissa.
            0 => f64::from_bits(bits),
            // A random mantissa with the exponent of a layout coordinate.
            1 => {
                let mant = (bits >> 12) & 0x000f_ffff_ffff_ffff;
                let exp = 1010 + ((bits >> 9) % 24);
                f64::from_bits((bits & 0x8000_0000_0000_0000) | (exp << 52) | mant)
            }
            // A value on a decimal boundary: `k/10^prec`.
            2 => ((bits >> 8) % 1_000_000_000) as f64 / pow10,
            // A neighbour of one.
            3 => {
                let v = ((bits >> 8) % 1_000_000_000) as f64 / pow10;
                ulp_neighbours(v)[(bits >> 2) as usize % 2]
            }
            // An exact tie: the odd multiple of `1/2^(prec+1)`, which is
            // `odd·5^prec/2^(prec+1)` and is exactly representable.
            4 => {
                let odd = 2 * ((bits >> 8) % 1_000_000) + 1;
                odd as f64 * pow5 / pow2
            }
            // A neighbour of a tie.
            5 => {
                let odd = 2 * ((bits >> 8) % 1_000_000) + 1;
                let v = odd as f64 * pow5 / pow2;
                ulp_neighbours(v)[(bits >> 2) as usize % 2]
            }
            // The edge of the path's range: the scaled integer is `2^64`, which is one too
            // many for a `u64`, and its neighbour below.
            6 => ulp_neighbours(18_446_744_073_709_551_616f64 / pow10)[(bits >> 2) as usize % 2],
            // The edge itself.
            _ => 18_446_744_073_709_551_616f64 / pow10,
        }
    }

    /// The powers the random values are built from, for the precisions the writers use:
    /// `10^prec`, `5^prec` and `2^(prec+1)`.
    const RANDOM_POWERS: [(f64, f64, f64); 5] = [
        (1.0, 1.0, 2.0),
        (100.0, 25.0, 8.0),
        (1_000.0, 125.0, 16.0),
        (10_000.0, 625.0, 32.0),
        (1_000_000.0, 15_625.0, 128.0),
    ];

    /// The doubles next to `v`: the one above it and the one below it.
    fn ulp_neighbours(v: f64) -> [f64; 2] {
        [
            f64::from_bits(v.to_bits() + 1),
            f64::from_bits(v.to_bits() - 1),
        ]
    }

    #[test]
    fn fmt_f_into_appends_to_what_is_there() {
        let mut s = String::from("x");
        let _ = fmt_f_into(&mut s, 1.5, 2);
        let _ = fmt_f_into(&mut s, f64::NAN, 2);
        let _ = fmt_f_into(&mut s, f64::NEG_INFINITY, 2);
        assert_eq!(s, "x1.50nan-inf");
    }

    #[test]
    fn strtoul_reads_like_c() {
        assert_eq!(strtoul("12", 0), (12, 2));
        assert_eq!(strtoul(" 0x1f", 0), (31, 5));
        assert_eq!(strtoul("017", 0), (15, 3));
        assert_eq!(strtoul("0x", 0), (0, 1));
        assert_eq!(strtoul("+9z", 0), (9, 2));
        assert_eq!(strtoul("junk", 0), (0, 0));
        assert_eq!(strtoul("-1", 0), (u64::MAX, 2));
        assert_eq!(strtoul("99999999999999999999999", 0), (u64::MAX, 23));
        assert_eq!(strtoul("017", 10), (17, 3));
        assert_eq!(
            strtoul("-4294967293", 10),
            (4294967293u64.wrapping_neg(), 11)
        );
    }

    use super::*;

    #[test]
    fn min_max_keep_c_operand_order() {
        assert!(c_min(1.0, f64::NAN).is_nan());
        assert_eq!(c_min(f64::NAN, 1.0), 1.0);
        assert!(c_max(1.0, f64::NAN).is_nan());
        assert_eq!(c_max(f64::NAN, 1.0), 1.0);
        assert_eq!(c_min(2, 3), 2);
        assert_eq!(c_max(2.0, 3.0), 3.0);
        // -0.0 < 0.0 is false: the second operand
        assert!(c_min(-0.0f64, 0.0).is_sign_positive());
    }

    #[test]
    fn exact_comparisons_compare_bits() {
        assert!(is_exactly_zero(0.0));
        assert!(!is_exactly_zero(-0.0));
        assert!(is_exactly_equal(f64::NAN, f64::NAN));
        assert!(!is_exactly_equal(0.0, -0.0));
    }

    #[test]
    fn round_converts_like_the_reference_build() {
        assert_eq!(c_round(2.5), 3);
        assert_eq!(c_round(-2.5), -3);
        assert_eq!(c_round(1e300), i32::MAX);
        assert_eq!(c_round(-1e300), i32::MIN);
        assert_eq!(c_round(f64::NAN), 0);
        assert_eq!(c_round(f64::INFINITY), i32::MAX);
        assert_eq!(c_round_low32(f64::INFINITY), -1);
        assert_eq!(c_round_low32(f64::NAN), 0);
        assert_eq!(c_round_low32(4294967296.0 + 7.2), 7);
        assert_eq!(c_round_low32(-2.5), -3);
    }

    #[test]
    fn strtol_saturates_and_atoi_truncates() {
        assert_eq!(strtol("  -42x"), (-42, 5));
        assert_eq!(strtol("+7"), (7, 2));
        assert_eq!(strtol("x1"), (0, 0));
        assert_eq!(strtol("-"), (0, 0));
        assert_eq!(strtol("\x0b\x0c9"), (9, 3));
        assert_eq!(strtol("9223372036854775807").0, i64::MAX);
        assert_eq!(strtol("9223372036854775808").0, i64::MAX);
        assert_eq!(strtol("-9223372036854775808").0, i64::MIN);
        assert_eq!(strtol("-9223372036854775809").0, i64::MIN);
        let many = "9".repeat(400);
        assert_eq!(strtol(&many), (i64::MAX, 400));
        assert_eq!(strtol(&format!("-{many}")), (i64::MIN, 401));
        assert_eq!(atoi("2147483647"), i32::MAX);
        assert_eq!(atoi("2147483648"), i32::MIN);
        assert_eq!(atoi("4294967296"), 0);
        assert_eq!(atoi("9223372036854775808"), -1);
        assert_eq!(atoi(&many), -1);
        assert_eq!(atoi(&format!("-{many}")), 0);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(scan_d(" 4294967296"), Some(0));
        assert_eq!(scan_d("-2147483649"), Some(i32::MAX));
        assert_eq!(scan_d("x"), None);
    }

    #[test]
    fn late_int_and_map_bool() {
        assert_eq!(late_int(None, 8, 0), 8);
        assert_eq!(late_int(Some(""), 8, 0), 8);
        assert_eq!(late_int(Some("x"), 8, 0), 8);
        assert_eq!(late_int(Some("2147483648"), 8, 0), 8);
        assert_eq!(late_int(Some("-99999999999999999999"), 8, 0), 0);
        assert_eq!(late_int(Some("5"), 8, 0), 5);
        assert!(!map_bool(Some("4294967296"), true));
        assert!(map_bool(Some("9223372036854775808"), false));
        assert!(map_bool(Some("Yes"), false));
        assert!(map_bool(Some("x"), true));
    }

    #[test]
    fn fmt_f_prints_like_printf() {
        assert_eq!(fmt_f(f64::NAN, 3), "nan");
        assert_eq!(fmt_f(-f64::NAN, 3), "nan");
        assert_eq!(fmt_f(f64::NEG_INFINITY, 2), "-inf");
        assert_eq!(fmt_f(1.23456, 3), "1.235");
    }
}
