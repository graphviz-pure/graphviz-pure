//! Smart initialisation for stress majorization (`start=self`, `mode=hier`):
//! `sparse_stress_subspace_majorization_kD` (`lib/neatogen/stress.c`),
//! `embed_graph`/`center_coordinate` (`embed_graph.c`), `PCA_alloc` and
//! `iterativePCA_1D` (`pca.c`), `closest_pairs2graph` (`closest.c`) and
//! `power_iteration` (`matrix_ops.c`). Uses libc `rand()` like C.

use super::bfs::{DistType, bfs, ngdijkstra};
use super::conjgrad::conjugate_gradient_f;
use super::graphdata::VtxData;
use super::matrix_ops::*;
use super::stress::{TOLERANCE_CG, with_new_weights};
use super::{NeatoParams, SolverError};
use crate::cutil::{RAND_MAX, qsort_by};
use graphviz_pure_core::math;

/// `embed_graph(graph, n, dim, &coords, reweight_graph)`: distances to
/// `dim` pivots chosen by max-min.
fn embed_graph(graph: &[VtxData], n: usize, dim: usize, reweight: bool) -> Vec<Vec<DistType>> {
    let g2;
    let gr = if reweight {
        g2 = with_new_weights(graph, n);
        &g2[..]
    } else {
        graph
    };
    let mut coords = vec![vec![0 as DistType; n]; dim];
    let mut dist = vec![0 as DistType; n];
    let mut node = (crate::cutil::rand() as usize) % n;
    if reweight {
        ngdijkstra(node, gr, n, &mut coords[0]);
    } else {
        bfs(node, gr, n, &mut coords[0]);
    }
    let mut max_dist = 0;
    for i in 0..n {
        dist[i] = coords[0][i];
        if dist[i] > max_dist {
            node = i;
            max_dist = dist[i];
        }
    }
    for i in 1..dim {
        if reweight {
            ngdijkstra(node, gr, n, &mut coords[i]);
        } else {
            bfs(node, gr, n, &mut coords[i]);
        }
        max_dist = 0;
        for j in 0..n {
            dist[j] = dist[j].min(coords[i][j]);
            if dist[j] > max_dist {
                node = j;
                max_dist = dist[j];
            }
        }
    }
    coords
}

/// `center_coordinate(coords, n, dim)`.
fn center_coordinate(coords: &mut [Vec<DistType>], n: usize) {
    for c in coords.iter_mut() {
        let mut sum = 0.0f64;
        for &v in c.iter().take(n) {
            sum += f64::from(v);
        }
        let avg = sum / n as f64;
        for v in c.iter_mut().take(n) {
            *v -= avg as DistType;
        }
    }
}

/// `power_iteration(square_mat, n, neigs, eigs)`; false if it did not converge.
fn power_iteration(m: &[Vec<f64>], n: usize, neigs: usize, eigs: &mut [Vec<f64>]) -> bool {
    let neigs = neigs.min(n);
    let mut tmp = vec![0.0f64; n];
    let mut last = vec![0.0f64; n];
    let max_iterations = 30 * n as i64;
    let tol = 1.0 - 1e-3;
    let mut evals = vec![0.0f64; neigs];
    let mut iteration = 0i64;
    let mut i = 0;
    'outer: while i < neigs {
        loop {
            for j in 0..n {
                eigs[i][j] = f64::from(crate::cutil::rand() % 100);
            }
            for j in 0..i {
                let alpha = -vectors_inner_product(n, &eigs[j], &eigs[i]);
                let (a, b) = eigs.split_at_mut(i);
                scadd(&mut b[0], n - 1, alpha, &a[j]);
            }
            let len = norm(&eigs[i], n - 1);
            if len >= 1e-10 {
                for v in eigs[i].iter_mut().take(n) {
                    *v *= 1.0 / len;
                }
                break;
            }
        }
        iteration = 0;
        loop {
            iteration += 1;
            last.copy_from_slice(&eigs[i][..n]);
            right_mult_with_vector_d(m, n, n, &eigs[i], &mut tmp);
            eigs[i][..n].copy_from_slice(&tmp);
            for j in 0..i {
                let alpha = -vectors_inner_product(n, &eigs[j], &eigs[i]);
                let (a, b) = eigs.split_at_mut(i);
                scadd(&mut b[0], n - 1, alpha, &a[j]);
            }
            let len = norm(&eigs[i], n - 1);
            if len < 1e-10 || iteration > max_iterations {
                break 'outer;
            }
            for v in eigs[i].iter_mut().take(n) {
                *v *= 1.0 / len;
            }
            let angle = vectors_inner_product(n, &eigs[i], &last);
            if angle.abs() >= tol {
                evals[i] = angle * len;
                break;
            }
        }
        i += 1;
    }
    while i < neigs {
        for j in 0..n {
            eigs[i][j] = f64::from(crate::cutil::rand() % 100);
        }
        for j in 0..i {
            let alpha = -vectors_inner_product(n, &eigs[j], &eigs[i]);
            let (a, b) = eigs.split_at_mut(i);
            scadd(&mut b[0], n - 1, alpha, &a[j]);
        }
        let len = norm(&eigs[i], n - 1);
        for v in eigs[i].iter_mut().take(n) {
            *v *= 1.0 / len;
        }
        evals[i] = 0.0;
        i += 1;
    }
    for i in 0..neigs.saturating_sub(1) {
        let mut largest_index = i;
        let mut largest_eval = evals[i];
        for j in (i + 1)..neigs {
            if largest_eval < evals[j] {
                largest_index = j;
                largest_eval = evals[j];
            }
        }
        if largest_index != i {
            eigs.swap(i, largest_index);
            evals[largest_index] = evals[i];
            evals[i] = largest_eval;
        }
    }
    iteration <= max_iterations
}

