//! xdot renderer: port of the xdot half of `plugin/core/gvrender_core_dot.c`. Draw operations
//! are collected per emit state and attached as `_draw_`, `_ldraw_`, `_hdraw_`, `_tdraw_`,
//! `_hldraw_`, `_tldraw_` attributes.

use crate::emit::job::{
    EmitState, Engine, Features, GRADIENT, GVRENDER_DOES_MAPS, GVRENDER_DOES_TARGETS,
    GVRENDER_DOES_TOOLTIPS, GVRENDER_DOES_TRANSFORM, Job, LoadImage, ObjKind, RGRADIENT,
};
use crate::emit::model::TextSpan;
use crate::emit::num::{c_fixed_trim_into, xdot_num_into};
use graphviz_pure_core::math;
use graphviz_pure_core::usershape::UserShape;
use graphviz_pure_core::{BoundingBox, Point};
use std::fmt::Write;

/// Index of the buffer an emit state writes to (`xbufs[]`).
fn buf_of(s: EmitState) -> usize {
    match s {
        EmitState::GDraw => 0,
        EmitState::CDraw | EmitState::NDraw | EmitState::EDraw => 1,
        EmitState::TDraw => 2,
        EmitState::HDraw => 3,
        EmitState::GLabel => 4,
        EmitState::CLabel | EmitState::NLabel | EmitState::ELabel => 5,
        EmitState::TLabel => 6,
        EmitState::HLabel => 7,
    }
}

fn escape_backslashes(s: &str) -> String {
    s.replace('\\', "\\\\")
}

pub(crate) struct XdotEngine {
    /// The versioned format (`xdot1.2`, `xdot1.4`) this engine renders, if any.
    fixed: Option<&'static str>,
    pub version: u16,
    pub version_s: String,
    bufs: [String; 8],
    penwidth: [f64; 12],
    textflags: [u32; 12],
    /// Attributes to set: object, name, value.
    pub sets: Vec<(ObjKind, &'static str, String)>,
    /// `xd->yOff`: the offset `attach_attrs_and_arrows` returned, for `yDir`.
    y_off: f64,
}

fn version_num(s: &str) -> u16 {
    let mut us: u32 = 0;
    for c in s.bytes().filter(u8::is_ascii_digit) {
        if us > (u32::from(u16::MAX) - u32::from(c - b'0')) / 10 {
            break;
        }
        us = us * 10 + u32::from(c - b'0');
    }
    us as u16
}

impl XdotEngine {
    /// `xdot_begin_graph` for format `xdot` (`fixed` is `Some("1.2")`/`Some("1.4")` for the
    /// versioned formats).
    pub fn new(xdotversion_attr: Option<&str>, fixed: Option<&str>, y_off: f64) -> Self {
        let (version, version_s) = match fixed {
            Some("1.4") => (14, "1.4".to_string()),
            Some("1.2") => (12, "1.2".to_string()),
            _ => match xdotversion_attr.filter(|s| !s.is_empty()) {
                Some(s) if version_num(s) > 10 => (version_num(s), s.to_string()),
                _ => (17, "1.7".to_string()),
            },
        };
        XdotEngine {
            fixed: match fixed {
                Some("1.4") => Some("xdot1.4"),
                Some("1.2") => Some("xdot1.2"),
                _ => None,
            },
            version,
            version_s,
            bufs: Default::default(),
            penwidth: [1.0; 12],
            textflags: [0; 12],
            sets: Vec::new(),
            y_off,
        }
    }

    fn buf(&mut self, j: &Job) -> &mut String {
        &mut self.bufs[buf_of(j.obj().emit_state)]
    }

    fn str_(&mut self, j: &Job, pfx: &str, s: &str) {
        let b = self.buf(j);
        let _ = write!(
            b,
            "{pfx}{} -{s} ",
            graphviz_pure_core::charset::encode(s).len()
        );
    }

