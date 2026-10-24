//! Stress majorization, ported from `lib/neatogen/stress.c`
//! (`stress_majorization_kD_mkernel`, `initLayout`, the distance models
//! `circuitModel`, `mdsModel`, `compute_apsp_packed`,
//! `compute_weighted_apsp_packed`, `compute_apsp_artificial_weights_packed`)
//! and `lib/neatogen/kkutils.c` (`compute_new_weights`, `compute_apsp*`).
//!
//! The solver works in `float` like C; see [`super::matrix_ops`] for the
//! operation-order conventions.

use super::bfs::{DistType, bfs, dijkstra_f, ngdijkstra};
use super::conjgrad::conjugate_gradient_mkernel;
use super::graphdata::{VtxData, has_ewgts};
use super::linalg::solve_circuit;
use super::matrix_ops::*;
use super::{MODEL_CIRCUIT, MODEL_MDS, MODEL_SUBSET, NeatoParams};
use crate::common::types::{LGraph, P_FIX};
use graphviz_pure_core::diag::{elapsed_sec, fmt_f, fprintf_stderr, start_timer, verbose_on};

pub const TOLERANCE_CG: f64 = 1e-3;
pub const DFLT_ITERATIONS: i64 = 200;
pub const DFLT_TOLERANCE: f64 = 1e-4;
pub const OPT_SMART_INIT: i32 = 0x4;
pub const OPT_EXP_FLAG: i32 = 0x3;
pub const NUM_PIVOTS_STRESS: usize = 40;

/// `hasPos(n)`.
pub fn has_pos(g: &LGraph<'_>, n: usize) -> bool {
    g.nodes[n].pinned > 0
}

/// `isFixed(n)`.
pub fn is_fixed(g: &LGraph<'_>, n: usize) -> bool {
    g.nodes[n].pinned > P_FIX
}

/// `initLayout(n, dim, coords, nodes)`: user positions or `drand48()`, then
/// centre each axis. Returns true if some node is pinned.
pub fn init_layout(
    g: &LGraph<'_>,
    n: usize,
    dim: usize,
    coords: &mut [Vec<f64>],
    nodes: &[usize],
) -> bool {
    let mut pinned = false;
    for i in 0..n {
        let np = nodes[i];
        if has_pos(g, np) {
            let pt = &g.nodes[np].pos;
            for (d, c) in coords.iter_mut().enumerate().take(dim) {
                c[i] = pt[d];
            }
            if is_fixed(g, np) {
                pinned = true;
            }
        } else {
            coords[0][i] = crate::cutil::drand48();
            coords[1][i] = crate::cutil::drand48();
            for c in coords.iter_mut().take(dim).skip(2) {
                c[i] = crate::cutil::drand48();
            }
        }
    }
    for c in coords.iter_mut().take(dim) {
        orthog1(&mut c[..n]);
    }
    pinned
}

/// `circuitModel(graph, nG)`: resistance distances; `None` if disconnected.
pub fn circuit_model(graph: &[VtxData], ng: usize) -> Option<Vec<f32>> {
    let mut gm = vec![vec![0.0f64; ng]; ng];
    let mut gm_inv = vec![vec![0.0f64; ng]; ng];
    let w = has_ewgts(graph);
    for i in 0..ng {
        for e in 1..graph[i].nedges() {
            let j = graph[i].edges[e] as usize;
            let v = if w {
                -1.0 / f64::from(graph[i].ewgts.as_ref().unwrap()[e])
            } else {
                -1.0
            };
            gm[i][j] = v;
            gm[j][i] = v;
        }
    }
    if !solve_circuit(ng, &mut gm, &mut gm_inv) {
        return None;
    }
    let mut dij = Vec::with_capacity(ng * (ng + 1) / 2);
    for i in 0..ng {
        for j in i..ng {
            let v = if i == j {
                0.0
            } else {
                (-2.0f64).mul_add(gm_inv[i][j], gm_inv[i][i] + gm_inv[j][j]) as f32
            };
            dij.push(v);
        }
    }
    Some(dij)
}

