//! Port of `lib/dotgen/compound.c`: with `compound=true`, edges with
//! `lhead`/`ltail` are clipped at the boundary of the named cluster.

use super::Dot;
use super::types::*;
use crate::common::arrows::{arrow_end_clip, arrow_start_clip};
use crate::common::geom::mid;
use crate::common::splines::bezier_split;
use crate::common::utils::{ClustMap, mk_clust_map};
use graphviz_pure_core::Point;
use graphviz_pure_core::cnum::c_min;

/// `boxIntersectf(pp, cp, bp)`: where segment `pp -> cp` leaves box `bp`.
fn box_intersectf(pp: Point, cp: Point, bp: &BoxF) -> Point {
    let (ll, ur) = (bp.ll, bp.ur);
    if cp.x < ll.x {
        let y = pp.y + ((ll.x - pp.x) * (pp.y - cp.y) / (pp.x - cp.x)).round();
        if y >= ll.y && y <= ur.y {
            return Point::new(ll.x, y);
        }
    }
    if cp.x > ur.x {
        let y = pp.y + ((ur.x - pp.x) * (pp.y - cp.y) / (pp.x - cp.x)).round();
        if y >= ll.y && y <= ur.y {
            return Point::new(ur.x, y);
        }
    }
    if cp.y < ll.y {
        let x = pp.x + ((ll.y - pp.y) * (pp.x - cp.x) / (pp.y - cp.y)).round();
        if x >= ll.x && x <= ur.x {
            return Point::new(x, ll.y);
        }
    }
    if cp.y > ur.y {
        let x = pp.x + ((ur.y - pp.y) * (pp.x - cp.x) / (pp.y - cp.y)).round();
        if x >= ll.x && x <= ur.x {
            return Point::new(x, ur.y);
        }
    }
    // C reports "segment does not intersect box" and asserts.
    pp
}

/// `inBoxf(p, bb)`.
fn in_boxf(p: Point, bb: &BoxF) -> bool {
    bb.ll.x <= p.x && p.x <= bb.ur.x && bb.ll.y <= p.y && p.y <= bb.ur.y
}

fn fcmp(a: f64, b: f64) -> i32 {
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}

/// `countVertCross(pts, xcoord)` / `countHorzCross(pts, ycoord)`.
fn count_cross(pts: &[Point; 4], c: f64, vert: bool) -> i32 {
    let v = |p: Point| if vert { p.x } else { p.y };
    let mut num = 0;
    let mut sign = fcmp(v(pts[0]), c);
    if sign == 0 {
        num += 1;
    }
    for p in &pts[1..] {
        let old = sign;
        sign = fcmp(v(*p), c);
        if sign != old && old != 0 {
            num += 1;
        }
    }
    num
}

/// `findVertical` (`vert`) / `findHorizontal`.
fn find_cross(pts: &[Point; 4], tmin: f64, tmax: f64, c: f64, lo: f64, hi: f64, vert: bool) -> f64 {
    if tmin == tmax {
        return tmin;
    }
    let no_cross = count_cross(pts, c, vert);
    if no_cross == 0 {
        return -1.0;
    }
    let (along, across) = if vert {
        (pts[3].x, pts[3].y)
    } else {
        (pts[3].y, pts[3].x)
    };
    if no_cross == 1 && (along - c).abs() <= 0.005 {
        return if lo <= across && across <= hi {
            tmax
        } else {
            -1.0
        };
    }
    let (_, left, right) = bezier_split(pts, 0.5);
    let t = find_cross(&left, tmin, (tmin + tmax) / 2.0, c, lo, hi, vert);
    if t >= 0.0 {
        return t;
    }
    find_cross(&right, (tmin + tmax) / 2.0, tmax, c, lo, hi, vert)
}

