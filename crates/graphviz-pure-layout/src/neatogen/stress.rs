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
use graphviz_pure_core::math;

pub const TOLERANCE_CG: f64 = 1e-3;
pub const DFLT_ITERATIONS: i64 = 200;
pub const DFLT_TOLERANCE: f64 = 1e-4;
pub const OPT_SMART_INIT: i32 = 0x4;
pub const OPT_EXP_FLAG: i32 = 0x3;
pub const NUM_PIVOTS_STRESS: usize = 40;

/// Largest `n` that keeps a second copy of `sqrt(lap2)`.
///
/// The copy is one packed triangle of `f32`. On the musl release build,
/// n = 1000 peaked at 16,848 KiB, over 1.10 × the 1.0 peak (16,359 KiB).
/// n = 500 peaked at 9,444 KiB, under its cap (10,168 KiB). See
/// `docs/perf/1.x-10x.md`.
const SQRT_CACHE_MAX_N: usize = 500;

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
                math::fma(-2.0, gm_inv[i][j], gm_inv[i][i] + gm_inv[j][j]) as f32
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

/// The rebuild's clamp, as one `f32` test.
///
/// `>= f32::MAX` or `< 0` becomes `0`. A `NaN` fails both comparisons and
/// stays a `NaN`.
fn clamp_stress_dist(dist: f32) -> f32 {
    if dist >= f32::MAX || dist < 0.0 {
        0.0
    } else {
        dist
    }
}

