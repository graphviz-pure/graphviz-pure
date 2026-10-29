//! Port of `lib/sparse/QuadTree.c`: the quadtree (octree in 3D) used by
//! sfdp for Barnes–Hut approximation of the repulsive forces.
//!
//! Cells live in an arena, and their `dim`-long vectors (`center`,
//! `average`, `force`) and the points' coordinates in flat arenas indexed by
//! cell or point, so building a tree allocates nothing per cell or point.
//! `rebuild_from_point_list` reuses the arenas of the previous tree. A
//! cell's children (`qts` in C) are `1 << dim` consecutive slots of
//! `children`. A cell's point list (`l` in C, a linked list with the newest
//! entry first) is a list linked through `points`, also newest first.

use super::{MACHINEACC, distance_cropped};
use graphviz_pure_core::math;
use std::iter;
use std::ops::Range;

/// `node_data_struct`; its coordinates are `coords[p * dim..][..dim]`.
#[derive(Debug, Clone)]
struct NodeData {
    node_weight: f64,
    id: usize,
    /// The next (older) entry of the cell's point list.
    next: Option<usize>,
}

/// `QuadTree_struct`; its vectors are `center`, `average` and `force`
/// `[q * dim..][..dim]`.
#[derive(Debug, Clone)]
struct Cell {
    n: i32,
    total_weight: f64,
    width: f64,
    /// The first of the cell's `1 << dim` slots in `children`.
    qts: Option<usize>,
    /// The newest entry of the point list.
    l: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct QuadTree {
    dim: usize,
    /// `1 << dim`, the number of children of a cell.
    nq: usize,
    max_level: i32,
    cells: Vec<Cell>,
    center: Vec<f64>,
    average: Vec<f64>,
    /// `data`: the force accumulated on each cell (allocated lazily in C).
    force: Vec<f64>,
    children: Vec<Option<usize>>,
    points: Vec<NodeData>,
    coords: Vec<f64>,
    /// `xmin` and `xmax` of `rebuild_from_point_list`.
    bounds: Vec<f64>,
}
/// `point_distance(p1, p2, dim)`.
pub fn point_distance(p1: &[f64], p2: &[f64], dim: usize) -> f64 {
    let mut d = 0.0f64;
    for i in 0..dim {
        let t = p1[i] - p2[i];
        d = math::fma(t, t, d);
    }
    d.sqrt()
}

/// Output buffers of `QuadTree_get_supernodes`.
#[derive(Debug, Default, Clone)]
pub struct Supernodes {
    pub center: Vec<f64>,
    pub wgts: Vec<f64>,
    pub distances: Vec<f64>,
}

impl Supernodes {
    /// C: lib/sparse/QuadTree.c:QuadTree_get_supernodes_internal
    /// The fixed-size 2D append lowers to stores rather than a dynamic copy call.
    fn push_center(&mut self, coordinates: &[f64]) {
        if let &[x, y] = coordinates {
            self.center.extend([x, y]);
        } else {
            for &coordinate in coordinates {
                self.center.push(coordinate);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.wgts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wgts.is_empty()
    }
}

/// The entry `i` of a flat arena of `dim`-long vectors.
fn span(i: usize, dim: usize) -> Range<usize> {
    let start = i * dim;
    start..start + dim
}

impl QuadTree {
    /// Appends a cell whose center the caller has just pushed onto `center`.
    fn push_cell<const DIM: usize>(&mut self, width: f64) -> usize {
        debug_assert!(width > 0.0);
        let dim = if DIM == 0 { self.dim } else { DIM };
        let q = self.cells.len();
        self.cells.push(Cell {
            n: 0,
            total_weight: 0.0,
            width,
            qts: None,
            l: None,
        });
        self.average.extend(iter::repeat_n(0.0, dim));
        self.force.extend(iter::repeat_n(0.0, dim));
        q
    }

    /// `QuadTree_new(dim, center, width, max_level)`.
    pub fn new(dim: usize, center: &[f64], width: f64, max_level: i32) -> Self {
        let mut q = QuadTree {
            dim,
            nq: 1 << dim,
            max_level,
            ..QuadTree::default()
        };
        q.center.extend_from_slice(&center[..dim]);
        q.push_cell::<0>(width);
        q
    }

    /// `QuadTree_new_from_point_list(dim, n, max_level, coord)`.
    pub fn new_from_point_list(dim: usize, n: usize, max_level: i32, coord: &[f64]) -> Self {
        let mut qt = QuadTree::default();
        qt.rebuild_from_point_list(dim, n, max_level, coord);
        qt
    }

    /// `QuadTree_new_from_point_list(dim, n, max_level, coord)` into this
    /// tree's arenas, replacing the tree.
    pub fn rebuild_from_point_list(&mut self, dim: usize, n: usize, max_level: i32, coord: &[f64]) {
        if dim == 2 {
            self.rebuild_dim::<2>(dim, n, max_level, coord);
        } else {
            self.rebuild_dim::<0>(dim, n, max_level, coord);
        }
    }

    /// C: lib/sparse/QuadTree.c:QuadTree_new_from_point_list
    /// DIM=0 retains arbitrary dimensions; DIM=2 makes the ordinary cell spans fixed.
    fn rebuild_dim<const DIM: usize>(
        &mut self,
        dim: usize,
        n: usize,
        max_level: i32,
        coord: &[f64],
    ) {
        let dim = if DIM == 0 { dim } else { DIM };
        self.dim = dim;
        self.nq = 1 << dim;
        self.max_level = max_level;
        self.cells.clear();
        self.center.clear();
        self.average.clear();
        self.force.clear();
        self.children.clear();
        self.points.clear();
        self.coords.clear();
        self.bounds.clear();
        self.bounds.extend_from_slice(&coord[..dim]);
        self.bounds.extend_from_slice(&coord[..dim]);
        let (xmin, xmax) = self.bounds.split_at_mut(dim);
        for i in 1..n {
            let pt = &coord[span(i, dim)];
            for k in 0..dim {
                xmin[k] = xmin[k].min(pt[k]);
                xmax[k] = xmax[k].max(pt[k]);
            }
        }
        let mut width = xmax[0] - xmin[0];
        for i in 0..dim {
            self.center.push((xmin[i] + xmax[i]) * 0.5);
            width = width.max(xmax[i] - xmin[i]);
        }
        width = width.max(0.00001);
        width *= 0.52;
        self.push_cell::<DIM>(width);
        for i in 0..n {
            self.add_dim::<DIM>(&coord[span(i, dim)], 1.0, i);
        }
    }

    /// `QuadTree_get_quadrant(dim, center, coord)` for the point `p` in the
    /// cell `q`.
    fn get_quadrant<const DIM: usize>(&self, q: usize, p: usize) -> usize {
        let dim = if DIM == 0 { self.dim } else { DIM };
        let center = &self.center[span(q, dim)];
        let coord = &self.coords[span(p, dim)];
        let mut d = 0usize;
        for i in (0..dim).rev() {
            if coord[i] - center[i] < 0.0 {
                d *= 2;
            } else {
                d = 2 * d + 1;
            }
        }
        d
    }

    /// `QuadTree_new_in_quadrant(dim, center, width, max_level, i)` with the
    /// center of the cell `q`.
    fn new_in_quadrant<const DIM: usize>(&mut self, q: usize, width: f64, mut i: usize) -> usize {
        let dim = if DIM == 0 { self.dim } else { DIM };
        for k in span(q, dim) {
            let c = self.center[k];
            self.center.push(if i.is_multiple_of(2) {
                c - width
            } else {
                c + width
            });
            i = (i - i % 2) / 2;
        }
        self.push_cell::<DIM>(width)
    }

    /// The child of the cell `q`, whose children start at `first`, in the
    /// quadrant of the point `p`, created with width `half` if missing.
    fn child_for<const DIM: usize>(
        &mut self,
        q: usize,
        first: usize,
        p: usize,
        half: f64,
    ) -> usize {
        let ii = self.get_quadrant::<DIM>(q, p);
        match self.children[first..][ii] {
            Some(c) => c,
            None => {
                let c = self.new_in_quadrant::<DIM>(q, half, ii);
                self.children[first..][ii] = Some(c);
                c
            }
        }
    }

    /// `QuadTree_add_internal(q, coord, weight, id, level)` for the point `p`
    /// of the arena.
    fn add_internal<const DIM: usize>(&mut self, q: usize, p: usize, level: i32) {
        let dim = if DIM == 0 { self.dim } else { DIM };
        // C checks that `coord` is inside the box but only reports it.
        let _ = MACHINEACC;
        let weight = self.points[p].node_weight;
        let coord = &self.coords[span(p, dim)];
        let average = &mut self.average[span(q, dim)];
        if self.cells[q].n == 0 {
            let c = &mut self.cells[q];
            c.n = 1;
            c.total_weight = weight;
            average.copy_from_slice(coord);
            self.points[p].next = None;
            c.l = Some(p);
        } else if level < self.max_level {
            let c = &mut self.cells[q];
            c.total_weight += weight;
            let n = f64::from(c.n);
            for i in 0..dim {
                average[i] = math::fma(average[i], n, coord[i]) / (n + 1.0);
            }
            let first = match c.qts {
                Some(first) => first,
                None => {
                    let first = self.children.len();
                    self.children.extend(iter::repeat_n(None, self.nq));
                    c.qts = Some(first);
                    first
                }
            };
            let half = c.width / 2.0;
            let child = self.child_for::<DIM>(q, first, p, half);
            self.add_internal::<DIM>(child, p, level + 1);
            if let Some(old) = self.cells[q].l.take() {
                debug_assert_eq!(self.cells[q].n, 1);
                let child = self.child_for::<DIM>(q, first, old, half);
                self.add_internal::<DIM>(child, old, level + 1);
            }
            self.cells[q].n += 1;
        } else {
            let c = &mut self.cells[q];
            debug_assert!(c.qts.is_none());
            c.n += 1;
            c.total_weight += weight;
            // C increments n before averaging with n + 1.
            let n = f64::from(c.n);
            for i in 0..dim {
                average[i] = math::fma(average[i], n, coord[i]) / (n + 1.0);
            }
            self.points[p].next = c.l;
            c.l = Some(p);
        }
    }

    /// `QuadTree_add(q, coord, weight, id)`.
    pub fn add(&mut self, coord: &[f64], weight: f64, id: usize) {
        if self.dim == 2 {
            self.add_dim::<2>(coord, weight, id);
        } else {
            self.add_dim::<0>(coord, weight, id);
        }
    }

    /// C: lib/sparse/QuadTree.c:QuadTree_add
    fn add_dim<const DIM: usize>(&mut self, coord: &[f64], weight: f64, id: usize) {
        let dim = if DIM == 0 { self.dim } else { DIM };
        let p = self.points.len();
        self.coords.extend_from_slice(&coord[..dim]);
        self.points.push(NodeData {
            node_weight: weight,
            id,
            next: None,
        });
        self.add_internal::<DIM>(0, p, 0);
    }

    /// The children of a cell whose first child slot is `first`.
    fn children(&self, first: usize) -> &[Option<usize>] {
        &self.children[first..][..self.nq]
    }

    fn get_supernodes_internal(
        &self,
        qt: Option<usize>,
        bh: f64,
        pt: &[f64],
        nodeid: usize,
        out: &mut Supernodes,
        counts: &mut f64,
    ) {
        *counts += 1.0;
        let Some(q) = qt else { return };
        let dim = self.dim;
        let c = &self.cells[q];
        let mut l = c.l;
        while let Some(p) = l {
            let node = &self.points[p];
            if node.id != nodeid {
                let coord = &self.coords[span(p, dim)];
                out.push_center(coord);
                out.wgts.push(node.node_weight);
                out.distances.push(point_distance(pt, coord, dim));
            }
            l = node.next;
        }
        if let Some(first) = c.qts {
            let dist = point_distance(&self.center[span(q, dim)], pt, dim);
            if c.width < bh * dist {
                let average = &self.average[span(q, dim)];
                out.push_center(average);
                out.wgts.push(c.total_weight);
                out.distances.push(point_distance(average, pt, dim));
            } else {
                for &child in self.children(first) {
                    self.get_supernodes_internal(child, bh, pt, nodeid, out, counts);
                }
            }
        }
    }

    /// `QuadTree_get_supernodes(qt, bh, pt, nodeid, ...)`; returns `counts`.
    pub fn get_supernodes(&self, bh: f64, pt: &[f64], nodeid: usize, out: &mut Supernodes) -> f64 {
        let mut counts = 0.0;
        out.center.clear();
        out.wgts.clear();
        out.distances.clear();
        // Accepted cells cover disjoint point sets, so the point count bounds
        // the number of supernodes even when no cell is accepted. Reserve
        // after clear: these capacities then suffice for every query and are
        // retained across rebuilt trees and embedding iterations.
        out.center.reserve(self.coords.len());
        out.wgts.reserve(self.points.len());
        out.distances.reserve(self.points.len());
        self.get_supernodes_internal(Some(0), bh, pt, nodeid, out, &mut counts);
        counts
    }

    fn repulsive_force_interact(
        &mut self,
        qt1: Option<usize>,
        qt2: Option<usize>,
        x: &[f64],
        force: &mut [f64],
        bh: f64,
        p: f64,
        kp: f64,
        counts: &mut [f64; 4],
    ) {
        let (Some(q1), Some(q2)) = (qt1, qt2) else {
            return;
        };
        let dim = self.dim;
        let (s1, s2) = (span(q1, dim), span(q2, dim));
        let dist = point_distance(&self.average[s1.clone()], &self.average[s2.clone()], dim);
        if self.cells[q1].width + self.cells[q2].width < bh * dist {
            counts[0] += 1.0;
            let w1 = self.cells[q1].total_weight;
            let w2 = self.cells[q2].total_weight;
            let denom = if p == -1.0 {
                dist * dist
            } else {
                math::powf(dist, 1.0 - p)
            };
            for (k1, k2) in s1.zip(s2) {
                let d = self.average[k1] - self.average[k2];
                let f = w1 * w2 * kp * d / denom;
                self.force[k1] += f;
                self.force[k2] -= f;
            }
            return;
        }
        let (l1, l2) = (self.cells[q1].l, self.cells[q2].l);
        if let (Some(head1), Some(head2)) = (l1, l2) {
            let mut e1 = Some(head1);
            while let Some(pa) = e1 {
                let NodeData {
                    node_weight: wgt1,
                    id: i1,
                    next,
                } = self.points[pa];
                e1 = next;
                let mut e2 = Some(head2);
                while let Some(pb) = e2 {
                    let NodeData {
                        node_weight: wgt2,
                        id: i2,
                        next,
                    } = self.points[pb];
                    e2 = next;
                    if (q1 == q2 && i2 < i1) || i1 == i2 {
                        continue;
                    }
                    counts[1] += 1.0;
                    let dist = distance_cropped(x, dim, i1, i2);
                    let (c1, c2) = (&self.coords[span(pa, dim)], &self.coords[span(pb, dim)]);
                    let denom = if p == -1.0 {
                        dist * dist
                    } else {
                        math::powf(dist, 1.0 - p)
                    };
                    for k in 0..dim {
                        let d = c1[k] - c2[k];
                        let f = wgt1 * wgt2 * kp * d / denom;
                        force[i1 * dim + k] += f;
                        force[i2 * dim + k] -= f;
                    }
                }
            }
            return;
        }
        let (l1_some, l2_some) = (l1.is_some(), l2.is_some());
        let nq = self.nq;
        // The children's slots do not change while the forces are computed.
        if q1 == q2 {
            let Some(first) = self.cells[q1].qts else {
                return;
            };
            for i in 0..nq {
                for j in i..nq {
                    let (c1, c2) = (self.children(first)[i], self.children(first)[j]);
                    self.repulsive_force_interact(c1, c2, x, force, bh, p, kp, counts);
                }
            }
        } else {
            let (w1, w2) = (self.cells[q1].width, self.cells[q2].width);
            let (parent, other) = if w1 > w2 && !l1_some {
                (q1, q2)
            } else if w2 > w1 && !l2_some {
                (q2, q1)
            } else if !l1_some {
                (q1, q2)
            } else if !l2_some {
                (q2, q1)
            } else {
                unreachable!("both cells at the leaf level");
            };
            let Some(first) = self.cells[parent].qts else {
                return;
            };
            for i in 0..nq {
                let c = self.children(first)[i];
                self.repulsive_force_interact(c, Some(other), x, force, bh, p, kp, counts);
            }
        }
    }

    fn repulsive_force_accumulate(&mut self, q: usize, force: &mut [f64], counts: &mut [f64; 4]) {
        let dim = self.dim;
        let wgt = self.cells[q].total_weight;
        counts[2] += 1.0;
        if let Some(head) = self.cells[q].l {
            let cell_force = &self.force[span(q, dim)];
            let mut l = Some(head);
            while let Some(p) = l {
                let node = &self.points[p];
                let wgt2 = node.node_weight / wgt;
                for k in 0..dim {
                    let t = &mut force[node.id * dim + k];
                    *t = math::fma(wgt2, cell_force[k], *t);
                }
                l = node.next;
            }
            return;
        }
        let Some(first) = self.cells[q].qts else {
            return;
        };
        for i in 0..self.nq {
            let Some(c) = self.children(first)[i] else {
                continue;
            };
            let wgt2 = self.cells[c].total_weight / wgt;
            for (kq, kc) in span(q, dim).zip(span(c, dim)) {
                let fk = self.force[kq];
                let t = &mut self.force[kc];
                *t = math::fma(wgt2, fk, *t);
            }
            self.repulsive_force_accumulate(c, force, counts);
        }
    }

    /// `QuadTree_get_repulsive_force(qt, force, x, bh, p, KP, counts)`.
    pub fn get_repulsive_force(
        &mut self,
        force: &mut [f64],
        x: &[f64],
        bh: f64,
        p: f64,
        kp: f64,
        counts: &mut [f64; 4],
    ) {
        let n = self.cells[0].n;
        let dim = self.dim;
        *counts = [0.0; 4];
        for f in force.iter_mut().take(dim * n as usize) {
            *f = 0.0;
        }
        self.repulsive_force_interact(Some(0), Some(0), x, force, bh, p, kp, counts);
        self.repulsive_force_accumulate(0, force, counts);
        for c in counts.iter_mut() {
            *c /= f64::from(n);
        }
    }
}

#[cfg(test)]
mod supernode_tests {
    use super::*;
    fn reference(
        tree: &QuadTree,
        qt: Option<usize>,
        bh: f64,
        pt: &[f64],
        nodeid: usize,
        out: &mut Supernodes,
        counts: &mut f64,
    ) {
        *counts += 1.0;
        let Some(q) = qt else { return };
        let dim = tree.dim;
        let c = &tree.cells[q];
        let mut l = c.l;
        while let Some(p) = l {
            let node = &tree.points[p];
            if node.id != nodeid {
                let coord = &tree.coords[span(p, dim)];
                out.center.extend_from_slice(coord);
                out.wgts.push(node.node_weight);
                out.distances.push(point_distance(pt, coord, dim));
            }
            l = node.next;
        }
        if let Some(first) = c.qts {
            let dist = point_distance(&tree.center[span(q, dim)], pt, dim);
            if c.width < bh * dist {
                let average = &tree.average[span(q, dim)];
                out.center.extend_from_slice(average);
                out.wgts.push(c.total_weight);
                out.distances.push(point_distance(average, pt, dim));
            } else {
                for &child in tree.children(first) {
                    reference(tree, child, bh, pt, nodeid, out, counts);
                }
            }
        }
    }

    #[test]
    fn scalar_centers_match_slice_collector_without_regrowth() {
        for dim in [2, 3] {
            let n = 31;
            let x: Vec<f64> = (0..n * dim).map(|i| ((i * 17) % 41) as f64).collect();
            let mut tree = QuadTree::default();
            tree.rebuild_from_point_list(dim, n, 10, &x);
            let mut actual = Supernodes::default();
            tree.get_supernodes(0.0, &x[..dim], 0, &mut actual);
            let capacity = (
                actual.center.capacity(),
                actual.wgts.capacity(),
                actual.distances.capacity(),
            );
            for bh in [0.0, 0.6, 100.0] {
                for nodeid in 0..n {
                    let pt = &x[nodeid * dim..][..dim];
                    let mut expected = Supernodes::default();
                    let mut counts = 0.0;
                    reference(&tree, Some(0), bh, pt, nodeid, &mut expected, &mut counts);
                    assert_eq!(
                        tree.get_supernodes(bh, pt, nodeid, &mut actual).to_bits(),
                        counts.to_bits()
                    );
                    for (a, b) in [
                        (&actual.center, &expected.center),
                        (&actual.wgts, &expected.wgts),
                        (&actual.distances, &expected.distances),
                    ] {
                        assert_eq!(
                            a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                            b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                        );
                    }
                    assert_eq!(
                        (
                            actual.center.capacity(),
                            actual.wgts.capacity(),
                            actual.distances.capacity()
                        ),
                        capacity
                    );
                }
            }
        }
    }
}

// Test-only pre-specialization build methods. Apart from the reference_ prefix,
// these are the old builder; they do not invoke the specialized build chain.
#[cfg(test)]
impl QuadTree {
    fn reference_push_cell(&mut self, width: f64) -> usize {
        debug_assert!(width > 0.0);
        let q = self.cells.len();
        self.cells.push(Cell {
            n: 0,
            total_weight: 0.0,
            width,
            qts: None,
            l: None,
        });
        self.average.extend(iter::repeat_n(0.0, self.dim));
        self.force.extend(iter::repeat_n(0.0, self.dim));
        q
    }

    fn reference_rebuild_from_point_list(
        &mut self,
        dim: usize,
        n: usize,
        max_level: i32,
        coord: &[f64],
    ) {
        self.dim = dim;
        self.nq = 1 << dim;
        self.max_level = max_level;
        self.cells.clear();
        self.center.clear();
        self.average.clear();
        self.force.clear();
        self.children.clear();
        self.points.clear();
        self.coords.clear();
        self.bounds.clear();
        self.bounds.extend_from_slice(&coord[..dim]);
        self.bounds.extend_from_slice(&coord[..dim]);
        let (xmin, xmax) = self.bounds.split_at_mut(dim);
        for i in 1..n {
            let pt = &coord[span(i, dim)];
            for k in 0..dim {
                xmin[k] = xmin[k].min(pt[k]);
                xmax[k] = xmax[k].max(pt[k]);
            }
        }
        let mut width = xmax[0] - xmin[0];
        for i in 0..dim {
            self.center.push((xmin[i] + xmax[i]) * 0.5);
            width = width.max(xmax[i] - xmin[i]);
        }
        width = width.max(0.00001);
        width *= 0.52;
        self.reference_push_cell(width);
        for i in 0..n {
            self.reference_add(&coord[span(i, dim)], 1.0, i);
        }
    }

    /// `QuadTree_get_quadrant(dim, center, coord)` for the point `p` in the
    /// cell `q`.
    fn reference_get_quadrant(&self, q: usize, p: usize) -> usize {
        let dim = self.dim;
        let center = &self.center[span(q, dim)];
        let coord = &self.coords[span(p, dim)];
        let mut d = 0usize;
        for i in (0..dim).rev() {
            if coord[i] - center[i] < 0.0 {
                d *= 2;
            } else {
                d = 2 * d + 1;
            }
        }
        d
    }

    /// `QuadTree_new_in_quadrant(dim, center, width, max_level, i)` with the
    /// center of the cell `q`.
    fn reference_new_in_quadrant(&mut self, q: usize, width: f64, mut i: usize) -> usize {
        for k in span(q, self.dim) {
            let c = self.center[k];
            self.center.push(if i.is_multiple_of(2) {
                c - width
            } else {
                c + width
            });
            i = (i - i % 2) / 2;
        }
        self.reference_push_cell(width)
    }

    /// The child of the cell `q`, whose children start at `first`, in the
    /// quadrant of the point `p`, created with width `half` if missing.
    fn reference_child_for(&mut self, q: usize, first: usize, p: usize, half: f64) -> usize {
        let ii = self.reference_get_quadrant(q, p);
        match self.children[first..][ii] {
            Some(c) => c,
            None => {
                let c = self.reference_new_in_quadrant(q, half, ii);
                self.children[first..][ii] = Some(c);
                c
            }
        }
    }

    /// `QuadTree_add_internal(q, coord, weight, id, level)` for the point `p`
    /// of the arena.
    fn reference_add_internal(&mut self, q: usize, p: usize, level: i32) {
        let dim = self.dim;
        // C checks that `coord` is inside the box but only reports it.
        let _ = MACHINEACC;
        let weight = self.points[p].node_weight;
        let coord = &self.coords[span(p, dim)];
        let average = &mut self.average[span(q, dim)];
        if self.cells[q].n == 0 {
            let c = &mut self.cells[q];
            c.n = 1;
            c.total_weight = weight;
            average.copy_from_slice(coord);
            self.points[p].next = None;
            c.l = Some(p);
        } else if level < self.max_level {
            let c = &mut self.cells[q];
            c.total_weight += weight;
            let n = f64::from(c.n);
            for i in 0..dim {
                average[i] = math::fma(average[i], n, coord[i]) / (n + 1.0);
            }
            let first = match c.qts {
                Some(first) => first,
                None => {
                    let first = self.children.len();
                    self.children.extend(iter::repeat_n(None, self.nq));
                    c.qts = Some(first);
                    first
                }
            };
            let half = c.width / 2.0;
            let child = self.reference_child_for(q, first, p, half);
            self.reference_add_internal(child, p, level + 1);
            if let Some(old) = self.cells[q].l.take() {
                debug_assert_eq!(self.cells[q].n, 1);
                let child = self.reference_child_for(q, first, old, half);
                self.reference_add_internal(child, old, level + 1);
            }
            self.cells[q].n += 1;
        } else {
            let c = &mut self.cells[q];
            debug_assert!(c.qts.is_none());
            c.n += 1;
            c.total_weight += weight;
            // C increments n before averaging with n + 1.
            let n = f64::from(c.n);
            for i in 0..dim {
                average[i] = math::fma(average[i], n, coord[i]) / (n + 1.0);
            }
            self.points[p].next = c.l;
            c.l = Some(p);
        }
    }

    /// `QuadTree_add(q, coord, weight, id)`.
    fn reference_add(&mut self, coord: &[f64], weight: f64, id: usize) {
        let p = self.points.len();
        self.coords.extend_from_slice(&coord[..self.dim]);
        self.points.push(NodeData {
            node_weight: weight,
            id,
            next: None,
        });
        self.reference_add_internal(0, p, 0);
    }
}

#[cfg(test)]
mod build_specialization_tests {
    use super::*;

    fn same_arenas(a: &QuadTree, b: &QuadTree) {
        assert_eq!((a.dim, a.nq, a.max_level), (b.dim, b.nq, b.max_level));
        assert_eq!(a.children, b.children);
        assert_eq!(
            a.cells
                .iter()
                .map(|c| (c.n, c.total_weight.to_bits(), c.width.to_bits(), c.qts, c.l))
                .collect::<Vec<_>>(),
            b.cells
                .iter()
                .map(|c| (c.n, c.total_weight.to_bits(), c.width.to_bits(), c.qts, c.l))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            a.points
                .iter()
                .map(|p| (p.node_weight.to_bits(), p.id, p.next))
                .collect::<Vec<_>>(),
            b.points
                .iter()
                .map(|p| (p.node_weight.to_bits(), p.id, p.next))
                .collect::<Vec<_>>()
        );
        for (a, b) in [
            (&a.center, &b.center),
            (&a.average, &b.average),
            (&a.force, &b.force),
            (&a.coords, &b.coords),
            (&a.bounds, &b.bounds),
        ] {
            assert_eq!(
                a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fixed_build_matches_old_builder_field_for_field_and_reuses_arenas() {
        let mut seed = 0x539a_3b71_f481_220du64;
        let mut actual = QuadTree::default();
        let mut expected = QuadTree::default();
        for dim in [2, 1, 3, 2] {
            for n in [1, 67, 250] {
                for depth in [0, 1, 4, 17] {
                    for scale in [0.0, 1e-150, 1.0, 1e150] {
                        let coords: Vec<_> = (0..n * dim)
                            .map(|_| {
                                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                                (((seed >> 32) % 1024) as f64 / 512.0 - 1.0) * scale
                            })
                            .collect();
                        actual.rebuild_from_point_list(dim, n, depth, &coords);
                        expected.reference_rebuild_from_point_list(dim, n, depth, &coords);
                        same_arenas(&actual, &expected);
                    }
                }
            }
        }
    }

    #[test]
    fn fixed_weighted_insertion_keeps_point_relocation_and_leaf_list_order() {
        for dim in [1, 2, 3] {
            for depth in [0, 1, 10] {
                let mut actual = QuadTree::new(dim, &vec![0.0; dim], 2.0, depth);
                let mut expected = QuadTree::new(dim, &vec![0.0; dim], 2.0, depth);
                for id in 0..67 {
                    let pt: Vec<_> = (0..dim)
                        .map(|k| {
                            if id % 3 == 0 {
                                -0.0
                            } else {
                                ((id * 17 + k * 7) % 29) as f64 / 15.0 - 1.0
                            }
                        })
                        .collect();
                    let weight = (id % 5 + 1) as f64 * 0.25;
                    actual.add(&pt, weight, id);
                    expected.reference_add(&pt, weight, id);
                    same_arenas(&actual, &expected);
                }
            }
        }
    }
}
