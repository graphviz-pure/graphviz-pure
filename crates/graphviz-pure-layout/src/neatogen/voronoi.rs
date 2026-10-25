//! Voronoi-based overlap removal (`overlap=voronoi`, and `oscale`), ported
//! from `lib/neatogen/adjust.c` (`makeInfo`, `vAdjust`, `sAdjust`, `rmEquality`,
//! `newPos`, ...), Fortune's sweep in `voronoi.c`, `hedges.c`, `heap.c`,
//! `edges.c`, `site.c`, `geometry.c`, `info.c` and the node polygons and
//! overlap test of `poly.c`.

use super::adjust::AdjustMode;
use crate::common::shapes::InsideFn;
use crate::common::types::{LGraph, Sub};
use crate::cutil::qsort_by;
use graphviz_pure_core::Point;
use graphviz_pure_core::math;

const LE: usize = 0;
const RE: usize = 1;
const INCR: f64 = 0.05;
const DFLT_SAMPLE: usize = 20;

// ------------------------------------------------------------ polygons

const BOX: u8 = 1;
const CIRCLE: u8 = 2;

#[derive(Clone, Default)]
struct Poly {
    origin: Point,
    corner: Point,
    verts: Vec<Point>,
    kind: u8,
}

fn bbox(verts: &[Point]) -> (Point, Point) {
    let (mut x0, mut y0, mut x1, mut y1) = (verts[0].x, verts[0].y, verts[0].x, verts[0].y);
    for v in &verts[1..] {
        x0 = x0.min(v.x);
        y0 = y0.min(v.y);
        x1 = x1.max(v.x);
        y1 = y1.max(v.y);
    }
    (Point::new(x0, y0), Point::new(x1, y1))
}

fn is_box(v: &[Point]) -> bool {
    if v.len() != 4 {
        return false;
    }
    if v[0].y == v[1].y {
        v[2].y == v[3].y && v[0].x == v[3].x && v[1].x == v[2].x
    } else {
        v[0].x == v[1].x && v[2].x == v[3].x && v[0].y == v[3].y && v[1].y == v[2].y
    }
}

fn gen_round(g: &LGraph<'_>, n: usize, xm: f64, ym: f64) -> Vec<Point> {
    let isides = g
        .nattr(n, "samplepoints")
        .map(crate::common::utils::atoi)
        .unwrap_or(0);
    let sides = if isides < 3 {
        DFLT_SAMPLE
    } else {
        isides as usize
    };
    let nd = &g.nodes[n];
    (0..sides)
        .map(|i| {
            let a = i as f64 / sides as f64 * std::f64::consts::PI * 2.0;
            Point::new(
                (nd.width / 2.0 + xm) * math::cos(a),
                (nd.height / 2.0 + ym) * math::sin(a),
            )
        })
        .collect()
}

/// `makeAddPoly` (additive margin, inches) or `makePoly` (scale factors). `None`, with C's
/// error, for a shape `shapeOf` does not know (`epsf`).
fn make_poly(g: &LGraph<'_>, n: usize, xm: f64, ym: f64, add: bool) -> Option<Poly> {
    let nd = &g.nodes[n];
    let sh = &nd.shape;
    let mut pp = Poly::default();
    let mut verts;
    if nd.clust.is_some() {
        // ND_clust(n): a box of the node's size
        let (bx, by) = if add {
            (nd.width / 2.0 + xm, nd.height / 2.0 + ym)
        } else {
            (nd.width / 2.0, nd.height / 2.0)
        };
        pp.kind = BOX;
        verts = vec![
            Point::new(bx, by),
            Point::new(-bx, by),
            Point::new(-bx, -by),
            Point::new(bx, -by),
        ];
    } else {
        match sh.inside {
            InsideFn::Record => {
                // the root field's box (field_t b), without record_init's +1 height
                let (llx, lly, urx, ury) = match sh.record_box {
                    Some(b) => (b.ll.x, b.ll.y, b.ur.x, b.ur.y),
                    None => (
                        -nd.width * 36.0,
                        -nd.height * 36.0,
                        nd.width * 36.0,
                        nd.height * 36.0,
                    ),
                };
                verts = if add {
                    vec![
                        Point::new(llx / 72.0 - xm, lly / 72.0 - ym),
                        Point::new(urx / 72.0 + xm, lly / 72.0 - ym),
                        Point::new(urx / 72.0 + xm, ury / 72.0 + ym),
                        Point::new(llx / 72.0 - xm, ury / 72.0 + ym),
                    ]
                } else {
                    vec![
                        Point::new(llx / 72.0, lly / 72.0),
                        Point::new(urx / 72.0, lly / 72.0),
                        Point::new(urx / 72.0, ury / 72.0),
                        Point::new(llx / 72.0, ury / 72.0),
                    ]
                };
                pp.kind = BOX;
            }
            InsideFn::Epsf => {
                let f = if add { "makeAddPoly" } else { "makePoly" };
                graphviz_pure_core::diag::agerrorf(&format!(
                    "{f}: unknown shape type {}\n",
                    sh.name
                ));
                return None;
            }
            InsideFn::Point => {
                pp.kind = CIRCLE;
                verts = if add {
                    gen_round(g, n, xm, ym)
                } else {
                    gen_round(g, n, 0.0, 0.0)
                };
            }
            _ => {
                let sides = sh.sides;
                let name = sh.name.as_str();
                if add {
                    pp.kind = if name == "box"
                        || (name == "polygon"
                            && is_box(&sh.vertices[..sides.min(sh.vertices.len())]))
                    {
                        BOX
                    } else if sides < 3 && sh.regular {
                        CIRCLE
                    } else {
                        0
                    };
                    if sides >= 3 {
                        let v = &sh.vertices;
                        verts = if pp.kind == BOX {
                            vec![
                                Point::new(v[0].x / 72.0 + xm, v[0].y / 72.0 + ym),
                                Point::new(v[1].x / 72.0 - xm, v[1].y / 72.0 + ym),
                                Point::new(v[2].x / 72.0 - xm, v[2].y / 72.0 - ym),
                                Point::new(v[3].x / 72.0 + xm, v[3].y / 72.0 - ym),
                            ]
                        } else {
                            (0..sides)
                                .map(|i| {
                                    let h = math::hypot(v[i].x, v[i].y);
                                    Point::new(
                                        v[i].x * (1.0 + xm / h) / 72.0,
                                        v[i].y * (1.0 + ym / h) / 72.0,
                                    )
                                })
                                .collect()
                        };
                    } else {
                        verts = gen_round(g, n, xm, ym);
                    }
                } else {
                    verts = if sides >= 3 {
                        (0..sides)
                            .map(|i| Point::new(sh.vertices[i].x / 72.0, sh.vertices[i].y / 72.0))
                            .collect()
                    } else {
                        gen_round(g, n, 0.0, 0.0)
                    };
                    pp.kind = if name == "box" || (name == "polygon" && is_box(&verts)) {
                        BOX
                    } else if sides < 3 && sh.regular {
                        CIRCLE
                    } else {
                        0
                    };
                }
            }
        }
    }
    if !add && (xm != 1.0 || ym != 1.0) {
        for v in &mut verts {
            v.x *= xm;
            v.y *= ym;
        }
    }
    let (o, c) = bbox(&verts);
    pp.origin = o;
    pp.corner = c;
    pp.verts = verts;
    Some(pp)
}