/// `compute_weighted_apsp_packed(graph, n)`.
fn compute_weighted_apsp_packed(graph: &[VtxData], n: usize) -> Vec<f32> {
    let mut dij = Vec::with_capacity(n * (n + 1) / 2);
    let mut di = vec![0.0f32; n];
    for i in 0..n {
        dijkstra_f(i, graph, n, &mut di);
        dij.extend_from_slice(&di[i..n]);
    }
    dij
}

/// `mdsModel(graph, nG)`: shortest paths with edge entries replaced by `len`.
pub fn mds_model(graph: &[VtxData], ng: usize) -> Option<Vec<f32>> {
    if !has_ewgts(graph) {
        return None;
    }
    let mut dij = compute_weighted_apsp_packed(graph, ng);
    let mut shift = 0;
    let mut delta = 0.0f64;
    for i in 0..ng {
        shift += i;
        let w = graph[i].ewgts.as_ref().unwrap();
        for e in 1..graph[i].nedges() {
            let j = graph[i].edges[e] as usize;
            if j < i {
                continue;
            }
            let (slot, we) = (&mut dij[i * ng + j - shift], w[e]);
            delta += f64::from((*slot - we).abs());
            *slot = we;
        }
    }
    graphviz_pure_core::verbose_msg!("mdsModel: delta = {}\n", fmt_f(delta, 6));
    Some(dij)
}

/// `compute_apsp_packed(graph, n)`: BFS distances.
pub fn compute_apsp_packed(graph: &[VtxData], n: usize) -> Vec<f32> {
    let mut dij = Vec::with_capacity(n * (n + 1) / 2);
    let mut di = vec![0 as DistType; n];
    for i in 0..n {
        bfs(i, graph, n, &mut di);
        for &d in &di[i..n] {
            dij.push(d as f32);
        }
    }
    dij
}

fn common_neighbors(graph: &[VtxData], u: usize, v_vector: &[i32]) -> usize {
    (1..graph[u].nedges())
        .filter(|&j| v_vector[graph[u].edges[j] as usize] > 0)
        .count()
}

/// The re-weighting of `compute_new_weights`: `deg_i + deg_j - 2 * common`,
/// optionally floored by the existing lengths.
fn artificial_weights(graph: &[VtxData], n: usize, floor_by_old: bool) -> Vec<Vec<f32>> {
    let mut vtx_vec = vec![0i32; n];
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut w = vec![0.0f32; graph[i].nedges()];
        for j in 1..graph[i].nedges() {
            vtx_vec[graph[i].edges[j] as usize] = 1;
        }
        let deg_i = graph[i].nedges() - 1;
        for j in 1..=deg_i {
            let nb = graph[i].edges[j] as usize;
            let deg_j = graph[nb].nedges() - 1;
            let v = (deg_i + deg_j - 2 * common_neighbors(graph, nb, &vtx_vec)) as f32;
            w[j] = if floor_by_old {
                v.max(graph[i].ewgts.as_ref().unwrap()[j])
            } else {
                v
            };
        }
        for j in 1..graph[i].nedges() {
            vtx_vec[graph[i].edges[j] as usize] = 0;
        }
        out.push(w);
    }
    out
}

/// `compute_new_weights(graph, n)` applied to a copy of `graph`.
pub fn with_new_weights(graph: &[VtxData], n: usize) -> Vec<VtxData> {
    let w = artificial_weights(graph, n, false);
    graph
        .iter()
        .zip(w)
        .map(|(v, w)| VtxData {
            ewgts: Some(w),
            ..v.clone()
        })
        .collect()
}

/// `compute_apsp_artificial_weights_packed(graph, n)`.
pub fn compute_apsp_artificial_weights_packed(graph: &[VtxData], n: usize) -> Vec<f32> {
    if has_ewgts(graph) {
        let w = artificial_weights(graph, n, true);
        let g2: Vec<VtxData> = graph
            .iter()
            .zip(w)
            .map(|(v, w)| VtxData {
                ewgts: Some(w),
                ..v.clone()
            })
            .collect();
        compute_weighted_apsp_packed(&g2, n)
    } else {
        let g2 = with_new_weights(graph, n);
        // compute_apsp_packed with weights present: bfs uses the (truncated) weights.
        compute_apsp_packed(&g2, n)
    }
}

