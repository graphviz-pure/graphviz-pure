//! Obstacle-avoiding edge routing for `splines=true|polyline` in the neato
//! family, ported from `lib/neatogen/neatosplines.c`: `makeObstacle`,
//! the `vconfig` branch of `spline_edges_`, `getPath`, `makeSpline`,
//! `makePolyline`.
//!
//! Multi-edges and edges with boundary ports go through `makeMultiSpline`
//! ([`super::multispline`]) first, as in C's GTS builds.

use super::adjust::Expand;
use super::legal::plegal_arrangement;
use crate::common::shapes::InsideFn;
use crate::common::splines::{add_edge_labels, clip_and_install};
use crate::common::types::{EdgeType, LGraph, Sub};
use graphviz_pure_core::Point;
use graphviz_pure_core::math;
use graphviz_pure_geom::pathplan::{
    POLYID_NONE, Pedge, Vconfig, in_poly, make_polyline, pobsopen, pobspath, proutespline,
};

/// Visibility configuration over node obstacles.
pub struct Router {
    obs: Vec<Vec<Point>>,
    vconfig: Vconfig,
    /// `router_t* rtr` of `spline_edges_`, built on first use (`mkRouter`).
    rtr: std::cell::RefCell<Option<Option<super::multispline::MRouter>>>,
}

fn ellipse_tangent_slope(a: f64, b: f64, p: Point) -> f64 {
    let sign_y = if p.y >= 0.0 { 1.0 } else { -1.0 };
    -sign_y * (b * p.x) / (a * a.mul_add(a, -(p.x * p.x)).sqrt())
}

fn line_intersection(p0: Point, m0: f64, p1: Point, m1: f64) -> Point {
    let x = ((-m1).mul_add(p1.x, m0.mul_add(p0.x, -p0.y)) + p1.y) / (m0 - m1);
    let y = m0.mul_add(x - p0.x, p0.y);
    Point::new(x, y)
}

fn circumscribed_corner(a: f64, b: f64, i: usize, nsides: usize) -> Point {
    use std::f64::consts::PI;
    let angle0 = 2.0 * PI * (i as f64 - 0.5) / nsides as f64;
    let angle1 = 2.0 * PI * (i as f64 + 0.5) / nsides as f64;
    let p0 = Point::new(a * math::cos(angle0), b * math::sin(angle0));
    let p1 = Point::new(a * math::cos(angle1), b * math::sin(angle1));
    let m0 = ellipse_tangent_slope(a, b, p0);
    let m1 = ellipse_tangent_slope(a, b, p1);
    line_intersection(p0, m0, p1, m1)
}

/// makeSpline's and makePolyline's `Verbose > 1` line: `<kind> <tail> <head>`.
pub(crate) fn verbose_edge(g: &LGraph<'_>, kind: &str, e: usize) {
    if graphviz_pure_core::diag::verbose_byte() > 1
        && let Some(ed) = g.edges.get(e)
    {
        let [t, h] = graphviz_pure_core::agnameof_buffer([
            g.node_agnameof(ed.tail),
            g.node_agnameof(ed.head),
        ]);
        graphviz_pure_core::diag::fprintf_stderr(&format!("{kind} {t} {h}\n"));
    }
}

