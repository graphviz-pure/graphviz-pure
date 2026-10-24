//! DiG-CoLa (`mode=hier`): hierarchy-constrained stress majorization.
//!
//! - `opt_arrangement.c`: `construct_b`, `compute_y_coords`
//! - `compute_hierarchy.c`: `compute_hierarchy`
//! - `smart_ini_x.c`: `IMDS_given_dim` and its classical-MDS helpers
//! - `quad_prog_solve.c`: `initConstrainedMajorization`,
//!   `ensureMonotonicOrderingWithGaps`, `constrained_majorization_new_with_gaps`
//! - `constrained_majorization.c`: `stress_majorization_with_hierarchy`

use super::NeatoParams;
use super::bfs::{DistType, bfs, ngdijkstra};
use super::conjgrad::{conjugate_gradient, conjugate_gradient_f, conjugate_gradient_mkernel};
use super::graphdata::VtxData;
use super::matrix_ops::*;
use super::quad_prog_vpsc::unpack_matrix;
use super::stress::*;
use super::{MODEL_CIRCUIT, MODEL_MDS, MODEL_SUBSET, SolverError};
use crate::common::types::LGraph;
use crate::cutil::qsort_by;
use graphviz_pure_core::cnum::c_max;

const LOCAL_CONSTR_MAJOR_ITERATIONS: i32 = 15;
const LEVELS_SEP_TOL: f64 = 1e-1;
const HIERARCHY_CG_TOL: f64 = 1e-3;
const RANGE: i32 = 500;
const SCALE_FACTOR: DistType = 256;

/// `init_vec_orth1(n, vec)`: `rand() % 500`, centred.
fn init_vec_orth1(n: usize, vec: &mut [f64]) {
    for v in vec.iter_mut().take(n) {
        *v = (crate::cutil::rand() % RANGE) as f64;
    }
    orthog1(&mut vec[..n]);
}

/// `construct_b(graph, n, b)`: the balance vector `sum_j w_ij * delta_ij`.
fn construct_b(graph: &[VtxData], n: usize, b: &mut [f64]) {
    for i in 0..n {
        let mut b_i = 0.0f64;
        if graph[0].edists.is_none() {
            continue;
        }
        let ew = graph[i].ewgts.as_ref().unwrap();
        let ed = graph[i].edists.as_ref().unwrap();
        for j in 1..graph[i].nedges() {
            b_i += f64::from(ew[j] * f32::from(ed[j]));
        }
        b[i] = b_i;
    }
}

/// `compute_y_coords(graph, n, y_coords, max_iterations)`: solve `L y = b`
/// with uniform edge weights.
pub fn compute_y_coords(
    graph: &[VtxData],
    n: usize,
    y_coords: &mut [f64],
    max_iterations: usize,
) -> Result<(), SolverError> {
    let mut b = vec![0.0f64; n];
    construct_b(graph, n, &mut b);
    init_vec_orth1(n, y_coords);
    // replace original edge weights (lengths) with uniform weights
    let uniform: Vec<VtxData> = graph[..n]
        .iter()
        .map(|v| {
            let mut w = vec![1.0f32; v.nedges()];
            w[0] = -((v.nedges() - 1) as f32);
            VtxData {
                edges: v.edges.clone(),
                ewgts: Some(w),
                eweights: v.eweights.clone(),
                edists: v.edists.clone(),
            }
        })
        .collect();
    conjugate_gradient(&uniform, y_coords, &b, n, HIERARCHY_CG_TOL, max_iterations)
}

/// `compute_hierarchy(graph, n, abs_tol, relative_tol, given_coords, ...)`:
/// `(ordering, levels, num_levels)`; `ordering[levels[i]]` is the first node
/// of level `i + 1`.
pub fn compute_hierarchy(
    graph: &[VtxData],
    n: usize,
    abs_tol: f64,
    relative_tol: f64,
    given_coords: Option<&[f64]>,
) -> Result<(Vec<i32>, Vec<i32>, usize), SolverError> {
    let owned;
    let y: &[f64] = match given_coords {
        Some(y) => y,
        None => {
            let mut v = vec![0.0f64; n];
            compute_y_coords(graph, n, &mut v, n)?;
            owned = v;
            &owned
        }
    };
    let mut ordering: Vec<i32> = (0..n as i32).collect();
    if n > 1 {
        qsort_by(&mut ordering, |a, b| {
            let (pa, pb) = (y[*a as usize], y[*b as usize]);
            if pa < pb {
                -1
            } else if pa > pb {
                1
            } else {
                0
            }
        });
    }
    let yo = |i: usize| y[ordering[i] as usize];
    let hierarchy_span = yo(n - 1) - yo(0);
    let tol = c_max(abs_tol, relative_tol * hierarchy_span / (n as f64 - 1.0));
    let mut levels = Vec::new();
    for i in 1..n {
        if yo(i) - yo(i - 1) > tol {
            levels.push(i as i32);
        }
    }
    let num_levels = levels.len();
    if num_levels == 0 {
        levels.push(n as i32);
    }
    Ok((ordering, levels, num_levels))
}

