//! Node drawing: port of the `*_gencode` functions of `lib/common/shapes.c` (`poly_gencode`,
//! `point_gencode`, `record_gencode`, `gen_fields`, `checkStyle`, `stylenode`, `penColor`,
//! `findFill`, `Mcircle_hack`) and of `round_corners` with its helpers.

use super::attrs::{StyleItem, parse_style};
use super::emit::{DEFAULT_COLOR, DEFAULT_FILL, find_stop_color};
use super::job::{EMIT_CLUSTERS_LAST, EmitState, Emitter, Engine, FILL, GRADIENT, RGRADIENT};
use super::model::TextLabel;
use super::num::fmt_g;
use graphviz_pure_core::Point;
pub(crate) use graphviz_pure_core::cnum::is_exactly_zero;
use graphviz_pure_core::math;
use graphviz_pure_geom::node_size::PolygonGeometry;
use graphviz_pure_geom::shapes::{RecordField, ShapeKind};

/// `IS_BOX` (`shapes.c:77-79`).
pub(crate) fn is_rect(p: &PolygonGeometry) -> bool {
    p.sides == 4
        && p.orientation.round() as i32 % 90 == 0
        && is_exactly_zero(p.distortion)
        && is_exactly_zero(p.skew)
}

/// `p_ellipse` (`shapes.c:4128-4142`).
pub(crate) fn p_ellipse(xrad: f64, yrad: f64, np: usize) -> Vec<Point> {
    let delta = 2.0 * std::f64::consts::PI / np as f64;
    let mut theta = 0.0;
    let mut af = Vec::with_capacity(np);
    for _ in 0..np {
        af.push(Point::new(xrad * math::cos(theta), yrad * math::sin(theta)));
        theta += delta;
    }
    af
}

/// `rect2poly` (`shapes.c:4120-4126`).
pub(crate) fn rect2poly(p: [Point; 2]) -> [Point; 4] {
    [
        p[0],
        Point::new(p[1].x, p[0].y),
        p[1],
        Point::new(p[0].x, p[1].y),
    ]
}

/// `graphviz_polygon_style_t.shape` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CornerShape {
    #[default]
    None,
    Dogear,
    Tab,
    Folder,
    Box3d,
    Component,
    Promoter,
    Cds,
    Terminator,
    Utr,
    Primersite,
    Restrictionsite,
    Fivepoverhang,
    Threepoverhang,
    Noverhang,
    Assembly,
    Signature,
    Insulator,
    Ribosite,
    Rnastab,
    Proteasesite,
    Proteinstab,
    Rpromoter,
    Rarrow,
    Larrow,
    Lpromoter,
    Cylinder,
}

/// `graphviz_polygon_style_t`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PolyStyle {
    pub filled: bool,
    pub radial: bool,
    pub rounded: bool,
    pub diagonals: bool,
    pub auxlabels: bool,
    pub invisible: bool,
    pub striped: bool,
    pub wedged: bool,
    pub underline: bool,
    pub shape: CornerShape,
}

impl PolyStyle {
    fn special_corners(&self) -> bool {
        self.rounded || self.diagonals || self.shape != CornerShape::None
    }
}