/// `makeObstacle(n, pmargin, isOrtho=false)`: the node's outline as a
/// clockwise polygon grown by the margin.
pub fn make_obstacle(g: &LGraph<'_>, n: usize, pmargin: &Expand) -> Option<Vec<Point>> {
    let nd = &g.nodes[n];
    let sh = &nd.shape;
    let c = nd.coord;
    // shapeOf() is SH_POLY for star and cylinder too: `star_fns` and
    // `cylinder_fns` both use `poly_init` (lib/common/shapes.c), so they get a
    // polygon obstacle from their outline vertices like any other polygon.
    match sh.inside {
        InsideFn::Poly | InsideFn::Star | InsideFn::Point => {
            let is_poly = sh.inside != InsideFn::Point && sh.sides >= 3;
            let sides = if is_poly { sh.sides } else { 8 };
            let mut ps = vec![Point::default(); sides];
            let extra = usize::from(sh.peripheries >= 1 && sh.penwidth > 0.0);
            let outline_periphery = sh.peripheries + extra;
            let off = if outline_periphery >= 1 {
                (outline_periphery - 1) * sh.sides
            } else {
                0
            };
            for j in 0..sides {
                let polyp = if is_poly {
                    let v = sh.vertices[off + j];
                    if pmargin.do_add {
                        if sides == 4 {
                            let (xm, ym) = match j {
                                0 => (pmargin.x, pmargin.y),
                                1 => (-pmargin.x, pmargin.y),
                                2 => (-pmargin.x, -pmargin.y),
                                _ => (pmargin.x, -pmargin.y),
                            };
                            Point::new(v.x + xm, v.y + ym)
                        } else {
                            let h = math::hypot(v.x, v.y);
                            Point::new(v.x * (1.0 + pmargin.x / h), v.y * (1.0 + pmargin.y / h))
                        }
                    } else {
                        Point::new(v.x * pmargin.x, v.y * pmargin.y)
                    }
                } else {
                    let (mx, my) = if pmargin.do_add {
                        (pmargin.x, pmargin.y)
                    } else {
                        (0.0, 0.0)
                    };
                    let a = (sh.outline_width + mx) / 2.0;
                    let b = (sh.outline_height + my) / 2.0;
                    circumscribed_corner(a, b, j, sides)
                };
                ps[sides - j - 1] = Point::new(polyp.x + c.x, polyp.y + c.y);
            }
            Some(ps)
        }
        InsideFn::Record | InsideFn::Epsf => {
            let (llx, lly, urx, ury) = if sh.inside == InsideFn::Record {
                // C `makeObstacle` (neatosplines.c:447): `b = fld->b`, the
                // root field box, which excludes record_init's +1pt height.
                match sh.record_box {
                    Some(b) => (b.ll.x, b.ll.y, b.ur.x, b.ur.y),
                    None => (
                        -nd.width * 36.0,
                        -nd.height * 36.0,
                        nd.width * 36.0,
                        nd.height * 36.0,
                    ),
                }
            } else {
                (-nd.lw, -nd.ht, nd.rw, nd.ht)
            };
            let pts = if pmargin.do_add {
                [
                    Point::new(llx - pmargin.x, lly - pmargin.y),
                    Point::new(llx - pmargin.x, ury + pmargin.y),
                    Point::new(urx + pmargin.x, ury + pmargin.y),
                    Point::new(urx + pmargin.x, lly - pmargin.y),
                ]
                .map(|p| Point::new(p.x + c.x, p.y + c.y))
            } else {
                // recPt: C (clang) fuses `x * m->x + c.x`.
                let rec_pt = |x: f64, y: f64| {
                    Point::new(x.mul_add(pmargin.x, c.x), y.mul_add(pmargin.y, c.y))
                };
                [
                    rec_pt(llx, lly),
                    rec_pt(llx, ury),
                    rec_pt(urx, ury),
                    rec_pt(urx, lly),
                ]
            };
            Some(pts.to_vec())
        }
    }
}

impl Router {
    /// Builds obstacles for the nodes of `sub`; `None` when they touch
    /// (C warns and falls back to straight lines).
    pub fn new(g: &mut LGraph<'_>, sub: &Sub, margin: &Expand, _et: EdgeType) -> Option<Self> {
        let mut obs = Vec::new();
        for &n in &sub.nodes {
            match make_obstacle(g, n, margin) {
                Some(p) => {
                    g.nodes[n].lim = obs.len() as i64;
                    obs.push(p);
                }
                None => g.nodes[n].lim = i64::from(POLYID_NONE),
            }
        }
        if !plegal_arrangement(&obs) {
            // neatosplines.c:625; expand_t holds doubles (adjust.h:41-44).
            graphviz_pure_core::diag::agwarningf(&format!(
                "some nodes with margin ({},{}) touch - falling back to straight line edges\n",
                graphviz_pure_core::cnum::fmt_f(margin.x, 2),
                graphviz_pure_core::cnum::fmt_f(margin.y, 2)
            ));
            return None;
        }
        let vconfig = pobsopen(&obs);
        Some(Router {
            obs,
            vconfig,
            rtr: std::cell::RefCell::new(None),
        })
    }

