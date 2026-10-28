//! The xdot drawing-operation parser of `lib/xdot/xdot.c` (`parseXDot`), used by the JSON
//! writers to expand `_draw_` attributes and by `init_xdot` for `_background`.

/// A colour operand: a plain colour or a gradient (`xdot_color`).
#[derive(Debug, Clone, PartialEq)]
pub enum Grad {
    None(String),
    Linear(f64, f64, f64, f64, Vec<(f64, String)>),
    Radial(f64, f64, f64, f64, f64, f64, Vec<(f64, String)>),
}

/// One drawing operation (`xdot_op`).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Ellipse(bool, [f64; 4]),
    Polygon(bool, Vec<(f64, f64)>),
    Bezier(bool, Vec<(f64, f64)>),
    Polyline(Vec<(f64, f64)>),
    Text(f64, f64, i64, f64, String),
    Color(bool, Grad),
    Font(f64, String),
    Style(String),
    Image,
    FontChar(u64),
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        // `gv_isspace`: C's white space, `\v` included.
        while self.i < self.s.len() && crate::cnum::is_c_space(self.s[self.i]) {
            self.i += 1;
        }
    }
    /// The longest UTF-8 prefix of the rest (numbers are ASCII).
    fn rest(&self) -> &str {
        let r = &self.s[self.i..];
        match std::str::from_utf8(r) {
            Ok(s) => s,
            Err(e) => std::str::from_utf8(&r[..e.valid_up_to()]).unwrap_or(""),
        }
    }
    fn real(&mut self) -> Option<f64> {
        let rest = self.rest();
        let (v, r) = crate::strtod::strtod_rest(rest)?;
        self.i += rest.len() - r.len();
        Some(v)
    }
    /// `parseInt`: `strtol`, truncated to C's `int`.
    fn int(&mut self) -> Option<i32> {
        let rest = self.rest();
        let (v, n) = crate::cnum::strtol(rest);
        if n == 0 {
            return None;
        }
        self.i += n;
        Some(v as i32)
    }
    /// `parseUInt`: `strtoul`, truncated to C's `unsigned`.
    fn uint(&mut self) -> Option<u32> {
        let (v, n) = crate::cnum::strtoul(self.rest(), 10);
        if n == 0 {
            return None;
        }
        self.i += n;
        Some(v as u32)
    }
    fn string(&mut self) -> Option<String> {
        let n = self.int()?;
        if n <= 0 {
            return None;
        }
        while self.i < self.s.len() && self.s[self.i] != b'-' {
            self.i += 1;
        }
        if self.i >= self.s.len() {
            return None;
        }
        self.i += 1;
        let start = self.i;
        let mut accounted = 0;
        let mut j = 0usize;
        while accounted < n {
            let c = *self.s.get(start + j)?;
            if c != b'\\' || (j > 0 && self.s[start + j - 1] == b'\\') {
                accounted += 1;
            }
            j += 1;
        }
        // count characters, not bytes: extend over UTF-8 continuation bytes
        let mut end = start + j;
        while end < self.s.len() && (self.s[end] & 0xC0) == 0x80 {
            end += 1;
        }
        self.i = end;
        Some(crate::charset::decode_slice(&self.s[start..end]))
    }
    fn polyline(&mut self) -> Option<Vec<(f64, f64)>> {
        let n = self.uint()?;
        // Not sized from `n`: a count the input doesn't hold points for would overflow.
        let mut v = Vec::new();
        for _ in 0..n {
            let x = self.real()?;
            let y = self.real()?;
            v.push((x, y));
        }
        Some(v)
    }
}

fn parse_color(s: &str) -> Option<Grad> {
    // As `parse_xdot`: C's bytes, whose strings `P::string` decodes again.
    let bytes = crate::charset::encode(s);
    let b = &*bytes;
    let stops = |p: &mut P| -> Option<Vec<(f64, String)>> {
        let n = p.uint()?;
        let mut v = Vec::new();
        for _ in 0..n {
            let f = p.real()?;
            let c = p.string()?;
            v.push((f, c));
        }
        Some(v)
    };
    match b.first() {
        Some(b'[') => {
            let mut p = P { s: b, i: 1 };
            let (x0, y0, x1, y1) = (p.real()?, p.real()?, p.real()?, p.real()?);
            Some(Grad::Linear(x0, y0, x1, y1, stops(&mut p)?))
        }
        Some(b'(') => {
            let mut p = P { s: b, i: 1 };
            let (x0, y0, r0, x1, y1, r1) = (
                p.real()?,
                p.real()?,
                p.real()?,
                p.real()?,
                p.real()?,
                p.real()?,
            );
            Some(Grad::Radial(x0, y0, r0, x1, y1, r1, stops(&mut p)?))
        }
        // `parseXDotColor`: a plain colour starts with `#`, `/` or an ASCII letter or digit.
        Some(b'#' | b'/') => Some(Grad::None(s.to_string())),
        Some(c) if c.is_ascii_alphanumeric() => Some(Grad::None(s.to_string())),
        _ => None,
    }
}

