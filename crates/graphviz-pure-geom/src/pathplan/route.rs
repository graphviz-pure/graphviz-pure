//! `lib/pathplan/route.c`: fit a piecewise Bézier through a polyline route
//! while staying clear of barrier segments.

use super::solvers::solve3;
use super::{Pedge, Ppoint};
use graphviz_pure_core::math;

const EPSILON1: f64 = 1e-3;
const EPSILON2: f64 = 1e-6;

#[derive(Clone, Copy, Default)]
struct Tna {
    t: f64,
    a: [Ppoint; 2],
}

fn add(p1: Ppoint, p2: Ppoint) -> Ppoint {
    Ppoint::new(p1.x + p2.x, p1.y + p2.y)
}
fn sub(p1: Ppoint, p2: Ppoint) -> Ppoint {
    Ppoint::new(p1.x - p2.x, p1.y - p2.y)
}
fn dist(p1: Ppoint, p2: Ppoint) -> f64 {
    math::hypot(p2.x - p1.x, p2.y - p1.y)
}
fn scale(p: Ppoint, c: f64) -> Ppoint {
    Ppoint::new(p.x * c, p.y * c)
}
fn dot(p1: Ppoint, p2: Ppoint) -> f64 {
    p1.x.mul_add(p2.x, p1.y * p2.y)
}
fn b0(t: f64) -> f64 {
    let tmp = 1.0 - t;
    tmp * tmp * tmp
}
fn b1(t: f64) -> f64 {
    let tmp = 1.0 - t;
    3.0 * t * tmp * tmp
}
fn b2(t: f64) -> f64 {
    let tmp = 1.0 - t;
    3.0 * t * t * tmp
}
fn b3(t: f64) -> f64 {
    t * t * t
}
fn b01(t: f64) -> f64 {
    let tmp = 1.0 - t;
    tmp * tmp * 3.0f64.mul_add(t, tmp)
}
fn b23(t: f64) -> f64 {
    let tmp = 1.0 - t;
    t * t * 3.0f64.mul_add(tmp, t)
}

fn normv(mut v: Ppoint) -> Ppoint {
    let mut d = v.x.mul_add(v.x, v.y * v.y);
    if d > 1e-6 {
        d = d.sqrt();
        v.x /= d;
        v.y /= d;
    }
    v
}

/// `Proutespline(barriers, n, input_route, endpoint_slopes, &output)`.
/// Returns the control points (`3k + 1` of them) or `Err` on failure.
pub fn proutespline(
    barriers: &[Pedge],
    input_route: &[Ppoint],
    endpoint_slopes: [Ppoint; 2],
) -> Result<Vec<Ppoint>, super::PathplanError> {
    let ev0 = normv(endpoint_slopes[0]);
    let ev1 = normv(endpoint_slopes[1]);
    let mut ops = vec![input_route[0]];
    let mut tnas = Vec::new();
    really_route_spline(barriers, input_route, ev0, ev1, &mut ops, &mut tnas)?;
    Ok(ops)
}

fn really_route_spline(
    edges: &[Pedge],
    inps: &[Ppoint],
    ev0: Ppoint,
    ev1: Ppoint,
    ops: &mut Vec<Ppoint>,
    tnas: &mut Vec<Tna>,
) -> Result<(), super::PathplanError> {
    let inpn = inps.len();
    tnas.clear();
    tnas.resize(inpn, Tna::default());
    for i in 1..inpn {
        tnas[i].t = tnas[i - 1].t + dist(inps[i], inps[i - 1]);
    }
    for i in 1..inpn {
        tnas[i].t /= tnas[inpn - 1].t;
    }
    for t in tnas.iter_mut() {
        t.a[0] = scale(ev0, b1(t.t));
        t.a[1] = scale(ev1, b2(t.t));
    }
    let (p1, v1, p2, v2) = mkspline(inps, tnas, ev0, ev1);
    if splinefits(edges, p1, v1, p2, v2, inps, ops) {
        return Ok(());
    }
    let cp1 = add(p1, scale(v1, 1.0 / 3.0));
    let cp2 = sub(p2, scale(v2, 1.0 / 3.0));
    let mut maxi: isize = -1;
    let mut maxd = -1.0;
    for i in 1..inpn.saturating_sub(1) {
        let t = tnas[i].t;
        let px = b3(t).mul_add(
            p2.x,
            b2(t).mul_add(cp2.x, b0(t).mul_add(p1.x, b1(t) * cp1.x)),
        );
        let py = b3(t).mul_add(
            p2.y,
            b2(t).mul_add(cp2.y, b0(t).mul_add(p1.y, b1(t) * cp1.y)),
        );
        let d = dist(Ppoint::new(px, py), inps[i]);
        if d > maxd {
            maxd = d;
            maxi = i as isize;
        }
    }
    if maxi < 1 {
        return Err(super::PathplanError);
    }
    let spliti = maxi as usize;
    let splitv1 = normv(sub(inps[spliti], inps[spliti - 1]));
    let splitv2 = normv(sub(inps[spliti + 1], inps[spliti]));
    let splitv = normv(add(splitv1, splitv2));
    really_route_spline(edges, &inps[..=spliti], ev0, splitv, ops, tnas)?;
    really_route_spline(edges, &inps[spliti..], splitv, ev1, ops, tnas)
}

