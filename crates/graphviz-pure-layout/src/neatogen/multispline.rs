//! Port of `lib/neatogen/multispline.c` (`mkRouter`, `makeMultiSpline`,
//! `freeRouter`) and the `mkSurface` half of `lib/neatogen/delaunay.c`.
//!
//! Homebrew's 16.1.0 links GTS, so `spline_edges_` routes multi-edges and
//! edges with boundary ports (`splines=true|polyline`) through a router built
//! on the constrained Delaunay triangulation of the free space between the
//! node obstacles: the shortest path of triangles between the endpoints
//! becomes a guiding polygon, `Pshortestpath` finds the polyline inside it and
//! `Proutespline` fits the spline; multi-edges get fanned control points
//! (`mkCtrlPts`).
//!
//! `mkSurface`: GTS inserts the points into an enclosing triangle
//! (`gts_delaunay_add_vertex`), then each constraint segment
//! (`gts_delaunay_add_constraint`, re-triangulating the cut polygons with
//! `triangulate_polygon`), destroys the enclosing vertices and removes faces
//! whose constraint edges run against the face orientation
//! (`delaunay_remove_holes`). Here the constrained Delaunay triangulation is
//! computed with Bowyer–Watson insertion, Sloan's edge flips for the
//! constraints and Lawson flips, on exact predicates; it is the same
//! triangulation whenever no four points are co-circular. Face and neighbour
//! numbering follow creation order rather than GTS's pointer-hash order,
//! which only matters for exact ties in the triangle shortest path.

use super::robust::{incircle, orient2d};
use crate::common::splines::{add_edge_labels, clip_and_install};
use crate::common::types::{EdgeType, LGraph, Sub};
use graphviz_pure_core::Point;
use graphviz_pure_core::math;
use graphviz_pure_geom::pathplan::visibility::{area2, wind};
use graphviz_pure_geom::pathplan::{Pedge, make_polyline, proutespline, shortest::pshortestpath};
use std::collections::{HashMap, HashSet};

// ---- mkSurface: constrained Delaunay triangulation ----

/// `surface_t`: CCW faces and up to three neighbouring faces (`-1` padded).
#[derive(Debug, Clone, Default)]
pub struct Surface {
    pub faces: Vec<[usize; 3]>,
    pub neigh: Vec<[i32; 3]>,
}

struct Cdt {
    p: Vec<(f64, f64)>,
    tris: Vec<[usize; 3]>,
    alive: Vec<bool>,
    /// Directed CCW edge `(a, b)` -> triangle.
    emap: HashMap<(usize, usize), usize>,
}

impl Cdt {
    fn orient(&self, a: usize, b: usize, c: usize) -> f64 {
        orient2d(self.p[a], self.p[b], self.p[c])
    }

    fn add_tri(&mut self, mut t: [usize; 3]) -> usize {
        if self.orient(t[0], t[1], t[2]) < 0.0 {
            t.swap(0, 1);
        }
        let k = self.tris.len();
        self.tris.push(t);
        self.alive.push(true);
        for i in 0..3 {
            self.emap.insert((t[i], t[(i + 1) % 3]), k);
        }
        k
    }

    fn kill(&mut self, k: usize) {
        self.alive[k] = false;
        let t = self.tris[k];
        for i in 0..3 {
            let key = (t[i], t[(i + 1) % 3]);
            if self.emap.get(&key) == Some(&k) {
                self.emap.remove(&key);
            }
        }
    }

    /// Strictly inside the circumcircle of the CCW triangle `k`.
    fn in_circle(&self, k: usize, q: usize) -> bool {
        let t = self.tris[k];
        incircle(self.p[t[0]], self.p[t[1]], self.p[t[2]], self.p[q]) > 0.0
    }