fn area_2(a: Point, b: Point, c: Point) -> f64 {
    (a.y - b.y).mul_add(c.x - b.x, -((c.y - b.y) * (a.x - b.x)))
}

fn left_of(a: Point, b: Point, c: Point) -> bool {
    area_2(a, b, c) > 0.0
}

fn intersection(a: Point, b: Point, c: Point, d: Point) -> bool {
    let denom = c.x.mul_add(
        a.y - b.y,
        d.x.mul_add(b.y - a.y, a.x.mul_add(d.y - c.y, b.x * (c.y - d.y))),
    );
    if denom == 0.0 {
        return false;
    }
    let s =
        d.x.mul_add(c.y - a.y, a.x.mul_add(d.y - c.y, c.x * (a.y - d.y))) / denom;
    let t =
        -c.x.mul_add(b.y - a.y, a.x.mul_add(c.y - b.y, b.x * (a.y - c.y))) / denom;
    (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t)
}

fn edges_intersect(p: &[Point], q: &[Point]) -> bool {
    let (n, m) = (p.len() as i64, q.len() as i64);
    let (mut a, mut b, mut aa, mut ba) = (0i64, 0i64, 0i64, 0i64);
    loop {
        let a1 = ((a + n - 1) % n) as usize;
        let b1 = ((b + m - 1) % m) as usize;
        let (au, bu) = (a as usize, b as usize);
        let av = Point::new(p[au].x - p[a1].x, p[au].y - p[a1].y);
        let bv = Point::new(q[bu].x - q[b1].x, q[bu].y - q[b1].y);
        let cross = area_2(Point::new(0.0, 0.0), av, bv);
        let bha = left_of(p[a1], p[au], q[bu]);
        let ahb = left_of(q[b1], q[bu], p[au]);
        if intersection(p[a1], p[au], q[b1], q[bu]) {
            return true;
        }
        let adv_a = if cross == 0.0 && !bha && !ahb {
            true
        } else if cross >= 0.0 {
            bha
        } else {
            !ahb
        };
        if adv_a {
            aa += 1;
            a = (a + 1) % n;
        } else {
            ba += 1;
            b = (b + 1) % m;
        }
        if !((aa < n || ba < m) && aa < 2 * n && ba < 2 * m) {
            return false;
        }
    }
}

fn in_poly(vertex: &[Point], q: Point) -> bool {
    let n = vertex.len();
    let tp: Vec<Point> = vertex
        .iter()
        .map(|v| Point::new(v.x - q.x, v.y - q.y))
        .collect();
    let mut crossings = 0.0f64;
    for i in 0..n {
        let i1 = (i + n - 1) % n;
        if tp[i].y == 0.0 && tp[i1].y == 0.0 {
            if tp[i].x * tp[i1].x < 0.0 {
                return true;
            }
            continue;
        }
        if (tp[i].y >= 0.0 && tp[i1].y <= 0.0) || (tp[i1].y >= 0.0 && tp[i].y <= 0.0) {
            let x = tp[i].x.mul_add(tp[i1].y, -(tp[i1].x * tp[i].y)) / (tp[i1].y - tp[i].y);
            if x == 0.0 {
                return true;
            }
            if x > 0.0 {
                crossings += if tp[i].y == 0.0 || tp[i1].y == 0.0 {
                    0.5
                } else {
                    1.0
                };
            }
        }
    }
    (crossings as i64) % 2 == 1
}

fn in_box(p: Point, o: Point, c: Point) -> bool {
    p.x <= c.x && p.x >= o.x && p.y <= c.y && p.y >= o.y
}