fn mkspline(
    inps: &[Ppoint],
    tnas: &[Tna],
    ev0: Ppoint,
    ev1: Ppoint,
) -> (Ppoint, Ppoint, Ppoint, Ppoint) {
    let inpn = inps.len();
    let mut c = [[0.0f64; 2]; 2];
    let mut x = [0.0f64; 2];
    for i in 0..inpn {
        c[0][0] += dot(tnas[i].a[0], tnas[i].a[0]);
        c[0][1] += dot(tnas[i].a[0], tnas[i].a[1]);
        c[1][0] = c[0][1];
        c[1][1] += dot(tnas[i].a[1], tnas[i].a[1]);
        let tmp = sub(
            inps[i],
            add(
                scale(inps[0], b01(tnas[i].t)),
                scale(inps[inpn - 1], b23(tnas[i].t)),
            ),
        );
        x[0] += dot(tnas[i].a[0], tmp);
        x[1] += dot(tnas[i].a[1], tmp);
    }
    let det01 = c[0][0].mul_add(c[1][1], -(c[1][0] * c[0][1]));
    let det0x = c[0][0].mul_add(x[1], -(c[0][1] * x[0]));
    let detx1 = x[0].mul_add(c[1][1], -(x[1] * c[0][1]));
    let (mut scale0, mut scale3) = (0.0, 0.0);
    if det01.abs() >= 1e-6 {
        scale0 = detx1 / det01;
        scale3 = det0x / det01;
    }
    if det01.abs() < 1e-6 || scale0 <= 0.0 || scale3 <= 0.0 {
        let d01 = dist(inps[0], inps[inpn - 1]) / 3.0;
        scale0 = d01;
        scale3 = d01;
    }
    (
        inps[0],
        scale(ev0, scale0),
        inps[inpn - 1],
        scale(ev1, scale3),
    )
}

fn dist_n(p: &[Ppoint]) -> f64 {
    let mut rv = 0.0;
    for i in 1..p.len() {
        rv += math::hypot(p[i].x - p[i - 1].x, p[i].y - p[i - 1].y);
    }
    rv
}

fn splinefits(
    edges: &[Pedge],
    pa: Ppoint,
    va: Ppoint,
    pb: Ppoint,
    vb: Ppoint,
    inps: &[Ppoint],
    ops: &mut Vec<Ppoint>,
) -> bool {
    let forceflag = inps.len() == 2;
    let mut first = true;
    let mut a = 4.0f64;
    loop {
        let sps = [
            pa,
            Ppoint::new(pa.x + a * va.x / 3.0, pa.y + a * va.y / 3.0),
            Ppoint::new(pb.x - a * vb.x / 3.0, pb.y - a * vb.y / 3.0),
            pb,
        ];
        if first && dist_n(&sps) < dist_n(inps) - EPSILON1 {
            return false;
        }
        first = false;
        if splineisinside(edges, &sps) {
            ops.extend_from_slice(&sps[1..]);
            return true;
        }
        if a < 0.005 {
            if forceflag {
                ops.extend_from_slice(&sps[1..]);
                return true;
            }
            break;
        }
        if a > 0.01 {
            a /= 2.0;
        } else {
            a = 0.0;
        }
    }
    false
}