/// `splineIntersectf(pts, bb)`: cut the Bézier `pts` where it first meets
/// the box boundary; returns whether it does.
fn spline_intersectf(pts: &mut [Point; 4], bb: &BoxF) -> bool {
    let mut tmin = 2.0f64;
    let orig = *pts;
    let tries = [
        (bb.ll.x, bb.ll.y, bb.ur.y, true),
        (bb.ur.x, bb.ll.y, bb.ur.y, true),
        (bb.ll.y, bb.ll.x, bb.ur.x, false),
        (bb.ur.y, bb.ll.x, bb.ur.x, false),
    ];
    for (i, (c, lo, hi, vert)) in tries.into_iter().enumerate() {
        let tmax = if i == 0 { 1.0 } else { c_min(1.0, tmin) };
        let t = find_cross(pts, 0.0, tmax, c, lo, hi, vert);
        if t >= 0.0 && t < tmin {
            *pts = bezier_split(&orig, t).1;
            tmin = t;
        }
    }
    tmin < 2.0
}

impl<'a> Dot<'a> {
    /// `fillMap`'s preorder of the cluster tree under `g`, with each cluster's `agnameof`.
    fn clusters_preorder(&self, g: G, out: &mut Vec<(graphviz_pure_core::AgName<'a>, G)>) {
        for &cl in &self.graphs[g].clust {
            out.push((self.graph_agnameof(cl), cl));
            self.clusters_preorder(cl, out);
        }
    }

    /// `makeCompoundEdge(e, clustMap)`.
    fn make_compound_edge(&mut self, e: E, map: &ClustMap<G>) {
        // `getCluster`
        let get = |k: &str| {
            let s = self.edge_attr(e, k).filter(|s| !s.is_empty())?;
            let c = map.get(s);
            if c.is_none() {
                graphviz_pure_core::diag::agwarningf(&format!("cluster named {s} not found\n"));
            }
            c
        };
        let lh = get("lhead");
        let lt = get("ltail");
        if lt.is_none() && lh.is_none() {
            return;
        }
        let Some(spl) = self.edges[e].spl.as_ref() else {
            return;
        };
        let (head, tail) = (self.edges[e].head, self.edges[e].tail);
        // `agnameof(agtail(e)), agnameof(aghead(e))`, then `agget(e, ...)`.
        // Both names share agnameof's static buffer when anonymous (compound.c:282-382).
        let [tname, hname] = graphviz_pure_core::agnameof_buffer([
            self.node_agnameof(tail),
            self.node_agnameof(head),
        ]);
        let warn = |what: &str, key: &str| {
            graphviz_pure_core::diag::agwarningf(&format!(
                "{tname} -> {hname}: {what} {}\n",
                self.edge_attr(e, key).unwrap_or("")
            ));
        };
        if spl.len() > 1 {
            graphviz_pure_core::diag::agwarningf(&format!(
                "{tname} -> {hname}: spline size > 1 not supported\n"
            ));
            return;
        }
        let mut bez = spl[0].clone();
        let size = bez.list.len();
        let attrs = self.input.edges[self.edges[e].real.unwrap()].attrs.clone();
        let (hc, tc) = (self.nodes[head].coord, self.nodes[tail].coord);
        let mut nbez = Bezier {
            eflag: bez.eflag,
            sflag: bez.sflag,
            ..Bezier::default()
        };
        let mut starti = 0usize;
        let mut endi = 0usize;
        let mut fixed = false;
        if let Some(lh) = lh {
            let bb = self.graphs[lh].bb;
            if !in_boxf(hc, &bb) {
                warn("head not inside head cluster", "lhead");
            } else {
                if in_boxf(bez.list[0], &bb) {
                    if in_boxf(tc, &bb) {
                        warn("tail is inside head cluster", "lhead");
                    } else if bez.sflag != 0 && !in_boxf(bez.sp, &bb) {
                        // C asserts `bez->sflag` here (`compound.c:324`) and aborts when the
                        // spline has no tail arrow (`splines=curved` routing a back edge,
                        // #490); `sp` is then unset. The port tests the flag first, as C's
                        // tail-cluster branch does below, and keeps the original head.
                        let p = box_intersectf(bez.list[0], bez.sp, &bb);
                        bez.list[3] = p;
                        bez.list[1] = mid(p, bez.sp);
                        bez.list[0] = mid(bez.list[1], bez.sp);
                        bez.list[2] = mid(bez.list[1], p);
                        if bez.eflag != 0 {
                            endi = arrow_end_clip(
                                &attrs,
                                &mut bez.list,
                                starti,
                                0,
                                &mut nbez,
                                bez.eflag,
                            );
                        }
                        endi += 3;
                        fixed = true;
                    }
                } else {
                    endi = 0;
                    while endi < size - 1 {
                        let mut pts = [
                            bez.list[endi],
                            bez.list[endi + 1],
                            bez.list[endi + 2],
                            bez.list[endi + 3],
                        ];
                        let hit = spline_intersectf(&mut pts, &bb);
                        bez.list[endi..endi + 4].copy_from_slice(&pts);
                        if hit {
                            break;
                        }
                        endi += 3;
                    }
                    if endi == size - 1 {
                        // C asserts `bez->eflag` here (`compound.c:342`, #490); with no head
                        // arrow there is no `ep` to move, as in the tail branch's `sflag` test.
                        if bez.eflag != 0 {
                            nbez.ep = box_intersectf(bez.ep, bez.list[endi], &bb);
                        }
                    } else {
                        if bez.eflag != 0 {
                            endi = arrow_end_clip(
                                &attrs,
                                &mut bez.list,
                                starti,
                                endi,
                                &mut nbez,
                                bez.eflag,
                            );
                        }
                        endi += 3;
                    }
                    fixed = true;
                }
            }
        }
        if !fixed {
            endi = size - 1;
            if bez.eflag != 0 {
                nbez.ep = bez.ep;
            }
        }
        fixed = false;
        if let Some(lt) = lt {
            let bb = self.graphs[lt].bb;
            if !in_boxf(tc, &bb) {
                warn("tail not inside tail cluster", "ltail");
            } else {
                if in_boxf(bez.list[endi], &bb) {
                    if in_boxf(hc, &bb) {
                        warn("head is inside tail cluster", "ltail");
                    } else if bez.eflag != 0 && !in_boxf(nbez.ep, &bb) {
                        let p = box_intersectf(bez.list[endi], nbez.ep, &bb);
                        starti = endi - 3;
                        bez.list[starti] = p;
                        bez.list[starti + 2] = mid(p, nbez.ep);
                        bez.list[starti + 3] = mid(bez.list[starti + 2], nbez.ep);
                        bez.list[starti + 1] = mid(bez.list[starti + 2], p);
                        if bez.sflag != 0 {
                            starti = arrow_start_clip(
                                &attrs,
                                &mut bez.list,
                                starti,
                                endi - 3,
                                &mut nbez,
                                bez.sflag,
                            );
                        }
                        fixed = true;
                    }
                } else {
                    starti = endi;
                    while starti > 0 {
                        let mut pts = [
                            bez.list[starti],
                            bez.list[starti - 1],
                            bez.list[starti - 2],
                            bez.list[starti - 3],
                        ];
                        if spline_intersectf(&mut pts, &bb) {
                            for (i, p) in pts.iter().enumerate() {
                                bez.list[starti - i] = *p;
                            }
                            break;
                        }
                        starti -= 3;
                    }
                    if starti == 0 && bez.sflag != 0 {
                        nbez.sp = box_intersectf(bez.sp, bez.list[starti], &bb);
                    } else if starti != 0 {
                        starti -= 3;
                        if bez.sflag != 0 {
                            starti = arrow_start_clip(
                                &attrs,
                                &mut bez.list,
                                starti,
                                endi - 3,
                                &mut nbez,
                                bez.sflag,
                            );
                        }
                    }
                    fixed = true;
                }
            }
        }
        if !fixed && bez.sflag != 0 {
            nbez.sp = bez.sp;
        }
        nbez.list = bez.list[starti..=endi].to_vec();
        self.edges[e].spl.as_mut().unwrap()[0] = nbez;
    }

    /// `dot_compoundEdges(g)`.
    pub fn dot_compound_edges(&mut self) {
        let mut clusters = Vec::new();
        self.clusters_preorder(0, &mut clusters);
        let map = mk_clust_map(clusters);
        for n in self.nodes_of(0) {
            for e in self.out_edges(0, n) {
                self.make_compound_edge(e, &map);
            }
        }
    }
}