    /// A triangle whose closure contains point `q`, by walking.
    fn locate(&self, q: usize) -> usize {
        let mut k = (0..self.tris.len()).rev().find(|&k| self.alive[k]).unwrap();
        let mut steps = 0usize;
        'walk: loop {
            steps += 1;
            if steps > 4 * self.tris.len() + 16 {
                break;
            }
            let t = self.tris[k];
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                if self.orient(a, b, q) < 0.0
                    && let Some(&n) = self.emap.get(&(b, a))
                {
                    k = n;
                    continue 'walk;
                }
            }
            return k;
        }
        // Fallback: exhaustive search.
        (0..self.tris.len())
            .find(|&k| {
                self.alive[k] && {
                    let t = self.tris[k];
                    (0..3).all(|i| self.orient(t[i], t[(i + 1) % 3], q) >= 0.0)
                }
            })
            .unwrap_or(k)
    }

    /// Bowyer–Watson insertion of point `q`.
    fn insert(&mut self, q: usize) {
        let start = self.locate(q);
        let mut cavity = vec![start];
        let mut in_cav: HashSet<usize> = HashSet::from([start]);
        let mut i = 0;
        while i < cavity.len() {
            let k = cavity[i];
            i += 1;
            let t = self.tris[k];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                if let Some(&n) = self.emap.get(&(b, a))
                    && !in_cav.contains(&n)
                    && self.in_circle(n, q)
                {
                    in_cav.insert(n);
                    cavity.push(n);
                }
            }
        }
        let mut boundary = Vec::new();
        for &k in &cavity {
            let t = self.tris[k];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                let outside = match self.emap.get(&(b, a)) {
                    Some(n) => !in_cav.contains(n),
                    None => true,
                };
                if outside {
                    boundary.push((a, b));
                }
            }
        }
        for &k in &cavity {
            self.kill(k);
        }
        for (a, b) in boundary {
            if self.orient(a, b, q) != 0.0 {
                self.add_tri([a, b, q]);
            }
        }
    }

    fn has_edge(&self, a: usize, b: usize) -> bool {
        self.emap.contains_key(&(a, b)) || self.emap.contains_key(&(b, a))
    }

    fn crosses(&self, u: usize, v: usize, a: usize, b: usize) -> bool {
        let o1 = self.orient(a, b, u);
        let o2 = self.orient(a, b, v);
        let o3 = self.orient(u, v, a);
        let o4 = self.orient(u, v, b);
        o1 * o2 < 0.0 && o3 * o4 < 0.0
    }

    /// Flip the diagonal `(u, v)`; returns the new diagonal.
    fn flip(&mut self, u: usize, v: usize) -> Option<(usize, usize)> {
        let t1 = *self.emap.get(&(u, v))?;
        let t2 = *self.emap.get(&(v, u))?;
        let w = self.tris[t1].iter().copied().find(|&x| x != u && x != v)?;
        let z = self.tris[t2].iter().copied().find(|&x| x != u && x != v)?;
        self.kill(t1);
        self.kill(t2);
        self.add_tri([u, z, w]);
        self.add_tri([z, v, w]);
        Some((z, w))
    }

    fn convex_quad(&self, u: usize, v: usize) -> Option<(usize, usize)> {
        let t1 = *self.emap.get(&(u, v))?;
        let t2 = *self.emap.get(&(v, u))?;
        let w = self.tris[t1].iter().copied().find(|&x| x != u && x != v)?;
        let z = self.tris[t2].iter().copied().find(|&x| x != u && x != v)?;
        let o1 = self.orient(w, z, u);
        let o2 = self.orient(w, z, v);
        (o1 * o2 < 0.0).then_some((w, z))
    }

    /// Insert the constraint `a`–`b` (Sloan), then restore the constrained
    /// Delaunay property (Lawson) around the new edges.
    fn add_constraint(&mut self, a: usize, b: usize, fixed: &HashSet<(usize, usize)>) {
        if self.has_edge(a, b) {
            return;
        }
        let mut queue: std::collections::VecDeque<(usize, usize)> = self
            .emap
            .keys()
            .filter(|&&(u, v)| u < v && self.crosses(u, v, a, b))
            .copied()
            .collect();
        let mut sorted: Vec<(usize, usize)> = queue.drain(..).collect();
        sorted.sort_unstable();
        queue.extend(sorted);
        let mut new_edges = Vec::new();
        let mut guard = 0usize;
        while let Some((u, v)) = queue.pop_front() {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            if self.convex_quad(u, v).is_none() {
                queue.push_back((u, v));
                continue;
            }
            let (z, w) = self.flip(u, v).unwrap();
            if self.crosses(z, w, a, b) {
                queue.push_back((z, w));
            } else {
                new_edges.push((z, w));
            }
        }
        let is_fixed = |x: usize, y: usize| {
            fixed.contains(&(x.min(y), x.max(y))) || (x.min(y), x.max(y)) == (a.min(b), a.max(b))
        };
        let mut changed = true;
        let mut rounds = 0;
        while changed && rounds < 1000 {
            changed = false;
            rounds += 1;
            for i in 0..new_edges.len() {
                let (u, v) = new_edges[i];
                if is_fixed(u, v) {
                    continue;
                }
                let (Some(&t1), Some(&t2)) = (self.emap.get(&(u, v)), self.emap.get(&(v, u)))
                else {
                    continue;
                };
                let z = self.tris[t2]
                    .iter()
                    .copied()
                    .find(|&x| x != u && x != v)
                    .unwrap();
                if self.in_circle(t1, z) && self.convex_quad(u, v).is_some() {
                    let ne = self.flip(u, v).unwrap();
                    new_edges[i] = ne;
                    changed = true;
                }
            }
        }
    }
}