// ---- smart_ini_x.c ----

fn standardize(orthog: &mut [f64], nvtxs: usize) {
    let mut avg = 0.0f64;
    for v in orthog.iter().take(nvtxs) {
        avg += v;
    }
    avg /= nvtxs as f64;
    for v in orthog.iter_mut().take(nvtxs) {
        *v -= avg;
    }
    let len = norm(orthog, nvtxs - 1);
    if len.abs() < f64::EPSILON {
        return;
    }
    let s = 1.0 / len;
    for v in orthog.iter_mut().take(nvtxs) {
        *v *= s;
    }
}

fn mat_mult_vec_orthog(
    mat: &[Vec<f32>],
    dim1: usize,
    dim2: usize,
    vec: &[f64],
    result: &mut [f64],
    orthog: &[f64],
) {
    for i in 0..dim1 {
        let mut sum = 0.0f64;
        for j in 0..dim2 {
            sum = f64::from(mat[i][j]).mul_add(vec[j], sum);
        }
        result[i] = sum;
    }
    let alpha = -vectors_inner_product(dim1, result, orthog);
    scadd(result, dim1 - 1, alpha, orthog);
}

fn scalar_mult(v: &mut [f64], n: usize, alpha: f64) {
    for x in v.iter_mut().take(n) {
        *x *= alpha;
    }
}

/// `power_iteration_orthog`.
#[allow(clippy::too_many_arguments)]
fn power_iteration_orthog(
    square_mat: &[Vec<f32>],
    n: usize,
    mut neigs: usize,
    eigs: &mut [Vec<f64>],
    evals: &mut [f64],
    orthog: &[f64],
    p_iteration_threshold: f64,
) {
    let mut tmp_vec = vec![0.0f64; n];
    let mut last_vec = vec![0.0f64; n];
    let tol = 1.0 - p_iteration_threshold;
    if neigs >= n {
        neigs = n;
    }
    let mut i = 0;
    let mut exited = false;
    'outer: while i < neigs {
        loop {
            // choose: guess the i-th eigen vector
            for j in 0..n {
                eigs[i][j] = (crate::cutil::rand() % 100) as f64;
            }
            let alpha = -vectors_inner_product(n, orthog, &eigs[i]);
            scadd(&mut eigs[i], n - 1, alpha, orthog);
            for j in 0..i {
                let (a, b) = eigs.split_at_mut(i);
                let alpha = -vectors_inner_product(n, &a[j], &b[0]);
                scadd(&mut b[0], n - 1, alpha, &a[j]);
            }
            let len = norm(&eigs[i], n - 1);
            if len < 1e-10 {
                continue;
            }
            scalar_mult(&mut eigs[i], n, 1.0 / len);
            break;
        }
        let mut angle;
        let mut len;
        loop {
            last_vec.copy_from_slice(&eigs[i][..n]);
            mat_mult_vec_orthog(square_mat, n, n, &eigs[i], &mut tmp_vec, orthog);
            eigs[i][..n].copy_from_slice(&tmp_vec);
            for j in 0..i {
                let (a, b) = eigs.split_at_mut(i);
                let alpha = -vectors_inner_product(n, &a[j], &b[0]);
                scadd(&mut b[0], n - 1, alpha, &a[j]);
            }
            len = norm(&eigs[i], n - 1);
            if len < 1e-10 {
                exited = true;
                break 'outer;
            }
            scalar_mult(&mut eigs[i], n, 1.0 / len);
            angle = vectors_inner_product(n, &eigs[i], &last_vec);
            if angle.abs() >= tol {
                break;
            }
        }
        evals[i] = angle * len;
        i += 1;
    }
    let _ = exited;
    while i < neigs {
        for j in 0..n {
            eigs[i][j] = (crate::cutil::rand() % 100) as f64;
        }
        for j in 0..i {
            let (a, b) = eigs.split_at_mut(i);
            let alpha = -vectors_inner_product(n, &a[j], &b[0]);
            scadd(&mut b[0], n - 1, alpha, &a[j]);
        }
        let len = norm(&eigs[i], n - 1);
        scalar_mult(&mut eigs[i], n, 1.0 / len);
        evals[i] = 0.0;
        i += 1;
    }
    // sort vectors by their evals
    for i in 0..neigs.saturating_sub(1) {
        let mut largest_index = i;
        let mut largest_eval = evals[largest_index];
        for j in i + 1..neigs {
            if largest_eval < evals[j] {
                largest_index = j;
                largest_eval = evals[largest_index];
            }
        }
        if largest_index != i {
            eigs.swap(i, largest_index);
            evals[largest_index] = evals[i];
            evals[i] = largest_eval;
        }
    }
}