/// `polyOverlap(p, pp, q, qp)`.
fn poly_overlap(p: Point, pp: &Poly, q: Point, qp: &Poly) -> bool {
    let op = Point::new(p.x + pp.origin.x, p.y + pp.origin.y);
    let cp = Point::new(p.x + pp.corner.x, p.y + pp.corner.y);
    let oq = Point::new(q.x + qp.origin.x, q.y + qp.origin.y);
    let cq = Point::new(q.x + qp.corner.x, q.y + qp.corner.y);
    if !(op.x <= cq.x && oq.x <= cp.x && op.y <= cq.y && oq.y <= cp.y) {
        return false;
    }
    if pp.kind & BOX != 0 && qp.kind & BOX != 0 {
        return true;
    }
    if pp.kind & CIRCLE != 0 && qp.kind & CIRCLE != 0 {
        let d = pp.corner.x - pp.origin.x + qp.corner.x - qp.origin.x;
        let dx = p.x - q.x;
        let dy = p.y - q.y;
        return dx.mul_add(dx, dy * dy) <= d * d / 4.0;
    }
    let tp1: Vec<Point> = pp
        .verts
        .iter()
        .map(|v| Point::new(v.x + p.x, v.y + p.y))
        .collect();
    let tp2: Vec<Point> = qp
        .verts
        .iter()
        .map(|v| Point::new(v.x + q.x, v.y + q.y))
        .collect();
    edges_intersect(&tp1, &tp2)
        || (in_box(tp1[0], oq, cq) && in_poly(&tp2, tp1[0]))
        || (in_box(tp2[0], op, cp) && in_poly(&tp1, tp2[0]))
}

// ------------------------------------------------------------ Fortune

#[derive(Clone, Copy, Default)]
struct SiteV {
    coord: Point,
    sitenbr: usize,
}

#[derive(Clone, Copy)]
struct EdgeV {
    a: f64,
    b: f64,
    c: f64,
    ep: [Option<usize>; 2],
    reg: [usize; 2],
}

#[derive(Clone, Copy, PartialEq)]
enum HeEdge {
    Null,
    Deleted,
    E(usize),
}

#[derive(Clone, Copy)]
struct Half {
    left: Option<usize>,
    right: Option<usize>,
    edge: HeEdge,
    pm: usize,
    vertex: Option<usize>,
    ystar: f64,
    pqnext: Option<usize>,
}

struct Vor<'a> {
    sites: Vec<SiteV>,
    edges: Vec<EdgeV>,
    he: Vec<Half>,
    hash: Vec<Option<usize>>,
    leftend: usize,
    rightend: usize,
    pq: Vec<Option<usize>>,
    pqcount: i64,
    pqmin: usize,
    bottomsite: usize,
    xmin: f64,
    deltax: f64,
    ymin: f64,
    ymax: f64,
    win: [f64; 4],
    info: &'a mut [Info],
}

fn clamp(lower: i64, v: f64, upper: i64) -> i64 {
    if v < lower as f64 {
        lower
    } else if v > upper as f64 {
        upper
    } else {
        v as i64
    }
}