/// `mkSurface(x, y, n, segs, nsegs)`.
pub fn mk_surface(x: &[f64], y: &[f64], segs: &[(usize, usize)]) -> Option<Surface> {
    let n = x.len();
    if n == 0 {
        return None;
    }
    let mut p: Vec<(f64, f64)> = (0..n).map(|i| (x[i], y[i])).collect();
    let enc = super::delaunay::triangle_enclosing(&p, 100.0);
    p.extend_from_slice(&enc);
    let mut cdt = Cdt {
        p,
        tris: Vec::new(),
        alive: Vec::new(),
        emap: HashMap::new(),
    };
    cdt.add_tri([n, n + 1, n + 2]);
    let mut seen: HashMap<(u64, u64), usize> = HashMap::new();
    let mut alias: Vec<usize> = (0..n).collect();
    for i in 0..n {
        let key = (cdt.p[i].0.to_bits(), cdt.p[i].1.to_bits());
        if let Some(&j) = seen.get(&key) {
            alias[i] = j;
            continue;
        }
        seen.insert(key, i);
        cdt.insert(i);
    }
    let mut fixed: HashSet<(usize, usize)> = HashSet::new();
    let mut dir: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    for &(s0, s1) in segs {
        let (a, b) = (alias[s0], alias[s1]);
        if a == b {
            continue;
        }
        cdt.add_constraint(a, b, &fixed);
        fixed.insert((a.min(b), a.max(b)));
        dir.insert((a.min(b), a.max(b)), (a, b));
    }
    // destroy the enclosing triangle, then remove the holes
    let mut faces = Vec::new();
    for k in 0..cdt.tris.len() {
        if !cdt.alive[k] {
            continue;
        }
        let t = cdt.tris[k];
        if t.iter().any(|&v| v >= n) {
            continue;
        }
        let hole = (0..3).any(|i| {
            let (u, v) = (t[i], t[(i + 1) % 3]);
            matches!(dir.get(&(u.min(v), u.max(v))), Some(&(c1, _)) if c1 != u)
        });
        if !hole {
            faces.push(k);
        }
    }
    let idx: HashMap<usize, usize> = faces.iter().enumerate().map(|(i, &k)| (k, i)).collect();
    let mut sf = Surface::default();
    for &k in &faces {
        let t = cdt.tris[k];
        let mut nb = [-1i32; 3];
        let mut c = 0;
        for i in 0..3 {
            let (u, v) = (t[i], t[(i + 1) % 3]);
            if let Some(o) = cdt.emap.get(&(v, u)).and_then(|o| idx.get(o)) {
                nb[c] = *o as i32;
                c += 1;
            }
        }
        sf.faces.push(t);
        sf.neigh.push(nb);
    }
    Some(sf)
}

// ---- router ----

#[derive(Debug, Clone, Copy, Default)]
struct Ipair {
    i: i32,
    j: i32,
}

#[derive(Debug, Clone, Default)]
struct TNode {
    edges: Vec<usize>,
    ctr: Point,
}

#[derive(Debug, Clone, Copy)]
struct TEdge {
    t: usize,
    h: usize,
    seg: Ipair,
    dist: f64,
}

#[derive(Debug, Clone, Default)]
struct TGraph {
    nodes: Vec<TNode>,
    edges: Vec<TEdge>,
}

impl TGraph {
    fn add_tri_edge(&mut self, t: usize, h: usize, seg: Ipair) {
        let (tc, hc) = (self.nodes[t].ctr, self.nodes[h].ctr);
        let ne = self.edges.len();
        self.edges.push(TEdge {
            t,
            h,
            seg,
            dist: dist(tc, hc),
        });
        self.nodes[t].edges.push(ne);
        self.nodes[h].edges.push(ne);
    }
}

/// `router_t`.
pub struct MRouter {
    ps: Vec<Point>,
    /// Points of obstacle `i` are `obs[i]..obs[i+1]`.
    obs: Vec<usize>,
    /// Obstacle side `(a, b)` (`a < b`) -> adjacent triangle.
    trimap: HashMap<(i32, i32), usize>,
    tn: usize,
    tg: TGraph,
}

const MARGIN: f64 = 32.0;

/// `DIST(p, q)`: `sqrt(dx*dx + dy*dy)` (fused).
fn dist(p: Point, q: Point) -> f64 {
    let (dx, dy) = (p.x - q.x, p.y - q.y);
    dx.mul_add(dx, dy * dy).sqrt()
}

fn shared_edge(p: &[usize; 3], q: &[usize; 3]) -> Ipair {
    let mut p1 = p[0];
    let mut p2 = p[1];
    if p1 == q[0] {
        if p2 != q[1] && p2 != q[2] {
            p2 = p[2];
        }
    } else if p1 == q[1] {
        if p2 != q[0] && p2 != q[2] {
            p2 = p[2];
        }
    } else if p1 == q[2] {
        if p2 != q[0] && p2 != q[1] {
            p2 = p[2];
        }
    } else {
        p1 = p[2];
    }
    if p1 > p2 {
        std::mem::swap(&mut p1, &mut p2);
    }
    Ipair {
        i: p1 as i32,
        j: p2 as i32,
    }
}