fn compute_avgs(dij: &[Vec<DistType>], n: usize) -> (Vec<f32>, f32) {
    let mut row_avg = vec![0.0f32; n];
    let mut sum = 0.0f64;
    for i in 0..n {
        let mut sum_row = 0.0f64;
        for j in 0..n {
            let d = f64::from(dij[i][j]);
            sum = d.mul_add(d, sum);
            sum_row = d.mul_add(d, sum_row);
        }
        row_avg[i] = sum_row as f32 / n as f32;
    }
    (row_avg, sum as f32 / (n * n) as f32)
}

fn compute_bij(dij: &[Vec<DistType>], n: usize) -> Vec<Vec<f32>> {
    let mut bij = vec![vec![0.0f32; n]; n];
    let (row_avg, all_avg) = compute_avgs(dij, n);
    for i in 0..n {
        for j in 0..=i {
            let d = dij[i][j] as f32;
            bij[i][j] = (-d).mul_add(d, row_avg[i]) + row_avg[j] - all_avg;
            bij[j][i] = bij[i][j];
        }
    }
    bij
}

fn cmds_orthog(
    n: usize,
    dim: usize,
    eigs: &mut [Vec<f64>],
    tol: f64,
    orthog: &[f64],
    dij: &[Vec<DistType>],
) {
    let bij = compute_bij(dij, n);
    let mut evals = vec![0.0f64; dim];
    let mut orthog_aux = orthog[..n].to_vec();
    standardize(&mut orthog_aux, n);
    power_iteration_orthog(&bij, n, dim, eigs, &mut evals, &orthog_aux, tol);
    for i in 0..dim {
        let s = evals[i].abs().sqrt();
        for j in 0..n {
            eigs[i][j] *= s;
        }
    }
}

/// `compute_apsp(graph, n)` (`kkutils.c`).
fn compute_apsp(graph: &[VtxData], n: usize) -> Vec<Vec<DistType>> {
    let mut d = vec![vec![0 as DistType; n]; n];
    let weighted = graph.first().is_some_and(|v| v.ewgts.is_some());
    for (i, row) in d.iter_mut().enumerate() {
        if weighted {
            ngdijkstra(i, graph, n, row);
        } else {
            bfs(i, graph, n, row);
        }
    }
    d
}