impl Vor<'_> {
    fn he_create(&mut self, e: HeEdge, pm: usize) -> usize {
        self.he.push(Half {
            left: None,
            right: None,
            edge: e,
            pm,
            vertex: None,
            ystar: 0.0,
            pqnext: None,
        });
        self.he.len() - 1
    }

    fn el_insert(&mut self, lb: usize, new: usize) {
        let r = self.he[lb].right.unwrap();
        self.he[new].left = Some(lb);
        self.he[new].right = Some(r);
        self.he[r].left = Some(new);
        self.he[lb].right = Some(new);
    }

    fn el_delete(&mut self, he: usize) {
        let (l, r) = (self.he[he].left.unwrap(), self.he[he].right.unwrap());
        self.he[l].right = Some(r);
        self.he[r].left = Some(l);
        self.he[he].edge = HeEdge::Deleted;
    }

    fn gethash(&mut self, b: i64) -> Option<usize> {
        if b < 0 || b >= self.hash.len() as i64 {
            return None;
        }
        let h = self.hash[b as usize]?;
        if self.he[h].edge != HeEdge::Deleted {
            return Some(h);
        }
        self.hash[b as usize] = None;
        None
    }

    #[allow(clippy::needless_late_init)]
    fn right_of(&self, el: usize, p: Point) -> bool {
        let HeEdge::E(ei) = self.he[el].edge else {
            unreachable!()
        };
        let e = self.edges[ei];
        let topsite = self.sites[e.reg[1]].coord;
        let right_of_site = p.x > topsite.x;
        let pm = self.he[el].pm;
        if right_of_site && pm == LE {
            return true;
        }
        if !right_of_site && pm == RE {
            return false;
        }
        let above;
        if e.a == 1.0 {
            let dyp = p.y - topsite.y;
            let dxp = p.x - topsite.x;
            let mut fast = false;
            let mut ab;
            if (!right_of_site && e.b < 0.0) || (right_of_site && e.b >= 0.0) {
                ab = dyp >= e.b * dxp;
                fast = ab;
            } else {
                ab = p.y.mul_add(e.b, p.x) > e.c;
                if e.b < 0.0 {
                    ab = !ab;
                }
                if !ab {
                    fast = true;
                }
            }
            if !fast {
                let dxs = topsite.x - self.sites[e.reg[0]].coord.x;
                ab = e.b * dxp.mul_add(dxp, -(dyp * dyp))
                    < dxs * dyp * e.b.mul_add(e.b, 1.0 + 2.0 * dxp / dxs);
                if e.b < 0.0 {
                    ab = !ab;
                }
            }
            above = ab;
        } else {
            let yl = (-e.a).mul_add(p.x, e.c);
            let t1 = p.y - yl;
            let t2 = p.x - topsite.x;
            let t3 = yl - topsite.y;
            above = t1 * t1 > t2.mul_add(t2, t3 * t3);
        }
        if pm == LE { above } else { !above }
    }

    fn leftbnd(&mut self, p: Point) -> usize {
        let hs = self.hash.len() as i64;
        let bucket = clamp(0, (p.x - self.xmin) / self.deltax * hs as f64, hs - 1);
        let mut he = self.gethash(bucket);
        if he.is_none() {
            let mut i = 1;
            loop {
                if let Some(h) = self.gethash(bucket - i) {
                    he = Some(h);
                    break;
                }
                if let Some(h) = self.gethash(bucket + i) {
                    he = Some(h);
                    break;
                }
                i += 1;
            }
        }
        let mut he = he.unwrap();
        if he == self.leftend || (he != self.rightend && self.right_of(he, p)) {
            loop {
                he = self.he[he].right.unwrap();
                if !(he != self.rightend && self.right_of(he, p)) {
                    break;
                }
            }
            he = self.he[he].left.unwrap();
        } else {
            loop {
                he = self.he[he].left.unwrap();
                if !(he != self.leftend && !self.right_of(he, p)) {
                    break;
                }
            }
        }
        if bucket > 0 && bucket < hs - 1 {
            self.hash[bucket as usize] = Some(he);
        }
        he
    }

    fn leftreg(&self, he: usize) -> usize {
        match self.he[he].edge {
            HeEdge::E(e) => self.edges[e].reg[if self.he[he].pm == LE { LE } else { RE }],
            _ => self.bottomsite,
        }
    }

    fn rightreg(&self, he: usize) -> usize {
        match self.he[he].edge {
            HeEdge::E(e) => self.edges[e].reg[if self.he[he].pm == LE { RE } else { LE }],
            _ => self.bottomsite,
        }
    }

    fn bisect(&mut self, s1: usize, s2: usize) -> usize {
        let (c1, c2) = (self.sites[s1].coord, self.sites[s2].coord);
        let dx = c2.x - c1.x;
        let dy = c2.y - c1.y;
        let mut c = dx
            .mul_add(dx, dy * dy)
            .mul_add(0.5, c1.x.mul_add(dx, c1.y * dy));
        let (a, b);
        if dx.abs() > dy.abs() {
            a = 1.0;
            b = dy / dx;
            c /= dx;
        } else {
            b = 1.0;
            a = dx / dy;
            c /= dy;
        }
        self.edges.push(EdgeV {
            a,
            b,
            c,
            ep: [None, None],
            reg: [s1, s2],
        });
        self.edges.len() - 1
    }

    fn hintersect(&mut self, el1: usize, el2: usize) -> Option<usize> {
        let (HeEdge::E(e1), HeEdge::E(e2)) = (self.he[el1].edge, self.he[el2].edge) else {
            return None;
        };
        let (ed1, ed2) = (self.edges[e1], self.edges[e2]);
        if ed1.reg[1] == ed2.reg[1] {
            return None;
        }
        let d = ed1.a.mul_add(ed2.b, -(ed1.b * ed2.a));
        if -1.0e-10 < d && d < 1.0e-10 {
            return None;
        }
        let xint = ed1.c.mul_add(ed2.b, -(ed2.c * ed1.b)) / d;
        let yint = ed2.c.mul_add(ed1.a, -(ed1.c * ed2.a)) / d;
        let (r1, r2) = (self.sites[ed1.reg[1]].coord, self.sites[ed2.reg[1]].coord);
        let (el, e) = if r1.y < r2.y || (r1.y == r2.y && r1.x < r2.x) {
            (el1, ed1)
        } else {
            (el2, ed2)
        };
        let right_of_site = xint >= self.sites[e.reg[1]].coord.x;
        if (right_of_site && self.he[el].pm == LE) || (!right_of_site && self.he[el].pm == RE) {
            return None;
        }
        self.sites.push(SiteV {
            coord: Point::new(xint, yint),
            sitenbr: usize::MAX,
        });
        Some(self.sites.len() - 1)
    }

    fn pq_bucket(&mut self, he: usize) -> usize {
        let deltay = self.ymax - self.ymin;
        let hs = self.pq.len() as i64;
        let b = clamp(
            0,
            (self.he[he].ystar - self.ymin) / deltay * hs as f64,
            hs - 1,
        ) as usize;
        if b < self.pqmin {
            self.pqmin = b;
        }
        b
    }

    fn pq_insert(&mut self, he: usize, v: usize, offset: f64) {
        self.he[he].vertex = Some(v);
        self.he[he].ystar = self.sites[v].coord.y + offset;
        let b = self.pq_bucket(he);
        let (ys, vx) = (self.he[he].ystar, self.sites[v].coord.x);
        // `last` is either the bucket head (None) or a halfedge.
        let mut last: Option<usize> = None;
        loop {
            let next = match last {
                None => self.pq[b],
                Some(l) => self.he[l].pqnext,
            };
            let Some(nx) = next else { break };
            let (nys, nvx) = (
                self.he[nx].ystar,
                self.sites[self.he[nx].vertex.unwrap()].coord.x,
            );
            let gt = ys > nys || (ys == nys && vx > nvx);
            if !gt {
                break;
            }
            last = Some(nx);
        }
        match last {
            None => {
                self.he[he].pqnext = self.pq[b];
                self.pq[b] = Some(he);
            }
            Some(l) => {
                self.he[he].pqnext = self.he[l].pqnext;
                self.he[l].pqnext = Some(he);
            }
        }
        self.pqcount += 1;
    }

    fn pq_delete(&mut self, he: usize) {
        if self.he[he].vertex.is_none() {
            return;
        }
        let b = self.pq_bucket(he);
        if self.pq[b] == Some(he) {
            self.pq[b] = self.he[he].pqnext;
        } else {
            let mut last = self.pq[b].unwrap();
            while self.he[last].pqnext != Some(he) {
                last = self.he[last].pqnext.unwrap();
            }
            self.he[last].pqnext = self.he[he].pqnext;
        }
        self.pqcount -= 1;
        self.he[he].vertex = None;
    }

    fn pq_min(&mut self) -> Point {
        while self.pq[self.pqmin].is_none() {
            self.pqmin += 1;
        }
        let h = self.pq[self.pqmin].unwrap();
        Point::new(
            self.sites[self.he[h].vertex.unwrap()].coord.x,
            self.he[h].ystar,
        )
    }

    fn pq_extractmin(&mut self) -> usize {
        let h = self.pq[self.pqmin].unwrap();
        self.pq[self.pqmin] = self.he[h].pqnext;
        self.pqcount -= 1;
        h
    }

    fn add_vertex(&mut self, s: usize, x: f64, y: f64) {
        let nbr = self.sites[s].sitenbr;
        let origin = self.sites[s].coord;
        add_vertex(&mut self.info[nbr], origin, Point::new(x, y));
    }

    fn clip_line(&mut self, ei: usize) {
        let e = self.edges[ei];
        let [pxmin, pxmax, pymin, pymax] = self.win;
        let (s1, s2) = if e.a == 1.0 && e.b >= 0.0 {
            (e.ep[1], e.ep[0])
        } else {
            (e.ep[0], e.ep[1])
        };
        let (mut x1, mut y1, mut x2, mut y2);
        if e.a == 1.0 {
            match s1 {
                Some(s) => {
                    y1 = self.sites[s].coord.y;
                    if y1 > pymax {
                        return;
                    } else if y1 >= pymin {
                        x1 = self.sites[s].coord.x;
                    } else {
                        y1 = pymin;
                        x1 = (-e.b).mul_add(y1, e.c);
                    }
                }
                None => {
                    y1 = pymin;
                    x1 = (-e.b).mul_add(y1, e.c);
                }
            }
            match s2 {
                Some(s) => {
                    y2 = self.sites[s].coord.y;
                    if y2 < pymin {
                        return;
                    } else if y2 <= pymax {
                        x2 = self.sites[s].coord.x;
                    } else {
                        y2 = pymax;
                        x2 = (-e.b).mul_add(y2, e.c);
                    }
                }
                None => {
                    y2 = pymax;
                    x2 = (-e.b).mul_add(y2, e.c);
                }
            }
            if (x1 > pxmax && x2 > pxmax) || (x1 < pxmin && x2 < pxmin) {
                return;
            }
            if x1 > pxmax {
                x1 = pxmax;
                y1 = (e.c - x1) / e.b;
            }
            if x1 < pxmin {
                x1 = pxmin;
                y1 = (e.c - x1) / e.b;
            }
            if x2 > pxmax {
                x2 = pxmax;
                y2 = (e.c - x2) / e.b;
            }
            if x2 < pxmin {
                x2 = pxmin;
                y2 = (e.c - x2) / e.b;
            }
        } else {
            match s1 {
                Some(s) => {
                    x1 = self.sites[s].coord.x;
                    if x1 > pxmax {
                        return;
                    } else if x1 >= pxmin {
                        y1 = self.sites[s].coord.y;
                    } else {
                        x1 = pxmin;
                        y1 = (-e.a).mul_add(x1, e.c);
                    }
                }
                None => {
                    x1 = pxmin;
                    y1 = (-e.a).mul_add(x1, e.c);
                }
            }
            match s2 {
                Some(s) => {
                    x2 = self.sites[s].coord.x;
                    if x2 < pxmin {
                        return;
                    } else if x2 <= pxmax {
                        y2 = self.sites[s].coord.y;
                    } else {
                        x2 = pxmax;
                        y2 = (-e.a).mul_add(x2, e.c);
                    }
                }
                None => {
                    x2 = pxmax;
                    y2 = (-e.a).mul_add(x2, e.c);
                }
            }
            if (y1 > pymax && y2 > pymax) || (y1 < pymin && y2 < pymin) {
                return;
            }
            if y1 > pymax {
                y1 = pymax;
                x1 = (e.c - y1) / e.a;
            }
            if y1 < pymin {
                y1 = pymin;
                x1 = (e.c - y1) / e.a;
            }
            if y2 > pymax {
                y2 = pymax;
                x2 = (e.c - y2) / e.a;
            }
            if y2 < pymin {
                y2 = pymin;
                x2 = (e.c - y2) / e.a;
            }
        }
        let (r0, r1) = (e.reg[0], e.reg[1]);
        self.add_vertex(r0, x1, y1);
        self.add_vertex(r0, x2, y2);
        self.add_vertex(r1, x1, y1);
        self.add_vertex(r1, x2, y2);
    }

    fn endpoint(&mut self, e: usize, lr: usize, s: usize) {
        self.edges[e].ep[lr] = Some(s);
        if self.edges[e].ep[RE - lr].is_none() {
            return;
        }
        self.clip_line(e);
    }

    fn ngdist(&self, s: usize, t: usize) -> f64 {
        let (a, b) = (self.sites[s].coord, self.sites[t].coord);
        math::hypot(a.x - b.x, a.y - b.y)
    }

    /// `voronoi(nextsite, context)` over the sorted node sites.
    fn run(&mut self, order: &[usize]) {
        let mut next = order.iter().copied();
        self.bottomsite = next.next().unwrap();
        let mut newsite = next.next();
        let mut newintstar = Point::default();
        loop {
            if self.pqcount != 0 {
                newintstar = self.pq_min();
            }
            let take_site = match newsite {
                Some(ns) => {
                    let c = self.sites[ns].coord;
                    self.pqcount == 0
                        || c.y < newintstar.y
                        || (c.y == newintstar.y && c.x < newintstar.x)
                }
                None => false,
            };
            if take_site {
                let ns = newsite.unwrap();
                let lbnd = self.leftbnd(self.sites[ns].coord);
                let rbnd = self.he[lbnd].right.unwrap();
                let bot = self.rightreg(lbnd);
                let e = self.bisect(bot, ns);
                let bisector = self.he_create(HeEdge::E(e), LE);
                self.el_insert(lbnd, bisector);
                if let Some(p) = self.hintersect(lbnd, bisector) {
                    self.pq_delete(lbnd);
                    let d = self.ngdist(p, ns);
                    self.pq_insert(lbnd, p, d);
                }
                let lbnd = bisector;
                let bisector = self.he_create(HeEdge::E(e), RE);
                self.el_insert(lbnd, bisector);
                if let Some(p) = self.hintersect(bisector, rbnd) {
                    let d = self.ngdist(p, ns);
                    self.pq_insert(bisector, p, d);
                }
                newsite = next.next();
            } else if self.pqcount != 0 {
                let lbnd = self.pq_extractmin();
                let llbnd = self.he[lbnd].left.unwrap();
                let rbnd = self.he[lbnd].right.unwrap();
                let rrbnd = self.he[rbnd].right.unwrap();
                let mut bot = self.leftreg(lbnd);
                let mut top = self.rightreg(rbnd);
                let v = self.he[lbnd].vertex.unwrap();
                let (HeEdge::E(le_), HeEdge::E(re_)) = (self.he[lbnd].edge, self.he[rbnd].edge)
                else {
                    unreachable!()
                };
                let (lpm, rpm) = (self.he[lbnd].pm, self.he[rbnd].pm);
                self.endpoint(le_, lpm, v);
                self.endpoint(re_, rpm, v);
                self.el_delete(lbnd);
                self.pq_delete(rbnd);
                self.el_delete(rbnd);
                let mut pm = LE;
                if self.sites[bot].coord.y > self.sites[top].coord.y {
                    std::mem::swap(&mut bot, &mut top);
                    pm = RE;
                }
                let e = self.bisect(bot, top);
                let bisector = self.he_create(HeEdge::E(e), pm);
                self.el_insert(llbnd, bisector);
                self.endpoint(e, RE - pm, v);
                if let Some(p) = self.hintersect(llbnd, bisector) {
                    self.pq_delete(llbnd);
                    let d = self.ngdist(p, bot);
                    self.pq_insert(llbnd, p, d);
                }
                if let Some(p) = self.hintersect(bisector, rrbnd) {
                    let d = self.ngdist(p, bot);
                    self.pq_insert(bisector, p, d);
                }
            } else {
                break;
            }
        }
        let mut lbnd = self.he[self.leftend].right.unwrap();
        while lbnd != self.rightend {
            if let HeEdge::E(e) = self.he[lbnd].edge {
                self.clip_line(e);
            }
            lbnd = self.he[lbnd].right.unwrap();
        }
    }
}