/// `parseXDot`: the operations of `s` up to the first one that does not parse (none when
/// the first does not).
pub fn parse_xdot(s: &str) -> Vec<Op> {
    // C parses the bytes; strings come back as raw-byte strings (`crate::charset`).
    let bytes = crate::charset::encode(s);
    let mut p = P { s: &bytes, i: 0 };
    let mut ops = Vec::new();
    loop {
        p.ws();
        let Some(&c) = p.s.get(p.i) else { break };
        p.i += 1;
        let op = match c {
            b'E' | b'e' => (|| Some([p.real()?, p.real()?, p.real()?, p.real()?]))()
                .map(|r| Op::Ellipse(c == b'E', r)),
            b'P' | b'p' => p.polyline().map(|v| Op::Polygon(c == b'P', v)),
            b'b' | b'B' => p.polyline().map(|v| Op::Bezier(c == b'b', v)),
            b'L' => p.polyline().map(Op::Polyline),
            b'c' | b'C' => p
                .string()
                .and_then(|cs| parse_color(&cs))
                .map(|g| Op::Color(c == b'C', g)),
            b'T' => (|| {
                let x = p.real()?;
                let y = p.real()?;
                let a = p.int()?;
                let w = p.real()?;
                let t = p.string()?;
                Some(Op::Text(x, y, i64::from(a), w, t))
            })(),
            b'F' => (|| Some(Op::Font(p.real()?, p.string()?)))(),
            b'S' => p.string().map(Op::Style),
            b'I' => (|| {
                p.real()?;
                p.real()?;
                p.real()?;
                p.real()?;
                p.string()?;
                Some(Op::Image)
            })(),
            b't' => p.uint().map(|v| Op::FontChar(u64::from(v))),
            _ => None,
        };
        match op {
            Some(o) => ops.push(o),
            None => break,
        }
    }
    ops
}

/// `GD_drawing(g)->xdots`: what `init_xdot` parsed from the graph's `_background` (or its
/// `_draw_`), which `emit_background` draws behind the graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Background {
    /// Each op (`exdot_op`) with its `bb` and, for a text op, its span. Only `init_nop`
    /// (`neato -n`) sets them, with `xdotBB`; elsewhere they stay as `parseXDotF`'s
    /// `gv_calloc` left them: zero boxes and no spans.
    pub ops: Vec<(Op, OpBox)>,
}

/// An `exdot_op`'s `bb` and, for a text op, its span.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpBox {
    pub ll: (f64, f64),
    pub ur: (f64, f64),
    pub span: Option<SpanSize>,
}

/// What `xdotBB` gave a text op's span: the font of the last `F` and the flags of the last
/// `t` before it, and the size `textspan_size` estimated.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpanSize {
    pub fontname: String,
    pub fontsize: f64,
    pub fontflags: u64,
    pub width: f64,
    pub height: f64,
    pub yoffset_centerline: f64,
}

/// `init_xdot` (`lib/common/emit.c`), called by `graph_init`: parses the root's
/// `_background`, or its `_draw_` when `_background` is empty, and warns when no op parses.
pub fn init_xdot(g: &crate::Graph) -> Option<Background> {
    let attr = |k: &str| g.graph_attrs.get(k).filter(|s| !s.is_empty());
    let p = attr("_background").or_else(|| attr("_draw_"))?;
    let ops = parse_xdot(p);
    if ops.is_empty() {
        let name = g.agnameof(crate::Obj::Root).name;
        crate::diag::agwarningf(&format!(
            "Could not parse \"_background\" attribute in graph {name}\n"
        ));
        crate::diag::agerr_prev(&format!("  \"{p}\"\n"));
        return None;
    }
    Some(Background {
        ops: ops.into_iter().map(|op| (op, OpBox::default())).collect(),
    })
}

