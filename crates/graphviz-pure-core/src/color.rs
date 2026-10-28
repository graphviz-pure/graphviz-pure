//! Color translation tables, hex/HSV parsing, and SVG color formatting ported from
//! Graphviz 16.1.0 (`lib/common/colxlate.c`, `lib/common/colortbl.h`, `lib/common/color.h`).

use crate::color_table::COLOR_LIB;
use std::fmt;

/// Resolved 8-bit RGBA color (`r`, `g`, `b`, `a`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for RgbaColor {
    #[inline]
    fn default() -> Self {
        Self::BLACK
    }
}

impl RgbaColor {
    /// Opaque black (`#000000`).
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white (`#ffffff`).
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    /// Transparent color (`invis` / `none` / `transparent` in Graphviz `colortbl.h`).
    pub const TRANSPARENT: Self = Self::rgba(255, 255, 254, 0);

    /// Creates a new `RgbaColor` from 8-bit RGBA components.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Creates an opaque `RgbaColor` (`a = 255`) from 8-bit RGB components.
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Creates an `RgbaColor` from 8-bit RGBA components.
    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Returns `true` if the color is completely transparent (`a == 0`).
    #[inline]
    pub const fn is_transparent(&self) -> bool {
        self.a == 0
    }

    /// Returns `true` if the color is completely opaque (`a == 255`).
    #[inline]
    pub const fn is_opaque(&self) -> bool {
        self.a == 255
    }

    /// Returns normalized alpha opacity in `[0.0, 1.0]`.
    #[inline]
    pub fn opacity(&self) -> f64 {
        f64::from(self.a) / 255.0
    }

    /// Formats the color as a 6-digit lowercase hex string (`#rrggbb`).
    pub fn to_hex_rgb(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Formats the color as an 8-digit lowercase hex string (`#rrggbbaa`).
    pub fn to_hex_rgba(&self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }

    /// Formats the color for SVG attributes:
    /// - `"none"` when `a == 0` (`transparent` / `invis` / `none`)
    /// - `"#rrggbb"` when `a == 255`
    /// - `"#rrggbbaa"` when `0 < a < 255`
    pub fn to_svg_hex(&self) -> String {
        if self.a == 0 {
            "none".to_string()
        } else if self.a == 255 {
            self.to_hex_rgb()
        } else {
            self.to_hex_rgba()
        }
    }

    /// Formats the color as a CSS/SVG `rgb(...)` or `rgba(...)` string.
    pub fn to_css_rgba(&self) -> String {
        if self.a == 0 {
            "rgba(0,0,0,0)".to_string()
        } else if self.a == 255 {
            format!("rgb({},{},{})", self.r, self.g, self.b)
        } else {
            let alpha = (f64::from(self.a) / 255.0 * 1000.0).round() / 1000.0;
            format!("rgba({},{},{},{})", self.r, self.g, self.b, alpha)
        }
    }

    /// Alias for [`Self::to_svg_hex`].
    #[inline]
    pub fn to_svg_string(&self) -> String {
        self.to_svg_hex()
    }

    /// Converts normalized HSV (`h, s, v` in `[0.0, 1.0]`) to an opaque `RgbaColor`,
    /// matching `hsv2rgb` in `lib/common/colxlate.c`.
    pub fn from_hsv(h: f64, s: f64, v: f64) -> Self {
        Self::from_hsva(h, s, v, 1.0)
    }

    /// Converts normalized HSVA (`h, s, v, a` in `[0.0, 1.0]`) to an `RgbaColor`,
    /// matching `hsv2rgb` and `RGBA_BYTE` conversion in `lib/common/colxlate.c`.
    pub fn from_hsva(h: f64, s: f64, v: f64, a: f64) -> Self {
        // fmax(fmin(x, 1.0), 0.0): a NaN becomes 1.
        let unit = |x: f64| if x.is_nan() { 1.0 } else { x.clamp(0.0, 1.0) };
        let (h, s, v, a) = (unit(h), unit(s), unit(v), unit(a));
        let (r, g, b) = hsv2rgb(h, s, v);
        Self {
            r: (r * 255.0) as u8,
            g: (g * 255.0) as u8,
            b: (b * 255.0) as u8,
            a: (a * 255.0) as u8,
        }
    }

    /// Converts this `RgbaColor` into normalized `(h, s, v, a)` in `[0.0, 1.0]`,
    /// matching `rgb2hsv` in `lib/common/colxlate.c`.
    pub fn to_hsva(&self) -> (f64, f64, f64, f64) {
        let r = f64::from(self.r) / 255.0;
        let g = f64::from(self.g) / 255.0;
        let b = f64::from(self.b) / 255.0;
        let a = f64::from(self.a) / 255.0;
        let (h, s, v) = rgb2hsv(r, g, b);
        (h, s, v, a)
    }
}

impl fmt::Display for RgbaColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_svg_hex())
    }
}