// ------------------------------------------------------------ info

struct Info {
    coord: Point,
    poly: Poly,
    verts: Vec<Point>,
    overlaps: bool,
    node: usize,
}

/// `compare(o, p, q)` of `info.c`: angular order around `o`.
fn compare(o: Point, p: Point, q: Point) -> i32 {
    if p.x == q.x && p.y == q.y {
        return 0;
    }
    let (x0, y0, x1, y1) = (p.x - o.x, p.y - o.y, q.x - o.x, q.y - o.y);
    if x0 >= 0.0 {
        if x1 < 0.0 {
            -1
        } else if x0 > 0.0 {
            if x1 > 0.0 {
                let (a, b) = (y1 / x1, y0 / x0);
                if b < a {
                    -1
                } else if b > a {
                    1
                } else if x0 < x1 {
                    -1
                } else {
                    1
                }
            } else if y1 > 0.0 {
                -1
            } else {
                1
            }
        } else if x1 > 0.0 {
            if y0 <= 0.0 { -1 } else { 1 }
        } else if y0 < y1 {
            if y1 <= 0.0 { 1 } else { -1 }
        } else if y0 <= 0.0 {
            -1
        } else {
            1
        }
    } else if x1 >= 0.0 {
        1
    } else {
        let (a, b) = (y1 / x1, y0 / x0);
        if b < a {
            -1
        } else if b > a {
            1
        } else if x0 > x1 {
            -1
        } else {
            1
        }
    }
}

