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

/// With coordinates at most this large, `area2` is within 0.0001 of the true area, the
/// tolerance of [`wind`] (`docs/perf/pathplan-shortcuts.md`). [`apart`] needs that.
const APART_MAX_COORD: f64 = 1e5;

fn within(p: Ppoint) -> bool {
    p.x.abs() <= APART_MAX_COORD && p.y.abs() <= APART_MAX_COORD
}

/// Whether `intersect(a, b, c, d)` is certainly false, for points [`within`] the limit: the
/// segments are apart along the axis `in_between` reads, or by enough along the other axis that
/// `wind` cannot call an end of `cd` collinear with `ab`. Four comparisons, against up to four
/// `wind`s (`docs/perf/pathplan-shortcuts.md`).
#[inline(always)]
fn apart(a: Ppoint, b: Ppoint, c: Ppoint, d: Ppoint) -> bool {
    let (ax0, ax1) = if a.x < b.x { (a.x, b.x) } else { (b.x, a.x) };
    let (ay0, ay1) = if a.y < b.y { (a.y, b.y) } else { (b.y, a.y) };
    let (cx0, cx1) = if c.x < d.x { (c.x, d.x) } else { (d.x, c.x) };
    let (cy0, cy1) = if c.y < d.y { (c.y, d.y) } else { (d.y, c.y) };
    let gap_x = (cx0 - ax1).max(ax0 - cx1);
    let gap_y = (cy0 - ay1).max(ay0 - cy1);
    if a.x != b.x {
        gap_x > 0.0 || (gap_y > 0.0 && gap_y * (ax1 - ax0) > 0.0002)
    } else {
        gap_y > 0.0 || (gap_x > 0.0 && gap_x * (ay1 - ay0) > 0.0002)
    }
}