/// Port of `hsv2rgb` from `lib/common/colxlate.c`.
pub fn hsv2rgb(mut h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    if s <= 0.0 {
        (v, v, v)
    } else {
        if h >= 1.0 {
            h = 0.0;
        }
        h *= 6.0;
        let i = h as i32;
        let f = h - f64::from(i);
        let p = v * (1.0 - s);
        // C (clang) fuses `1 - s * …` into one fma.
        let q = v * (-s).mul_add(f, 1.0);
        let t = v * (-s).mul_add(1.0 - f, 1.0);
        match i {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        }
    }
}

/// Port of `rgb2hsv` from `lib/common/colxlate.c`.
pub fn rgb2hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let rgbmin = r.min(g.min(b));
    let rgbmax = r.max(g.max(b));
    let st = if rgbmax > 0.0 {
        (rgbmax - rgbmin) / rgbmax
    } else {
        0.0
    };
    let mut ht = 0.0;
    if st > 0.0 {
        let denom = rgbmax - rgbmin;
        let rc = (rgbmax - r) / denom;
        let gc = (rgbmax - g) / denom;
        let bc = (rgbmax - b) / denom;
        if crate::cnum::is_exactly_equal(r, rgbmax) {
            ht = bc - gc;
        } else if crate::cnum::is_exactly_equal(g, rgbmax) {
            ht = 2.0 + rc - bc;
        } else if crate::cnum::is_exactly_equal(b, rgbmax) {
            ht = 4.0 + gc - rc;
        }
        ht *= 60.0;
        if ht < 0.0 {
            ht += 360.0;
        }
    }
    (ht / 360.0, st, rgbmax)
}

/// Resolves a Graphviz color specification (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`,
/// `"h,s,v"`, `"h s v"`, or X11/SVG/Brewer named color) to an [`RgbaColor`].
pub fn resolve_color(spec: &str) -> Option<RgbaColor> {
    resolve_color_with_scheme(spec, None)
}

/// Resolves a Graphviz color specification with an optional active `colorscheme`, following
/// `colorxlate` in `lib/common/colxlate.c`. Returns `None` for an unknown color (C yields opaque
/// black and warns; see [`warn_unknown_color`]).
pub fn resolve_color_with_scheme(spec: &str, colorscheme: Option<&str>) -> Option<RgbaColor> {
    let p = spec.trim_start_matches(' ');
    if p.is_empty() {
        return None;
    }

    // 1. Hex color: `#rrggbb[aa]`, or the 3 digit form `#rgb` (C `sscanf("#%2x%2x%2x%2x")`).
    if let Some(color) = parse_hex_color(p) {
        return Some(color);
    }

    // 2. HSV / HSVA numeric specification: starts with '.' or ASCII digit
    let first = p.as_bytes()[0];
    if (first == b'.' || first.is_ascii_digit())
        && let Some(hsv_color) = parse_hsv_color(p)
    {
        return Some(hsv_color);
    }

    // 3. Named color lookup (X11 / SVG / Brewer color schemes)
    let resolved_name = normalize_color_scheme_name(p, colorscheme);
    lookup_named_color(&resolved_name)
}

/// Prints `Warning: <name> is not a known color.` to stderr, once per distinct name, like
/// `gvrender_resolve_color`.
pub fn warn_unknown_color(name: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut seen) = seen.lock()
        && seen.insert(name.to_string())
    {
        crate::diag::agwarningf(&format!("{name} is not a known color.\n"));
    }
}

/// Formats a Graphviz color spec into an SVG color string (`#rrggbb`, `#rrggbbaa`, or `"none"`),
/// falling back to `"black"` (`#000000`) if unknown.
pub fn format_svg_color(spec: &str) -> String {
    resolve_color(spec).unwrap_or(RgbaColor::BLACK).to_svg_hex()
}

