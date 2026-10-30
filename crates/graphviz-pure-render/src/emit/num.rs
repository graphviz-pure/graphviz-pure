//! C number formatting used by the renderers: `gvprintdouble`, `gvprintnum`, `%.Ng`,
//! `agxbuf_trim_zeros`.
//!
//! Every `*_into` variant writes the number into the caller's buffer, so a renderer does not
//! allocate a `String` per number.

/// Strips trailing zeros after a decimal point, and the point itself (`agxbuf_trim_zeros`).
pub(crate) fn trim_zeros(mut s: String) -> String {
    if s.contains('.') {
        trim_zeros_at(&mut s, 0);
        // agxbuf_trim_zeros turns a remaining "-0" into "0"
        if s == "-0" {
            s = "0".to_string();
        }
    }
    s
}

/// `agxbuf_trim_zeros` on the text `s[start..]`: the trailing zeros after a decimal point, and
/// the point itself. Only a number with a decimal point is trimmed.
fn trim_zeros_at(s: &mut String, start: usize) {
    if !s[start..].contains('.') {
        return;
    }
    let mut end = s.len();
    while end > start && s.as_bytes()[end - 1] == b'0' {
        end -= 1;
    }
    if end > start && s.as_bytes()[end - 1] == b'.' {
        end -= 1;
    }
    s.truncate(end);
}

/// C `printf("%.{prec}f", v)` on the reference platform (macOS libc): non-finite values
/// print as `nan` (whatever the sign), `inf` and `-inf`, written into `out`.
pub(crate) fn c_fixed_into(out: &mut String, v: f64, prec: usize) {
    let _ = graphviz_pure_core::cnum::fmt_f_into(out, v, prec);
}

/// `gvprintdouble`: `%.02f` with trailing zeros removed; values within 0.005 of zero print `0`,
/// written into `out`.
pub(crate) fn gvdouble_into(out: &mut String, num: f64) {
    if num > -0.005 && num < 0.005 {
        out.push('0');
        return;
    }
    let start = out.len();
    c_fixed_into(out, num, 2);
    trim_fixed_zeros(out, start);
}

/// `%.02f` then `agxbuf_trim_zeros` (xdot numbers; `-0` is possible), written into `out`.
pub(crate) fn xdot_num_into(out: &mut String, v: f64) {
    c_fixed_trim_into(out, v, 2);
}

/// `%.{prec}f` then `agxbuf_trim_zeros`, written into `out`.
pub(crate) fn c_fixed_trim_into(out: &mut String, v: f64, prec: usize) {
    let start = out.len();
    c_fixed_into(out, v, prec);
    trim_fixed_zeros(out, start);
}

/// `agxbuf_trim_zeros` on the text written at `start`: the trailing zeros after a decimal
/// point, and the point itself, and a remaining "-0" turned into "0". The "-0" is turned only
/// for a text that had a decimal point, as the old `trim_zeros` did: a `-0` written without
/// one (a negative value rounded to zero at precision 0) stays as it is.
fn trim_fixed_zeros(out: &mut String, start: usize) {
    let had_point = out[start..].contains('.');
    trim_zeros_at(out, start);
    if had_point && &out[start..] == "-0" {
        out.truncate(start);
        out.push('0');
    }
}

/// `gvprintnum`: `%.03f`, trailing zeros and a leading `0` removed.
#[allow(dead_code)]
pub(crate) fn gvprintnum(number: f64) -> String {
    const MAXNEG: f64 = -999_999_999_999_999.99;
    if number < MAXNEG {
        return "-999999999999999.99".to_string();
    }
    if number > -MAXNEG {
        return "999999999999999.99".to_string();
    }
    let mut s = String::new();
    c_fixed_into(&mut s, number, 3);
    let s = trim_zeros(s);
    if let Some(rest) = s.strip_prefix("0.") {
        format!(".{rest}")
    } else if let Some(rest) = s.strip_prefix("-0.") {
        format!("-.{rest}")
    } else {
        s
    }
}

/// C `printf("%.{prec}g", v)`, written into `out`.
pub(crate) fn fmt_g_into(out: &mut String, v: f64, prec: usize) {
    graphviz_pure_core::cnum::fmt_g_into(out, v, prec);
}

