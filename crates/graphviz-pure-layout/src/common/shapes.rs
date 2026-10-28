//! Node shape geometry for edge clipping, ported from `lib/common/shapes.c`.
//!
//! C stores, per node, a `polygon_t` built by `poly_init`/`point_init`: the
//! vertices of every periphery plus one extra "outline" periphery half a
//! penwidth outside the outermost one. `clip_and_install` clips edges with the
//! shape's `insidefn` (`poly_inside`, `point_inside`, `record_inside`,
//! `epsf_inside`, `star_inside`), so edges end on that outline.
//!
//! Node *sizes* come from `graphviz_pure_geom::shapes` (shared with the other engines);
//! this module rebuilds C's vertex arrays from the final size, which agrees with
//! `poly_init` up to rounding because the vertices scale linearly with the size.

use graphviz_pure_core::alloc::{AllocFailure, element_count, gv_calloc_grow, gv_calloc_vec};
use graphviz_pure_core::cnum::{c_max, c_min, is_exactly_equal, is_exactly_zero};
use graphviz_pure_core::math;
use graphviz_pure_core::{NodeData, Point};

/// Whitespace in points between peripheries (`GAP` in `const.h`).
pub const GAP: f64 = 4.0;
const DEFAULT_NODEPENWIDTH: f64 = 1.0;

/// Which C `insidefn` a shape uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsideFn {
    Poly,
    Point,
    Record,
    Epsf,
    Star,
}

/// The parts of C's `polygon_t` and node size fields used by the inside tests.
#[derive(Debug, Clone)]
pub struct NodeShape {
    pub inside: InsideFn,
    /// `ND_shape(n)->name`.
    pub name: String,
    pub sides: usize,
    pub peripheries: usize,
    pub orientation: f64,
    pub distortion: f64,
    pub skew: f64,
    pub regular: bool,
    /// `sides * (peripheries + outline)` vertices, periphery-major.
    pub vertices: Vec<Point>,
    /// `ND_width`, `ND_height` in points.
    pub width: f64,
    pub height: f64,
    /// `ND_outline_width`, `ND_outline_height` in points.
    pub outline_width: f64,
    pub outline_height: f64,
    pub penwidth: f64,
    /// True for shapes with no `insidefn` (user shapes); edges are not clipped.
    pub no_clip: bool,
    /// `option.fixedshape` (`fixedsize=shape`): the inside test uses the
    /// polygon's own bounding box (`polyBB`) instead of the node size.
    pub fixedshape: bool,
    /// Box of the record's root field (`((field_t *)ND_shape_info(n))->b`),
    /// which `record_inside` tests against; unlike `ND_ht` it lacks the
    /// `+1` of `record_init`.
    pub record_box: Option<graphviz_pure_core::BoundingBox>,
}