/// `IMDS_given_dim(graph, n, given_coords, new_coords, conj_tol)`.
pub fn imds_given_dim(
    graph: &[VtxData],
    n: usize,
    x: &mut [f64],
    y: &mut [f64],
    conj_tol: f64,
) -> Result<(), SolverError> {
    let mut dij = compute_apsp(graph, n);
    for row in dij.iter_mut() {
        for d in row.iter_mut() {
            *d = d.wrapping_mul(SCALE_FACTOR);
        }
    }
    let uni_length;
    {
        let (mut sum1, mut sum2) = (0.0f64, 0.0f64);
        for i in 1..n {
            for j in 0..i {
                let ad = (x[i] - x[j]).abs();
                sum1 = (1.0 / f64::from(dij[i][j])).mul_add(ad, sum1);
                let dd = dij[i][j].wrapping_mul(dij[i][j]);
                sum2 = ((1.0 / f64::from(dd)) * ad).mul_add(ad, sum2);
            }
        }
        uni_length = if sum2.is_infinite() { 0.0 } else { sum1 / sum2 };
        for v in x.iter_mut().take(n) {
            *v *= uni_length;
        }
    }
    // smart ini:
    {
        let mut eigs = vec![y[..n].to_vec()];
        cmds_orthog(n, 1, &mut eigs, conj_tol, x, &dij);
        y[..n].copy_from_slice(&eigs[0]);
    }
    // Laplacian
    let mut lap = vec![vec![0.0f32; n]; n];
    for i in 0..n {
        let mut degree = 0.0f32;
        for j in 0..n {
            if j == i {
                continue;
            }
            let d = dij[i][j] as f32;
            lap[i][j] = -1.0f32 / (d * d);
            degree -= lap[i][j];
        }
        lap[i][i] = degree;
    }
    // residual distances
    for i in 1..n {
        let pos_i = x[i];
        for j in 0..i {
            let d = f64::from(dij[i][j]);
            let dx = pos_i - x[j];
            let diff = d.mul_add(d, -(dx * dx));
            let v = if diff > 0.0 {
                diff.sqrt() as DistType
            } else {
                0
            };
            dij[i][j] = v;
            dij[j][i] = v;
        }
    }
    let balance_of = |y: &[f64], lap: &[Vec<f32>], dij: &[Vec<DistType>], i: usize| -> f64 {
        let pos_i = y[i];
        let mut b = 0.0f64;
        for j in 0..n {
            if j == i {
                continue;
            }
            let t = f64::from(dij[i][j] as f32 * (-lap[i][j]));
            if pos_i >= y[j] {
                b += t;
            } else {
                b -= t;
            }
        }
        b
    };
    let mut balance: Vec<f64> = (0..n).map(|i| balance_of(y, &lap, &dij, i)).collect();
    let mut converged = false;
    let mut iterations2 = 0;
    while iterations2 < 200 && !converged {
        conjugate_gradient_f(&lap, y, &balance, n, conj_tol, n, true)?;
        converged = true;
        for i in 0..n {
            let b = balance_of(y, &lap, &dij, i);
            if b != balance[i] && (1.0 - b / balance[i]).abs() > 1e-5 {
                converged = false;
                balance[i] = b;
            }
        }
        iterations2 += 1;
    }
    if uni_length.abs() >= f64::EPSILON {
        for i in 0..n {
            x[i] /= uni_length;
            y[i] /= uni_length;
        }
    }
    Ok(())
}

// ---- quad_prog_solve.c ----

/// `CMajEnv`.
struct CMajEnv {
    n: usize,
    a: Vec<Vec<f32>>,
    ordering: Vec<i32>,
    levels: Vec<i32>,
    num_levels: usize,
    f_array1: Vec<f32>,
    f_array2: Vec<f32>,
    f_array3: Vec<f32>,
    f_array4: Vec<f32>,
}

fn init_constrained_majorization(
    packed: &[f32],
    n: usize,
    ordering: Vec<i32>,
    levels: Vec<i32>,
    num_levels: usize,
) -> CMajEnv {
    CMajEnv {
        n,
        a: unpack_matrix(packed, n),
        ordering,
        levels,
        num_levels,
        f_array1: vec![0.0; n],
        f_array2: vec![0.0; n],
        f_array3: vec![0.0; n],
        f_array4: vec![0.0; n],
    }
}

/// `quicksort_placef(place, ordering, first, last)`.
fn quicksort_placef(place: &[f32], ordering: &mut [i32], first: usize, last: isize) {
    if (first as isize) < last {
        qsort_by(&mut ordering[first..=last as usize], |a, b| {
            let (d1, d2) = (place[*a as usize], place[*b as usize]);
            if d1 < d2 {
                -1
            } else if d1 > d2 {
                1
            } else {
                0
            }
        });
    }
}

fn ensure_monotonic_ordering_with_gaps(
    place: &mut [f32],
    n: usize,
    ordering: &mut [i32],
    levels: &[i32],
    num_levels: usize,
    levels_gap: f32,
) {
    let mut lower_bound = -1e9f32;
    let mut level: usize = usize::MAX;
    let mut max_in_level = 0usize;
    for i in 0..n {
        if i >= max_in_level {
            level = level.wrapping_add(1);
            max_in_level = if level == num_levels {
                n
            } else {
                levels[level] as usize
            };
            lower_bound = if i > 0 {
                place[ordering[i - 1] as usize] + levels_gap
            } else {
                -1e9f32
            };
            quicksort_placef(place, ordering, i, max_in_level as isize - 1);
        }
        let node = ordering[i] as usize;
        if place[node] < lower_bound {
            place[node] = lower_bound;
        }
    }
}

fn equals(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-2f32
}

