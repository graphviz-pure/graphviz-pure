//! Delaunay triangulation edges (`delaunay_tri` in `lib/neatogen/delaunay.c`).
//!
//! C uses GTS's incremental Delaunay triangulation inside the enclosing
//! triangle of `gts_triangle_enclosing(..., 100)`, with Shewchuk's exact
//! `incircle`. This is an incremental Bowyer–Watson triangulation inside the
//! same enclosing triangle with exact-sign predicates ([`super::robust`]),
//! which yields the same triangulation unless four points are exactly
//! co-circular; the edge enumeration order (a GTS hash table) may differ.
//! Coincident points are merged like `gts_delaunay_add_vertex`.

use super::robust::{incircle, orient2d};
use graphviz_pure_core::math;

#[derive(Clone, Copy)]
struct Tri {
    v: [usize; 3],
}

fn orient(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> f64 {
    orient2d((ax, ay), (bx, by), (cx, cy))
}

fn in_circumcircle(p: &[(f64, f64)], t: &Tri, q: (f64, f64)) -> bool {
    let (a, b, c) = (p[t.v[0]], p[t.v[1]], p[t.v[2]]);
    let det = incircle(a, b, c, q);
    let o = orient2d(a, b, c);
    if o > 0.0 { det > 0.0 } else { det < 0.0 }
}

/// `gts_triangle_enclosing(klass, points, scale)`: an equilateral triangle
/// around the bounding box of the points (C fuses the marked multiply-adds).
pub(crate) fn triangle_enclosing(pts: &[(f64, f64)], scale: f64) -> [(f64, f64); 3] {
    // GTS walks the list built with g_slist_prepend, i.e. last point first.
    let mut it = pts.iter().rev();
    let first = it.next().expect("points");
    let (mut xmax, mut xmin, mut ymax, mut ymin) = (first.0, first.0, first.1, first.1);
    for p in it {
        if p.0 > xmax {
            xmax = p.0;
        } else if p.0 < xmin {
            xmin = p.0;
        }
        if p.1 > ymax {
            ymax = p.1;
        } else if p.1 < ymin {
            ymin = p.1;
        }
    }
    const SQRT3: f64 = 1.73205080757;
    let xo = (xmax + xmin) / 2.0;
    let yo = (ymax + ymin) / 2.0;
    let dx = xmax - xo;
    let dy = ymax - yo;
    let mut r = scale * math::fma(dx, dx, dy * dy).sqrt();
    if r == 0.0 {
        r = scale;
    }
    [
        (math::fma(r, SQRT3, xo), yo - r),
        (xo, math::fma(r, 2.0, yo)),
        (math::fma(-r, SQRT3, xo), yo - r),
    ]
}

/// Edges `(i, j)` (`i < j`, sorted) of the Delaunay triangulation of the
/// points, or the chain along x (or y for a vertical line) when all points
/// are collinear, as C does when GTS returns no edges.
pub fn delaunay_tri(x: &[f64], y: &[f64]) -> Vec<(usize, usize)> {
    let edges = delaunay_edges(x, y);
    if !edges.is_empty() {
        return edges;
    }
    // Collinear: chain the points sorted along x (y for a vertical line).
    let n = x.len();
    let mut vs: Vec<usize> = (0..n).collect();
    let key: &[f64] = if n > 1 && x[0] == x[1] { y } else { x };
    crate::cutil::qsort_by(&mut vs, |a, b| {
        if key[*a] < key[*b] {
            -1
        } else if key[*a] > key[*b] {
            1
        } else {
            0
        }
    });
    vs.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Edges `(i, j)` (`i < j`, sorted) of the Delaunay triangulation, empty when
/// the points are collinear (the edges GTS reports to `delaunay_triangulation`).
pub fn delaunay_edges(x: &[f64], y: &[f64]) -> Vec<(usize, usize)> {
    let n = x.len();
    let mut pts: Vec<(f64, f64)> = (0..n).map(|i| (x[i], y[i])).collect();
    if n == 0 {
        return Vec::new();
    }
    let enc = triangle_enclosing(&pts, 100.0);
    pts.extend_from_slice(&enc);
    let mut tris = vec![Tri {
        v: [n, n + 1, n + 2],
    }];
    let mut alias: Vec<usize> = (0..n).collect();
    'outer: for i in 0..n {
        for j in 0..i {
            if alias[j] == j && pts[j] == pts[i] {
                alias[i] = j;
                continue 'outer;
            }
        }
        let q = pts[i];
        let mut bad = Vec::new();
        for (k, t) in tris.iter().enumerate() {
            if in_circumcircle(&pts, t, q) {
                bad.push(k);
            }
        }
        if bad.is_empty() {
            continue;
        }
        let mut boundary: Vec<(usize, usize)> = Vec::new();
        for &k in &bad {
            let t = tris[k];
            for e in 0..3 {
                let (a, b) = (t.v[e], t.v[(e + 1) % 3]);
                let shared = bad.iter().any(|&o| {
                    o != k && {
                        let u = tris[o].v;
                        (0..3).any(|f| {
                            (u[f] == b && u[(f + 1) % 3] == a) || (u[f] == a && u[(f + 1) % 3] == b)
                        })
                    }
                });
                if !shared {
                    boundary.push((a, b));
                }
            }
        }
        let mut keep: Vec<Tri> = tris
            .iter()
            .enumerate()
            .filter(|(k, _)| !bad.contains(k))
            .map(|(_, t)| *t)
            .collect();
        for (a, b) in boundary {
            let mut t = Tri { v: [a, b, i] };
            if orient(pts[a].0, pts[a].1, pts[b].0, pts[b].1, q.0, q.1) < 0.0 {
                t.v.swap(0, 1);
            }
            keep.push(t);
        }
        tris = keep;
    }
    let mut edges = std::collections::BTreeSet::new();
    for t in &tris {
        if t.v.iter().any(|&v| v >= n) {
            continue;
        }
        if orient(
            pts[t.v[0]].0,
            pts[t.v[0]].1,
            pts[t.v[1]].0,
            pts[t.v[1]].1,
            pts[t.v[2]].0,
            pts[t.v[2]].1,
        ) == 0.0
        {
            continue;
        }
        for e in 0..3 {
            let (a, b) = (t.v[e], t.v[(e + 1) % 3]);
            edges.insert((a.min(b), a.max(b)));
        }
    }
    edges.into_iter().collect()
}

/// `UG_graph(x, y, n)`: the relative neighbourhood graph, as adjacency lists
/// (without the vertex itself). Edge order follows [`delaunay_edges`], not
/// GTS's hash-table order.
pub fn ug_graph(x: &[f64], y: &[f64]) -> Vec<Vec<usize>> {
    let n = x.len();
    if n == 2 {
        return vec![vec![1], vec![0]];
    } else if n == 1 {
        return vec![Vec::new()];
    }
    let mut g: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (s, d) in delaunay_edges(x, y) {
        g[s].push(d);
        g[d].push(s);
    }
    for i in 0..n {
        let (xi, yi) = (x[i], y[i]);
        let mut j = 0;
        while j < g[i].len() {
            let nj = g[i][j];
            let (xj, yj) = (x[nj], y[nj]);
            let dist_ij = math::fma(xj - xi, xj - xi, (yj - yi) * (yj - yi));
            let mut removed = false;
            let mut k = 0;
            while k < g[i].len() && !removed {
                let nk = g[i][k];
                let dist_ik = math::fma(x[nk] - xi, x[nk] - xi, (y[nk] - yi) * (y[nk] - yi));
                if dist_ik < dist_ij {
                    let dist_jk = math::fma(x[nk] - xj, x[nk] - xj, (y[nk] - yj) * (y[nk] - yj));
                    if dist_jk < dist_ij {
                        g[i].swap_remove(j);
                        if let Some(p) = g[nj].iter().position(|&v| v == i) {
                            g[nj].swap_remove(p);
                        }
                        removed = true;
                    }
                }
                k += 1;
            }
            if !removed {
                j += 1;
            }
        }
    }
    g
}
