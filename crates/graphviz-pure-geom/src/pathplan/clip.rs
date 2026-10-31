//! The ear clipping of `lib/pathplan/shortest.c:triangulate`, and `connecttris`, without their
//! repeated work.
//!
//! C clips the first vertex whose diagonal passes `isdiagonal`, then starts again from the first
//! vertex, so it re-tests the same vertices after every clip, and it then finds the triangles that
//! share a side by comparing every pair. This clips the same vertices in the same order, by two
//! facts (`docs/perf/pathplan-shortcuts.md`):
//!
//! - `isdiagonal` is a pure function of the points it reads, so a refusal stands while what caused
//!   it stands: the corner's four points, or a side that crosses the diagonal;
//! - a clip knows which triangle made each side it uses, so nothing needs to compare triangles.

use super::Ppoint;
use super::shortest::{NONE, Triangle, make_tri};
use super::triang::{Refused, refusal};

/// Why a vertex was refused the last time it was tried, while that still holds. Vertices by
/// their index in the polygon.
#[derive(Clone, Copy)]
enum Refusal {
    Untried,
    /// The test of the corner failed, on these neighbours.
    Corner {
        before: u32,
        after: u32,
        far: u32,
    },
    /// The side from `from` to `to` crosses the diagonal to `far`.
    Side {
        far: u32,
        from: u32,
        to: u32,
    },
}

#[derive(Clone, Copy)]
struct Vertex {
    /// The vertex after this one in the polygon left to clip; `NIL` once it is clipped.
    next: u32,
    /// The triangle and edge that made the side from this vertex to `next`, if a clip did.
    side_of: (usize, usize),
    refusal: Refusal,
}

const NIL: u32 = u32::MAX;