fn add_vertex(ip: &mut Info, origin: Point, tmp: Point) {
    let mut i = 0;
    while i < ip.verts.len() {
        let c = compare(origin, tmp, ip.verts[i]);
        if c == 0 {
            return;
        }
        if c < 0 {
            break;
        }
        i += 1;
    }
    ip.verts.insert(i, tmp);
}

fn dist_2(p: Point, q: Point) -> f64 {
    let (dx, dy) = (p.x - q.x, p.y - q.y);
    dx.mul_add(dx, dy * dy)
}

fn count_overlap(info: &mut [Info], iter: u32) -> u32 {
    let mut count = 0;
    for ip in info.iter_mut() {
        ip.overlaps = false;
    }
    for i in 0..info.len().saturating_sub(1) {
        for j in (i + 1)..info.len() {
            if poly_overlap(info[i].coord, &info[i].poly, info[j].coord, &info[j].poly) {
                count += 1;
                info[i].overlaps = true;
                info[j].overlaps = true;
            }
        }
    }
    if graphviz_pure_core::diag::verbose_byte() > 1 {
        graphviz_pure_core::diag::fprintf_stderr(&format!("overlap [{iter}] : {count}\n"));
    }
    count
}

fn sort_sites(info: &[Info]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..info.len()).collect();
    qsort_by(&mut order, |a, b| {
        let (s1, s2) = (info[*a].coord, info[*b].coord);
        if s1.y < s2.y {
            -1
        } else if s1.y > s2.y {
            1
        } else if s1.x < s2.x {
            -1
        } else if s1.x > s2.x {
            1
        } else {
            0
        }
    });
    order
}