/// Static shape description (`polygon_t` initialisers and the `Shapes[]` table).
#[derive(Debug, Clone, Copy)]
struct Desc {
    inside: InsideFn,
    regular: bool,
    peripheries: usize,
    sides: usize,
    orientation: f64,
    distortion: f64,
    skew: f64,
    vgen: Gen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gen {
    Regular,
    Star,
    Cylinder,
}

const fn d(peripheries: usize, sides: usize) -> Desc {
    Desc {
        inside: InsideFn::Poly,
        regular: false,
        peripheries,
        sides,
        orientation: 0.0,
        distortion: 0.0,
        skew: 0.0,
        vgen: Gen::Regular,
    }
}

fn describe(name: &str) -> Desc {
    let mut x = match name {
        "box" | "rect" | "rectangle" => d(1, 4),
        "polygon" => d(1, 0),
        "ellipse" | "oval" => d(1, 1),
        "circle" => Desc {
            regular: true,
            ..d(1, 1)
        },
        "point" => Desc {
            regular: true,
            inside: InsideFn::Point,
            ..d(1, 1)
        },
        "egg" => Desc {
            distortion: -0.3,
            ..d(1, 1)
        },
        "triangle" => d(1, 3),
        "none" | "plaintext" | "plain" => d(0, 4),
        "diamond" => Desc {
            orientation: 45.0,
            ..d(1, 4)
        },
        "trapezium" => Desc {
            distortion: -0.4,
            ..d(1, 4)
        },
        "parallelogram" => Desc {
            skew: 0.6,
            ..d(1, 4)
        },
        "house" => Desc {
            distortion: -0.64,
            ..d(1, 5)
        },
        "pentagon" => d(1, 5),
        "hexagon" => d(1, 6),
        "septagon" => d(1, 7),
        "octagon" => d(1, 8),
        "note" | "tab" | "folder" | "box3d" | "component" | "underline" => d(1, 4),
        "cylinder" => Desc {
            vgen: Gen::Cylinder,
            ..d(1, 19)
        },
        "square" => Desc {
            regular: true,
            ..d(1, 4)
        },
        "doublecircle" => Desc {
            regular: true,
            ..d(2, 1)
        },
        "doubleoctagon" => d(2, 8),
        "tripleoctagon" => d(3, 8),
        "invtriangle" => Desc {
            orientation: 180.0,
            ..d(1, 3)
        },
        "invtrapezium" => Desc {
            orientation: 180.0,
            distortion: -0.4,
            ..d(1, 4)
        },
        "invhouse" => Desc {
            orientation: 180.0,
            distortion: -0.64,
            ..d(1, 5)
        },
        "Mdiamond" => Desc {
            orientation: 45.0,
            ..d(1, 4)
        },
        "Msquare" => Desc {
            regular: true,
            ..d(1, 4)
        },
        "Mcircle" => Desc {
            regular: true,
            ..d(1, 1)
        },
        "promoter" | "cds" | "terminator" | "utr" | "insulator" | "ribosite" | "rnastab"
        | "proteasesite" | "proteinstab" | "primersite" | "restrictionsite" | "fivepoverhang"
        | "threepoverhang" | "noverhang" | "assembly" | "signature" | "rpromoter" | "rarrow"
        | "larrow" | "lpromoter" => d(1, 4),
        "record" | "Mrecord" => Desc {
            inside: InsideFn::Record,
            ..d(1, 4)
        },
        "epsf" => Desc {
            inside: InsideFn::Epsf,
            ..d(1, 4)
        },
        "star" => Desc {
            inside: InsideFn::Star,
            vgen: Gen::Star,
            ..d(1, 10)
        },
        // Unknown names bind to Shapes[0], the box.
        _ => d(1, 4),
    };
    if x.inside == InsideFn::Point {
        x.sides = 2;
    }
    x
}

fn attr_f64(node: &NodeData, key: &str) -> Option<f64> {
    let s = node.attrs.get(key)?;
    graphviz_pure_core::strtod::strtod_rest(s).map(|(v, _)| v)
}

/// `late_double`: attribute value if it parses, clamped below by `low`.
fn late_double(node: &NodeData, key: &str, def: f64, low: f64) -> f64 {
    match attr_f64(node, key) {
        Some(v) if v < low => low,
        Some(v) => v,
        None => def,
    }
}

fn late_int(node: &NodeData, key: &str, def: i32, low: i32) -> i32 {
    crate::common::utils::late_int(&node.attrs, key, def, low)
}

const ALPHA: f64 = std::f64::consts::PI / 10.0;
const ALPHA2: f64 = 2.0 * ALPHA;
const ALPHA3: f64 = 3.0 * ALPHA;
const ALPHA4: f64 = 2.0 * ALPHA2;

/// `star_vertices`: fills 10 vertices for `sz`, returning the aspect-adjusted size.
fn star_vertices(sz0: Point) -> (Vec<Point>, Point) {
    let mut sz = sz0;
    let aspect = (1.0 + math::sin(ALPHA3)) / (2.0 * math::cos(ALPHA));
    let a = sz.y / sz.x;
    if a > aspect {
        sz.x = sz.y / aspect;
    } else if a < aspect {
        sz.y = sz.x * aspect;
    }
    let r = sz.x / (2.0 * math::cos(ALPHA));
    let r0 = r * math::cos(ALPHA) * math::cos(ALPHA4) / (math::sin(ALPHA4) * math::cos(ALPHA2));
    let offset = (r * (1.0 - math::sin(ALPHA3))) / 2.0;
    let mut theta = ALPHA;
    let mut v = vec![Point::default(); 10];
    let mut i = 0;
    while i < 10 {
        // C (clang) fuses `r * sin(theta) - offset`.
        v[i] = Point::new(r * math::cos(theta), r.mul_add(math::sin(theta), -offset));
        theta += ALPHA2;
        v[i + 1] = Point::new(r0 * math::cos(theta), r0.mul_add(math::sin(theta), -offset));
        theta += ALPHA2;
        i += 2;
    }
    (v, sz)
}

fn cylinder_vertices(bb: Point) -> Vec<Point> {
    let x = bb.x / 2.0;
    let y = bb.y / 2.0;
    let yr = bb.y / 11.0;
    let mut v = vec![Point::default(); 19];
    v[0] = Point::new(x, y - yr);
    // C (clang) fuses `y - (1 - 0.551784) * yr`.
    v[1] = Point::new(x, (-(1.0 - 0.551784f64)).mul_add(yr, y));
    v[2] = Point::new(0.551784 * x, y);
    v[3] = Point::new(0.0, y);
    v[4] = Point::new(-0.551784 * x, y);
    v[5] = Point::new(-x, v[1].y);
    v[6] = Point::new(-x, y - yr);
    v[7] = v[6];
    v[8] = Point::new(-x, yr - y);
    v[9] = v[8];
    v[10] = Point::new(-x, -v[1].y);
    v[11] = Point::new(v[4].x, -v[4].y);
    v[12] = Point::new(v[3].x, -v[3].y);
    v[13] = Point::new(v[2].x, -v[2].y);
    v[14] = Point::new(v[1].x, -v[1].y);
    v[15] = Point::new(v[0].x, -v[0].y);
    v[16] = v[15];
    v[17] = v[0];
    v[18] = v[0];
    v
}

/// Unit polygon of `poly_init` for `bb = (1, 1)`, before scaling.
fn regular_unit_vertices(sides: usize, orientation: f64, distortion: f64, skew: f64) -> Vec<Point> {
    use std::f64::consts::{PI, SQRT_2};
    let sectorangle = 2.0 * PI / sides as f64;
    let sidelength = math::sin(sectorangle / 2.0);
    let skewdist = math::hypot(distortion.abs() + skew.abs(), 1.0);
    let gdistortion = distortion * SQRT_2 / math::cos(sectorangle / 2.0);
    let gskew = skew / 2.0;
    let mut angle = (sectorangle - PI) / 2.0;
    let (mut sinx, mut cosx) = (math::sin(angle), math::cos(angle));
    let mut r = Point::new(0.5 * cosx, 0.5 * sinx);
    angle += (PI - sectorangle) / 2.0;
    let is_box = sides == 4
        && (orientation % 90.0).abs() < 0.5
        && is_exactly_zero(distortion)
        && is_exactly_zero(skew);
    let mut v = vec![Point::default(); sides];
    for i in 0..sides {
        angle += sectorangle;
        sinx = math::sin(angle);
        cosx = math::cos(angle);
        // C (clang) fuses these multiply-adds (the `fmadd`s in poly_init of libgvc).
        r.x = sidelength.mul_add(cosx, r.x);
        r.y = sidelength.mul_add(sinx, r.y);
        let mut p = Point::new(
            r.x.mul_add(r.y.mul_add(gdistortion, skewdist), r.y * gskew),
            r.y,
        );
        // RADIANS(orientation) + atan2(P.y, P.x), fused
        let alpha = (orientation / 180.0).mul_add(PI, math::atan2(p.y, p.x));
        let (s, c) = (math::sin(alpha), math::cos(alpha));
        let h = math::hypot(p.x, p.y);
        p.x = h * c;
        p.y = h * s;
        v[i] = p;
        if is_box {
            v[1] = Point::new(-p.x, p.y);
            v[2] = Point::new(-p.x, -p.y);
            v[3] = Point::new(p.x, -p.y);
            break;
        }
    }
    v
}

/// Adds periphery and outline vertices to the base polygon (the `outp > 1`
/// branch of `poly_init`). Returns the outer-periphery and outline sizes.
fn add_peripheries(
    v: &mut Vec<Point>,
    sides: usize,
    peripheries: usize,
    outp: usize,
    penwidth: f64,
) -> Result<(Point, Point), AllocFailure> {
    // `gv_calloc(outp * sides, sizeof(pointf))`
    gv_calloc_grow(v, element_count(outp, sides))?;
    let mut bb = Point::new(0.0, 0.0);
    for p in &v[..sides] {
        bb.x = bb.x.max(2.0 * p.x.abs());
        bb.y = bb.y.max(2.0 * p.y.abs());
    }
    let mut outline_bb = bb;
    if outp <= 1 {
        return Ok((bb, outline_bb));
    }
    let (mut sinx, mut cosx) = (0.0f64, 0.0f64);
    let i_end = sides;
    let rr = v[0];
    let mut q = v[0];
    for j in 1..sides {
        q = v[(i_end - j) % sides];
        if !is_exactly_equal(q.x, rr.x) || !is_exactly_equal(q.y, rr.y) {
            break;
        }
    }
    let mut beta = math::atan2(rr.y - q.y, rr.x - q.x);
    let mut qprev = q;
    for i in 0..sides {
        let mut q = v[i];
        if !(is_exactly_equal(q.x, qprev.x) && is_exactly_equal(q.y, qprev.y)) {
            let mut r = q;
            for j in 1..sides {
                r = v[(i + j) % sides];
                if !is_exactly_equal(r.x, q.x) || !is_exactly_equal(r.y, q.y) {
                    break;
                }
            }
            let alpha = beta;
            beta = math::atan2(r.y - q.y, r.x - q.x);
            let gamma = (alpha + std::f64::consts::PI - beta) / 2.0;
            let temp = GAP / math::sin(gamma);
            sinx = math::sin(alpha - gamma) * temp;
            cosx = math::cos(alpha - gamma) * temp;
        }
        qprev = q;
        for j in 1..peripheries {
            q.x += cosx;
            q.y += sinx;
            v[i + j * sides] = q;
        }
        if outp > peripheries {
            q.x += cosx * penwidth / 2.0 / GAP;
            q.y += sinx * penwidth / 2.0 / GAP;
            v[i + peripheries * sides] = q;
        }
    }
    let po = peripheries.max(1);
    for i in 0..sides {
        let p = v[i + (po - 1) * sides];
        bb.x = bb.x.max(2.0 * p.x.abs());
        bb.y = bb.y.max(2.0 * p.y.abs());
        let q = v[i + (outp - 1) * sides];
        outline_bb.x = outline_bb.x.max(2.0 * q.x.abs());
        outline_bb.y = outline_bb.y.max(2.0 * q.y.abs());
    }
    Ok((bb, outline_bb))
}

impl NodeShape {
    /// Builds the C shape data of `node`, whose final `width`/`height` (inches)
    /// have already been computed.
    ///
    /// The vertices of the peripheries are C's `gv_calloc` array, which reports a
    /// failed allocation and exits; the port returns the error.
    pub fn new(node: &NodeData) -> Result<Self, AllocFailure> {
        let name = node
            .attrs
            .get("shape")
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("ellipse")
            .to_string();
        let desc = describe(&name);
        let width = node.width * 72.0;
        let height = node.height * 72.0;
        let penwidth = late_double(node, "penwidth", DEFAULT_NODEPENWIDTH, 0.0);
        let peripheries = late_int(node, "peripheries", desc.peripheries as i32, 0) as usize;
        let mut sides = desc.sides;
        let mut orientation = desc.orientation;
        let mut distortion = desc.distortion;
        let mut skew = desc.skew;
        let regular = desc.regular
            || crate::common::utils::mapbool(node.attrs.get("regular").map(String::as_str));
        let mut shape = NodeShape {
            inside: desc.inside,
            name: name.clone(),
            sides,
            peripheries,
            orientation,
            distortion,
            skew,
            regular,
            vertices: Vec::new(),
            width,
            height,
            outline_width: width,
            outline_height: height,
            penwidth,
            no_clip: false,
            fixedshape: node
                .attrs
                .get("fixedsize")
                .is_some_and(|v| v.eq_ignore_ascii_case("shape")),
            record_box: None,
        };
        match desc.inside {
            InsideFn::Record | InsideFn::Epsf => {
                shape.outline_width = width;
                shape.outline_height = height;
                return Ok(shape);
            }
            InsideFn::Point => {
                // point_init: sz = width; extra peripheries add GAP each.
                let outp_base = peripheries.max(1);
                let outp = if peripheries >= 1 && penwidth > 0.0 {
                    outp_base + 1
                } else {
                    outp_base
                };
                // `gv_calloc(outp * sides, sizeof(pointf))`
                let mut v = gv_calloc_vec::<Point>(element_count(outp, 2))?;
                let inner = width / 2.0 - GAP * peripheries.saturating_sub(1) as f64;
                let mut p = Point::new(inner, inner);
                v[0] = Point::new(-p.x, -p.y);
                v[1] = p;
                let mut i = 2;
                for _ in 1..peripheries {
                    p.x += GAP;
                    p.y += GAP;
                    v[i] = Point::new(-p.x, -p.y);
                    v[i + 1] = p;
                    i += 2;
                }
                if peripheries >= 1 && penwidth > 0.0 && outp > peripheries {
                    p.x += penwidth / 2.0;
                    p.y += penwidth / 2.0;
                    v[i] = Point::new(-p.x, -p.y);
                    v[i + 1] = p;
                }
                shape.sides = 2;
                shape.outline_width = 2.0 * p.x;
                shape.outline_height = 2.0 * p.x;
                shape.vertices = v;
                return Ok(shape);
            }
            _ => {}
        }
        orientation += late_double(node, "orientation", 0.0, -360.0);
        if sides == 0 {
            skew = late_double(node, "skew", 0.0, -100.0);
            sides = late_int(node, "sides", 4, 0) as usize;
            distortion = late_double(node, "distortion", 0.0, -100.0);
        }
        if sides <= 2 && (!is_exactly_zero(distortion) || !is_exactly_zero(skew)) {
            sides = 120;
        }
        let mut outp = peripheries.max(1);
        if peripheries >= 1 && penwidth > 0.0 {
            outp += 1;
        }
        if sides < 3 {
            // Ellipses: two vertices per periphery.
            // `gv_calloc(outp * sides, sizeof(pointf))`
            let mut v = gv_calloc_vec::<Point>(element_count(outp, 2))?;
            let inner_x = width / 2.0 - GAP * peripheries.saturating_sub(1) as f64;
            let inner_y = height / 2.0 - GAP * peripheries.saturating_sub(1) as f64;
            let mut p = Point::new(inner_x, inner_y);
            v[0] = Point::new(-p.x, -p.y);
            v[1] = p;
            let mut i = 2;
            for _ in 1..peripheries {
                p.x += GAP;
                p.y += GAP;
                v[i] = Point::new(-p.x, -p.y);
                v[i + 1] = p;
                i += 2;
            }
            let (mut ow, mut oh) = (width, height);
            if outp > peripheries {
                p.x += penwidth / 2.0;
                p.y += penwidth / 2.0;
                let i = 2 * peripheries;
                v[i] = Point::new(-p.x, -p.y);
                v[i + 1] = p;
                ow = 2.0 * p.x;
                oh = 2.0 * p.y;
            }
            shape.sides = 2;
            shape.vertices = v;
            shape.outline_width = ow;
            shape.outline_height = oh;
        } else {
            let (unit, xmax, ymax) = match desc.vgen {
                Gen::Star => {
                    let (v, sz) = star_vertices(Point::new(1.0, 1.0));
                    (v, sz.x / 2.0, sz.y / 2.0)
                }
                Gen::Cylinder => (cylinder_vertices(Point::new(1.0, 1.0)), 0.5, 0.5),
                Gen::Regular => {
                    let v = regular_unit_vertices(sides, orientation, distortion, skew);
                    let xm = v.iter().fold(0.0f64, |m, p| m.max(p.x.abs()));
                    let ym = v.iter().fold(0.0f64, |m, p| m.max(p.y.abs()));
                    (v, xm, ym)
                }
            };
            // Base polygon size: the outer periphery must come out at the
            // node's size, so shrink the base for extra peripheries.
            let mut base = Point::new(width, height);
            let mut v;
            let mut iter = 0;
            loop {
                let sx = base.x / (2.0 * xmax);
                let sy = base.y / (2.0 * ymax);
                v = unit
                    .iter()
                    .map(|p| Point::new(p.x * sx, p.y * sy))
                    .collect::<Vec<_>>();
                let (bb, outline) = add_peripheries(&mut v, sides, peripheries, outp, penwidth)?;
                shape.outline_width = outline.x;
                shape.outline_height = outline.y;
                if peripheries <= 1 || iter >= 8 {
                    break;
                }
                let (ex, ey) = (width - bb.x, height - bb.y);
                if ex.abs() < 1e-9 && ey.abs() < 1e-9 {
                    break;
                }
                base.x += ex;
                base.y += ey;
                iter += 1;
            }
            shape.sides = sides;
            shape.vertices = v;
        }
        shape.peripheries = peripheries;
        shape.orientation = orientation;
        shape.distortion = distortion;
        shape.skew = skew;
        Ok(shape)
    }