/// C: lib/pathplan/visibility.c:clear. `bounded`: every point is [`within`] the limit.
fn clear(
    pti: Ppoint,
    ptj: Ppoint,
    (start, end): (usize, usize),
    v: usize,
    pts: &[Ppoint],
    next: &[usize],
    bounded: bool,
) -> bool {
    for k in (0..start).chain(end..v) {
        let (c, d) = (pts[k], pts[next[k]]);
        if bounded && apart(pti, ptj, c, d) {
            #[cfg(any(test, feature = "pathplan-check"))]
            assert!(
                !intersect(pti, ptj, c, d),
                "apart: {pti:?} {ptj:?} {c:?} {d:?}"
            );
            continue;
        }
        if intersect(pti, ptj, c, d) {
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
    let bounded = conf.p.iter().all(|&p| within(p));
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
                && clear(pts[i], pts[ju], (v, v), v, pts, next, bounded)
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
    let bounded = within(p) && pts.iter().all(|&q| within(q));
    for k in (0..start).chain(end..v) {
        let pk = pts[k];
        vadj[k] = if in_cone(pts[conf.prev[k]], pk, pts[conf.next[k]], p)
            && clear(p, pk, (start, end), v, pts, &conf.next, bounded)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*: a fixed sequence, so a failure names its case.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Points that make the tolerance matter: on a coarse grid, a hair off it, at every scale.
    fn point(rng: &mut Rng) -> Ppoint {
        let scale = [1.0, 72.0, 1e3, 1e5][rng.below(4) as usize];
        let hair = [0.0, 1e-9, 1e-6, 1e-5, 5e-5, 1e-4, 2e-4, 1e-3][rng.below(8) as usize];
        let coord = |rng: &mut Rng| {
            let grid = rng.below(9) as f64 / 8.0 * 2.0 - 1.0;
            let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
            (grid * scale + sign * hair * rng.below(3) as f64).clamp(-1e5, 1e5)
        };
        Ppoint::new(coord(rng), coord(rng))
    }

    #[test]
    fn apart_segments_never_intersect() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut skipped, mut crossing) = (0u32, 0u32);
        for case in 0..2_000_000 {
            let (a, b, c, d) = (
                point(&mut rng),
                point(&mut rng),
                point(&mut rng),
                point(&mut rng),
            );
            let hit = intersect(a, b, c, d);
            if apart(a, b, c, d) {
                assert!(!hit, "case {case}: {a:?} {b:?} {c:?} {d:?}");
                skipped += 1;
            }
            crossing += u32::from(hit);
        }
        assert!(
            skipped > 100_000 && crossing > 100_000,
            "{skipped} {crossing}"
        );
    }

    #[test]
    fn a_small_gap_along_the_other_axis_is_not_apart() {
        // cd starts a hair above ab, between its ends: wind calls it collinear, so `intersect`
        // says yes though the boxes do not touch
        let (a, b) = (Ppoint::new(0.0, 0.0), Ppoint::new(1.0, 0.0));
        let (c, d) = (Ppoint::new(0.5, 1e-5), Ppoint::new(0.5, 1.0));
        assert!(intersect(a, b, c, d));
        assert!(!apart(a, b, c, d));
        // the same with ab upright
        let (a, b) = (Ppoint::new(0.0, 0.0), Ppoint::new(0.0, 1.0));
        let (c, d) = (Ppoint::new(1e-5, 0.5), Ppoint::new(1.0, 0.5));
        assert!(intersect(a, b, c, d));
        assert!(!apart(a, b, c, d));
        // and by enough: the same shapes further off are skipped
        let (c, d) = (Ppoint::new(0.01, 0.5), Ppoint::new(1.0, 0.5));
        assert!(!intersect(a, b, c, d));
        assert!(apart(a, b, c, d));
    }

    #[test]
    fn the_limit_keeps_large_and_nan_coordinates_out() {
        assert!(within(Ppoint::new(1e5, -1e5)));
        assert!(!within(Ppoint::new(1.0000001e5, 0.0)));
        assert!(!within(Ppoint::new(0.0, -1.0000001e5)));
        assert!(!within(Ppoint::new(f64::NAN, 0.0)));
        assert!(!within(Ppoint::new(0.0, f64::NAN)));
    }

    /// Random convex obstacles, which is what the routers give.
    fn obstacles(rng: &mut Rng) -> (Vec<Ppoint>, Vec<usize>, Vec<usize>) {
        let (mut pts, mut next, mut starts) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..2 + rng.below(8) {
            let (cx, cy) = (rng.below(400) as f64, rng.below(400) as f64);
            let sides = 3 + rng.below(6) as usize;
            let first = pts.len();
            starts.push(first);
            for s in 0..sides {
                let angle = s as f64 / sides as f64 * std::f64::consts::TAU;
                pts.push(Ppoint::new(
                    cx + 20.0 * math::cos(angle),
                    cy + 20.0 * math::sin(angle),
                ));
                next.push(if s + 1 == sides { first } else { first + s + 1 });
            }
        }
        starts.push(pts.len());
        (pts, next, starts)
    }

    #[test]
    fn clear_answers_as_it_does_without_the_skip() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let (mut open, mut blocked) = (0, 0);
        for case in 0..400 {
            let (pts, next, starts) = obstacles(&mut rng);
            let v = pts.len();
            for i in 0..v {
                for j in 0..i {
                    let plain = clear(pts[i], pts[j], (v, v), v, &pts, &next, false);
                    let skip = clear(pts[i], pts[j], (v, v), v, &pts, &next, true);
                    assert_eq!(plain, skip, "case {case}: {i} {j}");
                    if plain {
                        open += 1;
                    } else {
                        blocked += 1;
                    }
                }
            }
            // and from a point outside the list, ignoring one polygon
            let p = Ppoint::new(rng.below(400) as f64, rng.below(400) as f64);
            let (s, e) = (starts[0], starts[1]);
            for k in e..v {
                let plain = clear(p, pts[k], (s, e), v, &pts, &next, false);
                assert_eq!(
                    plain,
                    clear(p, pts[k], (s, e), v, &pts, &next, true),
                    "case {case}"
                );
            }
        }
        assert!(open > 1000 && blocked > 1000, "{open} {blocked}");
    }
}