fn rm_equality(info: &mut [Info]) -> Vec<usize> {
    let order = sort_sites(info);
    let n = order.len();
    let mut ip = 0;
    while ip < n {
        let mut jp = ip + 1;
        let ci = info[order[ip]].coord;
        if jp >= n || info[order[jp]].coord.x != ci.x || info[order[jp]].coord.y != ci.y {
            ip = jp;
            continue;
        }
        let mut cnt = 2;
        let mut kp = jp + 1;
        while kp < n && info[order[kp]].coord.x == ci.x && info[order[kp]].coord.y == ci.y {
            cnt += 1;
            jp = kp;
            kp = jp + 1;
        }
        if kp < n && info[order[kp]].coord.y == ci.y {
            let xdel = (info[order[kp]].coord.x - ci.x) / f64::from(cnt);
            for (i, j) in ((ip + 1)..kp).enumerate() {
                // C (clang) fuses `coord.x += i * xdel`.
                let x = &mut info[order[j]].coord.x;
                *x = ((i + 1) as f64).mul_add(xdel, *x);
            }
        } else {
            for (a, j) in (ip..).zip((ip + 1)..kp) {
                let pa = &info[order[a]].poly;
                let mut xdel = pa.corner.x - pa.origin.x;
                let pj = &info[order[j]].poly;
                xdel += pj.corner.x - pj.origin.x;
                info[order[j]].coord.x = info[order[a]].coord.x + xdel / 2.0;
            }
        }
        ip = kp;
    }
    order
}

fn area_of(a: Point, b: Point, c: Point) -> f64 {
    c.x.mul_add(a.y - b.y, a.x.mul_add(b.y - c.y, b.x * (c.y - a.y)))
        .abs()
        / 2.0
}

