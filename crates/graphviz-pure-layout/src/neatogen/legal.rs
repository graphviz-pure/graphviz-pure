//! `Plegal_arrangement` from `lib/neatogen/legal.c`: do any obstacle
//! polygons intersect or nest? A plane sweep over polygon sides.

use crate::cutil::qsort_by;
use graphviz_pure_core::{BoundingBox, Point};

struct Verts<'a> {
    pos: Vec<Point>,
    poly: Vec<usize>,
    start: Vec<usize>,
    finish: Vec<usize>,
    _p: std::marker::PhantomData<&'a ()>,
}

impl Verts<'_> {
    fn after(&self, v: usize) -> usize {
        let p = self.poly[v];
        if v == self.finish[p] {
            self.start[p]
        } else {
            v + 1
        }
    }
    fn prior(&self, v: usize) -> usize {
        let p = self.poly[v];
        if v == self.start[p] {
            self.finish[p]
        } else {
            v - 1
        }
    }
}

fn sign(v: f64) -> i32 {
    if v < 0.0 {
        -1
    } else if v > 0.0 {
        1
    } else {
        0
    }
}

fn sgnarea(vs: &Verts<'_>, l: usize, m: usize) -> [i32; 3] {
    let a = vs.pos[l].x;
    let b = vs.pos[l].y;
    let c = vs.pos[vs.after(l)].x - a;
    let d = vs.pos[vs.after(l)].y - b;
    let e = vs.pos[m].x - a;
    let f = vs.pos[m].y - b;
    let g = vs.pos[vs.after(m)].x - a;
    let h = vs.pos[vs.after(m)].y - b;
    let i0 = sign(c.mul_add(f, -(d * e)));
    let i1 = sign(c.mul_add(h, -(d * g)));
    [i0, i1, i0 * i1]
}

fn between(f: f64, g: f64, h: f64) -> i32 {
    if f < g {
        if g < h {
            1
        } else if g > h {
            -1
        } else {
            0
        }
    } else if f > g {
        if g > h {
            1
        } else if g < h {
            -1
        } else {
            0
        }
    } else {
        0
    }
}

fn online(vs: &Verts<'_>, l: usize, m: usize, i: i32) -> i32 {
    let a = vs.pos[l];
    let b = vs.pos[vs.after(l)];
    let c = if i == 0 {
        vs.pos[m]
    } else {
        vs.pos[vs.after(m)]
    };
    if a.x == b.x {
        i32::from(a.x == c.x && -1 != between(a.y, c.y, b.y))
    } else {
        between(a.x, c.x, b.x)
    }
}

fn slope(p: Point, q: Point) -> f64 {
    (p.y - q.y) / (p.x - q.x)
}

fn intpoint(vs: &Verts<'_>, l: usize, m: usize, cond: i32) -> Option<Point> {
    if cond <= 0 {
        return None;
    }
    let ls = vs.pos[l];
    let le = vs.pos[vs.after(l)];
    let ms = vs.pos[m];
    let me = vs.pos[vs.after(m)];
    match cond {
        3 => {
            if ls.x == le.x {
                let x = ls.x;
                Some(Point::new(x, slope(ms, me).mul_add(x - me.x, me.y)))
            } else if ms.x == me.x {
                let x = ms.x;
                Some(Point::new(x, slope(ls, le).mul_add(x - le.x, le.y)))
            } else {
                let m1 = slope(ms, me);
                let m2 = slope(ls, le);
                let c1 = (-m1).mul_add(ms.x, ms.y);
                let c2 = (-m2).mul_add(ls.x, ls.y);
                Some(Point::new(
                    (c2 - c1) / (m1 - m2),
                    m1.mul_add(c2, -(c1 * m2)) / (m1 - m2),
                ))
            }
        }
        2 => {
            let (pt1, pt2);
            if online(vs, l, m, 0) == -1 {
                pt1 = ms;
                pt2 = if online(vs, m, l, 1) == -1 {
                    if online(vs, m, l, 0) == -1 { le } else { ls }
                } else {
                    me
                };
            } else if online(vs, l, m, 1) == -1 {
                pt1 = me;
                pt2 = if online(vs, l, m, 0) == -1 {
                    if online(vs, m, l, 0) == -1 { le } else { ls }
                } else {
                    ms
                };
            } else {
                if online(vs, m, l, 0) != -1 {
                    return None;
                }
                pt1 = ls;
                pt2 = le;
            }
            Some(Point::new((pt1.x + pt2.x) / 2.0, (pt1.y + pt2.y) / 2.0))
        }
        _ => {
            if (ls.x - le.x) * (ms.y - ls.y) == (ls.y - le.y) * (ms.x - ls.x) {
                Some(ms)
            } else {
                Some(me)
            }
        }
    }
}