    /// Replaces the reconstructed vertices with C's exact `poly_init` /
    /// `point_init` data from [`graphviz_pure_geom::node_size`].
    pub fn apply_geometry(&mut self, geom: &graphviz_pure_geom::node_size::NodeGeometry) {
        self.outline_width = geom.outline_width * 72.0;
        self.outline_height = geom.outline_height * 72.0;
        if let Some(r) = &geom.record {
            self.record_box = Some(r.bbox);
        }
        if let Some(p) = &geom.polygon {
            self.sides = p.sides;
            self.peripheries = p.peripheries;
            self.orientation = p.orientation;
            self.distortion = p.distortion;
            self.skew = p.skew;
            self.regular = p.regular;
            self.vertices = p.vertices.clone();
        }
    }

    /// Index of the first vertex of the outline used by the inside tests.
    fn outp(&self) -> usize {
        if self.peripheries >= 1 && self.penwidth > 0.0 {
            self.peripheries * self.sides
        } else if self.peripheries < 1 {
            0
        } else {
            (self.peripheries - 1) * self.sides
        }
    }
}

/// Per-clip state of C's `inside_t` (`s.last` speeds up polygon tests and can
/// change results for non-convex shapes, so it is kept). Built by
/// [`crate::common::splines::inside_ctx`], which reads the node's sizes from the layout.
#[derive(Debug, Clone, Copy)]
pub struct InsideCtx {
    pub last: usize,
    /// Port box (`inside_context->s.bp`), in node coordinates.
    pub bp: Option<graphviz_pure_core::BoundingBox>,
    /// `GD_rankdir` of the graph (0 = TB); points are rotated by `90 * rankdir`.
    pub rankdir: i32,
    /// `s.scalex`, `s.scaley` of `poly_inside` (node size over `lw + rw` and
    /// `ht`); `None` means 1, which holds unless dot widened `rw` for loops.
    pub scale: Option<Point>,
    /// `ND_lw`, `ND_rw` and `ND_ht` of the node when it is clipped (dot has them
    /// flipped under `rankdir=LR`/`RL`); `epsf_inside` tests against these.
    pub extent: (f64, f64, f64),
}

fn same_side(p0: Point, p1: Point, l0: Point, l1: Point) -> bool {
    let a = -(l1.y - l0.y);
    let b = l1.x - l0.x;
    let c = a.mul_add(l0.x, b * l0.y);
    let s0 = a.mul_add(p0.x, b * p0.y) - c >= 0.0;
    let s1 = a.mul_add(p1.x, b * p1.y) - c >= 0.0;
    s0 == s1
}

pub use super::ports::ccwrotatepf;

fn inside_box(p: Point, b: &graphviz_pure_core::BoundingBox) -> bool {
    b.ll.x <= p.x && p.x <= b.ur.x && b.ll.y <= p.y && p.y <= b.ur.y
}

impl NodeShape {
    /// The shape's `insidefn` for point `p` in node coordinates.
    pub fn inside(&self, ctx: &mut InsideCtx, p: Point) -> bool {
        let pr = ccwrotatepf(p, 90 * ctx.rankdir);
        match self.inside {
            InsideFn::Poly => self.poly_inside(ctx, pr),
            InsideFn::Point => {
                let radius = self.vertices[self.outp() + 1].x;
                if pr.x.abs() > radius || pr.y.abs() > radius {
                    return false;
                }
                math::hypot(pr.x, pr.y) <= radius
            }
            InsideFn::Record => {
                // record_inside tests the rotated point against fld0->b, which record_init
                // builds at the node's unflipped width and height (shapes.c:3700), so unlike
                // epsf it does not read the layout's (flipped) ND_lw/ND_rw/ND_ht.
                let mut b =
                    ctx.bp
                        .or(self.record_box)
                        .unwrap_or(graphviz_pure_core::BoundingBox::new(
                            -self.width / 2.0,
                            -self.height / 2.0,
                            self.width / 2.0,
                            self.height / 2.0,
                        ));
                let e = self.penwidth / 2.0;
                b.ll.x -= e;
                b.ll.y -= e;
                b.ur.x += e;
                b.ur.y += e;
                inside_box(pr, &b)
            }
            InsideFn::Epsf => {
                let (lw, rw, ht) = ctx.extent;
                let x2 = ht / 2.0;
                pr.y >= -x2 && pr.y <= x2 && pr.x >= -lw && pr.x <= rw
            }
            InsideFn::Star => {
                if let Some(b) = ctx.bp {
                    return inside_box(pr, &b);
                }
                let outp = self.outp();
                let o = Point::new(0.0, 0.0);
                let mut outcnt = 0;
                let mut i = 0;
                while i < self.sides {
                    let q = self.vertices[i + outp];
                    let r = self.vertices[(i + 4) % self.sides + outp];
                    if !same_side(pr, o, q, r) {
                        outcnt += 1;
                    }
                    if outcnt == 2 {
                        return false;
                    }
                    i += 2;
                }
                true
            }
        }
    }