    /// Path-finding pass: `ED_path(e) = getPath(e, vconfig, true)`.
    pub fn path_pass(&self, g: &mut LGraph<'_>, sub: &Sub) {
        for &n in &sub.nodes {
            let outs: Vec<usize> = g.outs(sub, n).collect();
            for e in outs {
                let (t, h) = (g.edges[e].tail, g.edges[e].head);
                let p = crate::common::geom::add(g.nodes[t].coord, g.edges[e].tail_port.p);
                let q = crate::common::geom::add(g.nodes[h].coord, g.edges[e].head_port.p);
                if std::env::var_os("GVDUMP").is_some() {
                    graphviz_pure_core::diag::dump(&format!(
                        "OP {} {:?},{:?} {} {:?},{:?}",
                        g.nodes[t].lim, p.x, p.y, g.nodes[h].lim, q.x, q.y
                    ));
                }
                // C's `Pobspath` loops forever when `q` is unreachable (see `pobspath`);
                // use the straight segment it returns when `p` sees `q`.
                let path = pobspath(
                    &self.vconfig,
                    p,
                    g.nodes[t].lim as i32,
                    q,
                    g.nodes[h].lim as i32,
                )
                .unwrap_or_else(|| vec![p, q]);
                g.edges[e].path = path;
            }
        }
    }

    /// `makeSpline(e, obs, npoly, true)`.
    fn make_spline(&self, g: &mut LGraph<'_>, e: usize) {
        let line = g.edges[e].path.clone();
        let p = line[0];
        let q = line[line.len() - 1];
        let mut pp = None;
        let mut qp = None;
        for (i, o) in self.obs.iter().enumerate() {
            if pp.is_none() && in_poly(o, p) {
                pp = Some(i);
            }
            if qp.is_none() && in_poly(o, q) {
                qp = Some(i);
            }
        }
        let mut barriers = Vec::new();
        for (i, o) in self.obs.iter().enumerate() {
            if Some(i) == pp || Some(i) == qp {
                continue;
            }
            for j in 0..o.len() {
                let k = if j + 1 >= o.len() { 0 } else { j + 1 };
                barriers.push(Pedge { a: o[j], b: o[k] });
            }
        }
        let Ok(spline) = proutespline(&barriers, &line, [Point::default(), Point::default()])
        else {
            return;
        };
        let h = g.edges[e].head;
        verbose_edge(g, "spline", e);
        clip_and_install(g, e, h, &spline);
        add_edge_labels(g, e);
    }

    /// `makePolyline(e)`.
    fn make_polyline(&self, g: &mut LGraph<'_>, e: usize) {
        let spl = make_polyline(&g.edges[e].path);
        let h = g.edges[e].head;
        verbose_edge(g, "polyline", e);
        clip_and_install(g, e, h, &spl);
        add_edge_labels(g, e);
    }

    /// Spline-drawing pass for one non-loop representative edge.
    pub fn route_edge(&self, g: &mut LGraph<'_>, sub: &Sub, e: usize, et: EdgeType) {
        let boundary_port = g.edges[e].tail_port.side != 0 || g.edges[e].head_port.side != 0;
        if g.edges[e].count > 1 || boundary_port {
            let fail = if g.edges[e].path.len() == 2 && !boundary_port {
                super::neatosplines::make_straight_edge(g, sub, e, et);
                false
            } else {
                let mut slot = self.rtr.borrow_mut();
                let rtr = slot.get_or_insert_with(|| super::multispline::MRouter::new(&self.obs));
                match rtr {
                    Some(r) => {
                        super::multispline::make_multi_spline(g, sub, e, r, et == EdgeType::Pline)
                    }
                    None => true,
                }
            };
            if !fail {
                return;
            }
        }
        let cnt = if g.concentrate { 1 } else { g.edges[e].count };
        let mut e0 = Some(e);
        for _ in 0..cnt {
            let Some(ee) = e0 else { break };
            if et == EdgeType::Spline {
                self.make_spline(g, ee);
            } else {
                self.make_polyline(g, ee);
            }
            e0 = g.edges[ee].to_virt;
        }
    }
}