/// `compute_apsp_artificial_weights(graph, n)` (`kkutils.c`): full matrix.
pub fn compute_apsp_artificial_weights(graph: &[VtxData], n: usize) -> Vec<Vec<DistType>> {
    let g2 = with_new_weights(graph, n);
    let mut out = vec![vec![0; n]; n];
    for (i, row) in out.iter_mut().enumerate() {
        ngdijkstra(i, &g2, n, row);
    }
    out
}

/// `compute_stressf(coords, lap, dim, n, exp)`: the layout's stress.
fn compute_stressf(coords: &[Vec<f32>], lap: &[f32], dim: usize, n: usize, exp: i32) -> f64 {
    let mut sum = 0.0f64;
    // the packed matrix row by row: each row's diagonal entry, then its neighbours'
    let mut lap = lap.iter();
    for i in 0..n.saturating_sub(1) {
        lap.next();
        for j in (i..n).skip(1) {
            let mut dist = 0.0f64;
            for c in coords.iter().take(dim) {
                let (ci, cj) = (c.get(i).copied(), c.get(j).copied());
                let d = ci.unwrap_or_default() - cj.unwrap_or_default();
                dist += f64::from(d * d);
            }
            let dist = dist.sqrt();
            let l = f64::from(lap.next().copied().unwrap_or_default());
            let dij = if exp == 2 { 1.0 / l.sqrt() } else { 1.0 / l };
            // C (clang) fuses this: fmadd((Dij - dist)^2, lap, sum)
            sum = ((dij - dist) * (dij - dist)).mul_add(l, sum);
        }
    }
    sum
}