/// `constrained_majorization_new_with_gaps(e, b, coords, cur_axis, max_iterations, levels_gap)`.
fn constrained_majorization_new_with_gaps(
    e: &mut CMajEnv,
    b: &[f32],
    place: &mut [f32],
    max_iterations: i32,
    levels_gap: f32,
) {
    let n = e.n;
    let num_levels = e.num_levels;
    if max_iterations <= 0 {
        return;
    }
    ensure_monotonic_ordering_with_gaps(
        place,
        n,
        &mut e.ordering,
        &e.levels,
        num_levels,
        levels_gap,
    );
    let lap = &e.a;
    let ordering = &mut e.ordering;
    let levels = &e.levels;
    let desired_place = &mut e.f_array1;
    let prefix_desired_place = &mut e.f_array2;
    let suffix_desired_place = &mut e.f_array3;
    let gap = &mut e.f_array4;
    let mut block: Vec<usize> = Vec::new();

    let mut lev = vec![0i32; n];
    let mut level: i32 = -1;
    let mut max_in_level = 0usize;
    for i in 0..n {
        if i >= max_in_level {
            level += 1;
            max_in_level = if level as usize == num_levels {
                n
            } else {
                levels[level as usize] as usize
            };
        }
        lev[ordering[i] as usize] = level;
    }

    let mut converged = false;
    let mut counter = 0;
    while counter < max_iterations && !converged {
        converged = true;
        let mut lower_bound = -1e9f32;
        let mut left = 0usize;
        while left < n {
            let cur_place = place[ordering[left] as usize];
            let mut target_place = cur_place;
            gap[ordering[left] as usize] = 0.0;
            let mut right = left + 1;
            while right < n {
                // C indexes `lev` by position here
                if lev[right] > lev[right - 1] {
                    target_place += levels_gap;
                }
                let node = ordering[right] as usize;
                if f64::from(place[node] - target_place).abs() > 1e-9 {
                    break;
                }
                gap[node] = place[node] - cur_place;
                right += 1;
            }
            // desired place of the block's reference point per node
            for i in left..right {
                let node = ordering[i] as usize;
                let mut new_place_i = -b[node];
                let lap_node = &lap[node];
                for j in 0..n {
                    if j == node {
                        continue;
                    }
                    new_place_i = lap_node[j].mul_add(place[j], new_place_i);
                }
                desired_place[node] = new_place_i / (-lap_node[node]) - gap[node];
            }
            // reorder block by levels, then by relaxed desired position
            block.clear();
            let mut i = left;
            while i < right {
                let level = lev[ordering[i] as usize];
                let first_next_level = if level as usize == num_levels {
                    right
                } else {
                    right.min(levels[level as usize] as usize)
                };
                for j in i..first_next_level {
                    let node = ordering[j] as usize;
                    if desired_place[node] < cur_place {
                        block.push(node);
                    }
                }
                // C tests `desired_place[node] == cur_place` here. When every
                // edge len in the component is below 1, the integer BFS of
                // `compute_apsp_packed` (lib/neatogen/bfs.c) truncates every
                // distance to 0, so the Laplacian is all zero (or all NaN with
                // levelsgap > 0) and desired_place is NaN. NaN fails all three
                // tests, the block misses nodes of [left, right), and C's
                // `constrained_majorization_new_with_gaps` fails the
                // `gv_list_get` bounds assertion when it reads the block's last
                // element. Treat an unordered (NaN) desired place as equal, so
                // the block always holds exactly the nodes of [left, right), as
                // the rest of the pass assumes. No split of such a block has a
                // positive movement, so none is chosen and the block stays put.
                for j in i..first_next_level {
                    let node = ordering[j] as usize;
                    let ord = desired_place[node].partial_cmp(&cur_place);
                    if ord.is_none_or(std::cmp::Ordering::is_eq) {
                        block.push(node);
                    }
                }
                for j in i..first_next_level {
                    let node = ordering[j] as usize;
                    if desired_place[node] > cur_place {
                        block.push(node);
                    }
                }
                i = first_next_level;
            }
            let blen = block.len();
            // desired places of prefixes
            let mut des_place_block = 0.0f32;
            let mut block_deg = 0.0f32;
            for i in 0..blen {
                let node = block[i];
                let lap_node = &lap[node];
                let mut tbc = 0.0f32;
                for &bj in &block[..i] {
                    tbc -= lap_node[bj];
                }
                tbc *= 2.0;
                let num = tbc.mul_add(
                    cur_place,
                    block_deg.mul_add(des_place_block, (-lap_node[node]) * desired_place[node]),
                );
                des_place_block = num / (block_deg - lap_node[node] + tbc);
                prefix_desired_place[i] = des_place_block;
                block_deg += tbc - lap_node[node];
            }
            if blen == n {
                prefix_desired_place[n - 1] = cur_place;
            }
            // desired places of suffixes
            des_place_block = 0.0;
            block_deg = 0.0;
            for i in (0..blen).rev() {
                let node = block[i];
                let lap_node = &lap[node];
                let mut tbc = 0.0f32;
                for &bj in &block[i + 1..] {
                    tbc -= lap_node[bj];
                }
                tbc *= 2.0;
                let num = tbc.mul_add(
                    cur_place,
                    block_deg.mul_add(des_place_block, (-lap_node[node]) * desired_place[node]),
                );
                des_place_block = num / (block_deg - lap_node[node] + tbc);
                suffix_desired_place[i] = des_place_block;
                block_deg += tbc - lap_node[node];
            }
            if blen == n {
                suffix_desired_place[0] = cur_place;
            }
            // find best place to split block
            let mut best_i = usize::MAX;
            let mut max_movement = 0.0f64;
            for i in 0..blen {
                let mut suffix_des_place = suffix_desired_place[i];
                let mut prefix_des_place = if i > 0 {
                    prefix_desired_place[i - 1]
                } else {
                    suffix_des_place
                };
                if suffix_des_place < prefix_des_place {
                    if suffix_des_place < cur_place {
                        if prefix_des_place > cur_place {
                            prefix_des_place = cur_place;
                        }
                        suffix_des_place = prefix_des_place;
                    } else if prefix_des_place > cur_place {
                        prefix_des_place = suffix_des_place;
                    }
                }
                let movement = f64::from((blen - i) as f32).mul_add(
                    f64::from(suffix_des_place - cur_place).abs(),
                    f64::from(i as f32) * f64::from(prefix_des_place - cur_place).abs(),
                );
                if movement > max_movement {
                    max_movement = movement;
                    best_i = i;
                }
            }
            let last_gap = |gap: &[f32], block: &[usize]| gap[block[block.len() - 1]];
            if best_i != usize::MAX {
                let mut suffix_des_place = suffix_desired_place[best_i];
                let mut prefix_des_place = if best_i > 0 {
                    prefix_desired_place[best_i - 1]
                } else {
                    suffix_des_place
                };
                let upper_bound = if right >= n {
                    1e9f32
                } else if lev[ordering[right] as usize] > lev[ordering[right - 1] as usize] {
                    place[ordering[right] as usize] - levels_gap - last_gap(gap, &block)
                } else {
                    place[ordering[right] as usize] - last_gap(gap, &block)
                };
                suffix_des_place = suffix_des_place.min(upper_bound);
                prefix_des_place = prefix_des_place.max(lower_bound);
                if suffix_des_place < prefix_des_place {
                    if suffix_des_place < cur_place {
                        if prefix_des_place > cur_place {
                            prefix_des_place = cur_place;
                        }
                        suffix_des_place = prefix_des_place;
                    } else if prefix_des_place > cur_place {
                        prefix_des_place = suffix_des_place;
                    }
                }
                for &bn in &block[..best_i] {
                    place[bn] = prefix_des_place + gap[bn];
                }
                for &bn in &block[best_i..] {
                    place[bn] = suffix_des_place + gap[bn];
                }
                let lastp = place[block[blen - 1]];
                lower_bound = if right < n
                    && lev[ordering[right] as usize] > lev[ordering[right - 1] as usize]
                {
                    lastp + levels_gap
                } else {
                    lastp
                };
                for i in left..right {
                    ordering[i] = block[i - left] as i32;
                }
                converged &=
                    equals(prefix_des_place, cur_place) && equals(suffix_des_place, cur_place);
            } else {
                let lastp = place[block[blen - 1]];
                lower_bound = if right < n
                    && lev[ordering[right] as usize] > lev[ordering[right - 1] as usize]
                {
                    lastp + levels_gap
                } else {
                    lastp
                };
            }
            left = right;
        }
        orthog1f(&mut place[..n]);
        counter += 1;
    }
}