/// Reads up to `max` hex digits like one `%{max}x` conversion, skipping leading whitespace.
fn scan_hex(s: &str, max: usize) -> Option<(u32, &str)> {
    let s = s.trim_start();
    let n = s
        .bytes()
        .take(max)
        .take_while(u8::is_ascii_hexdigit)
        .count();
    if n == 0 {
        return None;
    }
    Some((u32::from_str_radix(&s[..n], 16).ok()?, &s[n..]))
}

fn parse_hex_color(p: &str) -> Option<RgbaColor> {
    let rest = p.strip_prefix('#')?;
    let mut vals = [0u32, 0, 0, 255];
    let mut cur = rest;
    let mut count = 0;
    for slot in vals.iter_mut() {
        let Some((v, next)) = scan_hex(cur, 2) else {
            break;
        };
        *slot = v;
        cur = next;
        count += 1;
    }
    if count >= 3 {
        return Some(RgbaColor::rgba(
            vals[0] as u8,
            vals[1] as u8,
            vals[2] as u8,
            vals[3] as u8,
        ));
    }
    if p.len() == 4 {
        let (mut r, rest) = scan_hex(rest, 1)?;
        let (mut g, rest) = scan_hex(rest, 1)?;
        let (mut b, _) = scan_hex(rest, 1)?;
        r |= r << 4;
        g |= g << 4;
        b |= b << 4;
        return Some(RgbaColor::rgb(r as u8, g as u8, b as u8));
    }
    None
}

fn parse_hsv_color(p: &str) -> Option<RgbaColor> {
    // colorxlate: commas become blanks, then `sscanf("%lf%lf%lf%lf") >= 3`, so text after
    // the numbers (`0.1,0.9,0.7;0.3` of a weighted list) is ignored.
    let canon: String = p.chars().map(|c| if c == ',' { ' ' } else { c }).collect();
    let r = crate::strtod::sscanf(&canon, "%lf%lf%lf%lf");
    let [h, sat, v, a, _] = r.values;
    // `A = 1.0` before the scan.
    let a = if r.count >= 4 { a } else { 1.0 };
    (r.count >= 3).then(|| RgbaColor::from_hsva(h, sat, v, a))
}

/// Port of `resolveColor` from `lib/common/colxlate.c`.
fn normalize_color_scheme_name(str_in: &str, colorscheme: Option<&str>) -> String {
    let lower = str_in.to_ascii_lowercase();
    if lower == "black" || lower == "white" || lower == "lightgrey" {
        return lower;
    }

    let is_non_dflt = |scheme: Option<&str>| -> Option<String> {
        let s = scheme?;
        if s.is_empty() || s.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("x11")) {
            None
        } else {
            Some(s.to_ascii_lowercase())
        }
    };

    if let Some(c2) = lower.strip_prefix('/') {
        if let Some(slash_idx) = c2.find('/') {
            if let Some(after_double) = c2.strip_prefix('/') {
                if let Some(scheme) = is_non_dflt(colorscheme) {
                    format!("/{scheme}/{after_double}")
                } else {
                    after_double.to_string()
                }
            } else if c2.starts_with("x11/") {
                c2[slash_idx + 1..].to_string()
            } else {
                lower
            }
        } else {
            c2.to_string()
        }
    } else if let Some(scheme) = is_non_dflt(colorscheme) {
        format!("/{scheme}/{lower}")
    } else {
        lower
    }
}