// `newpos` (`lib/neatogen/adjust.c`). The fan from `verts[0]` needs three
// vertices. `addCorners` uses strict `<`, so a site that `rmEquality` pushed
// outside the window `chkBoundBox` fixed earlier can have none (`verts` NULL;
// `sortSites` resets it each pass). C's `ip->verts[0]` on that cell reads
// through a NULL pointer, which is undefined behaviour. The reference build
// writes NaN because clang sinks the load past the zero-trip loop; an -O0 or
// gcc build of C would likely crash. The port writes that build's output.
// Indexing `verts[0]` here panicked (#450).
fn newpos(ip: &mut Info) {
    let n = ip.verts.len();
    let (mut total, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
    if n >= 3 {
        let anchor = ip.verts[0];
        for i in 1..n.saturating_sub(1) {
            let (p, q) = (ip.verts[i], ip.verts[i + 1]);
            let area = area_of(anchor, p, q);
            let x = (anchor.x + p.x + q.x) / 3.0;
            let y = (anchor.y + p.y + q.y) / 3.0;
            cx = area.mul_add(x, cx);
            cy = area.mul_add(y, cy);
            total += area;
        }
    }
    ip.coord = Point::new(cx / total, cy / total);
}

/// One `voronoi()` pass followed by `newPos` over the current sites.
fn voronoi_pass(info: &mut [Info], win: [f64; 4], do_all: bool, do_sort: bool, order_in: &[usize]) {
    let n = info.len();
    let order = if do_sort {
        sort_sites(info)
    } else {
        order_in.to_vec()
    };
    for ip in info.iter_mut() {
        ip.verts.clear();
    }
    let sqrt_nsites = ((n + 4) as f64).sqrt() as usize;
    let (mut xmin, mut xmax) = (f64::MAX, -f64::MAX);
    for &k in &order {
        xmin = xmin.min(info[k].coord.x);
        xmax = xmax.max(info[k].coord.x);
    }
    let ymin = info[order[0]].coord.y;
    let ymax = info[order[n - 1]].coord.y;
    let sites: Vec<SiteV> = info
        .iter()
        .enumerate()
        .map(|(i, ip)| SiteV {
            coord: ip.coord,
            sitenbr: i,
        })
        .collect();
    let mut v = Vor {
        sites,
        edges: Vec::new(),
        he: Vec::new(),
        hash: vec![None; 2 * sqrt_nsites],
        leftend: 0,
        rightend: 0,
        pq: vec![None; 4 * sqrt_nsites],
        pqcount: 0,
        pqmin: 0,
        bottomsite: 0,
        xmin,
        deltax: xmax - xmin,
        ymin,
        ymax,
        win,
        info,
    };
    v.leftend = v.he_create(HeEdge::Null, 0);
    v.rightend = v.he_create(HeEdge::Null, 0);
    let (l, r) = (v.leftend, v.rightend);
    v.he[l].right = Some(r);
    v.he[r].left = Some(l);
    let hs = v.hash.len();
    v.hash[0] = Some(l);
    v.hash[hs - 1] = Some(r);
    v.run(&order);
    // addCorners + newpos
    let [pxmin, pxmax, pymin, pymax] = win;
    let corners = [
        Point::new(pxmin, pymin), // sw
        Point::new(pxmax, pymin), // se
        Point::new(pxmin, pymax), // nw
        Point::new(pxmax, pymax), // ne
    ];
    let mut best = [0usize; 4];
    let mut bestd = [0.0f64; 4];
    // C compares in the order sw, se, nw, ne but adds sw, se, nw, ne.
    for (k, c) in corners.iter().enumerate() {
        bestd[k] = dist_2(v.info[0].coord, *c);
    }
    for i in 1..n {
        for (k, c) in corners.iter().enumerate() {
            let d = dist_2(v.info[i].coord, *c);
            if d < bestd[k] {
                bestd[k] = d;
                best[k] = i;
            }
        }
    }
    for (k, c) in corners.iter().enumerate() {
        let o = v.info[best[k]].coord;
        add_vertex(&mut v.info[best[k]], o, *c);
    }
    for ip in v.info.iter_mut() {
        if do_all || ip.overlaps {
            newpos(ip);
        }
    }
}

/// `vAdjust` (`mode == Vor`) or `sAdjust` (`mode == Scale`).
pub fn adjust(g: &mut LGraph<'_>, sub: &Sub, mode: AdjustMode) -> bool {
    let mut pmargin = super::adjust::sep_factor(g);
    let add = pmargin.do_add;
    if add {
        pmargin.x /= 72.0;
        pmargin.y /= 72.0;
    }
    // makeInfo: the first node without a polygon fails it, and removeOverlapWith gives up
    let Some(mut info) = sub
        .nodes
        .iter()
        .map(|&n| {
            Some(Info {
                coord: Point::new(g.nodes[n].pos[0], g.nodes[n].pos[1]),
                poly: make_poly(g, n, pmargin.x, pmargin.y, add)?,
                verts: Vec::new(),
                overlaps: false,
                node: n,
            })
        })
        .collect::<Option<Vec<Info>>>()
    else {
        return false;
    };
    // chkBoundBox
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, -f64::MAX, -f64::MAX);
    for ip in &info {
        x0 = x0.min(ip.poly.origin.x + ip.coord.x);
        y0 = y0.min(ip.poly.origin.y + ip.coord.y);
        x1 = x1.max(ip.poly.corner.x + ip.coord.x);
        y1 = y1.max(ip.poly.corner.y + ip.coord.y);
    }
    let margin = g
        .gattr("voro_margin")
        .filter(|s| !s.is_empty())
        .map(graphviz_pure_core::strtod::atof)
        .unwrap_or(0.05);
    let yd = margin * (y1 - y0);
    let xd = margin * (x1 - x0);
    let mut win = [x0 - xd, x1 + xd, y0 - yd, y1 + yd];
    let changed = if mode == AdjustMode::Scale {
        let mut iter_cnt = 0;
        if count_overlap(&mut info, iter_cnt) == 0 {
            false
        } else {
            rm_equality(&mut info);
            loop {
                let f = 1.0 + INCR;
                for ip in info.iter_mut() {
                    ip.coord.x *= f;
                    ip.coord.y *= f;
                }
                iter_cnt = iter_cnt.wrapping_add(1);
                if count_overlap(&mut info, iter_cnt) == 0 {
                    break;
                }
            }
            graphviz_pure_core::verbose_msg!("Number of iterations = {iter_cnt}\n");
            true
        }
    } else {
        let (mut iter_cnt, mut increase_cnt) = (0u32, 0u32);
        let mut overlap_cnt = count_overlap(&mut info, iter_cnt);
        if overlap_cnt == 0 {
            false
        } else {
            // geomUpdate(st, 0) keeps the order from rmEquality's sort.
            let order = rm_equality(&mut info);
            let mut do_all = false;
            let mut bad_level = 0;
            let mut first = true;
            loop {
                voronoi_pass(&mut info, win, do_all, !first, &order);
                first = false;
                iter_cnt = iter_cnt.wrapping_add(1);
                let cnt = count_overlap(&mut info, iter_cnt);
                if cnt == 0 {
                    break;
                }
                if cnt >= overlap_cnt {
                    bad_level += 1;
                } else {
                    bad_level = 0;
                }
                overlap_cnt = cnt;
                do_all = true;
                if bad_level > 0 {
                    increase_cnt = increase_cnt.wrapping_add(1);
                    let (xd, yd) = (INCR * (win[1] - win[0]), INCR * (win[3] - win[2]));
                    win = [win[0] - xd, win[1] + xd, win[2] - yd, win[3] + yd];
                }
            }
            graphviz_pure_core::gv_info!("adjust.c", 456, "Number of iterations = {iter_cnt}");
            graphviz_pure_core::gv_info!("adjust.c", 457, "Number of increases = {increase_cnt}");
            true
        }
    };
    if changed {
        for ip in &info {
            g.nodes[ip.node].pos[0] = ip.coord.x;
            g.nodes[ip.node].pos[1] = ip.coord.y;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(verts: &[Point]) -> Info {
        Info {
            coord: Point::new(0.0, 0.0),
            poly: Poly::default(),
            verts: verts.to_vec(),
            overlaps: true,
            node: 0,
        }
    }

    // Issue #450. C's `newpos` (`adjust.c`) divides by the fan's area. With
    // fewer than three vertices the loop does not run, the area stays 0, and
    // the site is NaN. `verts` may be empty: clang does not load `verts[0]`.
    #[test]
    fn newpos_of_a_cell_with_fewer_than_three_vertices_is_nan() {
        for verts in [
            &[][..],
            &[Point::new(1.0, 2.0)][..],
            &[Point::new(1.0, 2.0), Point::new(3.0, 4.0)][..],
        ] {
            let mut ip = cell(verts);
            newpos(&mut ip);
            assert!(ip.coord.x.is_nan() && ip.coord.y.is_nan(), "{verts:?}");
        }
    }

    #[test]
    fn newpos_centroid_of_a_triangle_is_unchanged() {
        let mut ip = cell(&[
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
        ]);
        newpos(&mut ip);
        assert_eq!(ip.coord, Point::new(1.0 / 3.0, 1.0 / 3.0));
    }
}
