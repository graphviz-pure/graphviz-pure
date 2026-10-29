//! `lib/pathplan/visibility.c` (visibility graph) and `shortestpth.c`
//! (Dijkstra over the dense visibility matrix).

use super::cvt::Vconfig;
use super::inpoly::in_poly;
use super::{POLYID_UNKNOWN, Ppoint};
use graphviz_pure_core::math;

/// `area2(a, b, c)`: twice the signed area of `abc`.
pub fn area2(a: Ppoint, b: Ppoint, c: Ppoint) -> f64 {
    math::fma(a.y - b.y, c.x - b.x, -((c.y - b.y) * (a.x - b.x)))
}

/// `wind(a, b, c)`: 1 counter-clockwise, -1 clockwise, 0 collinear (with tolerance).
pub fn wind(a: Ppoint, b: Ppoint, c: Ppoint) -> i32 {
    let w = area2(a, b, c);
    if w > 0.0001 {
        1
    } else if w < -0.0001 {
        -1
    } else {
        0
    }
}

fn in_between(a: Ppoint, b: Ppoint, c: Ppoint) -> bool {
    if a.x != b.x {
        (a.x < c.x && c.x < b.x) || (b.x < c.x && c.x < a.x)
    } else {
        (a.y < c.y && c.y < b.y) || (b.y < c.y && c.y < a.y)
    }
}

fn intersect(a: Ppoint, b: Ppoint, c: Ppoint, d: Ppoint) -> bool {
    let a_abc = wind(a, b, c);
    if a_abc == 0 && in_between(a, b, c) {
        return true;
    }
    let a_abd = wind(a, b, d);
    if a_abd == 0 && in_between(a, b, d) {
        return true;
    }
    let a_cda = wind(c, d, a);
    let a_cdb = wind(c, d, b);
    a_abc * a_abd < 0 && a_cda * a_cdb < 0
}

fn in_cone(a0: Ppoint, a1: Ppoint, a2: Ppoint, b: Ppoint) -> bool {
    let m = wind(b, a0, a1);
    let p = wind(b, a1, a2);
    if wind(a0, a1, a2) > 0 {
        m >= 0 && p >= 0
    } else {
        m >= 0 || p >= 0
    }
}

/// `dist2(a, b)`.
pub fn dist2(a: Ppoint, b: Ppoint) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    math::fma(dx, dx, dy * dy)
}

fn dist(a: Ppoint, b: Ppoint) -> f64 {
    dist2(a, b).sqrt()
}

fn clear(
    pti: Ppoint,
    ptj: Ppoint,
    start: usize,
    end: usize,
    v: usize,
    pts: &[Ppoint],
    next: &[usize],
) -> bool {
    for k in 0..start {
        if intersect(pti, ptj, pts[k], pts[next[k]]) {
            return false;
        }
    }
    for k in end..v {
        if intersect(pti, ptj, pts[k], pts[next[k]]) {
            return false;
        }
    }
    true
}

/// `visibility(conf)` / `compVis`: fill `conf.vis` with distances between
/// mutually visible obstacle vertices.
pub fn visibility(conf: &mut Vconfig) {
    let v = conf.n;
    conf.vis = vec![vec![0.0; v]; v];
    for i in 0..v {
        let previ = conf.prev[i];
        let d = dist(conf.p[i], conf.p[previ]);
        conf.vis[i][previ] = d;
        conf.vis[previ][i] = d;
        let mut j: isize = if previ as isize == i as isize - 1 {
            i as isize - 2
        } else {
            i as isize - 1
        };
        while j >= 0 {
            let ju = j as usize;
            let (pts, next, prev) = (&conf.p, &conf.next, &conf.prev);
            if in_cone(pts[prev[i]], pts[i], pts[next[i]], pts[ju])
                && in_cone(pts[prev[ju]], pts[ju], pts[next[ju]], pts[i])
                && clear(pts[i], pts[ju], v, v, v, pts, next)
            {
                let d = dist(pts[i], pts[ju]);
                conf.vis[i][ju] = d;
                conf.vis[ju][i] = d;
            }
            j -= 1;
        }
    }
}

fn polyhit(conf: &Vconfig, p: Ppoint) -> i32 {
    for i in 0..conf.npoly {
        let poly = &conf.p[conf.start[i]..conf.start[i + 1]];
        if in_poly(poly, p) {
            return i as i32;
        }
    }
    super::POLYID_NONE
}