/// `polygon_t.option` of the built-in shapes.
fn shape_option(k: ShapeKind) -> PolyStyle {
    use CornerShape as C;
    use ShapeKind as S;
    let shape = |s| PolyStyle {
        shape: s,
        ..Default::default()
    };
    match k {
        S::Note => shape(C::Dogear),
        S::Tab => shape(C::Tab),
        S::Folder => shape(C::Folder),
        S::Box3d => shape(C::Box3d),
        S::Component => shape(C::Component),
        S::Cylinder => shape(C::Cylinder),
        S::Promoter => shape(C::Promoter),
        S::Cds => shape(C::Cds),
        S::Terminator => shape(C::Terminator),
        S::Utr => shape(C::Utr),
        S::Insulator => shape(C::Insulator),
        S::Ribosite => shape(C::Ribosite),
        S::Rnastab => shape(C::Rnastab),
        S::Proteasesite => shape(C::Proteasesite),
        S::Proteinstab => shape(C::Proteinstab),
        S::Primersite => shape(C::Primersite),
        S::Restrictionsite => shape(C::Restrictionsite),
        S::Fivepoverhang => shape(C::Fivepoverhang),
        S::Threepoverhang => shape(C::Threepoverhang),
        S::Noverhang => shape(C::Noverhang),
        S::Assembly => shape(C::Assembly),
        S::Signature => shape(C::Signature),
        S::Rpromoter => shape(C::Rpromoter),
        S::Rarrow => shape(C::Rarrow),
        S::Larrow => shape(C::Larrow),
        S::Lpromoter => shape(C::Lpromoter),
        S::Underline => PolyStyle {
            underline: true,
            ..Default::default()
        },
        S::Mdiamond | S::Mcircle => PolyStyle {
            diagonals: true,
            auxlabels: true,
            ..Default::default()
        },
        S::Msquare => PolyStyle {
            diagonals: true,
            ..Default::default()
        },
        _ => PolyStyle::default(),
    }
}

fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

/// `interpolate_pointf` (`lib/common/geomprocs.h`): `p + t * (q - p)`, which
/// Apple clang contracts into a fused multiply-add (`fmla.2d` in
/// `alloc_interpolation_points`), so the port fuses it explicitly.
fn interpolate(t: f64, p: Point, q: Point) -> Point {
    Point::new(t.mul_add(q.x - p.x, p.x), t.mul_add(q.y - p.y, p.y))
}