fn lookup_named_color(name: &str) -> Option<RgbaColor> {
    let key = name.to_ascii_lowercase();
    let idx = COLOR_LIB
        .binary_search_by(|(n, _)| (*n).cmp(key.as_str()))
        .ok()?;
    let [r, g, b, a] = COLOR_LIB[idx].1;
    Some(RgbaColor::rgba(r, g, b, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_named_colors_table_is_sorted() {
        for window in COLOR_LIB.windows(2) {
            assert!(
                window[0].0 < window[1].0,
                "{:?} >= {:?}",
                window[0].0,
                window[1].0
            );
        }
    }

    #[test]
    fn test_x11_svg_named_colors() {
        assert_eq!(
            resolve_color("lightblue"),
            Some(RgbaColor::rgb(173, 216, 230))
        );
        assert_eq!(
            resolve_color("lightgrey"),
            Some(RgbaColor::rgb(211, 211, 211))
        );
        assert_eq!(
            resolve_color("LightGray"),
            Some(RgbaColor::rgb(211, 211, 211))
        );
        assert_eq!(resolve_color("red"), Some(RgbaColor::rgb(255, 0, 0)));
        assert_eq!(resolve_color("gold"), Some(RgbaColor::rgb(255, 215, 0)));
        assert_eq!(
            resolve_color("darkslategray"),
            Some(RgbaColor::rgb(47, 79, 79))
        );
        assert_eq!(
            resolve_color("darkslategrey"),
            Some(RgbaColor::rgb(47, 79, 79))
        );
        assert_eq!(
            resolve_color("transparent"),
            Some(RgbaColor::rgba(255, 255, 254, 0))
        );
        assert_eq!(resolve_color("none").unwrap().to_svg_hex(), "none");
        assert!(resolve_color("invis").unwrap().is_transparent());
    }

    #[test]
    fn test_hex_colors() {
        assert_eq!(resolve_color("#ff0000"), Some(RgbaColor::rgb(255, 0, 0)));
        assert_eq!(
            resolve_color("#ff000080"),
            Some(RgbaColor::rgba(255, 0, 0, 128))
        );
        assert_eq!(
            resolve_color("#f0a"),
            Some(RgbaColor::rgb(0xff, 0x00, 0xaa))
        );
        // C `sscanf` semantics: `#1234` is neither `#rrggbb` nor the 3 digit form.
        assert_eq!(resolve_color("#1234"), None);
        assert_eq!(resolve_color("#add8e6").unwrap().to_svg_hex(), "#add8e6");
        assert_eq!(
            resolve_color("#ff000080").unwrap().to_svg_hex(),
            "#ff000080"
        );
        assert_eq!(
            resolve_color("#12345"),
            Some(RgbaColor::rgba(0x12, 0x34, 0x5, 255))
        );
    }

    #[test]
    fn test_hsv_colors() {
        // Pure red: h=0, s=1, v=1
        assert_eq!(
            resolve_color("0.0,1.0,1.0"),
            Some(RgbaColor::rgb(255, 0, 0))
        );
        // Pure green: h=1/3, s=1, v=1
        let green = resolve_color("0.333333, 1.0, 1.0").unwrap();
        assert_eq!(green.g, 255);
        assert!(green.r <= 1 && green.b == 0);
        // Space-separated HSVA
        let semi = resolve_color("0.0 0.0 0.5 0.5").unwrap();
        assert_eq!(semi, RgbaColor::rgba(127, 127, 127, 127));
    }

    #[test]
    fn test_full_tables_match_c() {
        // Expected values taken from `dot -Tsvg` 16.1.0.
        let hex = |spec: &str, scheme: Option<&str>| {
            resolve_color_with_scheme(spec, scheme)
                .unwrap_or_else(|| panic!("{spec} unresolved"))
                .to_hex_rgb()
        };
        assert_eq!(hex("/blues9/5", None), "#6baed6");
        assert_eq!(hex("/set312/7", None), "#b3de69");
        assert_eq!(hex("/rdylgn11/3", None), "#f46d43");
        assert_eq!(hex("/svg/aliceblue", None), "#f0f8ff");
        assert_eq!(hex("/svg/yellowgreen", None), "#9acd32");
        assert_eq!(hex("/x11/navy", None), "#000080");
        assert_eq!(hex("5", Some("blues9")), "#6baed6");
        assert_eq!(hex("//3", Some("blues9")), "#c6dbef");
        assert_eq!(hex("navy", Some("X11")), "#000080");
    }

    #[test]
    fn test_unknown_colors_are_not_resolved() {
        assert_eq!(resolve_color("nosuchcolor"), None);
        assert_eq!(resolve_color("/blues9/10"), None);
        assert_eq!(resolve_color("9"), None);
        assert_eq!(resolve_color_with_scheme("9", Some("blues3")), None);
        assert_eq!(resolve_color_with_scheme("red", Some("blues9")), None);
        assert_eq!(resolve_color("gray101"), None);
        assert_eq!(resolve_color("#ffff"), None);
    }
}