/// `PCA_alloc(coords, dim, n, new_coords, new_dim)`.
fn pca_alloc(
    coords: &[Vec<DistType>],
    dim: usize,
    n: usize,
    new_coords: &mut [Vec<f64>],
    new_dim: usize,
) {
    let mut eigs = vec![vec![0.0f64; dim]; new_dim];
    let mut dd = vec![vec![0.0f64; dim]; dim];
    for i in 0..dim {
        for j in 0..=i {
            let mut sum = 0.0f64;
            for k in 0..n {
                sum += f64::from(coords[i][k].wrapping_mul(coords[j][k]));
            }
            dd[i][j] = sum;
            dd[j][i] = sum;
        }
    }
    power_iteration(&dd, dim, new_dim, &mut eigs);
    for j in 0..new_dim {
        for i in 0..n {
            let mut sum = 0.0f64;
            for k in 0..dim {
                sum = math::fma(f64::from(coords[k][i]), eigs[j][k], sum);
            }
            new_coords[j][i] = sum;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Pair {
    left: usize,
    right: usize,
    dist: f64,
}

struct PairHeap {
    data: Vec<Pair>,
    size: usize,
}

impl PairHeap {
    fn greater(&self, i: usize, j: usize) -> bool {
        self.data[i].dist < self.data[j].dist
            || (self.data[i].dist == self.data[j].dist && crate::cutil::rand() % 2 != 0)
    }

    fn heapify(&mut self, mut i: usize) {
        loop {
            let (l, r) = (2 * i, 2 * i + 1);
            let mut largest = if l < self.size && self.greater(l, i) {
                l
            } else {
                i
            };
            if r < self.size && self.greater(r, largest) {
                largest = r;
            }
            if largest == i {
                break;
            }
            self.data.swap(largest, i);
            i = largest;
        }
    }

    fn extract_max(&mut self) -> Option<Pair> {
        if self.size == 0 {
            return None;
        }
        let m = self.data[0];
        self.data[0] = self.data[self.size - 1];
        self.size -= 1;
        self.heapify(0);
        Some(m)
    }

    fn insert(&mut self, e: Pair) {
        let mut i = self.size;
        if self.data.len() <= i {
            self.data.push(e);
        } else {
            self.data[i] = e;
        }
        self.size += 1;
        while i > 0 && self.greater(i, i / 2) {
            self.data.swap(i, i / 2);
            i /= 2;
        }
    }
}

/// `closest_pairs2graph(place, n, num_pairs, &graph)`.
fn closest_pairs2graph(place: &[f64], n: usize, num_pairs: usize) -> Vec<VtxData> {
    let mut ordering: Vec<usize> = (0..n).collect();
    qsort_by(&mut ordering, |a, b| {
        if place[*a] < place[*b] {
            -1
        } else if place[*a] > place[*b] {
            1
        } else {
            0
        }
    });
    let mut inv = vec![0usize; n];
    for (i, &o) in ordering.iter().enumerate() {
        inv[o] = i;
    }
    let mut heap = PairHeap {
        data: Vec::new(),
        size: n.saturating_sub(1),
    };
    for i in 0..n.saturating_sub(1) {
        heap.data.push(Pair {
            left: ordering[i],
            right: ordering[i + 1],
            dist: place[ordering[i + 1]] - place[ordering[i]],
        });
    }
    if n != 0 {
        let mut j = (n - 1) / 2;
        loop {
            heap.heapify(j);
            if j == 0 {
                break;
            }
            j -= 1;
        }
    }
    let mut left = vec![0usize; n];
    let mut right = vec![0usize; n];
    for i in 1..n {
        left[ordering[i]] = ordering[i - 1];
    }
    for i in 0..n.saturating_sub(1) {
        right[ordering[i]] = ordering[i + 1];
    }
    let mut stack: Vec<Pair> = Vec::new();
    for _ in 0..num_pairs {
        let Some(pair) = heap.extract_max() else {
            break;
        };
        stack.push(pair);
        let li = inv[pair.left];
        let ri = inv[pair.right];
        if li > 0 {
            let nb = ordering[li - 1];
            if inv[right[nb]] < ri {
                heap.insert(Pair {
                    left: nb,
                    right: pair.right,
                    dist: place[pair.right] - place[nb],
                });
                right[nb] = pair.right;
                left[pair.right] = nb;
            }
        }
        if ri < n - 1 {
            let nb = ordering[ri + 1];
            if inv[left[nb]] > li {
                heap.insert(Pair {
                    left: pair.left,
                    right: nb,
                    dist: place[nb] - place[pair.left],
                });
                left[nb] = pair.left;
                right[pair.left] = nb;
            }
        }
    }
    // construct_graph: self loop first, edges popped from the stack.
    let mut g: Vec<VtxData> = (0..n)
        .map(|i| VtxData {
            edges: vec![i as i32],
            ewgts: Some(vec![0.0]),
            eweights: None,
            edists: None,
        })
        .collect();
    while let Some(p) = stack.pop() {
        let (u, v) = (p.left, p.right);
        if g[u].edges.contains(&(v as i32)) {
            continue;
        }
        g[u].edges.push(v as i32);
        g[u].ewgts.as_mut().unwrap().push(1.0);
        g[v].edges.push(u as i32);
        g[v].ewgts.as_mut().unwrap().push(1.0);
        g[u].ewgts.as_mut().unwrap()[0] -= 1.0;
        g[v].ewgts.as_mut().unwrap()[0] -= 1.0;
    }
    g
}

/// `iterativePCA_1D(coords, dim, n, new_direction)`.
fn iterative_pca_1d(
    coords: &[Vec<f64>],
    dim: usize,
    n: usize,
    new_direction: &mut Vec<f64>,
) -> bool {
    let lap = closest_pairs2graph(&coords[0], n, 4 * n);
    // mult_sparse_dense_mat_transpose(lap, coords, n, dim) -> n x dim (float)
    let mut mat1 = vec![vec![0.0f32; dim]; n];
    for i in 0..n {
        let w = lap[i].ewgts.as_ref().unwrap();
        for j in 0..dim {
            let mut sum = 0.0f64;
            for (k, &e) in lap[i].edges.iter().enumerate() {
                sum = math::fma(f64::from(w[k]), coords[j][e as usize], sum);
            }
            mat1[i][j] = sum as f32;
        }
    }
    // mult_dense_mat_d(coords, mat1, dim, n, dim) -> dim x dim
    let mut mat = vec![vec![0.0f64; dim]; dim];
    for i in 0..dim {
        for j in 0..dim {
            let mut sum = 0.0f64;
            for k in 0..n {
                sum = math::fma(coords[i][k], f64::from(mat1[k][j]), sum);
            }
            mat[i][j] = sum;
        }
    }
    let mut eigs = vec![std::mem::take(new_direction)];
    let r = power_iteration(&mat, dim, 1, &mut eigs);
    *new_direction = eigs.pop().unwrap();
    r
}

fn distance_kd(coords: &[Vec<f64>], dim: usize, i: usize, j: usize) -> f64 {
    let mut sum = 0.0f64;
    for c in coords.iter().take(dim) {
        let t = c[i] - c[j];
        sum = math::fma(t, t, sum);
    }
    sum.sqrt()
}

struct DistData {
    edges: Vec<usize>,
    edist: Vec<DistType>,
}

fn compute_stress1(coords: &[Vec<f64>], d: &[DistData], dim: usize, n: usize, exp: i32) -> f64 {
    let mut sum = 0.0f64;
    for i in 0..n {
        for (j, &node) in d[i].edges.iter().enumerate() {
            if node <= i {
                continue;
            }
            let dist = distance_kd(coords, dim, i, node);
            let dij = f64::from(d[i].edist[j]);
            sum += if exp == 2 {
                (dij - dist) * (dij - dist) / (dij * dij)
            } else {
                (dij - dist) * (dij - dist) / dij
            };
        }
    }
    sum
}

/// `sparse_stress_subspace_majorization_kD(...)`.
#[allow(clippy::too_many_arguments)]
pub fn sparse_stress_subspace_majorization_kd(
    params: &NeatoParams,
    graph: &[VtxData],
    n: usize,
    coords: &mut [Vec<f64>],
    dim: usize,
    smart_ini: bool,
    exp: i32,
    reweight_graph: bool,
    n_iterations: usize,
    num_centers: usize,
) -> Result<(), SolverError> {
    let subspace_dim = 50.min(n);
    let num_centers_local = n.min((2 * subspace_dim).max(50));
    let mut full = embed_graph(graph, n, num_centers_local, reweight_graph);
    center_coordinate(&mut full, n);
    let mut subspace = vec![vec![0.0f64; n]; subspace_dim];
    pca_alloc(&full, num_centers_local, n, &mut subspace, subspace_dim);

    let g2;
    let gr = if reweight_graph {
        g2 = with_new_weights(graph, n);
        &g2[..]
    } else {
        graph
    };
    let mut center_index = vec![-1i64; n];
    let mut inv_center = vec![0usize; num_centers];
    let mut dij = vec![vec![0 as DistType; n]; num_centers];
    let mut dist = vec![0 as DistType; n];
    if num_centers > 0 {
        let mut node = (crate::cutil::rand() as usize) % n;
        center_index[node] = 0;
        inv_center[0] = node;
        if reweight_graph {
            ngdijkstra(node, gr, n, &mut dij[0]);
        } else {
            bfs(node, gr, n, &mut dij[0]);
        }
        let mut max_dist = 0;
        for i in 0..n {
            dist[i] = dij[0][i];
            if dist[i] > max_dist {
                node = i;
                max_dist = dist[i];
            }
        }
        for i in 1..num_centers {
            center_index[node] = i as i64;
            inv_center[i] = node;
            if reweight_graph {
                ngdijkstra(node, gr, n, &mut dij[i]);
            } else {
                bfs(node, gr, n, &mut dij[i]);
            }
            max_dist = 0;
            for j in 0..n {
                dist[j] = dist[j].min(dij[i][j]);
                if dist[j] > max_dist
                    || (dist[j] == max_dist
                        && (crate::cutil::rand() as usize).is_multiple_of(j + 1))
                {
                    node = j;
                    max_dist = dist[j];
                }
            }
        }
    }
    let mut distances: Vec<DistData> = Vec::with_capacity(n);
    for i in 0..n {
        if center_index[i] >= 0 {
            let idx = center_index[i] as usize;
            let mut e = Vec::with_capacity(n - 1);
            let mut d = Vec::with_capacity(n - 1);
            for j in (0..i).chain(i + 1..n) {
                e.push(j);
                d.push(dij[idx][j]);
            }
            distances.push(DistData { edges: e, edist: d });
        } else {
            let e: Vec<usize> = inv_center.clone();
            let d: Vec<DistType> = (0..num_centers).map(|k| dij[k][i]).collect();
            distances.push(DistData { edges: e, edist: d });
        }
    }
    // Laplacian
    let mut lap: Vec<VtxData> = Vec::with_capacity(n);
    for (i, dd) in distances.iter().enumerate() {
        let mut edges = vec![i as i32];
        let mut ewgts = vec![0.0f32];
        let mut degree = 0.0f64;
        for (j, &e) in dd.edges.iter().enumerate() {
            edges.push(e as i32);
            let dl = dd.edist[j] as f32;
            let w = if exp == 2 {
                -1.0f32 / (dl * dl)
            } else {
                -1.0f32 / dl
            };
            ewgts.push(w);
            degree -= f64::from(w);
        }
        ewgts[0] = degree as f32;
        lap.push(VtxData {
            edges,
            ewgts: Some(ewgts),
            eweights: None,
            edists: None,
        });
    }
    // directions
    let mut directions = vec![vec![0.0f64; subspace_dim]; dim];
    if smart_ini {
        if dim != 2 {
            for (k, d) in directions.iter_mut().enumerate() {
                if k < subspace_dim {
                    d[k] = 1.0;
                }
            }
        } else {
            directions[0][0] = 1.0;
            let mut d1 = vec![0.0f64; subspace_dim];
            if !iterative_pca_1d(&subspace, subspace_dim, n, &mut d1) {
                d1 = vec![0.0; subspace_dim];
                d1[1] = 1.0;
            }
            directions[1] = d1;
        }
    } else {
        for d in directions.iter_mut() {
            for v in d.iter_mut() {
                *v = f64::from(crate::cutil::rand()) / f64::from(RAND_MAX);
            }
        }
    }
    for k in 0..dim {
        right_mult_with_vector_transpose(
            &subspace,
            n,
            subspace_dim,
            &directions[k],
            &mut coords[k],
        );
    }
    // restriction of the Laplacian to the subspace
    let mut tmp_mat = vec![vec![0.0f32; subspace_dim]; n];
    for i in 0..n {
        let w = lap[i].ewgts.as_ref().unwrap();
        for j in 0..subspace_dim {
            let mut sum = 0.0f64;
            for (k, &e) in lap[i].edges.iter().enumerate() {
                sum = math::fma(f64::from(w[k]), subspace[j][e as usize], sum);
            }
            tmp_mat[i][j] = sum as f32;
        }
    }
    let mut matrix = vec![vec![0.0f32; subspace_dim]; subspace_dim];
    for i in 0..subspace_dim {
        for j in 0..subspace_dim {
            let mut sum = 0.0f64;
            for k in 0..n {
                sum = math::fma(subspace[i][k], f64::from(tmp_mat[k][j]), sum);
            }
            matrix[i][j] = sum as f32;
        }
    }
    let mut b = vec![0.0f64; n];
    let mut b_restricted = vec![0.0f64; subspace_dim];
    let mut old_stress = compute_stress1(coords, &distances, dim, n, exp);
    let mut converged = false;
    let mut iterations = 0;
    while iterations < n_iterations && !converged {
        for k in 0..dim {
            for i in 0..n {
                let mut degree = 0.0f64;
                b[i] = 0.0;
                let w = lap[i].ewgts.as_ref().unwrap();
                for j in 1..lap[i].nedges() {
                    let node = lap[i].edges[j] as usize;
                    let dist_ij = distance_kd(coords, dim, i, node);
                    if dist_ij > 1e-30 {
                        // -ewgts[j] * dist_list[j] is a float product.
                        let l_ij = f64::from(-w[j] * distances[i].edist[j - 1] as f32) / dist_ij;
                        degree -= l_ij;
                        b[i] = math::fma(l_ij, coords[k][node], b[i]);
                    }
                }
                b[i] = math::fma(degree, coords[k][i], b[i]);
            }
            right_mult_with_vector_d(&subspace, subspace_dim, n, &b, &mut b_restricted);
            conjugate_gradient_f(
                &matrix,
                &mut directions[k],
                &b_restricted,
                subspace_dim,
                TOLERANCE_CG,
                subspace_dim,
                false,
            )?;
            right_mult_with_vector_transpose(
                &subspace,
                n,
                subspace_dim,
                &directions[k],
                &mut coords[k],
            );
        }
        if iterations % 2 == 0 {
            let new_stress = compute_stress1(coords, &distances, dim, n, exp);
            converged = (new_stress - old_stress).abs() / (new_stress + 1e-10) < params.epsilon;
            old_stress = new_stress;
        }
        iterations += 1;
    }
    Ok(())
}