// ---- constrained_majorization.c ----

fn distance_kd(coords: &[Vec<f64>], dim: usize, i: usize, j: usize) -> f64 {
    let mut sum = 0.0f64;
    for c in coords.iter().take(dim) {
        let d = c[i] - c[j];
        sum = d.mul_add(d, sum);
    }
    sum.sqrt()
}

/// `stress_majorization_with_hierarchy(graph, n, d_coords, nodes, dim, opts, model, maxi, levels_gap)`.
#[allow(clippy::too_many_arguments)]
pub fn stress_majorization_with_hierarchy(
    g: &LGraph<'_>,
    params: &NeatoParams,
    graph: &[VtxData],
    n: usize,
    d_coords: &mut [Vec<f64>],
    nodes: &[usize],
    dim: usize,
    opts: i32,
    model: i32,
    maxi: i64,
    levels_gap: f64,
) -> Result<i64, SolverError> {
    let smart_ini = opts & OPT_SMART_INIT != 0;
    let abs_tol = 1e-2;
    let relative_tol = LEVELS_SEP_TOL;
    let mut directionality = false;
    if graph[0].edists.is_some() {
        for v in graph.iter().take(n) {
            let ed = v.edists.as_ref().unwrap();
            for j in 1..v.nedges() {
                directionality |= ed[j] != 0;
            }
        }
    }
    let mkernel = |d: &mut [Vec<f64>], dim: usize, maxi: i64| {
        stress_majorization_kd_mkernel(g, params, graph, n, d, nodes, dim, opts, model, maxi)
    };
    if !directionality {
        return mkernel(d_coords, dim, maxi);
    }

    let (ordering, levels, num_levels);
    if smart_ini {
        if dim > 2 {
            mkernel(&mut d_coords[1..], dim - 1, 15)?;
            for i in 0..n {
                d_coords[dim - 1][i] = d_coords[1][i];
            }
        }
        let (xs, rest) = d_coords.split_at_mut(1);
        let x = &mut xs[0];
        let y = &mut rest[0];
        compute_y_coords(graph, n, y, n)?;
        let (o, l, nl) = compute_hierarchy(graph, n, abs_tol, relative_tol, Some(y))?;
        if nl < 1 {
            return mkernel(d_coords, dim, maxi);
        }
        if levels_gap > 0.0 {
            let mut displacement = 0.0f64;
            for i in 0..nl {
                let li = l[i] as usize;
                displacement += c_max(
                    0.0,
                    levels_gap - (y[o[li] as usize] + displacement - y[o[li - 1] as usize]),
                );
                let stop = if i < nl - 1 { l[i + 1] as usize } else { n };
                for j in li..stop {
                    y[o[j] as usize] += displacement;
                }
            }
        }
        if dim == 2 {
            imds_given_dim(graph, n, y, x, params.epsilon)?;
        }
        ordering = o;
        levels = l;
        num_levels = nl;
    } else {
        init_layout(g, n, dim, d_coords, nodes);
        let (o, l, nl) = compute_hierarchy(graph, n, abs_tol, relative_tol, None)?;
        ordering = o;
        levels = l;
        num_levels = nl;
    }
    if n == 1 || maxi == 0 {
        return Ok(0);
    }

    use graphviz_pure_core::diag::{elapsed_sec, fmt_f, start_timer};
    let verbose = graphviz_pure_core::diag::verbose_on();
    if verbose {
        start_timer();
    }
    let mut dij: Option<Vec<f32>> = None;
    if model == MODEL_SUBSET {
        graphviz_pure_core::verbose_msg!("Calculating subset model");
        dij = Some(compute_apsp_artificial_weights_packed(graph, n));
    } else if model == MODEL_CIRCUIT {
        dij = circuit_model(graph, n);
        if dij.is_none() {
            graphviz_pure_core::diag::agwarningf(
                "graph is disconnected. Hence, the circuit model\n",
            );
            graphviz_pure_core::diag::agerr_prev(
                "is undefined. Reverting to the shortest path model.\n",
            );
        }
    } else if model == MODEL_MDS {
        graphviz_pure_core::verbose_msg!("Calculating MDS model");
        dij = mds_model(graph, n);
    }
    let mut dij = dij.unwrap_or_else(|| {
        graphviz_pure_core::verbose_msg!("Calculating shortest paths");
        compute_apsp_packed(graph, n)
    });
    graphviz_pure_core::diag::verbose_phase("Setting initial positions");
    let length = n + n * (n - 1) / 2;

    if !smart_ini {
        // for numerical stability, scale down layout
        let mut max = 1.0f64;
        for c in d_coords.iter().take(dim) {
            for &v in c.iter().take(n) {
                max = max.max(v.abs());
            }
        }
        for c in d_coords.iter_mut().take(dim) {
            for v in c.iter_mut().take(n) {
                *v *= 10.0 / max;
            }
        }
    }
    if levels_gap > 0.0 {
        let sum1 = (n * (n - 1) / 2) as f64;
        let mut sum2 = 0.0f64;
        let mut count = 0;
        for i in 0..n - 1 {
            count += 1;
            for j in i + 1..n {
                sum2 += distance_kd(d_coords, dim, i, j) / f64::from(dij[count]);
                count += 1;
            }
        }
        let scale_ratio = (sum2 / sum1) as f32;
        for v in dij.iter_mut().take(length) {
            *v *= scale_ratio;
        }
    }
    for c in d_coords.iter_mut().take(dim) {
        orthog1(&mut c[..n]);
    }
    let y_0 = d_coords[1][0];
    for v in d_coords[1].iter_mut().take(n) {
        *v -= y_0;
    }
    let mut coords: Vec<Vec<f32>> = (0..dim)
        .map(|i| d_coords[i][..n].iter().map(|&v| v as f32).collect())
        .collect();
    let constant_term = (n * (n - 1) / 2) as f64;
    graphviz_pure_core::verbose_msg!(": {} sec", fmt_f(elapsed_sec(), 2));

    let mut lap2 = dij;
    let lap_length = n + n * (n - 1) / 2;
    square_vec(&mut lap2[..lap_length]);
    invert_vec(&mut lap2[..lap_length]);
    let mut degrees = vec![0.0f64; n];
    {
        let mut count = 0;
        for i in 0..n - 1 {
            let mut degree = 0.0f64;
            count += 1;
            for j in 1..n - i {
                let val = lap2[count];
                degree += f64::from(val);
                degrees[i + j] -= f64::from(val);
                count += 1;
            }
            degrees[i] -= degree;
        }
        let (mut step, mut count) = (n, 0);
        for d in degrees.iter().take(n) {
            lap2[count] = *d as f32;
            count += step;
            step -= 1;
        }
    }

    let mut b: Vec<Vec<f32>> = vec![vec![0.0f32; n]; dim];
    let mut tmp_coords = vec![0.0f32; n];
    let mut dist_accumulator = vec![0.0f32; n];
    let mut lap1 = vec![0.0f32; lap_length];
    let mut old_stress = f64::MAX;
    let mut cenv = init_constrained_majorization(&lap2, n, ordering, levels, num_levels);
    let mut converged = false;
    let mut iterations = 0i64;
    while iterations < maxi && !converged {
        for d in degrees.iter_mut() {
            *d = 0.0;
        }
        sqrt_vecf(&lap2[..lap_length], &mut lap1[..lap_length]);
        let mut count = 0;
        for i in 0..n - 1 {
            let len = n - i - 1;
            for v in dist_accumulator.iter_mut() {
                *v = 0.0;
            }
            for c in coords.iter().take(dim) {
                for x in 0..len {
                    let t = (-1.0f32).mul_add(c[i + 1 + x], c[i]);
                    tmp_coords[x] = t * t;
                }
                for x in 0..len {
                    dist_accumulator[x] += tmp_coords[x];
                }
            }
            invert_sqrt_vec(&mut dist_accumulator[..len]);
            for v in dist_accumulator.iter_mut().take(len) {
                if *v >= f32::MAX || *v < 0.0 {
                    *v = 0.0;
                }
            }
            count += 1;
            let mut degree = 0.0f64;
            for j in 0..len {
                lap1[count] *= dist_accumulator[j];
                let val = lap1[count];
                degree += f64::from(val);
                degrees[i + j + 1] -= f64::from(val);
                count += 1;
            }
            degrees[i] -= degree;
        }
        {
            let (mut step, mut count) = (n, 0);
            for d in degrees.iter().take(n) {
                lap1[count] = *d as f32;
                count += step;
                step -= 1;
            }
        }
        for k in 0..dim {
            right_mult_with_vector_ff(&lap1, n, &coords[k], &mut b[k]);
        }
        let mut new_stress = 0.0f64;
        for k in 0..dim {
            new_stress += vectors_inner_productf(n, &coords[k], &b[k]);
        }
        new_stress *= 2.0;
        new_stress += constant_term;
        for k in 0..dim {
            right_mult_with_vector_ff(&lap2, n, &coords[k], &mut tmp_coords);
            new_stress -= vectors_inner_productf(n, &coords[k], &tmp_coords);
        }
        converged = (new_stress - old_stress).abs() / (old_stress + 1e-10).abs() < params.epsilon;
        converged |= iterations > 1 && new_stress > old_stress;
        old_stress = new_stress;
        for k in 0..dim {
            if k == 1 {
                constrained_majorization_new_with_gaps(
                    &mut cenv,
                    &b[k],
                    &mut coords[k],
                    LOCAL_CONSTR_MAJOR_ITERATIONS,
                    levels_gap as f32,
                );
            } else {
                let (ck, bk) = (&mut coords[k], &mut b[k]);
                conjugate_gradient_mkernel(&lap2, ck, bk, n, TOLERANCE_CG, n)?;
            }
        }
        iterations += 1;
    }
    for i in 0..dim {
        for j in 0..n {
            d_coords[i][j] = f64::from(coords[i][j]);
        }
    }
    Ok(iterations)
}