impl MRouter {
    /// `mkRouter(obsp, npoly)`.
    pub fn new(obsp: &[Vec<Point>]) -> Option<Self> {
        let mut bb = (f64::MAX, f64::MAX, -f64::MAX, -f64::MAX);
        let mut npts = 0;
        for o in obsp {
            for p in o {
                bb.0 = bb.0.min(p.x);
                bb.2 = bb.2.max(p.x);
                bb.1 = bb.1.min(p.y);
                bb.3 = bb.3.max(p.y);
                npts += 1;
            }
        }
        bb.0 -= MARGIN;
        bb.1 -= MARGIN;
        bb.2 += MARGIN;
        bb.3 += MARGIN;
        npts += 4;
        let mut pts = Vec::with_capacity(npts);
        let mut segs = Vec::with_capacity(npts);
        // bounding box in CCW order
        pts.push(Point::new(bb.0, bb.1));
        pts.push(Point::new(bb.2, bb.1));
        pts.push(Point::new(bb.2, bb.3));
        pts.push(Point::new(bb.0, bb.3));
        for i in 1..=4 {
            segs.push((i - 1, if i < 4 { i } else { 0 }));
        }
        // obstacles (CW) and their constraint segments
        let mut obsi = Vec::with_capacity(obsp.len() + 1);
        let mut ix = 4;
        for o in obsp {
            obsi.push(ix);
            let start = ix;
            for j in 1..=o.len() {
                segs.push((ix, if j < o.len() { ix + 1 } else { start }));
                pts.push(o[j - 1]);
                ix += 1;
            }
        }
        obsi.push(ix);
        let x: Vec<f64> = pts.iter().map(|p| p.x).collect();
        let y: Vec<f64> = pts.iter().map(|p| p.y).collect();
        let sf = mk_surface(&x, &y, &segs)?;
        // mapSegToTri
        let mut trimap = HashMap::new();
        for (i, f) in sf.faces.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (f[k] as i32, f[(k + 1) % 3] as i32);
                trimap.entry((a.min(b), a.max(b))).or_insert(i);
            }
        }
        // mkTriGraph
        let nf = sf.faces.len();
        let mut tg = TGraph {
            nodes: vec![TNode::default(); nf + 2],
            edges: Vec::new(),
        };
        for (i, f) in sf.faces.iter().enumerate() {
            let (a, b, c) = (pts[f[0]], pts[f[1]], pts[f[2]]);
            tg.nodes[i].ctr = Point::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
        }
        for i in 0..nf {
            for ne in 0..3 {
                let j = sf.neigh[i][ne];
                if j == -1 {
                    break;
                }
                let j = j as usize;
                if i < j {
                    let seg = shared_edge(&sf.faces[i], &sf.faces[j]);
                    tg.add_tri_edge(i, j, seg);
                }
            }
        }
        Some(MRouter {
            ps: pts,
            obs: obsi,
            trimap,
            tn: nf,
            tg,
        })
    }

    /// `addEndpoint(rtr, p, v, v_id, sides)`; `false` where C aborts: a
    /// side of the obstacle in no triangle (`findMap`'s `assert(ip)`, e.g. the
    /// zero-length sides of a cylinder's repeated vertices) or no edge added
    /// (`assert(ne > 0)`). Routing the endpoint anyway made `mk_poly` loop.
    fn add_endpoint(&mut self, p: Point, obs_id: usize, v_id: usize, sides: u8) -> bool {
        use crate::common::ports::{BOTTOM, LEFT, RIGHT, TOP};
        let starti = self.obs[obs_id];
        let endi = self.obs[obs_id + 1];
        let add = |p: Point, d: (f64, f64)| Point::new(p.x + d.0, p.y + d.1);
        const N: (f64, f64) = (0.0, 1.0);
        const NE: (f64, f64) = (1.0, 1.0);
        const E: (f64, f64) = (1.0, 0.0);
        const SE: (f64, f64) = (1.0, -1.0);
        const S: (f64, f64) = (0.0, -1.0);
        const SW: (f64, f64) = (-1.0, -1.0);
        const W: (f64, f64) = (-1.0, 0.0);
        const NW: (f64, f64) = (-1.0, 1.0);
        let (vr, v0, v1) = match sides {
            s if s == TOP => (add(p, N), add(p, NW), add(p, NE)),
            s if s == TOP | RIGHT => (add(p, NE), add(p, N), add(p, E)),
            s if s == RIGHT => (add(p, E), add(p, NE), add(p, SE)),
            s if s == BOTTOM | RIGHT => (add(p, SE), add(p, E), add(p, S)),
            s if s == BOTTOM => (add(p, S), add(p, SE), add(p, SW)),
            s if s == BOTTOM | LEFT => (add(p, SW), add(p, S), add(p, W)),
            s if s == LEFT => (add(p, W), add(p, SW), add(p, NW)),
            s if s == TOP | LEFT => (add(p, NW), add(p, W), add(p, N)),
            _ => (p, p, p),
        };
        self.tg.nodes[v_id].edges.clear();
        self.tg.nodes[v_id].ctr = p;
        for i in starti..endi {
            let seg = Ipair {
                i: i as i32,
                j: if i < endi - 1 {
                    i as i32 + 1
                } else {
                    starti as i32
                },
            };
            let key = (seg.i.min(seg.j), seg.i.max(seg.j));
            let Some(&t) = self.trimap.get(&key) else {
                return false;
            };
            let (pi, pj) = (self.ps[seg.i as usize], self.ps[seg.j as usize]);
            if sides != 0
                && !in_cone(v0, p, v1, pi)
                && !in_cone(v0, p, v1, pj)
                && !ray_seg(p, vr, pi, pj)
            {
                continue;
            }
            self.tg.add_tri_edge(v_id, t, seg);
        }
        !self.tg.nodes[v_id].edges.is_empty()
    }

    /// `edgeToSeg(tg, i, j)`; `None` where C aborts on `assert(0)`: no
    /// triangle edge joins `i` and `j`.
    fn edge_to_seg(&self, i: usize, j: usize) -> Option<Ipair> {
        self.tg.nodes[i]
            .edges
            .iter()
            .map(|&k| &self.tg.edges[k])
            .find(|ep| ep.t == j || ep.h == j)
            .map(|ep| ep.seg)
    }

    /// `triPath(g, n, v0, v1, pq)`: `dad` from `v1` back to `v0`.
    fn tri_path(&self, n: usize, v0: usize, v1: usize) -> Option<Vec<i32>> {
        const UNSEEN: f64 = -(f32::MAX as f64);
        let mut dad = vec![0i32; n];
        let mut pq = FPq::new(n);
        for v in pq.vals.iter_mut().skip(1).take(n) {
            *v = UNSEEN;
        }
        pq.cnt = 0;
        dad[v0] = -1;
        pq.set_val(v0 as i32, 0.0);
        if !pq.insert(v0 as i32) {
            return None;
        }
        loop {
            let i = pq.remove();
            if i == -1 {
                break;
            }
            let i = i as usize;
            let vi = -pq.val(i as i32);
            pq.set_val(i as i32, vi);
            if i == v1 {
                break;
            }
            for &k in &self.tg.nodes[i].edges {
                let e = &self.tg.edges[k];
                let adjn = if e.t == i { e.h } else { e.t };
                if pq.val(adjn as i32) < 0.0 {
                    let d = -(pq.val(i as i32) + e.dist);
                    if pq.val(adjn as i32) == UNSEEN {
                        pq.set_val(adjn as i32, d);
                        dad[adjn] = i as i32;
                        if !pq.insert(adjn as i32) {
                            return None;
                        }
                    } else if pq.val(adjn as i32) < d {
                        pq.update(adjn as i32, d);
                        dad[adjn] = i as i32;
                    }
                }
            }
        }
        Some(dad)
    }
}

