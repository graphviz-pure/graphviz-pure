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
    trim_zeros_at(out, start);
    // agxbuf_trim_zeros turns a remaining "-0" into "0"
    if &out[start..] == "-0" {
        out.truncate(start);
        out.push('0');
    }
}

/// `%.02f` then `agxbuf_trim_zeros` (xdot numbers; `-0` is possible), written into `out`.
pub(crate) fn xdot_num_into(out: &mut String, v: f64) {
    c_fixed_trim_into(out, v, 2);
}

/// `%.{prec}f` then `agxbuf_trim_zeros`, written into `out`.
pub(crate) fn c_fixed_trim_into(out: &mut String, v: f64, prec: usize) {
    let start = out.len();
    c_fixed_into(out, v, prec);
    trim_zeros_at(out, start);
    // agxbuf_trim_zeros turns a remaining "-0" into "0"
    if &out[start..] == "-0" {
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
    fn the_into_variants_append_to_the_buffer() {
        let mut s = String::from("[");
        gvdouble_into(&mut s, 12.5);
        s.push(',');
        xdot_num_into(&mut s, 20.0);
        s.push(']');
        assert_eq!(s, "[12.5,20]");
    }
}