/// `stress_majorization_kD_mkernel(graph, n, d_coords, nodes, dim, opts, model, maxi)`.
/// Returns the iteration count, or `Err` when the solver fails.
#[allow(clippy::too_many_arguments)]
pub fn stress_majorization_kd_mkernel(
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
) -> Result<i64, super::SolverError> {
    if maxi < 0 {
        return Ok(0);
    }
    if verbose_on() {
        start_timer();
    }
    let smart_ini = opts & OPT_SMART_INIT != 0;
    let exp = opts & OPT_EXP_FLAG;
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
    let mut lap2 = match dij {
        Some(d) => d,
        None => {
            graphviz_pure_core::verbose_msg!("Calculating shortest paths");
            if has_ewgts(graph) {
                compute_weighted_apsp_packed(graph, n)
            } else {
                compute_apsp_packed(graph, n)
            }
        }
    };

    graphviz_pure_core::diag::verbose_phase("Setting initial positions");
    let have_pinned;
    if smart_ini && n > 1 {
        have_pinned = false;
        super::smart_ini::sparse_stress_subspace_majorization_kd(
            params,
            graph,
            n,
            d_coords,
            dim,
            smart_ini,
            exp,
            model == MODEL_SUBSET,
            50,
            NUM_PIVOTS_STRESS,
        )?;
        for c in d_coords.iter_mut().take(dim) {
            let mut max = 1.0f64;
            for &v in c.iter().take(n) {
                if v.abs() > max {
                    max = v.abs();
                }
            }
            for v in c.iter_mut().take(n) {
                *v /= max;
            }
            for v in c.iter_mut().take(n) {
                *v = 1e-6f64.mul_add(crate::cutil::drand48() - 0.5, *v);
            }
            orthog1(&mut c[..n]);
        }
    } else {
        have_pinned = init_layout(g, n, dim, d_coords, nodes);
    }
    graphviz_pure_core::verbose_msg!(": {} sec", fmt_f(elapsed_sec(), 2));
    if n == 1 || maxi == 0 {
        return Ok(0);
    }
    graphviz_pure_core::diag::verbose_phase("Setting up stress function");

    let mut coords: Vec<Vec<f32>> = (0..dim)
        .map(|i| d_coords[i][..n].iter().map(|&v| v as f32).collect())
        .collect();
    let constant_term: f32 = (n as f32) * ((n - 1) as f32) / 2.0;
    let lap_length = n * (n + 1) / 2;
    if exp == 2 {
        square_vec(&mut lap2);
    }
    invert_vec(&mut lap2);
    let mut degrees = vec![0.0f64; n];
    let mut count = 0;
    for i in 0..n.saturating_sub(1) {
        let mut degree = 0.0f64;
        count += 1;
        for j in 1..(n - i) {
            let val = lap2[count];
            degree += f64::from(val);
            degrees[i + j] -= f64::from(val);
            count += 1;
        }
        degrees[i] -= degree;
    }
    {
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
    graphviz_pure_core::diag::verbose_phase("Solving model: ");
    let mut converged = false;
    let mut iterations = 0i64;
    while iterations < maxi && !converged {
        for d in degrees.iter_mut() {
            *d = 0.0;
        }
        if exp == 2 {
            sqrt_vecf(&lap2, &mut lap1);
        }
        let mut count = 0;
        for i in 0..n - 1 {
            let len = n - i - 1;
            for v in dist_accumulator.iter_mut().take(len) {
                *v = 0.0;
            }
            for c in coords.iter().take(dim) {
                let ci = c[i];
                for x in 0..len {
                    let tmp = (-1.0f32).mul_add(c[i + 1 + x], ci);
                    dist_accumulator[x] = tmp.mul_add(tmp, dist_accumulator[x]);
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
            if exp == 2 {
                for j in 0..len {
                    lap1[count] *= dist_accumulator[j];
                    let val = lap1[count];
                    degree += f64::from(val);
                    degrees[i + j + 1] -= f64::from(val);
                    count += 1;
                }
            } else {
                for j in 0..len {
                    lap1[count] = dist_accumulator[j];
                    let val = lap1[count];
                    degree += f64::from(val);
                    degrees[i + j + 1] -= f64::from(val);
                    count += 1;
                }
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
        new_stress += f64::from(constant_term);
        for k in 0..dim {
            right_mult_with_vector_ff(&lap2, n, &coords[k], &mut tmp_coords);
            new_stress -= vectors_inner_productf(n, &coords[k], &tmp_coords);
        }
        {
            let diff = old_stress - new_stress;
            let change = diff.abs();
            converged = change / old_stress < params.epsilon || new_stress < params.epsilon;
        }
        old_stress = new_stress;
        for k in 0..dim {
            if have_pinned {
                tmp_coords.copy_from_slice(&coords[k]);
                conjugate_gradient_mkernel(&lap2, &mut tmp_coords, &mut b[k], n, TOLERANCE_CG, n)?;
                for i in 0..n {
                    if !is_fixed(g, nodes[i]) {
                        coords[k][i] = tmp_coords[i];
                    }
                }
            } else {
                let (ck, bk) = (&mut coords[k], &mut b[k]);
                conjugate_gradient_mkernel(&lap2, ck, bk, n, TOLERANCE_CG, n)?;
            }
        }
        if verbose_on() && iterations % 5 == 0 {
            let nl = if iterations.wrapping_add(5) % 50 == 0 {
                "\n"
            } else {
                ""
            };
            fprintf_stderr(&format!("{} {nl}", fmt_f(new_stress, 3)));
        }
        iterations += 1;
    }
    if verbose_on() {
        fprintf_stderr(&format!(
            "\nfinal e = {} {iterations} iterations {} sec\n",
            fmt_f(compute_stressf(&coords, &lap2, dim, n, exp), 6),
            fmt_f(elapsed_sec(), 2)
        ));
    }
    for i in 0..dim {
        for j in 0..n {
            d_coords[i][j] = f64::from(coords[i][j]);
        }
    }
    Ok(iterations)
}