/// `fPQ.h` with `PQTYPE int`, `PQVTYPE double` and guard `-1` (value 0).
struct FPq {
    pq: Vec<i32>,
    cnt: usize,
    size: usize,
    /// `vals[n + 1]` holds `N_VAL(n)` (the guard -1 maps to slot 0).
    vals: Vec<f64>,
    idxs: Vec<usize>,
}

impl FPq {
    fn new(sz: usize) -> Self {
        let mut pq = vec![0i32; sz + 1];
        pq[0] = -1;
        FPq {
            pq,
            cnt: 0,
            size: sz,
            vals: vec![0.0; sz + 2],
            idxs: vec![0; sz + 2],
        }
    }
    fn val(&self, n: i32) -> f64 {
        self.vals[(n + 1) as usize]
    }
    fn set_val(&mut self, n: i32, v: f64) {
        self.vals[(n + 1) as usize] = v;
    }
    fn set_idx(&mut self, n: i32, k: usize) {
        self.idxs[(n + 1) as usize] = k;
    }
    fn upheap(&mut self, mut k: usize) {
        let x = self.pq[k];
        let v = self.val(x);
        let mut next = k / 2;
        loop {
            let n = self.pq[next];
            if self.val(n) >= v {
                break;
            }
            self.pq[k] = n;
            self.set_idx(n, k);
            k = next;
            next /= 2;
        }
        self.pq[k] = x;
        self.set_idx(x, k);
    }
    fn insert(&mut self, np: i32) -> bool {
        if self.cnt == self.size {
            return false;
        }
        self.cnt += 1;
        let c = self.cnt;
        self.pq[c] = np;
        self.upheap(c);
        true
    }
    fn downheap(&mut self, mut k: usize) {
        let x = self.pq[k];
        let v = self.val(x);
        let lim = self.cnt / 2;
        while k <= lim {
            let mut j = k + k;
            let mut n = self.pq[j];
            if j < self.cnt && self.val(n) < self.val(self.pq[j + 1]) {
                j += 1;
                n = self.pq[j];
            }
            if v >= self.val(n) {
                break;
            }
            self.pq[k] = n;
            self.set_idx(n, k);
            k = j;
        }
        self.pq[k] = x;
        self.set_idx(x, k);
    }
    fn remove(&mut self) -> i32 {
        if self.cnt > 0 {
            let n = self.pq[1];
            self.pq[1] = self.pq[self.cnt];
            self.cnt -= 1;
            if self.cnt > 0 {
                self.downheap(1);
            }
            n
        } else {
            self.pq[0]
        }
    }
    fn update(&mut self, n: i32, d: f64) {
        self.set_val(n, d);
        let k = self.idxs[(n + 1) as usize];
        self.upheap(k);
    }
}

// ---- tripoly ----

/// `tripoly_t`: the guiding polygon and, per vertex, the opposite sides of
/// the path triangles containing it (polygon indices).
struct TriPoly {
    poly: Vec<Point>,
    tri_map: Vec<Vec<(i32, i32)>>,
}

const NSMALL: f64 = -0.0000000001;

/// `inCone(a, b, c, q)`.
fn in_cone(a: Point, b: Point, c: Point, q: Point) -> bool {
    area2(q, a, b) >= NSMALL && area2(q, b, c) >= NSMALL
}