/// `%f` (six decimals), written into `out`.
pub(crate) fn fmt_f6_into(out: &mut String, v: f64) {
    c_fixed_into(out, v, 6);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(v: f64, prec: usize) -> String {
        let mut s = String::new();
        fmt_g_into(&mut s, v, prec);
        s
    }

    fn dbl(v: f64) -> String {
        let mut s = String::new();
        gvdouble_into(&mut s, v);
        s
    }

    fn xdot(v: f64) -> String {
        let mut s = String::new();
        xdot_num_into(&mut s, v);
        s
    }

    /// The `trim_zeros` the port printed before the `*_into` variants existed, kept
    /// verbatim as the differential test's oracle.
    fn trim_zeros_oracle(mut s: String) -> String {
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
            // agxbuf_trim_zeros turns a remaining "-0" into "0"
            if s == "-0" {
                s = "0".to_string();
            }
        }
        s
    }

    /// The `c_fixed` the port printed before the `*_into` variants existed: the old
    /// `core::cnum::fmt_f`, kept verbatim as the oracle.
    fn c_fixed_oracle(v: f64, prec: usize) -> String {
        if v.is_nan() {
            "nan".to_string()
        } else if v.is_infinite() {
            if v > 0.0 { "inf" } else { "-inf" }.to_string()
        } else {
            format!("{v:.prec$}")
        }
    }

    /// The `gvdouble` the port printed before the `*_into` variants existed: `%.02f` with
    /// the trailing zeros removed.
    fn gvdouble_oracle(num: f64) -> String {
        if num > -0.005 && num < 0.005 {
            return "0".to_string();
        }
        trim_zeros_oracle(c_fixed_oracle(num, 2))
    }

    /// The `xdot_num` the port printed before the `*_into` variants existed: `%.02f` with
    /// the trailing zeros removed.
    fn xdot_num_oracle(v: f64) -> String {
        trim_zeros_oracle(c_fixed_oracle(v, 2))
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

    /// The prefixes a writer may find in the buffer: the text before the number must not be
    /// touched by the in-place trimming.
    const PREFIXES: [&str; 5] = ["0.", "10", "1.00", "-", " "];

    #[test]
    fn g_format_matches_printf() {
        assert_eq!(g(205.8, 5), "205.8");
        assert_eq!(g(0.75, 5), "0.75");
        assert_eq!(g(123456.0, 5), "1.2346e+05");
        assert_eq!(g(0.0001234, 5), "0.0001234");
        assert_eq!(g(0.00001234, 5), "1.234e-05");
        assert_eq!(g(99999.5, 5), "1e+05");
        assert_eq!(g(1.0, 5), "1");
        assert_eq!(g(-2.5, 5), "-2.5");
        assert_eq!(g(1.0, 6), "1");
        assert_eq!(g(0.333333333, 6), "0.333333");
    }

    #[test]
    fn printdouble_and_xdot() {
        assert_eq!(dbl(-0.004), "0");
        assert_eq!(dbl(12.5), "12.5");
        assert_eq!(dbl(-209.8), "-209.8");
        assert_eq!(xdot(-0.001), "0");
        assert_eq!(xdot(20.0), "20");
        assert_eq!(gvprintnum(0.5), ".5");
    }

    #[test]
    fn the_into_variants_print_what_the_old_functions_printed() {
        for v in doubles() {
            let mut got = String::new();
            gvdouble_into(&mut got, v);
            assert_eq!(got, gvdouble_oracle(v), "gvdouble_into({v:?})");

            let mut got = String::new();
            xdot_num_into(&mut got, v);
            assert_eq!(got, xdot_num_oracle(v), "xdot_num_into({v:?})");

            for prec in 0..=17 {
                let mut got = String::new();
                c_fixed_into(&mut got, v, prec);
                assert_eq!(got, c_fixed_oracle(v, prec), "c_fixed_into({v:?}, {prec})");

                let mut got = String::new();
                c_fixed_trim_into(&mut got, v, prec);
                assert_eq!(
                    got,
                    trim_zeros_oracle(c_fixed_oracle(v, prec)),
                    "c_fixed_trim_into({v:?}, {prec})"
                );

                let mut got = String::new();
                fmt_f6_into(&mut got, v);
                assert_eq!(got, c_fixed_oracle(v, 6), "fmt_f6_into({v:?})");
            }
        }
    }

    #[test]
    fn the_into_variants_append_after_a_prefix() {
        for v in doubles() {
            for prefix in PREFIXES {
                let mut s = String::from(prefix);
                gvdouble_into(&mut s, v);
                assert_eq!(
                    s,
                    format!("{prefix}{}", gvdouble_oracle(v)),
                    "gvdouble_into({v:?}) after {prefix:?}"
                );

                let mut s = String::from(prefix);
                xdot_num_into(&mut s, v);
                assert_eq!(
                    s,
                    format!("{prefix}{}", xdot_num_oracle(v)),
                    "xdot_num_into({v:?}) after {prefix:?}"
                );

                let mut s = String::from(prefix);
                fmt_f6_into(&mut s, v);
                assert_eq!(
                    s,
                    format!("{prefix}{}", c_fixed_oracle(v, 6)),
                    "fmt_f6_into({v:?}) after {prefix:?}"
                );

                for prec in [0, 2, 3, 17] {
                    let mut s = String::from(prefix);
                    c_fixed_into(&mut s, v, prec);
                    assert_eq!(
                        s,
                        format!("{prefix}{}", c_fixed_oracle(v, prec)),
                        "c_fixed_into({v:?}, {prec}) after {prefix:?}"
                    );

                    let mut s = String::from(prefix);
                    c_fixed_trim_into(&mut s, v, prec);
                    assert_eq!(
                        s,
                        format!("{prefix}{}", trim_zeros_oracle(c_fixed_oracle(v, prec))),
                        "c_fixed_trim_into({v:?}, {prec}) after {prefix:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_into_variants_append_to_the_buffer() {
        let mut s = String::from("[");
        gvdouble_into(&mut s, 12.5);
        s.push(',');
        xdot_num_into(&mut s, 20.0);
        s.push(']');
        assert_eq!(s, "[12.5,20]");
    }
}
