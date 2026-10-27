//! Port of `lib/sparse/QuadTree.c`: the quadtree (octree in 3D) used by
//! sfdp for Barnes–Hut approximation of the repulsive forces.
//!
//! Cells live in an arena; `qts` holds child indices. A cell's point list
//! (`l` in C, a linked list with the newest entry first) is a `Vec` in
//! insertion order that is walked backwards.

use super::{MACHINEACC, distance_cropped};
use graphviz_pure_core::math;

/// `node_data_struct`.
#[derive(Debug, Clone)]
struct NodeData {
    node_weight: f64,
    coord: Vec<f64>,
    id: usize,
}

/// `QuadTree_struct`.
#[derive(Debug, Clone)]
struct Cell {
    n: i32,
    total_weight: f64,
    center: Vec<f64>,
    width: f64,
    average: Vec<f64>,
    qts: Option<Vec<Option<usize>>>,
    l: Vec<NodeData>,
    /// `data`: the force accumulated on the cell (allocated lazily in C).
    force: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct QuadTree {
    dim: usize,
    max_level: i32,
    cells: Vec<Cell>,
}

/// `point_distance(p1, p2, dim)`.
pub fn point_distance(p1: &[f64], p2: &[f64], dim: usize) -> f64 {
    let mut d = 0.0f64;
    for i in 0..dim {
        let t = p1[i] - p2[i];
        d = t.mul_add(t, d);
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
    pub fn len(&self) -> usize {
        self.wgts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wgts.is_empty()
    }
}

impl QuadTree {
    fn new_cell(&mut self, center: &[f64], width: f64) -> usize {
        debug_assert!(width > 0.0);
        self.cells.push(Cell {
            n: 0,
            total_weight: 0.0,
            center: center.to_vec(),
            width,
            average: Vec::new(),
            qts: None,
            l: Vec::new(),
            force: vec![0.0; self.dim],
        });
        self.cells.len() - 1
    }

    /// `QuadTree_new(dim, center, width, max_level)`.
    pub fn new(dim: usize, center: &[f64], width: f64, max_level: i32) -> Self {
        let mut q = QuadTree {
            dim,
            max_level,
            cells: Vec::new(),
        };
        q.new_cell(center, width);
        q
    }

    /// `QuadTree_new_from_point_list(dim, n, max_level, coord)`.
    pub fn new_from_point_list(dim: usize, n: usize, max_level: i32, coord: &[f64]) -> Self {
        let mut xmin = coord[..dim].to_vec();
        let mut xmax = coord[..dim].to_vec();
        for i in 1..n {
            for k in 0..dim {
                xmin[k] = xmin[k].min(coord[i * dim + k]);
                xmax[k] = xmax[k].max(coord[i * dim + k]);
            }
        }
        let mut width = xmax[0] - xmin[0];
        let mut center = vec![0.0; dim];
        for i in 0..dim {
            center[i] = (xmin[i] + xmax[i]) * 0.5;
            width = width.max(xmax[i] - xmin[i]);
        }
        width = width.max(0.00001);
        width *= 0.52;
        let mut qt = QuadTree::new(dim, &center, width, max_level);
        for i in 0..n {
            qt.add(&coord[i * dim..i * dim + dim], 1.0, i);
        }
        qt
    }

    /// `QuadTree_get_quadrant(dim, center, coord)`.
    fn get_quadrant(dim: usize, center: &[f64], coord: &[f64]) -> usize {
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

    /// `QuadTree_new_in_quadrant(dim, center, width, max_level, i)`.
    fn new_in_quadrant(&mut self, center: &[f64], width: f64, mut i: usize) -> usize {
        let mut c = center.to_vec();
        for ck in c.iter_mut() {
            if i.is_multiple_of(2) {
                *ck -= width;
            } else {
                *ck += width;
            }
            i = (i - i % 2) / 2;
        }
        self.new_cell(&c, width)
    }

    /// `QuadTree_add_internal(q, coord, weight, id, level)`.
    fn add_internal(&mut self, q: usize, coord: &[f64], weight: f64, id: usize, level: i32) {
        let dim = self.dim;
        // C checks that `coord` is inside the box but only reports it.
        let _ = MACHINEACC;
        if self.cells[q].n == 0 {
            let c = &mut self.cells[q];
            c.n = 1;
            c.total_weight = weight;
            c.average = coord[..dim].to_vec();
            c.l.push(NodeData {
                node_weight: weight,
                coord: coord[..dim].to_vec(),
                id,
            });
        } else if level < self.max_level {
            {
                let c = &mut self.cells[q];
                c.total_weight += weight;
                let n = f64::from(c.n);
                for i in 0..dim {
                    c.average[i] = c.average[i].mul_add(n, coord[i]) / (n + 1.0);
                }
                if c.qts.is_none() {
                    c.qts = Some(vec![None; 1 << dim]);
                }
            }
            let center = self.cells[q].center.clone();
            let half = self.cells[q].width / 2.0;
            let ii = Self::get_quadrant(dim, &center, coord);
            let child = match self.cells[q].qts.as_ref().unwrap()[ii] {
                Some(c) => c,
                None => {
                    let c = self.new_in_quadrant(&center, half, ii);
                    self.cells[q].qts.as_mut().unwrap()[ii] = Some(c);
                    c
                }
            };
            self.add_internal(child, coord, weight, id, level + 1);
            if !self.cells[q].l.is_empty() {
                debug_assert_eq!(self.cells[q].n, 1);
                let old = self.cells[q].l.last().unwrap().clone();
                let ii = Self::get_quadrant(dim, &center, &old.coord);
                let child = match self.cells[q].qts.as_ref().unwrap()[ii] {
                    Some(c) => c,
                    None => {
                        let c = self.new_in_quadrant(&center, half, ii);
                        self.cells[q].qts.as_mut().unwrap()[ii] = Some(c);
                        c
                    }
                };
                self.add_internal(child, &old.coord, old.node_weight, old.id, level + 1);
                self.cells[q].l.clear();
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
                c.average[i] = c.average[i].mul_add(n, coord[i]) / (n + 1.0);
            }
            c.l.push(NodeData {
                node_weight: weight,
                coord: coord[..dim].to_vec(),
                id,
            });
        }
    }

    /// `QuadTree_add(q, coord, weight, id)`.
    pub fn add(&mut self, coord: &[f64], weight: f64, id: usize) {
        self.add_internal(0, coord, weight, id, 0);
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
        for l in c.l.iter().rev() {
            if l.id != nodeid {
                out.center.extend_from_slice(&l.coord);
                out.wgts.push(l.node_weight);
                out.distances.push(point_distance(pt, &l.coord, dim));
            }
        }
        if let Some(qts) = &c.qts {
            let dist = point_distance(&c.center, pt, dim);
            if c.width < bh * dist {
                out.center.extend_from_slice(&c.average);
                out.wgts.push(c.total_weight);
                out.distances.push(point_distance(&c.average, pt, dim));
            } else {
                for &child in qts.iter() {
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
        let dist = point_distance(&self.cells[q1].average, &self.cells[q2].average, dim);
        if self.cells[q1].width + self.cells[q2].width < bh * dist {
            counts[0] += 1.0;
            let w1 = self.cells[q1].total_weight;
            let w2 = self.cells[q2].total_weight;
            let denom = if p == -1.0 {
                dist * dist
            } else {
                math::powf(dist, 1.0 - p)
            };
            for k in 0..dim {
                let d = self.cells[q1].average[k] - self.cells[q2].average[k];
                let f = w1 * w2 * kp * d / denom;
                self.cells[q1].force[k] += f;
                self.cells[q2].force[k] -= f;
            }
            return;
        }
        let l1_some = !self.cells[q1].l.is_empty();
        let l2_some = !self.cells[q2].l.is_empty();
        if l1_some && l2_some {
            let n1 = self.cells[q1].l.len();
            let n2 = self.cells[q2].l.len();
            for a in (0..n1).rev() {
                let (wgt1, i1) = {
                    let l = &self.cells[q1].l[a];
                    (l.node_weight, l.id)
                };
                for b in (0..n2).rev() {
                    let (wgt2, i2) = {
                        let l = &self.cells[q2].l[b];
                        (l.node_weight, l.id)
                    };
                    if (q1 == q2 && i2 < i1) || i1 == i2 {
                        continue;
                    }
                    counts[1] += 1.0;
                    let dist = distance_cropped(x, dim, i1, i2);
                    let denom = if p == -1.0 {
                        dist * dist
                    } else {
                        math::powf(dist, 1.0 - p)
                    };
                    for k in 0..dim {
                        let d = self.cells[q1].l[a].coord[k] - self.cells[q2].l[b].coord[k];
                        let f = wgt1 * wgt2 * kp * d / denom;
                        force[i1 * dim + k] += f;
                        force[i2 * dim + k] -= f;
                    }
                }
            }
            return;
        }
        let nq = 1usize << dim;
        if q1 == q2 {
            let qts = self.cells[q1].qts.clone().unwrap();
            for i in 0..nq {
                for j in i..nq {
                    self.repulsive_force_interact(qts[i], qts[j], x, force, bh, p, kp, counts);
                }
            }
        } else {
            let (w1, w2) = (self.cells[q1].width, self.cells[q2].width);
            if w1 > w2 && !l1_some {
                let qts = self.cells[q1].qts.clone().unwrap();
                for &c in &qts {
                    self.repulsive_force_interact(c, Some(q2), x, force, bh, p, kp, counts);
                }
            } else if w2 > w1 && !l2_some {
                let qts = self.cells[q2].qts.clone().unwrap();
                for &c in &qts {
                    self.repulsive_force_interact(c, Some(q1), x, force, bh, p, kp, counts);
                }
            } else if !l1_some {
                let qts = self.cells[q1].qts.clone().unwrap();
                for &c in &qts {
                    self.repulsive_force_interact(c, Some(q2), x, force, bh, p, kp, counts);
                }
            } else if !l2_some {
                let qts = self.cells[q2].qts.clone().unwrap();
                for &c in &qts {
                    self.repulsive_force_interact(c, Some(q1), x, force, bh, p, kp, counts);
                }
            } else {
                unreachable!("both cells at the leaf level");
            }
        }
    }

    fn repulsive_force_accumulate(&mut self, q: usize, force: &mut [f64], counts: &mut [f64; 4]) {
        let dim = self.dim;
        let wgt = self.cells[q].total_weight;
        counts[2] += 1.0;
        if !self.cells[q].l.is_empty() {
            let c = &self.cells[q];
            for l in c.l.iter().rev() {
                let wgt2 = l.node_weight / wgt;
                for k in 0..dim {
                    let t = &mut force[l.id * dim + k];
                    *t = wgt2.mul_add(c.force[k], *t);
                }
            }
            return;
        }
        let Some(qts) = self.cells[q].qts.clone() else {
            return;
        };
        for c in qts.into_iter().flatten() {
            let wgt2 = self.cells[c].total_weight / wgt;
            for k in 0..dim {
                let fk = self.cells[q].force[k];
                let t = &mut self.cells[c].force[k];
                *t = wgt2.mul_add(fk, *t);
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