/// `raySeg(v, w, a, b)`: does the ray `v->w` meet segment `a--b`?
fn ray_seg(v: Point, w: Point, a: Point, b: Point) -> bool {
    let wa = wind(v, w, a);
    let wb = wind(v, w, b);
    if wa == wb {
        return false;
    }
    if wa == 0 {
        wind(v, b, w) * wind(v, b, a) >= 0
    } else {
        wind(v, a, w) * wind(v, a, b) >= 0
    }
}

/// `line_intersect(a, b, c, d, &p)` (`lib/common/geom.c`).
fn line_intersect(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let mv = Point::new(b.x - a.x, b.y - a.y);
    let lv = Point::new(d.x - c.x, d.y - c.y);
    let ln = Point::new(-lv.y, lv.x);
    let lc = -ln.x.mul_add(c.x, ln.y * c.y);
    let dt = ln.x.mul_add(mv.x, ln.y * mv.y);
    if dt.abs() < 0.0000000001 {
        return None;
    }
    let s = (ln.x.mul_add(a.x, ln.y * a.y) + lc) / dt;
    Some(Point::new(a.x - s * mv.x, a.y - s * mv.y))
}

/// `triPoint(trip, vx, v, w, &ip)`: where ray `v->w` leaves the polygon.
fn tri_point(trip: &TriPoly, vx: usize, v: Point, w: Point) -> Option<Point> {
    for &(i, j) in &trip.tri_map[vx] {
        let (a, b) = (trip.poly[i as usize], trip.poly[j as usize]);
        if ray_seg(v, w, a, b)
            && let Some(p) = line_intersect(v, w, a, b)
        {
            return Some(p);
        }
    }
    None
}

/// `ctrlPtIdx(v, polys)`.
fn ctrl_pt_idx(v: Point, poly: &[Point]) -> Option<usize> {
    (1..poly.len()).find(|&i| poly[i].x == v.x && poly[i].y == v.y)
}

const SEP: f64 = 15.0;

/// `mkCtrlPts(s, mult, prev, v, nxt, trip)`.
fn mk_ctrl_pts(
    s: usize,
    mult: usize,
    prev: Point,
    v: Point,
    nxt: Point,
    trip: &TriPoly,
) -> Option<Vec<Point>> {
    let idx = ctrl_pt_idx(v, &trip.poly)?;
    let mut ps = vec![Point::default(); mult];
    let theta =
        (math::atan2(nxt.y - v.y, nxt.x - v.x) + math::atan2(prev.y - v.y, prev.x - v.x)) / 2.0;
    let mut sin_theta = math::sin(theta);
    let mut cos_theta = math::cos(theta);
    let mut w = Point::new(
        100.0f64.mul_add(cos_theta, v.x),
        100.0f64.mul_add(sin_theta, v.y),
    );
    let flip = if idx > s {
        wind(prev, v, w) != 1
    } else {
        wind(prev, v, w) != -1
    };
    if flip {
        sin_theta *= -1.0;
        cos_theta *= -1.0;
        w = Point::new(
            100.0f64.mul_add(cos_theta, v.x),
            100.0f64.mul_add(sin_theta, v.y),
        );
    }
    let q = tri_point(trip, idx, v, w)?;
    let d = dist(q, v);
    let sep = if d >= mult as f64 * SEP {
        SEP
    } else {
        d / mult as f64
    };
    for i in 0..mult {
        let fi = i as f64;
        let pt = Point::new(
            (fi * sep).mul_add(cos_theta, v.x),
            (fi * sep).mul_add(sin_theta, v.y),
        );
        if idx < s {
            ps[i] = pt;
        } else {
            ps[mult - i - 1] = pt;
        }
    }
    Some(ps)
}