fn real_intersect(vs: &Verts<'_>, first: usize, second: usize, p: Point) -> bool {
    let vft = vs.pos[first];
    let avft = vs.pos[vs.after(first)];
    let vsd = vs.pos[second];
    let avsd = vs.pos[vs.after(second)];
    let real = (vft.x != avft.x && vsd.x != avsd.x)
        || (vft.x == avft.x && vft != p && avft != p)
        || (vsd.x == avsd.x && vsd != p && avsd != p);
    if real && graphviz_pure_core::diag::verbose_byte() > 1 {
        let f = |v: f64| graphviz_pure_core::diag::fmt_f(v, 3);
        // putSeg(1, firstv), putSeg(2, secondv)
        let seg = |i: i32, a: Point, b: Point| {
            format!(
                "seg#{i} : ({}, {}) ({}, {})\n",
                f(a.x),
                f(a.y),
                f(b.x),
                f(b.y)
            )
        };
        graphviz_pure_core::diag::fprintf_stderr(&format!(
            "\nintersection at {} {}\n{}{}",
            f(p.x),
            f(p.y),
            seg(1, vft, avft),
            seg(2, vsd, avsd)
        ));
    }
    real
}

fn find_intersection(vs: &Verts<'_>, l: usize, m: usize) -> bool {
    let i = sgnarea(vs, l, m);
    if i[2] > 0 {
        return false;
    }
    let p = if i[2] < 0 {
        let j = sgnarea(vs, m, l);
        if j[2] > 0 {
            return false;
        }
        let cond = if j[2] < 0 {
            3
        } else {
            online(vs, m, l, j[0].abs())
        };
        intpoint(vs, l, m, cond)
    } else {
        let cond = if i[0] == i[1] {
            2 * online(vs, l, m, 0).max(online(vs, l, m, 1))
        } else {
            online(vs, l, m, i[0].abs())
        };
        intpoint(vs, l, m, cond)
    };
    match p {
        Some(p) => real_intersect(vs, l, m, p),
        None => false,
    }
}

fn gt(vs: &Verts<'_>, i: usize, j: usize) -> i32 {
    let (a, b) = (vs.pos[i], vs.pos[j]);
    if a.x > b.x {
        1
    } else if a.x < b.x {
        -1
    } else if a.y > b.y {
        1
    } else if a.y < b.y {
        -1
    } else {
        0
    }
}

fn find_ints(vs: &Verts<'_>) -> bool {
    let n = vs.pos.len();
    let mut pvertex: Vec<usize> = (0..n).collect();
    qsort_by(&mut pvertex, |a, b| gt(vs, *a, *b));
    let mut all: Vec<usize> = Vec::new();
    let mut active = vec![None::<usize>; n];
    for &pt1 in &pvertex {
        let mut pt2 = vs.prior(pt1);
        let mut templ = pt2;
        for _ in 0..2 {
            match gt(vs, pt1, pt2) {
                -1 => {
                    for &tempa in &all {
                        if find_intersection(vs, tempa, templ) {
                            return true;
                        }
                    }
                    all.push(templ);
                    active[templ] = Some(templ);
                }
                1 => {
                    // C reports the error and the arrangement counts as illegal.
                    let Some(t) = active[templ] else {
                        graphviz_pure_core::diag::agerrorf("trying to delete a non-line\n");
                        return true;
                    };
                    if let Some(pos) = all.iter().position(|&x| x == t) {
                        all.remove(pos);
                    }
                    active[templ] = None;
                }
                _ => {}
            }
            pt2 = vs.after(pt1);
            templ = pt1;
        }
    }
    false
}

fn inbox(p: Point, bb: &BoundingBox) -> bool {
    p.x <= bb.ur.x && p.x >= bb.ll.x && p.y <= bb.ur.y && p.y >= bb.ll.y
}

/// `Plegal_arrangement(polys, n)`: true when no two polygons overlap.
pub fn plegal_arrangement(polys: &[Vec<Point>]) -> bool {
    let mut vs = Verts {
        pos: Vec::new(),
        poly: Vec::new(),
        start: Vec::new(),
        finish: Vec::new(),
        _p: std::marker::PhantomData,
    };
    let mut bbs = Vec::with_capacity(polys.len());
    for (i, p) in polys.iter().enumerate() {
        vs.start.push(vs.pos.len());
        let mut bb = BoundingBox::new(f64::MAX, f64::MAX, -f64::MAX, -f64::MAX);
        for q in p {
            bb.ll.x = bb.ll.x.min(q.x);
            bb.ll.y = bb.ll.y.min(q.y);
            bb.ur.x = bb.ur.x.max(q.x);
            bb.ur.y = bb.ur.y.max(q.y);
            vs.pos.push(*q);
            vs.poly.push(i);
        }
        vs.finish.push(vs.pos.len() - 1);
        bbs.push(bb);
    }
    if find_ints(&vs) {
        return false;
    }
    for i in 0..polys.len() {
        let pt = polys[i][0];
        for j in (i + 1)..polys.len() {
            let nested = |a: &BoundingBox, b: &BoundingBox| inbox(a.ll, b) && inbox(a.ur, b);
            if nested(&bbs[i], &bbs[j]) {
                if graphviz_pure_geom::pathplan::in_poly(&polys[j], pt) {
                    return false;
                }
            } else if nested(&bbs[j], &bbs[i])
                && graphviz_pure_geom::pathplan::in_poly(&polys[i], polys[j][0])
            {
                return false;
            }
        }
    }
    true
}
