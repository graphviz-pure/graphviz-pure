//! `lib/pathplan/shortest.c`: Euclidean shortest path between two points
//! inside a simple polygon (triangulate, find the triangle strip, run the
//! funnel algorithm).

use super::Ppoint;
use super::triang::{ISCCW, ISCW, ccw};

pub(super) const NONE: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tedge {
    pnl0: usize,
    pnl1: usize,
    pub(super) right_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Triangle {
    mark: i32,
    pub(super) e: [Tedge; 3],
}

/// Point-and-link nodes: indices `0..pn` are polygon points, `pn` and
/// `pn + 1` the two endpoints.
struct Pnl {
    pp: Vec<Ppoint>,
    link: Vec<usize>,
}

struct Deque {
    pnlps: Vec<usize>,
    fpnlpi: usize,
    lpnlpi: usize,
    apex: usize,
}

impl Deque {
    fn add_front(&mut self, pnl: &mut Pnl, p: usize) {
        if self.lpnlpi >= self.fpnlpi {
            pnl.link[p] = self.pnlps[self.fpnlpi];
        }
        self.fpnlpi -= 1;
        self.pnlps[self.fpnlpi] = p;
    }
    fn add_back(&mut self, pnl: &mut Pnl, p: usize) {
        if self.lpnlpi >= self.fpnlpi {
            pnl.link[p] = self.pnlps[self.lpnlpi];
        }
        self.lpnlpi = self.lpnlpi.wrapping_add(1);
        self.pnlps[self.lpnlpi] = p;
    }
}

fn pointintri(tris: &[Triangle], pnl: &Pnl, trii: usize, p: Ppoint) -> bool {
    let mut sum = 0;
    for ei in 0..3 {
        let e = tris[trii].e[ei];
        if ccw(pnl.pp[e.pnl0], pnl.pp[e.pnl1], p) != ISCW {
            sum += 1;
        }
    }
    sum == 3 || sum == 0
}

fn marktripath(tris: &mut [Triangle], trii: usize, trij: usize) -> bool {
    if tris[trii].mark != 0 {
        return false;
    }
    tris[trii].mark = 1;
    if trii == trij {
        return true;
    }
    for ei in 0..3 {
        let r = tris[trii].e[ei].right_index;
        if r != NONE && marktripath(tris, r, trij) {
            return true;
        }
    }
    tris[trii].mark = 0;
    false
}

fn finddqsplit(dq: &Deque, pnl: &Pnl, p: usize) -> usize {
    let mut index = dq.fpnlpi;
    while index < dq.apex {
        if ccw(
            pnl.pp[dq.pnlps[index + 1]],
            pnl.pp[dq.pnlps[index]],
            pnl.pp[p],
        ) == ISCCW
        {
            return index;
        }
        index += 1;
    }
    let mut index = dq.lpnlpi;
    while index > dq.apex {
        if ccw(
            pnl.pp[dq.pnlps[index - 1]],
            pnl.pp[dq.pnlps[index]],
            pnl.pp[p],
        ) == ISCW
        {
            return index;
        }
        index -= 1;
    }
    dq.apex
}

#[cfg(test)]
thread_local! {
    /// The C lines `prerror` reported, for the tests: it writes straight to stderr.
    static REPORTED: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `prerror(msg)` of `shortest.c`, with the C line number it reports.
fn prerror(line: u32, msg: &str) {
    #[cfg(test)]
    REPORTED.with(|r| r.borrow_mut().push(line));
    graphviz_pure_core::diag::fprintf_stderr(&format!("lib/pathplan/shortest.c:{line}: {msg}\n"));
}

/// `Pshortestpath(polyp, eps, &output)`: `Err` when an endpoint is outside
/// the polygon or triangulation fails.
pub fn pshortestpath(
    poly: &[Ppoint],
    eps: [Ppoint; 2],
) -> Result<Vec<Ppoint>, super::PathplanError> {
    let pn = poly.len();
    // Load points in CCW order, dropping repeated points.
    let mut minpi = NONE;
    let mut minx = f64::INFINITY;
    for (i, p) in poly.iter().enumerate() {
        if minx > p.x {
            minx = p.x;
            minpi = i;
        }
    }
    let p2 = poly[minpi];
    let p1 = poly[if minpi == 0 { pn - 1 } else { minpi - 1 }];
    let p3 = poly[(minpi + 1) % pn];
    let mut pnl = Pnl {
        pp: Vec::with_capacity(pn + 2),
        link: Vec::new(),
    };
    if (p1.x == p2.x && p2.x == p3.x && p3.y > p2.y) || ccw(p1, p2, p3) != ISCCW {
        for pi in (0..pn).rev() {
            if pi < pn - 1 && poly[pi].x == poly[pi + 1].x && poly[pi].y == poly[pi + 1].y {
                continue;
            }
            pnl.pp.extend(poly.get(pi));
        }
    } else {
        for pi in 0..pn {
            if pi > 0 && poly[pi].x == poly[pi - 1].x && poly[pi].y == poly[pi - 1].y {
                continue;
            }
            pnl.pp.extend(poly.get(pi));
        }
    }
    let pnll = pnl.pp.len();
    pnl.pp.push(eps[0]);
    pnl.pp.push(eps[1]);
    // pnls[k].link = &pnls[k % pn] initially.
    pnl.link = (0..pnll + 2)
        .map(|k| if k < pnll { k % pn } else { NONE })
        .collect();

    // Triangulate. C keeps the triangles found so far if it fails. A polygon of n points has
    // n - 2 triangles; a failed triangulation stops short of that.
    let mut tris: Vec<Triangle> = Vec::with_capacity(pnll.saturating_sub(2));
    let whole = super::clip::triangulate(pnl.pp.get(..pnll).unwrap_or(&[]), &mut tris);
    #[cfg(any(test, feature = "pathplan-check"))]
    {
        let (c_tris, c_whole) = triangulate_c(pnl.pp.get(..pnll).unwrap_or(&[]));
        assert_eq!(whole, c_whole, "triangulation: C's result differs");
        assert_eq!(tris, c_tris, "triangulation: C's triangles differ");
    }
    if !whole {
        prerror(333, "triangulation failed");
    }
    let Some(ftrii) = (0..tris.len()).find(|&t| pointintri(&tris, &pnl, t, eps[0])) else {
        prerror(182, "source point not in any triangle");
        return Err(super::PathplanError);
    };
    let Some(ltrii) = (0..tris.len()).find(|&t| pointintri(&tris, &pnl, t, eps[1])) else {
        prerror(193, "destination point not in any triangle");
        return Err(super::PathplanError);
    };
    if !marktripath(&mut tris, ftrii, ltrii) {
        // a straight line is better than failing
        prerror(203, "cannot find triangle path");
        return Ok(vec![eps[0], eps[1]]);
    }
    if ftrii == ltrii {
        return Ok(vec![eps[0], eps[1]]);
    }
    let e0 = pnll;
    let e1 = pnll + 1;
    let dqn = pn * 2;
    let mut dq = Deque {
        pnlps: vec![0; dqn],
        fpnlpi: dqn / 2,
        lpnlpi: dqn / 2 - 1,
        apex: 0,
    };
    dq.add_front(&mut pnl, e0);
    dq.apex = dq.fpnlpi;
    let mut trii = ftrii;
    while trii != NONE {
        tris[trii].mark = 2;
        let trip = tris[trii];
        let ei = (0..3)
            .find(|&ei| trip.e[ei].right_index != NONE && tris[trip.e[ei].right_index].mark == 1);
        let (lpnlp, rpnlp) = match ei {
            None => {
                if ccw(
                    eps[1],
                    pnl.pp[dq.pnlps[dq.fpnlpi]],
                    pnl.pp[dq.pnlps[dq.lpnlpi]],
                ) == ISCCW
                {
                    (dq.pnlps[dq.lpnlpi], e1)
                } else {
                    (e1, dq.pnlps[dq.lpnlpi])
                }
            }
            Some(ei) => {
                let pnlp = trip.e[(ei + 1) % 3].pnl1;
                if ccw(
                    pnl.pp[trip.e[ei].pnl0],
                    pnl.pp[pnlp],
                    pnl.pp[trip.e[ei].pnl1],
                ) == ISCCW
                {
                    (trip.e[ei].pnl1, trip.e[ei].pnl0)
                } else {
                    (trip.e[ei].pnl0, trip.e[ei].pnl1)
                }
            }
        };
        if trii == ftrii {
            dq.add_back(&mut pnl, lpnlp);
            dq.add_front(&mut pnl, rpnlp);
        } else if dq.pnlps[dq.fpnlpi] != rpnlp && dq.pnlps[dq.lpnlpi] != rpnlp {
            let split = finddqsplit(&dq, &pnl, rpnlp);
            dq.fpnlpi = split;
            dq.add_front(&mut pnl, rpnlp);
            if split > dq.apex {
                dq.apex = split;
            }
        } else {
            let split = finddqsplit(&dq, &pnl, lpnlp);
            dq.lpnlpi = split;
            dq.add_back(&mut pnl, lpnlp);
            if split < dq.apex {
                dq.apex = split;
            }
        }
        trii = NONE;
        for ei in 0..3 {
            let r = trip.e[ei].right_index;
            if r != NONE && tris[r].mark == 1 {
                trii = r;
                break;
            }
        }
    }
    // The path, from `e1` back along the links: counted first, so `out` is allocated once.
    let walk = || {
        std::iter::successors(Some(e1), |&p| {
            (p != e0).then(|| pnl.link[p]).filter(|&l| l != NONE)
        })
    };
    let mut out = Vec::with_capacity(walk().count());
    out.extend(walk().map(|p| pnl.pp[p]));
    out.reverse();
    Ok(out)
}

/// C: lib/pathplan/shortest.c:triangulate and connecttris, as they are: the reference for
/// `clip::triangulate`.
#[cfg(any(test, feature = "pathplan-check"))]
pub(super) fn triangulate_c(points: &[Ppoint]) -> (Vec<Triangle>, bool) {
    use super::triang::isdiagonal;
    let mut tris: Vec<Triangle> = Vec::new();
    let mut pts: Vec<usize> = (0..points.len()).collect();
    let mut coords = Vec::with_capacity(points.len());
    let mut whole = true;
    loop {
        let n = pts.len();
        if n > 3 {
            coords.clear();
            coords.extend(pts.iter().map(|&k| points[k]));
            let mut found = false;
            for i in 0..n {
                let ip1 = (i + 1) % n;
                let ip2 = (i + 2) % n;
                if isdiagonal(i, ip2, &coords) {
                    tris.push(make_tri(pts[i], pts[ip1], pts[ip2]));
                    pts.remove(ip1);
                    found = true;
                    break;
                }
            }
            if !found {
                whole = false;
                break;
            }
        } else {
            if n == 3 {
                tris.push(make_tri(pts[0], pts[1], pts[2]));
            }
            break;
        }
    }
    for trii in 0..tris.len() {
        for trij in (trii + 1)..tris.len() {
            for ei in 0..3 {
                for ej in 0..3 {
                    let (a, b) = (tris[trii].e[ei], tris[trij].e[ej]);
                    if (a.pnl0 == b.pnl0 && a.pnl1 == b.pnl1)
                        || (a.pnl0 == b.pnl1 && a.pnl1 == b.pnl0)
                    {
                        tris[trii].e[ei].right_index = trij;
                        tris[trij].e[ej].right_index = trii;
                    }
                }
            }
        }
    }
    (tris, whole)
}

pub(super) fn make_tri(a: usize, b: usize, c: usize) -> Triangle {
    Triangle {
        mark: 0,
        e: [
            Tedge {
                pnl0: a,
                pnl1: b,
                right_index: NONE,
            },
            Tedge {
                pnl0: b,
                pnl1: c,
                right_index: NONE,
            },
            Tedge {
                pnl0: c,
                pnl1: a,
                right_index: NONE,
            },
        ],
    }
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

    /// The points `pshortestpath` triangulates: counter-clockwise, without repeats.
    fn loaded(poly: &[Ppoint]) -> Vec<Ppoint> {
        let pn = poly.len();
        let minpi = (0..pn).fold(0, |m, i| if poly[m].x > poly[i].x { i } else { m });
        let p2 = poly[minpi];
        let p1 = poly[if minpi == 0 { pn - 1 } else { minpi - 1 }];
        let p3 = poly[(minpi + 1) % pn];
        let reverse = (p1.x == p2.x && p2.x == p3.x && p3.y > p2.y) || ccw(p1, p2, p3) != ISCCW;
        let mut out: Vec<Ppoint> = if reverse {
            poly.iter().rev().copied().collect()
        } else {
            poly.to_vec()
        };
        out.dedup_by(|a, b| a.x == b.x && a.y == b.y);
        out
    }

    /// A closed walk that need not be simple, so that some have no ear left to clip. The search
    /// goes on with the triangles it has, as C's does, and reports the failure once.
    #[test]
    fn a_failed_triangulation_is_reported_and_the_search_goes_on() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut failed, mut whole, mut routed) = (0, 0, 0);
        for case in 0..3000 {
            let n = 4 + rng.below(30) as usize;
            let grid = 2 + rng.below(24);
            let mut point = || Ppoint::new(rng.below(grid) as f64, rng.below(grid) as f64);
            let poly: Vec<Ppoint> = (0..n).map(|_| point()).collect();
            let eps = [point(), point()];
            let (_, c_whole) = triangulate_c(&loaded(&poly));
            REPORTED.with(|r| r.borrow_mut().clear());
            let result = pshortestpath(&poly, eps);
            let reported = REPORTED.with(|r| r.borrow().iter().filter(|&&l| l == 333).count());
            assert_eq!(reported, usize::from(!c_whole), "case {case}: {poly:?}");
            if c_whole {
                whole += 1;
            } else {
                failed += 1;
                routed += usize::from(result.is_ok());
            }
        }
        assert!(
            failed > 100 && whole > 100 && routed > 0,
            "{failed} {whole} {routed}"
        );
    }

    #[test]
    fn an_end_outside_every_triangle_is_refused() {
        let square = [
            Ppoint::new(0.0, 0.0),
            Ppoint::new(10.0, 0.0),
            Ppoint::new(10.0, 10.0),
            Ppoint::new(0.0, 10.0),
        ];
        let (inside, outside) = (Ppoint::new(5.0, 5.0), Ppoint::new(50.0, 50.0));
        REPORTED.with(|r| r.borrow_mut().clear());
        assert!(pshortestpath(&square, [outside, inside]).is_err());
        assert!(pshortestpath(&square, [inside, outside]).is_err());
        assert_eq!(REPORTED.with(|r| r.borrow().clone()), [182, 193]);
    }
}