/// `mkPoly(rtr, dad, s, t, p_s, p_t, &sx)`; `None` where C aborts because
/// `tri_path` never reached `t` (issue #138). An unreached node's `dad` is 0, so
/// the walk either lands on node 0 as its own parent (`assert(nxt != dad[nxt])`;
/// without the check the walk never ends) or, when node 0 was reached, follows a
/// path with a step that is no triangle edge (`edgeToSeg`'s `assert(0)`). A
/// valid path visits each node once, so a longer walk also fails.
fn mk_poly(
    rtr: &MRouter,
    dad: &[i32],
    s: usize,
    t: usize,
    p_s: Point,
    p_t: Point,
) -> Option<(TriPoly, usize)> {
    struct Side {
        v: i32,
        ts: Vec<(i32, i32)>,
    }
    // tri lists are prepend-linked in C: keep them newest first
    let add = |l: &mut Vec<(i32, i32)>, i: i32, j: i32| l.insert(0, (i, j));
    let mut nt = 0usize;
    let mut nxt = dad[t];
    while nxt as usize != s {
        nt += 1;
        if nxt == dad[nxt as usize] || nt > dad.len() {
            return None;
        }
        nxt = dad[nxt as usize];
    }
    let mut side1: Vec<Side> = Vec::with_capacity(nt + 4);
    let mut side2: Vec<Side> = Vec::with_capacity(nt + 4);
    let nxt0 = dad[t] as usize;
    let p = rtr.edge_to_seg(nxt0, t)?;
    side1.push(Side {
        v: p.i,
        ts: vec![(-1, p.j)],
    });
    side2.push(Side {
        v: p.j,
        ts: vec![(-1, p.i)],
    });
    let mut t = nxt0;
    let mut nxt = dad[t];
    while nxt >= 0 {
        let p = rtr.edge_to_seg(t, nxt as usize)?;
        let (c1, c2) = (side1.len() - 1, side2.len() - 1);
        let (s1v, s2v) = (side1[c1].v, side2[c2].v);
        if p.i == s1v {
            add(&mut side1[c1].ts, s2v, p.j);
            add(&mut side2[c2].ts, s1v, p.j);
            side2.push(Side {
                v: p.j,
                ts: vec![(s2v, s1v)],
            });
        } else if p.i == s2v {
            add(&mut side1[c1].ts, s2v, p.j);
            add(&mut side2[c2].ts, s1v, p.j);
            side1.push(Side {
                v: p.j,
                ts: vec![(s2v, s1v)],
            });
        } else if p.j == s1v {
            add(&mut side1[c1].ts, s2v, p.i);
            add(&mut side2[c2].ts, s1v, p.i);
            side2.push(Side {
                v: p.i,
                ts: vec![(s2v, s1v)],
            });
        } else {
            add(&mut side1[c1].ts, s2v, p.i);
            add(&mut side2[c2].ts, s1v, p.i);
            side1.push(Side {
                v: p.i,
                ts: vec![(s2v, s1v)],
            });
        }
        t = nxt as usize;
        nxt = dad[t];
    }
    let (c1, c2) = (side1.len() - 1, side2.len() - 1);
    let (s1v, s2v) = (side1[c1].v, side2[c2].v);
    add(&mut side1[c1].ts, -2, s2v);
    add(&mut side2[c2].ts, -2, s1v);

    let cnt1 = side1.len();
    let mut vmap: HashMap<i32, i32> = HashMap::new();
    vmap.insert(-1, 0);
    vmap.insert(-2, cnt1 as i32 + 1);
    let mut pts = Vec::with_capacity(nt + 4);
    let mut trim: Vec<Vec<(i32, i32)>> = vec![Vec::new(); nt + 4];
    pts.push(p_t);
    let mut idx = 1usize;
    for sd in &side1 {
        vmap.entry(sd.v).or_insert(idx as i32);
        pts.push(rtr.ps[sd.v as usize]);
        trim[idx] = sd.ts.clone();
        idx += 1;
    }
    pts.push(p_s);
    idx += 1;
    for sd in side2.iter().rev() {
        vmap.entry(sd.v).or_insert(idx as i32);
        pts.push(rtr.ps[sd.v as usize]);
        trim[idx] = sd.ts.clone();
        idx += 1;
    }
    for l in trim.iter_mut() {
        for e in l.iter_mut() {
            *e = (vmap[&e.0], vmap[&e.1]);
        }
    }
    pts.truncate(nt + 4);
    Some((
        TriPoly {
            poly: pts,
            tri_map: trim,
        },
        cnt1 + 1,
    ))
}

fn eqpt(p: Point, q: Point) -> bool {
    p.x == q.x && p.y == q.y
}

/// `tweakEnd(poly, s, q)`.
fn tweak_end(poly: &[Point], s: usize, q: Point) -> Point {
    let mut p = poly[s];
    let nxt = poly[(s + 1) % poly.len()];
    let prv = if s == 0 {
        poly[poly.len() - 1]
    } else {
        poly[s - 1]
    };
    if eqpt(q, nxt) || eqpt(q, prv) {
        let m = Point::new((nxt.x + prv.x) / 2.0 - p.x, (nxt.y + prv.y) / 2.0 - p.y);
        let d = math::hypot(m.x, m.y);
        p.x += 0.1 * m.x / d;
        p.y += 0.1 * m.y / d;
    }
    p
}

/// `tweakPath(poly, t, pl)`.
fn tweak_path(poly: &[Point], t: usize, pl: &mut [Point]) {
    let n = pl.len();
    pl[0] = tweak_end(poly, 0, pl[1]);
    pl[n - 1] = tweak_end(poly, t, pl[n - 2]);
}

fn poly_edges(poly: &[Point]) -> Vec<Pedge> {
    (0..poly.len())
        .map(|j| Pedge {
            a: poly[j],
            b: poly[(j + 1) % poly.len()],
        })
        .collect()
}

/// `finishEdge(e, spl, flip)`.
fn finish_edge(g: &mut LGraph<'_>, e: usize, mut spl: Vec<Point>, flip: bool) {
    if flip {
        spl.reverse();
    }
    let h = g.edges[e].head;
    super::routing::verbose_edge(g, "spline", e);
    clip_and_install(g, e, h, &spl);
    add_edge_labels(g, e);
}

/// `genroute(trip, t, e, doPolyline)`: `true` on failure.
/// genroute's `agwarningf("Could not create control points ...")` and `rv = 1`
/// (multispline.c:703, :723, :740, :759, :778).
fn route_failed(g: &LGraph<'_>, e: usize) -> bool {
    // Both names share agnameof's static buffer when anonymous.
    let [t, h] = graphviz_pure_core::agnameof_buffer([
        g.node_agnameof(g.edges[e].tail),
        g.node_agnameof(g.edges[e].head),
    ]);
    graphviz_pure_core::diag::agwarningf(&format!(
        "Could not create control points for multiple spline for edge ({t},{h})\n"
    ));
    true
}