/// `xdotBB`: the graph's bounding box `bb` grown to hold every op, setting each op's `bb`
/// and sizing each text op's span. A degenerate `bb` (one point) is not held.
pub fn xdot_bb(xd: &mut Background, bb: ((f64, f64), (f64, f64))) -> ((f64, f64), (f64, f64)) {
    let (mut ll, mut ur) = bb;
    if ll == ur {
        ll = (f64::MAX, f64::MAX);
        ur = (-f64::MAX, -f64::MAX);
    }
    let mut expand = |p: (f64, f64)| {
        ur.0 = ur.0.max(p.0);
        ll.0 = ll.0.min(p.0);
        ur.1 = ur.1.max(p.1);
        ll.1 = ll.1.min(p.1);
    };
    let mut fontsize = 0.0;
    let mut fontname: Option<String> = None;
    let mut fontflags = 0;
    for (op, b) in &mut xd.ops {
        match op {
            Op::Ellipse(_, [x, y, w, h]) => {
                b.ll = (*x - *w, *y - *h);
                b.ur = (*x + *w, *y + *h);
                expand(b.ll);
                expand(b.ur);
            }
            Op::Polygon(_, pts) | Op::Bezier(_, pts) | Op::Polyline(pts) => {
                // `ptsBB` reads the first point unchecked: with no points, past the end of a
                // zero-size `gv_calloc` block, zeroed on the reference build.
                let (first, rest) = pts
                    .split_first()
                    .map_or(((0.0, 0.0), &[][..]), |(&f, r)| (f, r));
                b.ll = first;
                b.ur = first;
                for &(x, y) in rest {
                    if x < b.ll.0 {
                        b.ll.0 = x;
                    } else if x > b.ur.0 {
                        b.ur.0 = x;
                    }
                    if y < b.ll.1 {
                        b.ll.1 = y;
                    } else if y > b.ur.1 {
                        b.ur.1 = y;
                    }
                }
                expand(b.ll);
                expand(b.ur);
            }
            Op::Text(x, y, align, _, text) => {
                // `textspan_size` asserts a font name: C fails with no `F` op before.
                if let Some(name) = &fontname {
                    let bold = fontflags & 1 != 0;
                    let italic = fontflags & 2 != 0;
                    let (w, h) =
                        crate::font::estimate_text_size_styled(text, name, fontsize, bold, italic);
                    let (lx, ux) = match align.signum() {
                        -1 => (*x, *x + w),
                        1 => (*x - w, *x),
                        _ => (*x - w / 2.0, *x + w / 2.0),
                    };
                    // yoffset_layout: the font size.
                    b.ur = (ux, *y + fontsize);
                    b.ll = (lx, b.ur.1 - h);
                    b.span = Some(SpanSize {
                        fontname: name.clone(),
                        fontsize,
                        fontflags,
                        width: w,
                        height: h,
                        yoffset_centerline: 0.1 * fontsize,
                    });
                    expand(b.ll);
                    expand(b.ur);
                }
            }
            Op::Font(size, name) => {
                fontsize = *size;
                fontname = Some(name.clone());
            }
            Op::FontChar(flags) => fontflags = *flags,
            _ => {}
        }
    }
    (ll, ur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_cs_int_and_unsigned() {
        // `parseUInt`: 4294967299 is 3 as an `unsigned`, and `strtoul` negates "-4294967293".
        for s in [
            "P 4294967299 0 0 10 0 10 10",
            "P -4294967293 0 0 10 0 10 10",
        ] {
            assert_eq!(
                parse_xdot(s),
                [Op::Polygon(
                    true,
                    vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
                )],
                "{s}"
            );
        }
        // `parseInt`: the alignment is LONG_MIN's low 32 bits, 0.
        assert!(matches!(
            parse_xdot("T 0 0 -9223372036854775808 5 2 -hi").as_slice(),
            [Op::Text(_, _, 0, _, _)]
        ));
        assert_eq!(parse_xdot("t -1"), [Op::FontChar(u64::from(u32::MAX))]);
    }

    #[test]
    fn a_colour_must_start_as_parsexdotcolor_accepts() {
        assert!(parse_xdot("c 3 -!!! E 0 0 1 1").is_empty());
        assert!(parse_xdot("c 2 -\u{e9} E 0 0 1 1").is_empty());
        for c in ["#ff0000", "/x11/red", "red", "9"] {
            let s = format!("c {} -{c}", c.len());
            assert_eq!(parse_xdot(&s), [Op::Color(false, Grad::None(c.into()))]);
        }
    }
}