/// `ptVis(conf, pp, p)`: visibility vector of `p` (length `N + 2`),
/// ignoring polygon `pp`.
pub fn pt_vis(conf: &Vconfig, mut pp: i32, p: Ppoint) -> Vec<f64> {
    let v = conf.n;
    let mut vadj = vec![0.0; v + 2];
    if pp == POLYID_UNKNOWN {
        pp = polyhit(conf, p);
    }
    let (start, end) = if pp >= 0 {
        (conf.start[pp as usize], conf.start[pp as usize + 1])
    } else {
        (v, v)
    };
    let pts = &conf.p;
    for k in (0..start).chain(end..v) {
        let pk = pts[k];
        vadj[k] = if in_cone(pts[conf.prev[k]], pk, pts[conf.next[k]], p)
            && clear(p, pk, start, end, v, pts, &conf.next)
        {
            dist(p, pk)
        } else {
            0.0
        };
    }
    vadj
}

/// `directVis(p, pp, q, qp, conf)`.
pub fn direct_vis(p: Ppoint, pp: i32, q: Ppoint, qp: i32, conf: &Vconfig) -> bool {
    let v = conf.n;
    let st = |i: i32| conf.start[i as usize];
    let (s1, e1, s2, e2) = if pp < 0 {
        if qp < 0 {
            (0, 0, 0, 0)
        } else {
            (0, 0, st(qp), st(qp + 1))
        }
    } else if qp < 0 {
        (0, 0, st(pp), st(pp + 1))
    } else if pp <= qp {
        (st(pp), st(pp + 1), st(qp), st(qp + 1))
    } else {
        (st(qp), st(qp + 1), st(pp), st(pp + 1))
    };
    let pts = &conf.p;
    for k in (0..s1).chain(e1..s2).chain(e2..v) {
        if intersect(p, q, pts[k], pts[conf.next[k]]) {
            return false;
        }
    }
    true
}

const UNSEEN: f64 = i32::MAX as f64;

/// `shortestPath(root, target, V, wadj)`: `dad` encodes the path back to `root`.
fn shortest_path(
    root: usize,
    target: usize,
    v: usize,
    wadj: &dyn Fn(usize, usize) -> f64,
) -> Vec<i32> {
    let mut dad = vec![-1i32; v];
    // val[-1] is the sentinel; store it at index 0.
    let mut vl = vec![0.0f64; v + 1];
    for k in 0..v {
        vl[k + 1] = -UNSEEN;
    }
    vl[0] = -(UNSEEN + 1.0);
    let mut min: isize = root as isize;
    while min != target as isize {
        let k = min as usize;
        vl[k + 1] *= -1.0;
        min = -1;
        if vl[k + 1] == UNSEEN {
            vl[k + 1] = 0.0;
        }
        for t in 0..v {
            if vl[t + 1] < 0.0 {
                let wkt = if k >= t { wadj(k, t) } else { wadj(t, k) };
                let newpri = -(vl[k + 1] + wkt);
                if wkt != 0.0 && vl[t + 1] < newpri {
                    vl[t + 1] = newpri;
                    dad[t] = k as i32;
                }
                if vl[t + 1] > vl[(min + 1) as usize] {
                    min = t as isize;
                }
            }
        }
    }
    dad
}

/// `makePath(p, pp, pvis, q, qp, qvis, conf)`: the path is `N (q)`, `dad[N]`,
/// ..., `N + 1 (p)`.
pub fn make_path(
    p: Ppoint,
    pp: i32,
    pvis: &[f64],
    q: Ppoint,
    qp: i32,
    qvis: &[f64],
    conf: &Vconfig,
) -> Vec<i32> {
    let v = conf.n;
    if direct_vis(p, pp, q, qp, conf) {
        let mut dad = vec![0i32; v + 2];
        dad[v] = (v + 1) as i32;
        dad[v + 1] = -1;
        return dad;
    }
    let wadj = |i: usize, j: usize| -> f64 {
        if i == v {
            qvis[j]
        } else if i == v + 1 {
            pvis[j]
        } else {
            conf.vis[i][j]
        }
    };
    shortest_path(v + 1, v, v + 2, &wadj)
}