fn genroute(
    g: &mut LGraph<'_>,
    sub: &Sub,
    trip: &TriPoly,
    t: usize,
    mut e: usize,
    do_polyline: bool,
) -> bool {
    let head = g.edges[e].head;
    let mult = g.edges[e].count.max(0) as usize;
    let eps = [trip.poly[0], trip.poly[t]];
    let dump = std::env::var_os("GVDUMP").is_some();
    if dump {
        let mut l = format!(
            "MP {} {} t={} mult={} poly",
            g.src.nodes[g.edges[e].tail].name, g.src.nodes[g.edges[e].head].name, t, mult
        );
        for p in &trip.poly {
            l.push_str(&format!(" {:?},{:?}", p.x, p.y));
        }
        graphviz_pure_core::diag::dump(&l);
    }
    let Ok(mut pl) = pshortestpath(&trip.poly, eps) else {
        if dump {
            graphviz_pure_core::diag::dump("MFAIL sp1");
        }
        return route_failed(g, e);
    };
    if dump {
        let mut l = String::from("MPL");
        for p in &pl {
            l.push_str(&format!(" {:?},{:?}", p.x, p.y));
        }
        graphviz_pure_core::diag::dump(&l);
    }
    if pl.len() == 2 {
        super::neatosplines::make_straight_edge(g, sub, e, EdgeType::None);
        return false;
    }
    if mult == 1 || g.concentrate {
        let poly = &trip.poly;
        let medges = poly_edges(poly);
        tweak_path(poly, t, &mut pl);
        let Ok(spl) = proutespline(&medges, &pl, [Point::default(), Point::default()]) else {
            return route_failed(g, e);
        };
        finish_edge(g, e, spl, false);
        return false;
    }
    let pn = 2 * (pl.len() - 1);
    let mut cpts = Vec::with_capacity(pl.len() - 2);
    for i in 0..pl.len() - 2 {
        match mk_ctrl_pts(t, mult + 1, pl[i], pl[i + 1], pl[i + 2], trip) {
            Some(c) => cpts.push(c),
            None => return route_failed(g, e),
        }
    }
    let mut poly = vec![Point::default(); pn];
    for i in 0..mult {
        poly[0] = eps[0];
        for j in 1..pl.len() - 1 {
            poly[j] = cpts[j - 1][i];
        }
        poly[pl.len() - 1] = eps[1];
        for j in 1..pl.len() - 1 {
            poly[pn - j] = cpts[j - 1][i + 1];
        }
        let Ok(mut mmpl) = pshortestpath(&poly, eps) else {
            return route_failed(g, e);
        };
        let spl = if do_polyline {
            make_polyline(&mmpl)
        } else {
            let medges = poly_edges(&poly);
            tweak_path(&poly, pl.len() - 1, &mut mmpl);
            match proutespline(&medges, &mmpl, [Point::default(), Point::default()]) {
                Ok(s) => s,
                Err(_) => return route_failed(g, e),
            }
        };
        let flip = g.edges[e].head != head;
        finish_edge(g, e, spl, flip);
        match g.edges[e].to_virt {
            Some(n) => e = n,
            None => break,
        }
    }
    false
}

/// `makeMultiSpline(e, rtr, doPolyline)`: `true` on failure.
pub fn make_multi_spline(
    g: &mut LGraph<'_>,
    sub: &Sub,
    e: usize,
    rtr: &mut MRouter,
    do_polyline: bool,
) -> bool {
    let line = g.edges[e].path.clone();
    if line.is_empty() {
        return true;
    }
    let (t, h) = (g.edges[e].tail, g.edges[e].head);
    let (Ok(tl), Ok(hl)) = (
        usize::try_from(g.nodes[t].lim),
        usize::try_from(g.nodes[h].lim),
    ) else {
        return true;
    };
    let t_p = line[0];
    let h_p = line[line.len() - 1];
    let t_id = rtr.tn;
    let h_id = rtr.tn + 1;
    let ecnt = rtr.tg.edges.len();
    let orig: Vec<usize> = rtr.tg.nodes.iter().map(|n| n.edges.len()).collect();
    let (ts, hs) = (g.edges[e].tail_port.side, g.edges[e].head_port.side);
    let added = rtr.add_endpoint(t_p, tl, t_id, ts) && rtr.add_endpoint(h_p, hl, h_id, hs);
    let sp = if added {
        rtr.tri_path(rtr.tn + 2, h_id, t_id)
    } else {
        None
    };
    let ret = match sp.and_then(|sp| mk_poly(rtr, &sp, h_id, t_id, h_p, t_p)) {
        Some((poly, idx)) => genroute(g, sub, &poly, idx, e, do_polyline),
        None => true,
    };
    // resetGraph
    rtr.tg.edges.truncate(ecnt);
    for (i, n) in rtr.tg.nodes.iter_mut().enumerate().take(rtr.tn) {
        n.edges.truncate(orig[i]);
    }
    ret
}