    /// `polyBB(poly)`: bounding box of the outermost periphery's vertices.
    pub(crate) fn poly_bb(&self) -> graphviz_pure_core::BoundingBox {
        let peris = self.peripheries.max(1);
        let verts = &self.vertices[(peris - 1) * self.sides..];
        let mut bb = graphviz_pure_core::BoundingBox {
            ll: verts[0],
            ur: verts[0],
        };
        for v in &verts[1..self.sides] {
            bb.ll.x = c_min(bb.ll.x, v.x);
            bb.ll.y = c_min(bb.ll.y, v.y);
            bb.ur.x = c_max(bb.ur.x, v.x);
            bb.ur.y = c_max(bb.ur.y, v.y);
        }
        bb
    }

    fn poly_inside(&self, ctx: &mut InsideCtx, p: Point) -> bool {
        if let Some(b) = ctx.bp {
            return inside_box(p, &b);
        }
        let p = match ctx.scale {
            Some(s) if !self.fixedshape => Point::new(p.x * s.x, p.y * s.y),
            _ if self.fixedshape && ctx.rankdir & 1 != 0 => {
                // fixedsize=shape with GD_flip: xsize/ysize are the polyBB height
                // and width, so the point is scaled by width/height and back.
                let bb = self.poly_bb();
                let (w, h) = (bb.ur.x - bb.ll.x, bb.ur.y - bb.ll.y);
                let sx = if h == 0.0 { w } else { w / h };
                let sy = if w == 0.0 { h } else { h / w };
                Point::new(p.x * sx, p.y * sy)
            }
            _ => p,
        };
        let sides = self.sides;
        // xsize = lw + rw = width and ysize = ht = height, so scale is 1.
        let (box_urx, box_ury) = if self.fixedshape {
            let bb = self.poly_bb();
            ((bb.ur.x - bb.ll.x) / 2.0, (bb.ur.y - bb.ll.y) / 2.0)
        } else {
            (self.outline_width / 2.0, self.outline_height / 2.0)
        };
        if p.x.abs() > box_urx || p.y.abs() > box_ury {
            return false;
        }
        if sides <= 2 {
            return math::hypot(p.x / box_urx, p.y / box_ury) < 1.0;
        }
        let outp = self.outp();
        let o = Point::new(0.0, 0.0);
        let vertex = &self.vertices;
        let mut i = ctx.last % sides;
        let mut i1 = (i + 1) % sides;
        let q = vertex[i + outp];
        let r = vertex[i1 + outp];
        if !same_side(p, o, q, r) {
            return false;
        }
        let s = same_side(p, q, r, o);
        if s && same_side(p, r, o, q) {
            return true;
        }
        for _ in 1..sides {
            if s {
                i = i1;
                i1 = (i + 1) % sides;
            } else {
                i1 = i;
                i = (i + sides - 1) % sides;
            }
            if !same_side(p, o, vertex[i + outp], vertex[i1 + outp]) {
                ctx.last = i;
                return false;
            }
        }
        ctx.last = i;
        true
    }
}