/// Off-diagonals of a packed triangle are all `>= 0`.
///
/// Diagonals are skipped. A stress Laplacian stores a negative degree sum
/// there, and the rebuild overwrites it without reading the square root.
/// A `NaN` or negative off-diagonal must not use the cached root: `sqrt_vecf`
/// leaves that `lap1` entry alone, and the next iteration multiplies whatever
/// value the previous multiply left behind.
fn offdiag_sqrt_is_stable(packed: &[f32], n: usize) -> bool {
    let mut rows = packed;
    let mut width = n;
    while width > 1 {
        let Some((_, rest)) = rows.split_first() else {
            return false;
        };
        let len = width.wrapping_sub(1);
        if rest.len() < len {
            return false;
        }
        let (off, next) = rest.split_at(len);
        if off.iter().any(|v| v.is_nan() || *v < 0.0) {
            return false;
        }
        rows = next;
        width = len;
    }
    true
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
            sum = math::fma((dij - dist) * (dij - dist), l, sum);
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
                *v = math::fma(1e-6, crate::cutil::drand48() - 0.5, *v);
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
    // `lap2` is not written again. One lower triangle serves every `lap2`
    // mat-vec in this solve, including the ones inside CG. For `stresswt=2`
    // the rebuild used to take `sqrt` of that triangle on every iteration.
    let cache_sqrt = exp == 2 && n <= SQRT_CACHE_MAX_N && offdiag_sqrt_is_stable(&lap2, n);
    #[cfg(all(feature = "parallel", not(target_family = "wasm")))]
    let lap2_lower = if super::matvec_parallel::use_parallel(n) {
        Some(super::matvec_parallel::pack_lower(&lap2, n))
    } else {
        None
    };

    let mut b: Vec<Vec<f32>> = vec![vec![0.0f32; n]; dim];
    let mut tmp_coords = vec![0.0f32; n];
    let mut dist_accumulator = vec![0.0f32; n];
    // One block either way. The second half holds `sqrt(lap2)` when the
    // cache is on; `heap_blocks` stays the same as a single triangle.
    let mut lap_buf = vec![
        0.0f32;
        if cache_sqrt {
            lap_length.saturating_mul(2)
        } else {
            lap_length
        }
    ];
    let (lap1, lap_sqrt) = lap_buf.split_at_mut(lap_length);
    if cache_sqrt {
        sqrt_vecf(&lap2, lap_sqrt);
    }
    let mut old_stress = f64::MAX;
    graphviz_pure_core::diag::verbose_phase("Solving model: ");
    let mut converged = false;
    let mut iterations = 0i64;
    while iterations < maxi && !converged {
        for d in degrees.iter_mut() {
            *d = 0.0;
        }
        if exp == 2 && !cache_sqrt {
            sqrt_vecf(&lap2, lap1);
        }
        let mut count: usize = 0;
        for i in 0..n - 1 {
            let len = n - i - 1;
            for v in dist_accumulator.iter_mut().take(len) {
                *v = 0.0;
            }
            // Each accumulator receives dimension 0's two FMAs before dimension
            // 1's. Within one dimension the lanes are independent, so the inner
            // loop may vectorize without reordering any sum.
            for c in coords.iter().take(dim) {
                let (left, right) = c.split_at(i.wrapping_add(1));
                let ci = left.last().copied().unwrap_or(0.0);
                for (acc, &cx) in dist_accumulator.iter_mut().zip(&right[..len]) {
                    let tmp = (-1.0f32).mul_add(cx, ci);
                    *acc = tmp.mul_add(tmp, *acc);
                }
            }
            invert_sqrt_vec(&mut dist_accumulator[..len]);
            // One range per row. `lap_sqrt[count]` on every off-diagonal stayed
            // a bounds check, and that check put the instructions back.
            let off_start = count.wrapping_add(1);
            count = off_start.wrapping_add(len);
            let off = &mut lap1[off_start..count];
            let mut degree = 0.0f64;
            if exp == 2 && cache_sqrt {
                let scales = &lap_sqrt[off_start..count];
                for ((slot, &scale), dist) in
                    off.iter_mut().zip(scales).zip(dist_accumulator.iter_mut())
                {
                    let dist_c = clamp_stress_dist(*dist);
                    // `sqrt_vecf` then `*=` is one `f32` multiply, not an FMA.
                    let val = scale * dist_c;
                    *slot = val;
                    *dist = val;
                }
            } else if exp == 2 {
                // `sqrt_vecf` just wrote this row. Read the root, then store.
                for (slot, dist) in off.iter_mut().zip(dist_accumulator.iter_mut()) {
                    let dist_c = clamp_stress_dist(*dist);
                    let val = *slot * dist_c;
                    *slot = val;
                    *dist = val;
                }
            } else {
                for (slot, dist) in off.iter_mut().zip(dist_accumulator.iter_mut()) {
                    let val = clamp_stress_dist(*dist);
                    *slot = val;
                    *dist = val;
                }
            }
            for (j, &val) in dist_accumulator.iter().take(len).enumerate() {
                let weight = f64::from(val);
                degree += weight;
                degrees[i + j + 1] -= weight;
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
            right_mult_with_vector_ff(lap1, n, &coords[k], &mut b[k]);
        }
        let mut new_stress = 0.0f64;
        for k in 0..dim {
            new_stress += vectors_inner_productf(n, &coords[k], &b[k]);
        }
        new_stress *= 2.0;
        new_stress += f64::from(constant_term);
        for k in 0..dim {
            #[cfg(all(feature = "parallel", not(target_family = "wasm")))]
            if let Some(lower) = lap2_lower.as_deref() {
                super::matvec_parallel::right_mult_prepared(
                    &lap2,
                    lower,
                    n,
                    &coords[k],
                    &mut tmp_coords,
                );
                new_stress -= vectors_inner_productf(n, &coords[k], &tmp_coords);
                continue;
            }
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
                #[cfg(all(feature = "parallel", not(target_family = "wasm")))]
                if let Some(lower) = lap2_lower.as_deref() {
                    super::conjgrad::conjugate_gradient_mkernel_prepared(
                        &lap2,
                        lower,
                        &mut tmp_coords,
                        &mut b[k],
                        n,
                        TOLERANCE_CG,
                        n,
                    )?;
                } else {
                    conjugate_gradient_mkernel(
                        &lap2,
                        &mut tmp_coords,
                        &mut b[k],
                        n,
                        TOLERANCE_CG,
                        n,
                    )?;
                }
                #[cfg(not(all(feature = "parallel", not(target_family = "wasm"))))]
                conjugate_gradient_mkernel(&lap2, &mut tmp_coords, &mut b[k], n, TOLERANCE_CG, n)?;
                for i in 0..n {
                    if !is_fixed(g, nodes[i]) {
                        coords[k][i] = tmp_coords[i];
                    }
                }
            } else {
                let (ck, bk) = (&mut coords[k], &mut b[k]);
                #[cfg(all(feature = "parallel", not(target_family = "wasm")))]
                if let Some(lower) = lap2_lower.as_deref() {
                    super::conjgrad::conjugate_gradient_mkernel_prepared(
                        &lap2,
                        lower,
                        ck,
                        bk,
                        n,
                        TOLERANCE_CG,
                        n,
                    )?;
                } else {
                    conjugate_gradient_mkernel(&lap2, ck, bk, n, TOLERANCE_CG, n)?;
                }
                #[cfg(not(all(feature = "parallel", not(target_family = "wasm"))))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offdiag_sqrt_is_stable_ignores_diagonals_and_rejects_nan() {
        // n = 3 packed: d00, a, b, d11, c, d22.
        let mut packed = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert!(offdiag_sqrt_is_stable(&packed, 3));
        packed[0] = f32::NAN;
        packed[3] = -4.0;
        packed[5] = f32::NEG_INFINITY;
        assert!(offdiag_sqrt_is_stable(&packed, 3));
        packed[1] = f32::NAN;
        assert!(!offdiag_sqrt_is_stable(&packed, 3));
        packed[1] = -0.5;
        assert!(!offdiag_sqrt_is_stable(&packed, 3));
        packed[1] = -0.0;
        assert!(offdiag_sqrt_is_stable(&packed, 3));
        assert!(offdiag_sqrt_is_stable(&[], 0));
        assert!(offdiag_sqrt_is_stable(&[f32::NAN], 1));
    }

    #[test]
    fn cached_sqrt_matches_sqrt_then_multiply() {
        let sources = [
            0.0f32,
            -0.0,
            1.0,
            2.0,
            1.0e-20,
            1.0e20,
            f32::MIN_POSITIVE,
            3.5,
            f32::MAX,
            f32::INFINITY,
        ];
        let dists = [
            0.0f32,
            -0.0,
            1.0,
            0.5,
            2.0,
            f32::MAX,
            f32::MIN_POSITIVE,
            -1.0,
            f32::INFINITY,
            f32::NAN,
        ];
        for &source in &sources {
            let mut once = [0.0f32];
            sqrt_vecf(&[source], &mut once);
            for &dist in &dists {
                let mut each = [0.0f32];
                sqrt_vecf(&[source], &mut each);
                each[0] *= dist;
                assert_eq!(
                    (once[0] * dist).to_bits(),
                    each[0].to_bits(),
                    "source {source} dist {dist}"
                );
            }
        }
    }
}