    fn color_str(rgba: [u8; 4]) -> String {
        if rgba[3] == 0xff {
            format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2])
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                rgba[0], rgba[1], rgba[2], rgba[3]
            )
        }
    }

    fn pencolor(&mut self, j: &Job) {
        let c = Self::color_str(j.obj().pencolor.rgba());
        self.str_(j, "c ", &c);
    }

    fn fillcolor(&mut self, j: &Job) {
        let c = Self::color_str(j.obj().fillcolor.rgba());
        self.str_(j, "C ", &c);
    }

    /// `xdot_point`: `y` through `yDir` with the offset `attach_attrs_and_arrows` returned.
    fn point(buf: &mut String, p: Point, y_off: f64) {
        xdot_num_into(buf, p.x);
        buf.push(' ');
        xdot_num_into(buf, crate::emit::y_dir(p.y, y_off));
        buf.push(' ');
    }

    fn points(&mut self, j: &Job, c: char, a: &[Point]) {
        let y_off = self.y_off;
        let b = self.buf(j);
        let _ = write!(b, "{c} {} ", a.len());
        for p in a {
            Self::point(b, *p, y_off);
        }
    }

    fn style(&mut self, j: &Job) {
        let o = j.obj();
        let es = o.emit_state as usize;
        if (o.penwidth - self.penwidth[es]).abs() >= 0.0005 {
            self.penwidth[es] = o.penwidth;
            let mut s = String::from("setlinewidth(");
            c_fixed_trim_into(&mut s, o.penwidth, 3);
            s.push(')');
            self.str_(j, "S ", &s);
        }
        let Some(raw) = o.rawstyle.clone() else {
            return;
        };
        for it in raw {
            if matches!(it.name.as_str(), "filled" | "bold" | "setlinewidth") {
                continue;
            }
            let mut s = it.name.clone();
            if !it.args.is_empty() {
                s.push('(');
                s.push_str(&it.args.join(","));
                s.push(')');
            }
            self.str_(j, "S ", &s);
        }
    }

    fn gradient_fillcolor(&mut self, j: &Job, filled: i32, a: &[Point]) {
        let o = j.obj();
        if self.version < 14 {
            self.fillcolor(j);
            return;
        }
        let angle = o.gradient_angle as f64 * std::f64::consts::PI / 180.0;
        let mut xb = String::new();
        if filled == GRADIENT {
            let g = crate::emit::geom::gradient_points(a, angle, 2);
            xb.push('[');
            Self::point(&mut xb, g[0], self.y_off);
            Self::point(&mut xb, g[1], self.y_off);
        } else {
            let g = crate::emit::geom::gradient_points(a, 0.0, 3);
            let r2 = g[1].y;
            let c1 = if o.gradient_angle == 0 {
                g[0]
            } else {
                // C (clang) fuses `G[0] + r2 / 4 * cos/sin(angle)`.
                Point::new(
                    math::fma(r2 / 4.0, math::cos(angle), g[0].x),
                    math::fma(r2 / 4.0, math::sin(angle), g[0].y),
                )
            };
            let r1 = r2 / 4.0;
            xb.push('(');
            Self::point(&mut xb, c1, self.y_off);
            xdot_num_into(&mut xb, r1);
            xb.push(' ');
            Self::point(&mut xb, g[0], self.y_off);
            xdot_num_into(&mut xb, r2);
            xb.push(' ');
        }
        xb.push_str("2 ");
        let stop = |xb: &mut String, v: f64, c: [u8; 4]| {
            c_fixed_trim_into(xb, v, 3);
            xb.push(' ');
            let cs = Self::color_str(c);
            let _ = write!(
                xb,
                "{} -{cs} ",
                graphviz_pure_core::charset::encode(&cs).len()
            );
        };
        if o.gradient_frac > 0.0 {
            stop(&mut xb, o.gradient_frac, o.fillcolor.rgba());
            stop(&mut xb, o.gradient_frac, o.stopcolor.rgba());
        } else {
            stop(&mut xb, 0.0, o.fillcolor.rgba());
            stop(&mut xb, 1.0, o.stopcolor.rgba());
        }
        xb.pop();
        xb.push(if filled == GRADIENT { ']' } else { ')' });
        self.str_(j, "C ", &xb);
    }

    fn fill(&mut self, j: &Job, filled: i32, a: &[Point]) {
        if filled == GRADIENT || filled == RGRADIENT {
            self.gradient_fillcolor(j, filled, a);
        } else {
            self.fillcolor(j);
        }
    }

    fn take(&mut self, i: usize) -> String {
        std::mem::take(&mut self.bufs[i])
    }

    fn reset(&mut self, states: &[EmitState]) {
        for s in states {
            self.penwidth[*s as usize] = 1.0;
            self.textflags[*s as usize] = 0;
        }
    }
}

impl Engine for XdotEngine {
    fn render_type(&self) -> &str {
        "xdot"
    }

    /// `core_loadimage_xdot` (`plugin/core/gvrender_core_dot.c`): an `I` operation.
    fn loadimage(&mut self, j: &Job, li: LoadImage, us: &UserShape, b: BoundingBox) {
        if li != LoadImage::Xdot {
            return;
        }
        let y_off = self.y_off;
        let buf = self.buf(j);
        buf.push_str("I ");
        Self::point(buf, b.ll, y_off);
        for v in [b.ur.x - b.ll.x, b.ur.y - b.ll.y] {
            xdot_num_into(buf, v);
            buf.push(' ');
        }
        self.str_(j, "", &us.name);
    }

    fn output_langname(&self) -> &str {
        match self.fixed {
            Some(v) => v,
            None => "xdot",
        }
    }