#[cfg(test)]
thread_local! {
    /// How many times `refusal` ran: for the test that the refusals save work.
    static TESTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The two triangles on a side are each other's neighbours there.
fn link(tris: &mut [Triangle], t: usize, ei: usize, (other, ej): (usize, usize)) {
    if other == NONE {
        return;
    }
    if let Some(edge) = tris.get_mut(t).and_then(|tri| tri.e.get_mut(ei)) {
        edge.right_index = other;
    }
    if let Some(edge) = tris.get_mut(other).and_then(|tri| tri.e.get_mut(ej)) {
        edge.right_index = t;
    }
}

/// Triangulates the polygon `points` as C does, into triangles over indices of `points`, with
/// their neighbours set. `false` when no vertex can be clipped: C reports that, and keeps the
/// triangles found so far.
pub(super) fn triangulate(points: &[Ppoint], tris: &mut Vec<Triangle>) -> bool {
    let count = points.len();
    let mut vertices: Vec<Vertex> = (0..count)
        .map(|i| Vertex {
            next: if i.wrapping_add(1) == count {
                0
            } else {
                i.wrapping_add(1) as u32
            },
            side_of: (NONE, 0),
            refusal: Refusal::Untried,
        })
        .collect();
    // the polygon left to clip, in C's array order, and its coordinates for `refusal`
    let mut pts: Vec<usize> = (0..count).collect();
    let mut coords: Vec<Ppoint> = points.to_vec();
    tris.reserve_exact(count.saturating_sub(2));
    let side_of = |vertices: &[Vertex], v: usize| vertices.get(v).map_or((NONE, 0), |x| x.side_of);
    loop {
        let n = pts.len();
        if n <= 3 {
            if let &[a, b, c] = pts.as_slice() {
                let t = tris.len();
                tris.push(make_tri(a, b, c));
                link(tris, t, 0, side_of(&vertices, a));
                link(tris, t, 1, side_of(&vertices, b));
                link(tris, t, 2, side_of(&vertices, c));
            }
            return true;
        }
        let at = |i: usize| pts.get(i.checked_rem(n).unwrap_or(0)).copied().unwrap_or(0);
        let mut clipped = None;
        for i in 0..n {
            let (v, after, far) = (at(i), at(i.wrapping_add(1)), at(i.wrapping_add(2)));
            let before = at(i.wrapping_add(n).wrapping_sub(1));
            let standing = match vertices.get(v).map_or(Refusal::Untried, |x| x.refusal) {
                Refusal::Untried => false,
                Refusal::Corner {
                    before: b,
                    after: a,
                    far: f,
                } => b as usize == before && a as usize == after && f as usize == far,
                Refusal::Side { far: f, from, to } => {
                    f as usize == far && vertices.get(from as usize).is_some_and(|x| x.next == to)
                }
            };
            if standing {
                continue;
            }
            #[cfg(test)]
            TESTED.with(|t| t.set(t.get() + 1));
            let refused = match refusal(i, i.wrapping_add(2).checked_rem(n).unwrap_or(0), &coords) {
                None => {
                    clipped = Some((i, v, after, far));
                    break;
                }
                Some(Refused::Corner) => Refusal::Corner {
                    before: before as u32,
                    after: after as u32,
                    far: far as u32,
                },
                Some(Refused::Side(j, jp1)) => Refusal::Side {
                    far: far as u32,
                    from: pts.get(j).copied().unwrap_or(0) as u32,
                    to: pts.get(jp1).copied().unwrap_or(0) as u32,
                },
            };
            if let Some(x) = vertices.get_mut(v) {
                x.refusal = refused;
            }
        }
        let Some((i, v, after, far)) = clipped else {
            return false;
        };
        let t = tris.len();
        tris.push(make_tri(v, after, far));
        link(tris, t, 0, side_of(&vertices, v));
        link(tris, t, 1, side_of(&vertices, after));
        // the diagonal is now a side of what is left, made by this triangle's third edge
        if let Some(x) = vertices.get_mut(v) {
            x.next = far as u32;
            x.side_of = (t, 2);
        }
        if let Some(x) = vertices.get_mut(after) {
            x.next = NIL;
        }
        let gone = i.wrapping_add(1).checked_rem(n).unwrap_or(0);
        pts.remove(gone);
        coords.remove(gone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathplan::shortest::triangulate_c;
    use graphviz_pure_core::math;

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

    /// A corridor of overlapping boxes, as dot's routing makes them, in the orientation
    /// `Pshortestpath` loads: down the left side, then up the right. Repeated points are dropped,
    /// as it drops them.
    fn corridor(rng: &mut Rng) -> Vec<Ppoint> {
        let boxes = 2 + rng.below(30) as usize;
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for b in 0..boxes {
            let y = b as f64 * 10.0;
            let l = rng.below(20) as f64;
            let r = 21.0 + rng.below(20) as f64;
            left.push(Ppoint::new(l, y));
            left.push(Ppoint::new(l, y + 10.0));
            right.push(Ppoint::new(r, y));
            right.push(Ppoint::new(r, y + 10.0));
        }
        left.reverse();
        let mut poly = left;
        poly.extend(right);
        poly.dedup_by(|a, b| a.x == b.x && a.y == b.y);
        poly
    }

    /// Any closed walk, simple or not: both must fail in the same place when no ear is left.
    fn wild(rng: &mut Rng) -> Vec<Ppoint> {
        let n = 3 + rng.below(40) as usize;
        let grid = 2 + rng.below(24);
        (0..n)
            .map(|_| Ppoint::new(rng.below(grid) as f64, rng.below(grid) as f64))
            .collect()
    }

    /// A star-shaped polygon, with many ears refused for many clips, in that orientation.
    fn star(rng: &mut Rng) -> Vec<Ppoint> {
        let n = 4 + rng.below(80) as usize;
        let mut poly: Vec<Ppoint> = (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                let r = 200.0 + rng.below(600) as f64;
                Ppoint::new(
                    (r * math::cos(a) * 8.0).round() / 8.0,
                    (r * math::sin(a) * 8.0).round() / 8.0,
                )
            })
            .collect();
        poly.reverse();
        poly
    }

    fn same_as_c(points: &[Ppoint], case: &str) -> bool {
        let mut tris = Vec::new();
        let ok = triangulate(points, &mut tris);
        let (c_tris, c_ok) = triangulate_c(points);
        assert_eq!(ok, c_ok, "{case}: {points:?}");
        assert_eq!(tris, c_tris, "{case}: {points:?}");
        ok
    }

    #[test]
    fn clips_the_same_triangles_as_c() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut whole, mut failed) = (0, 0);
        for case in 0..4000 {
            for points in [corridor(&mut rng), wild(&mut rng), star(&mut rng)] {
                if same_as_c(&points, &format!("case {case}")) {
                    whole += 1;
                } else {
                    failed += 1;
                }
            }
        }
        assert!(
            whole > 4000 && failed > 1000,
            "{whole} whole, {failed} failed"
        );
    }

    #[test]
    fn standing_refusals_save_the_tests() {
        // C tests every vertex up to the first ear, after every clip; a refusal that stands is
        // not tested again. Output alone cannot show this: without it the result is C's too.
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        let (mut fast, mut c_tests) = (0u64, 0u64);
        for _ in 0..200 {
            let points = star(&mut rng);
            let mut tris = Vec::new();
            TESTED.with(|t| t.set(0));
            if !triangulate(&points, &mut tris) {
                continue;
            }
            fast += TESTED.with(std::cell::Cell::get);
            let mut left = points.clone();
            while left.len() > 3 {
                let m = left.len();
                let ear = (0..m)
                    .find(|&i| {
                        c_tests += 1;
                        crate::pathplan::triang::isdiagonal(i, (i + 2) % m, &left)
                    })
                    .unwrap();
                left.remove((ear + 1) % m);
            }
        }
        assert!(c_tests > 1000, "{c_tests}");
        assert!(
            fast * 10 < c_tests * 9,
            "{fast} tests against C's {c_tests}"
        );
    }

    #[test]
    fn a_side_refusal_names_the_side_in_its_direction() {
        let mut rng = Rng(0x7777_5555_3333_1111);
        let mut sides = 0;
        for _ in 0..200 {
            let points = wild(&mut rng);
            let n = points.len();
            for i in 0..n {
                if let Some(Refused::Side(j, jp1)) = refusal(i, (i + 2) % n, &points) {
                    assert_eq!(jp1, (j + 1) % n);
                    sides += 1;
                }
            }
        }
        assert!(sides > 0);
    }

    #[test]
    fn too_few_points_make_no_triangle() {
        for n in 0..3 {
            let points: Vec<Ppoint> = (0..n).map(|i| Ppoint::new(i as f64, 0.0)).collect();
            assert!(same_as_c(&points, "too few"));
        }
    }
}