impl<E: Engine> Emitter<'_, E> {
    /// `checkStyle`: the style flags and the style list passed to the renderer.
    fn check_style(&self, n: usize) -> (PolyStyle, Option<Vec<StyleItem>>) {
        let nd = &self.job.m.nodes[n];
        let mut st = PolyStyle::default();
        let (_, desc) = nd.geom.shape.shape_desc();
        let is_box = desc.is_some_and(|p| {
            p.sides == 4
                && (p.orientation % 90.0).abs() < 0.5
                && is_exactly_zero(p.distortion)
                && is_exactly_zero(p.skew)
        });
        let is_ellipse = desc.is_some_and(|p| p.sides <= 2);
        let list = nd.attrs.nonempty("style").map(|s| {
            let mut kept = Vec::new();
            for it in parse_style(s) {
                match it.name.as_str() {
                    "filled" => {
                        st.filled = true;
                        kept.push(it);
                    }
                    "rounded" => st.rounded = true,
                    "diagonals" => st.diagonals = true,
                    "invis" => {
                        st.invisible = true;
                        kept.push(it);
                    }
                    "radial" => {
                        st.radial = true;
                        st.filled = true;
                    }
                    "striped" if is_box => st.striped = true,
                    "wedged" if is_ellipse => st.wedged = true,
                    _ => kept.push(it),
                }
            }
            kept
        });
        if desc.is_some() {
            let o = shape_option(nd.geom.shape);
            st.diagonals |= o.diagonals;
            st.auxlabels |= o.auxlabels;
            st.underline |= o.underline;
            st.rounded |= o.rounded;
            if o.shape != CornerShape::None {
                st.shape = o.shape;
            }
        }
        (st, list)
    }

    /// `stylenode`.
    fn stylenode(&mut self, n: usize) -> PolyStyle {
        let (st, list) = self.check_style(n);
        if let Some(l) = list {
            self.set_style(&l);
        }
        let a = self.job.m.nodes[n].attrs;
        if a.nonempty("penwidth").is_some() {
            let pw = a.late_double("penwidth", 1.0, 0.0);
            self.set_penwidth(pw);
        }
        st
    }

    /// `penColor`.
    fn pen_color(&mut self, n: usize) -> String {
        let a = self.job.m.nodes[n].attrs;
        let c = a.late_nnstring("color", DEFAULT_COLOR).to_string();
        self.set_pencolor(&c);
        c
    }

    /// `findFillDflt`.
    fn find_fill(&self, n: usize, dflt: &str) -> String {
        let a = self.job.m.nodes[n].attrs;
        a.nonempty("fillcolor")
            .or_else(|| a.nonempty("color"))
            .unwrap_or(dflt)
            .to_string()
    }

    /// Sets the fill for a filled shape: plain color or gradient. Returns the `filled` mode.
    fn set_node_fill(&mut self, n: usize, fillcolor: &str, radial: bool) -> i32 {
        if let Some((c0, c1, frac)) = find_stop_color(fillcolor) {
            self.set_fillcolor(&c0);
            let angle = self.job.m.nodes[n].attrs.late_int("gradientangle", 0, 0);
            self.set_gradient_vals(c1.as_deref().unwrap_or(DEFAULT_COLOR), angle, frac);
            if radial { RGRADIENT } else { GRADIENT }
        } else {
            self.set_fillcolor(fillcolor);
            FILL
        }
    }

    pub(crate) fn node_gencode(&mut self, n: usize) {
        let nd = &self.job.m.nodes[n];
        if nd.geom.record.is_some() {
            self.record_gencode(n);
        } else if nd.geom.shape == ShapeKind::Point {
            self.point_gencode(n);
        } else if nd.geom.shape == ShapeKind::Epsf {
            self.epsf_gencode(n);
        } else if nd.geom.polygon.is_some() {
            self.poly_gencode(n);
        }
    }

    /// `epsf_gencode`: nothing for a node whose `shapefile` did not load. C writes the
    /// PostScript call `X Y translate newpath user_shape_N` straight into the output file,
    /// whatever the format (`fprintf(job->output_file, ...)`), and so does the port.
    fn epsf_gencode(&mut self, n: usize) {
        let nd = &self.job.m.nodes[n];
        let Some((offset, macro_id)) = nd.geom.epsf else {
            return;
        };
        let do_map = self.anchored();
        if do_map && !(self.job.flags & EMIT_CLUSTERS_LAST != 0) {
            self.begin_obj_anchor();
        }
        let line = format!(
            "{} {} translate newpath user_shape_{macro_id}\n",
            fmt_g(nd.coord.x + offset.x, 5),
            fmt_g(nd.coord.y + offset.y, 5)
        );
        self.write_output_file(&line);
        let nd = &self.job.m.nodes[n];
        let mut label = nd.label.clone();
        label.pos = nd.coord;
        self.emit_label(EmitState::NLabel, &label);
        if do_map {
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.begin_obj_anchor();
            }
            self.end_anchor();
        }
    }

    fn anchored(&self) -> bool {
        let o = self.job.obj();
        o.url.is_some() || o.explicit_tooltip
    }

    fn poly_gencode(&mut self, n: usize) {
        let m = self.job.m;
        let nd = &m.nodes[n];
        let poly = nd.geom.polygon.as_ref().unwrap();
        let do_map = self.anchored();
        if do_map && self.job.flags & EMIT_CLUSTERS_LAST == 0 {
            self.begin_obj_anchor();
        }
        let sides = poly.sides;
        let mut peripheries = poly.peripheries;
        let xsize = (nd.lw + nd.rw) / (nd.geom.width * 72.0);
        let ysize = nd.ht / (nd.geom.height * 72.0);
        let style = self.stylenode(n);
        let mut fillcolor = String::new();
        let mut filled;
        if style.filled {
            fillcolor = self.find_fill(n, DEFAULT_FILL);
            filled = self.set_node_fill(n, &fillcolor.clone(), style.radial);
        } else if style.striped || style.wedged {
            fillcolor = self.find_fill(n, DEFAULT_FILL);
            filled = 1;
        } else {
            filled = 0;
        }
        let pencolor = self.pen_color(n);
        if peripheries == 0 && filled != 0 {
            peripheries = 1;
            self.set_pencolor("transparent");
        }
        let mut af = vec![Point::default(); sides];
        for j in 0..peripheries {
            for (i, a) in af.iter_mut().enumerate() {
                let p = poly.vertices[i + j * sides];
                // C (clang) fuses `P.x * xsize + ND_coord(n).x` (`fmla.2d`).
                *a = Point::new(
                    p.x.mul_add(xsize, nd.coord.x),
                    p.y.mul_add(ysize, nd.coord.y),
                );
            }
            if sides <= 2 {
                if style.wedged && j == 0 && fillcolor.contains(':') {
                    let fc = fillcolor.clone();
                    if self.wedged_ellipse(&af, &fc) > 1 {
                        graphviz_pure_core::diag::agerr_prev(&format!(
                            "in node {}\n",
                            self.job.m.nodes[n].name
                        ));
                    }
                    filled = 0;
                }
                self.ellipse([af[0], af[1]], filled);
                if style.diagonals {
                    self.mcircle_hack(n);
                }
            } else if style.striped {
                if j == 0 && self.striped_box(&af, &fillcolor, true) > 1 {
                    graphviz_pure_core::diag::agerr_prev(&format!(
                        "in node {}\n",
                        self.job.m.nodes[n].name
                    ));
                }
                self.polygon(&af, 0);
            } else if style.underline {
                self.set_pencolor("transparent");
                self.polygon(&af, filled);
                self.set_pencolor(&pencolor);
                self.polyline(&af[2..4]);
            } else if style.special_corners() {
                self.round_corners(&af, style, filled);
            } else {
                self.polygon(&af, filled);
            }
            filled = 0;
        }
        // a `custom` shape draws its `shapefile`, any other its `image`
        let name = if nd.geom.usershape {
            graphviz_pure_core::usershape::agget_shapefile(nd.attrs.get("shapefile"))
                .filter(|f| !f.is_empty())
        } else {
            nd.attrs.nonempty("image")
        };
        if let Some(name) = name {
            // the innermost periphery
            for (i, a) in af.iter_mut().enumerate() {
                let p = poly.vertices[i];
                *a = Point::new(
                    p.x.mul_add(xsize, nd.coord.x),
                    p.y.mul_add(ysize, nd.coord.y),
                );
            }
            let imagescale = nd.attrs.late_string("imagescale", "false");
            let imagepos = nd.attrs.late_string("imagepos", "mc");
            self.usershape(name, &af, filled != 0, imagescale, imagepos);
        }
        let mut label = nd.label.clone();
        label.pos = nd.coord;
        self.emit_label(EmitState::NLabel, &label);
        if do_map {
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.begin_obj_anchor();
            }
            self.end_anchor();
        }
    }

    /// `wedgedEllipse`: `parseSegs`' value (2 and 3 have printed a message).
    fn wedged_ellipse(&mut self, pf: &[Point], clrs: &str) -> i32 {
        let (segs, rv) = match super::emit::parse_segs(clrs) {
            Ok(x) => x,
            Err(e) => return e,
        };
        let ctr = Point::new((pf[0].x + pf[1].x) / 2.0, (pf[0].y + pf[1].y) / 2.0);
        let semi = Point::new(pf[1].x - ctr.x, pf[1].y - ctr.y);
        let save = self.job.obj().penwidth;
        if save > 0.5 {
            self.set_penwidth(0.5);
        }
        let mut angle0 = 0.0;
        let n = segs.len();
        for (i, s) in segs.iter().enumerate() {
            let Some(c) = &s.color else { break };
            if s.t <= 0.0 {
                continue;
            }
            self.set_fillcolor(c);
            let angle1 = if i + 1 == n {
                2.0 * std::f64::consts::PI
            } else {
                // C (clang) fuses `angle0 + 2 * M_PI * s.t`.
                (2.0 * std::f64::consts::PI).mul_add(s.t, angle0)
            };
            let pp = super::geom::elliptic_wedge(ctr, semi.x, semi.y, angle0, angle1);
            self.bezier(&pp, 1);
            angle0 = angle1;
        }
        if save > 0.5 {
            self.set_penwidth(save);
        }
        rv
    }

    fn mcircle_hack(&mut self, n: usize) {
        let nd = &self.job.m.nodes[n];
        let (y, x) = (0.7500, 0.6614);
        let p = Point::new(nd.rw * x, y * nd.ht / 2.0);
        let mut a0 = add(p, nd.coord);
        let mut a1 = Point::new(a0.x - 2.0 * p.x, a0.y);
        self.polyline(&[a0, a1]);
        a0.y -= 2.0 * p.y;
        a1.y = a0.y;
        self.polyline(&[a0, a1]);
    }

    fn point_gencode(&mut self, n: usize) {
        let m = self.job.m;
        let nd = &m.nodes[n];
        let poly = nd.geom.polygon.as_ref().unwrap();
        let do_map = self.anchored();
        if do_map && self.job.flags & EMIT_CLUSTERS_LAST == 0 {
            self.begin_obj_anchor();
        }
        let (st, _) = self.check_style(n);
        if st.invisible {
            self.set_style(&[StyleItem::simple("invis"), StyleItem::simple("filled")]);
        } else {
            self.set_style(&[StyleItem::simple("filled")]);
        }
        let pw = nd.attrs.late_double("penwidth", 1.0, 0.0);
        if nd.attrs.get("penwidth").is_some() || self.job.m.node_penwidth_declared {
            self.set_penwidth(pw);
        }
        let color = self.find_fill(n, "black");
        self.set_fillcolor(&color);
        self.pen_color(n);
        let mut peripheries = poly.peripheries;
        if peripheries == 0 {
            peripheries = 1;
            if !color.is_empty() {
                self.set_pencolor(&color);
            }
        }
        let mut filled = 1;
        for j in 0..peripheries {
            let v0 = poly.vertices[j * poly.sides];
            let v1 = poly.vertices[j * poly.sides + 1];
            self.ellipse([add(v0, nd.coord), add(v1, nd.coord)], filled);
            filled = 0;
        }
        if do_map {
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.begin_obj_anchor();
            }
            self.end_anchor();
        }
    }

    fn record_gencode(&mut self, n: usize) {
        let m = self.job.m;
        let nd = &m.nodes[n];
        let f = nd.geom.record.as_ref().unwrap();
        let mut bf = f.bbox;
        bf.ll = add(bf.ll, nd.coord);
        bf.ur = add(bf.ur, nd.coord);
        let do_map = self.anchored();
        if do_map && self.job.flags & EMIT_CLUSTERS_LAST == 0 {
            self.begin_obj_anchor();
        }
        let mut style = self.stylenode(n);
        self.pen_color(n);
        let filled = if style.filled {
            let fc = self.find_fill(n, DEFAULT_FILL);
            self.set_node_fill(n, &fc, style.radial)
        } else {
            0
        };
        if nd.geom.shape == ShapeKind::Mrecord {
            style.rounded = true;
        }
        if style.special_corners() {
            let af = [
                bf.ll,
                Point::new(bf.ur.x, bf.ll.y),
                bf.ur,
                Point::new(bf.ll.x, bf.ur.y),
            ];
            self.round_corners(&af, style, filled);
        } else {
            self.boxf(bf, filled);
        }
        self.gen_fields(n, f);
        if do_map {
            if self.job.flags & EMIT_CLUSTERS_LAST != 0 {
                self.begin_obj_anchor();
            }
            self.end_anchor();
        }
    }

    fn gen_fields(&mut self, n: usize, f: &RecordField) {
        let m = self.job.m;
        let nd = &m.nodes[n];
        if let Some(lg) = &f.label {
            let mut l = TextLabel::from_geometry(lg, &f.text, &nd.label.fontcolor);
            l.pos = add(
                Point::new(
                    (f.bbox.ll.x + f.bbox.ur.x) / 2.0,
                    (f.bbox.ll.y + f.bbox.ur.y) / 2.0,
                ),
                nd.coord,
            );
            l.set = true;
            self.emit_label(EmitState::NLabel, &l);
            self.pen_color(n);
        }
        for (i, sub) in f.children.iter().enumerate() {
            if i > 0 {
                let (a0, a1) = if f.horizontal {
                    (sub.bbox.ll, Point::new(sub.bbox.ll.x, sub.bbox.ur.y))
                } else {
                    (Point::new(sub.bbox.ll.x, sub.bbox.ur.y), sub.bbox.ur)
                };
                self.polyline(&[add(a0, nd.coord), add(a1, nd.coord)]);
            }
            self.gen_fields(n, sub);
        }
    }

    /// `alloc_interpolation_points`.
    fn interpolation_points(af: &[Point], style: PolyStyle, rounded: bool) -> Vec<Point> {
        let sides = af.len();
        let mut b = Vec::with_capacity(4 * sides + 4);
        let mut rbconst: f64 = 12.0;
        for seg in 0..sides {
            let p0 = af[seg];
            let p1 = if seg + 1 < sides { af[seg + 1] } else { af[0] };
            let d = math::hypot(p1.x - p0.x, p1.y - p0.y);
            rbconst = rbconst.min(d / 3.0);
        }
        for seg in 0..sides {
            let p0 = af[seg];
            let p1 = if seg + 1 < sides { af[seg + 1] } else { af[0] };
            let d = math::hypot(p1.x - p0.x, p1.y - p0.y);
            let mut t = rbconst / d;
            if matches!(style.shape, CornerShape::Box3d | CornerShape::Component) {
                t /= 3.0;
            } else if style.shape == CornerShape::Dogear {
                t /= 2.0;
            }
            if !rounded {
                b.push(p0);
            } else {
                b.push(interpolate(0.5 * t, p0, p1));
            }
            b.push(interpolate(t, p0, p1));
            b.push(interpolate(1.0 - t, p0, p1));
            if rounded {
                // `1.0 - RBCURVE * t`, which clang contracts to `fmadd(t, -0.5, 1.0)`. `0.5 * t`
                // is exact, so the result is the same unfused; the fma only mirrors C.
                b.push(interpolate(t.mul_add(-0.5, 1.0), p0, p1));
            }
        }
        b.push(b[0]);
        b.push(b[1]);
        b.push(b[2]);
        b
    }

    /// `round_corners`.
    pub(crate) fn round_corners(&mut self, af: &[Point], style: PolyStyle, filled: i32) {
        let sides = af.len();
        if style.diagonals {
            let b = Self::interpolation_points(af, style, false);
            self.polygon(af, filled);
            for seg in 0..sides {
                self.polyline(&[b[3 * seg + 2], b[3 * seg + 4]]);
            }
            return;
        }
        if style.shape == CornerShape::None {
            if style.rounded {
                let b = Self::interpolation_points(af, style, true);
                let mut pts = Vec::with_capacity(6 * sides + 2);
                for seg in 0..sides {
                    pts.push(b[4 * seg]);
                    pts.push(b[4 * seg + 1]);
                    pts.push(b[4 * seg + 1]);
                    pts.push(b[4 * seg + 2]);
                    pts.push(b[4 * seg + 2]);
                    pts.push(b[4 * seg + 3]);
                }
                pts.push(pts[0]);
                pts.push(pts[1]);
                self.bezier(&pts[1..], filled);
            }
            return;
        }
        if style.shape == CornerShape::Cylinder {
            let y02 = af[0].y + af[0].y;
            let mut v = [Point::default(); 7];
            v[0] = af[0];
            for i in 1..6 {
                v[i] = Point::new(af[i].x, y02 - af[i].y);
            }
            v[6] = af[6];
            self.bezier(af, filled);
            self.bezier(&v, 0);
            return;
        }
        let b = Self::interpolation_points(af, style, false);
        super::corners::draw_special(self, af, &b, style.shape, filled);
    }
}