    fn features(&self) -> Features {
        Features {
            flags: GVRENDER_DOES_TRANSFORM
                | GVRENDER_DOES_MAPS
                | GVRENDER_DOES_TARGETS
                | GVRENDER_DOES_TOOLTIPS,
            default_pad: 0.0,
            knowncolors: &[],
            rgba: true,
            default_dpi: 72.0,
        }
    }

    fn end_node(&mut self, j: &Job) {
        let kind = j.obj().kind;
        if !self.bufs[1].is_empty() {
            let v = self.take(1);
            self.sets.push((kind, "_draw_", v));
        }
        if !self.bufs[5].is_empty() {
            let v = escape_backslashes(&self.take(5));
            self.sets.push((kind, "_ldraw_", v));
        }
        self.reset(&[EmitState::NDraw, EmitState::NLabel]);
    }

    fn end_edge(&mut self, j: &Job) {
        let kind = j.obj().kind;
        for (i, name, esc) in [
            (1, "_draw_", false),
            (2, "_tdraw_", false),
            (3, "_hdraw_", false),
            (5, "_ldraw_", true),
            (6, "_tldraw_", false),
            (7, "_hldraw_", false),
        ] {
            if !self.bufs[i].is_empty() {
                let v = self.take(i);
                self.sets
                    .push((kind, name, if esc { escape_backslashes(&v) } else { v }));
            }
        }
        self.reset(&[
            EmitState::EDraw,
            EmitState::ELabel,
            EmitState::TDraw,
            EmitState::HDraw,
            EmitState::TLabel,
            EmitState::HLabel,
        ]);
    }

    fn end_cluster(&mut self, j: &Job) {
        let kind = j.obj().kind;
        let v = self.take(1);
        self.sets.push((kind, "_draw_", v));
        if let ObjKind::Cluster(c) = kind
            && j.m.clusters[c].label.is_some()
        {
            let v = self.take(5);
            self.sets.push((kind, "_ldraw_", v));
        }
        self.reset(&[EmitState::CDraw, EmitState::CLabel]);
    }

    fn end_graph(&mut self, j: &Job) {
        if !self.bufs[0].is_empty() {
            let v = self.take(0);
            self.sets.push((ObjKind::Root, "_draw_", v));
        }
        if j.m.label.is_some() {
            let v = escape_backslashes(&self.take(4));
            self.sets.push((ObjKind::Root, "_ldraw_", v));
        }
        let vs = self.version_s.clone();
        self.sets.push((ObjKind::Root, "xdotversion", vs));
    }

    fn textspan(&mut self, j: &Job, mut p: Point, span: &TextSpan) {
        let es = j.obj().emit_state as usize;
        {
            let b = self.buf(j);
            b.push_str("F ");
            xdot_num_into(b, span.font.size);
            b.push(' ');
        }
        self.str_(j, "", &span.font.name);
        self.pencolor(j);
        let jst = match span.just {
            'l' => -1,
            'r' => 1,
            _ => 0,
        };
        let masks = [0x1Fu32, 0x3F, 0x7F];
        if self.version >= 15 && usize::from(self.version - 15) < masks.len() {
            let bits = u32::from(span.font.flags) & masks[usize::from(self.version - 15)];
            if self.textflags[es] != bits {
                let b = self.buf(j);
                let _ = write!(b, "t {bits} ");
                self.textflags[es] = bits;
            }
        }
        p.y += span.yoffset_centerline;
        {
            let y_off = self.y_off;
            let b = self.buf(j);
            b.push_str("T ");
            Self::point(b, p, y_off);
            let _ = write!(b, "{jst} ");
            xdot_num_into(b, span.width);
            b.push(' ');
        }
        self.str_(j, "", &span.str);
    }

    fn ellipse(&mut self, j: &Job, a: [Point; 2], filled: i32) {
        self.style(j);
        self.pencolor(j);
        if filled != 0 {
            self.fill(j, filled, &a);
            self.buf(j).push_str("E ");
        } else {
            self.buf(j).push_str("e ");
        }
        let y_off = self.y_off;
        let b = self.buf(j);
        Self::point(b, a[0], y_off);
        xdot_num_into(b, a[1].x - a[0].x);
        b.push(' ');
        xdot_num_into(b, a[1].y - a[0].y);
        b.push(' ');
    }

    fn bezier(&mut self, j: &Job, a: &[Point], filled: i32) {
        self.style(j);
        self.pencolor(j);
        if filled != 0 {
            self.fill(j, filled, a);
            self.points(j, 'b', a);
        } else {
            self.points(j, 'B', a);
        }
    }

    fn polygon(&mut self, j: &Job, a: &[Point], filled: i32) {
        self.style(j);
        self.pencolor(j);
        if filled != 0 {
            self.fill(j, filled, a);
            self.points(j, 'P', a);
        } else {
            self.points(j, 'p', a);
        }
    }

    fn polyline(&mut self, j: &Job, a: &[Point]) {
        self.style(j);
        self.pencolor(j);
        self.points(j, 'L', a);
    }
}
