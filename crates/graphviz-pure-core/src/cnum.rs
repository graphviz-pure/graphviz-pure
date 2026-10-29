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
pub fn fmt_f_into<W: std::fmt::Write>(out: &mut W, v: f64, prec: usize) -> std::fmt::Result {
    if v.is_nan() {
        // C prints `nan` whatever the sign.
        out.write_str("nan")
    } else if v.is_infinite() {
        out.write_str(if v < 0.0 { "-inf" } else { "inf" })
    } else {
        write!(out, "{v:.prec$}")
    }
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
    // `d.dddde<exp>`; C writes the exponent with a sign and two digits.
    let start = out.len();
    let _ = write!(out, "{:.*e}", p - 1, v);
    let epos = start + out[start..].find('e').expect("LowerExp writes an exponent");
    let exp: i32 = out[epos + 1..].parse().unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
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
    /// The `%g` the port printed before [`fmt_g_into`] existed, as the differential test's
    /// reference: the same digits, each in its own `String`.
    fn fmt_g_reference(v: f64, prec: usize) -> String {
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

    /// The doubles the differential test walks: the edges, the powers of ten and a
    /// deterministic spread of random ones.
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
        v
    }

    #[test]
    fn fmt_g_into_prints_what_fmt_g_printed() {
        for v in doubles() {
            for prec in 1..=17 {
                let want = fmt_g_reference(v, prec);
                let mut got = String::new();
                fmt_g_into(&mut got, v, prec);
                assert_eq!(got, want, "fmt_g_into({v:?}, {prec})");
                assert_eq!(fmt_g(v, prec), want, "fmt_g({v:?}, {prec})");
            }
        }
    }

    #[test]
    fn fmt_f_into_prints_what_fmt_f_printed() {
        for v in doubles() {
            for prec in 0..=17 {
                let want = fmt_f(v, prec);
                let mut got = String::new();
                let _ = fmt_f_into(&mut got, v, prec);
                assert_eq!(got, want, "fmt_f_into({v:?}, {prec})");
            }
        }
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