fn splineisinside(edges: &[Pedge], sps: &[Ppoint; 4]) -> bool {
    let mut roots = [0.0f64; 4];
    for e in edges {
        let lps = [e.a, e.b];
        let rootn = splineintersectsline(sps, &lps, &mut roots);
        if rootn == 4 {
            continue;
        }
        for &t in roots.iter().take(rootn) {
            if !(EPSILON2..=1.0 - EPSILON2).contains(&t) {
                continue;
            }
            let td = t * t * t;
            let tc = 3.0 * t * t * (1.0 - t);
            let tb = 3.0 * t * (1.0 - t) * (1.0 - t);
            let ta = (1.0 - t) * (1.0 - t) * (1.0 - t);
            let ipx = td.mul_add(
                sps[3].x,
                tc.mul_add(sps[2].x, ta.mul_add(sps[0].x, tb * sps[1].x)),
            );
            let ipy = td.mul_add(
                sps[3].y,
                tc.mul_add(sps[2].y, ta.mul_add(sps[0].y, tb * sps[1].y)),
            );
            let ds = |q: Ppoint| (ipx - q.x).mul_add(ipx - q.x, (ipy - q.y) * (ipy - q.y));
            if ds(lps[0]) < EPSILON1 || ds(lps[1]) < EPSILON1 {
                continue;
            }
            return false;
        }
    }
    true
}

fn addroot(root: f64, roots: &mut [f64; 4], n: &mut usize) {
    if (0.0..=1.0).contains(&root) {
        roots[*n] = root;
        *n += 1;
    }
}

fn points2coeff(v0: f64, v1: f64, v2: f64, v3: f64) -> [f64; 4] {
    [
        v0,
        3.0 * (v1 - v0),
        (-6.0f64).mul_add(v1, 3.0f64.mul_add(v0, 3.0 * v2)),
        3.0f64.mul_add(v1, v3) - 3.0f64.mul_add(v2, v0),
    ]
}

fn horner(s: &[f64; 4], tv: f64) -> f64 {
    tv.mul_add(tv.mul_add(tv.mul_add(s[3], s[2]), s[1]), s[0])
}

fn splineintersectsline(sps: &[Ppoint; 4], lps: &[Ppoint; 2], roots: &mut [f64; 4]) -> usize {
    let xc = [lps[0].x, lps[1].x - lps[0].x];
    let yc = [lps[0].y, lps[1].y - lps[0].y];
    let mut rootn = 0;
    let mut xr = [0.0f64; 3];
    let mut yr = [0.0f64; 3];
    if xc[1] == 0.0 {
        if yc[1] == 0.0 {
            let mut s = points2coeff(sps[0].x, sps[1].x, sps[2].x, sps[3].x);
            s[0] -= xc[0];
            let xn = solve3(&s, &mut xr);
            let mut s = points2coeff(sps[0].y, sps[1].y, sps[2].y, sps[3].y);
            s[0] -= yc[0];
            let yn = solve3(&s, &mut yr);
            if xn == 4 {
                if yn == 4 {
                    return 4;
                }
                for &r in yr.iter().take(yn) {
                    addroot(r, roots, &mut rootn);
                }
            } else if yn == 4 {
                for &r in xr.iter().take(xn) {
                    addroot(r, roots, &mut rootn);
                }
            } else {
                for &a in xr.iter().take(xn) {
                    for &b in yr.iter().take(yn) {
                        if a == b {
                            addroot(a, roots, &mut rootn);
                        }
                    }
                }
            }
            rootn
        } else {
            let mut s = points2coeff(sps[0].x, sps[1].x, sps[2].x, sps[3].x);
            s[0] -= xc[0];
            let xn = solve3(&s, &mut xr);
            if xn == 4 {
                return 4;
            }
            for &tv in xr.iter().take(xn) {
                if (0.0..=1.0).contains(&tv) {
                    let s = points2coeff(sps[0].y, sps[1].y, sps[2].y, sps[3].y);
                    let sv = (horner(&s, tv) - yc[0]) / yc[1];
                    if (0.0..=1.0).contains(&sv) {
                        addroot(tv, roots, &mut rootn);
                    }
                }
            }
            rootn
        }
    } else {
        let rat = yc[1] / xc[1];
        let f = |p: Ppoint| (-rat).mul_add(p.x, p.y);
        let mut s = points2coeff(f(sps[0]), f(sps[1]), f(sps[2]), f(sps[3]));
        s[0] += rat.mul_add(xc[0], -yc[0]);
        let xn = solve3(&s, &mut xr);
        if xn == 4 {
            return 4;
        }
        for &tv in xr.iter().take(xn) {
            if (0.0..=1.0).contains(&tv) {
                let s = points2coeff(sps[0].x, sps[1].x, sps[2].x, sps[3].x);
                let sv = (horner(&s, tv) - xc[0]) / xc[1];
                if (0.0..=1.0).contains(&sv) {
                    addroot(tv, roots, &mut rootn);
                }
            }
        }
        rootn
    }
}
