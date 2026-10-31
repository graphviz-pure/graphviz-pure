//! `lib/pathplan/triang.c`: orientation tests and ear-clipping triangulation.

use super::Ppoint;
use graphviz_pure_core::math;

pub const ISCCW: i32 = 1;
pub const ISCW: i32 = 2;
pub const ISON: i32 = 3;

/// `ccw(p1, p2, p3)`.
pub fn ccw(p1: Ppoint, p2: Ppoint, p3: Ppoint) -> i32 {
    let d = math::fma(p1.y - p2.y, p3.x - p2.x, -((p3.y - p2.y) * (p1.x - p2.x)));
    if d > 0.0 {
        ISCW
    } else if d < 0.0 {
        ISCCW
    } else {
        ISON
    }
}

fn between(pa: Ppoint, pb: Ppoint, pc: Ppoint) -> bool {
    let pba = Ppoint::new(pb.x - pa.x, pb.y - pa.y);
    let pca = Ppoint::new(pc.x - pa.x, pc.y - pa.y);
    if ccw(pa, pb, pc) != ISON {
        return false;
    }
    math::fma(pca.x, pba.x, pca.y * pba.y) >= 0.0
        && math::fma(pca.x, pca.x, pca.y * pca.y) <= math::fma(pba.x, pba.x, pba.y * pba.y)
}

fn intersects(pa: Ppoint, pb: Ppoint, pc: Ppoint, pd: Ppoint) -> bool {
    let c1 = ccw(pa, pb, pc);
    let c2 = ccw(pa, pb, pd);
    let c3 = ccw(pc, pd, pa);
    let c4 = ccw(pc, pd, pb);
    if c1 == ISON || c2 == ISON || c3 == ISON || c4 == ISON {
        between(pa, pb, pc) || between(pa, pb, pd) || between(pc, pd, pa) || between(pc, pd, pb)
    } else {
        ((c1 == ISCCW) ^ (c2 == ISCCW)) && ((c3 == ISCCW) ^ (c4 == ISCCW))
    }
}

/// Why `isdiagonal` said no.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Refused {
    /// The test of the corner at `i`, on its four points, failed.
    Corner,
    /// The side from point `.0` to point `.1` crosses the diagonal.
    Side(usize, usize),
}

/// `isdiagonal(i, ip2, points, n, indexer)`, saying why not. `None`: it is a diagonal.
pub(super) fn refusal(i: usize, ip2: usize, pts: &[Ppoint]) -> Option<Refused> {
    let n = pts.len();
    let ip1 = (i + 1) % n;
    let im1 = (i + n - 1) % n;
    let res = if ccw(pts[im1], pts[i], pts[ip1]) == ISCCW {
        ccw(pts[i], pts[ip2], pts[im1]) == ISCCW && ccw(pts[ip2], pts[i], pts[ip1]) == ISCCW
    } else {
        ccw(pts[i], pts[ip2], pts[ip1]) == ISCW
    };
    if !res {
        return Some(Refused::Corner);
    }
    for j in 0..n {
        let jp1 = (j + 1) % n;
        if !(j == i || jp1 == i || j == ip2 || jp1 == ip2)
            && intersects(pts[i], pts[ip2], pts[j], pts[jp1])
        {
            return Some(Refused::Side(j, jp1));
        }
    }
    None
}

/// `isdiagonal(i, ip2, points, n, indexer)`: is `(i, ip2)` a diagonal?
pub fn isdiagonal(i: usize, ip2: usize, pts: &[Ppoint]) -> bool {
    refusal(i, ip2, pts).is_none()
}

/// `Ptriangulate(polygon, fn, vc)`: calls `f` with each triangle; `Err` if
/// no diagonal exists.
pub fn ptriangulate(
    polygon: &[Ppoint],
    f: &mut dyn FnMut(&[Ppoint; 3]),
) -> Result<(), super::PathplanError> {
    let mut pts: Vec<Ppoint> = polygon.to_vec();
    loop {
        let n = pts.len();
        if n > 3 {
            let mut found = false;
            for i in 0..n {
                let ip1 = (i + 1) % n;
                let ip2 = (i + 2) % n;
                if isdiagonal(i, ip2, &pts) {
                    f(&[pts[i], pts[ip1], pts[ip2]]);
                    pts.remove(ip1);
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(super::PathplanError);
            }
        } else {
            f(&[pts[0], pts[1], pts[2]]);
            return Ok(());
        }
    }
}
