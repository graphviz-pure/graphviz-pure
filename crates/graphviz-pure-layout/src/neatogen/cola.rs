//! `mode=ipsep`: stress majorization with separation constraints solved by
//! VPSC gradient projection (`constrained_majorization_ipsep.c`
//! `stress_majorization_cola`), plus `cluster_map` from `neatoinit.c`.

use super::NeatoParams;
use super::conjgrad::conjugate_gradient_mkernel;
use super::graphdata::VtxData;
use super::matrix_ops::*;
use super::quad_prog_vpsc::{
    CMajEnvVpsc, ClusterData, IpsepOptions, MAX_CYCLE_ERRORS, generate_nonoverlap_constraints,
    init_cmaj_vpsc_full, removeoverlaps,
};
use super::stress::*;
use super::{MODEL_CIRCUIT, MODEL_MDS, MODEL_SUBSET, SolverError};
use crate::common::types::{LGraph, Sub};

const LOCAL_CONSTR_MAJOR_ITERATIONS: i32 = 1000;

/// `cluster_map(mastergraph, g)`: node indices (within `sub`) of each
/// top-level cluster of the root graph, and the nodes in no cluster.
pub fn cluster_map(g: &LGraph<'_>, sub: &Sub) -> ClusterData {
    let src = g.src;
    let nn = sub.nodes.len();
    let mut assigned = vec![false; nn];
    let mut cdata = ClusterData::default();
    for si in super::neatoinit::top_level_subgraphs(src) {
        let sg = &src.subgraphs[si];
        if !super::neatoinit::is_a_cluster(sg) {
            continue;
        }
        let members = super::neatoinit::subgraph_nodes(src, si);
        let mut c = Vec::with_capacity(members.len());
        for n in members {
            // C searches `g` for the member and uses agnnodes(g) when it is
            // absent. ipsep always packs components, so `g` is one connected
            // component while the clusters come from the root graph; a member
            // in another component fails C's `bitarray_set` assertion
            // ("out of bounds access") here. Carried on, the index overruns
            // `generateNonoverlapConstraints` under overlap=ipsep, and otherwise
            // names a cluster dummy variable. The port keeps only the members
            // in this component, so each component sees its part of the
            // cluster (possibly none, laid out like an empty cluster).
            let Some(ind) = sub.nodes.iter().position(|&x| x == n) else {
                continue;
            };
            c.push(ind);
            assigned[ind] = true;
        }
        cdata.nvars += c.len();
        cdata.clusters.push(c);
    }
    cdata.bb = vec![Default::default(); cdata.clusters.len()];
    cdata.toplevel = (0..nn).filter(|&i| !assigned[i]).collect();
    cdata
}

/// Not in C, which aborts at the first failed `satisfyVPSC`: whether the two
/// dimensions' projections have failed with "Cycle Error!" more than
/// `MAX_CYCLE_ERRORS` times, so that the stress loop must end; warns if so
/// (#197).
fn too_many_cycle_errors(hor: &CMajEnvVpsc, vrt: &CMajEnvVpsc) -> bool {
    let over = hor.cycle_errors + vrt.cycle_errors > MAX_CYCLE_ERRORS;
    if over {
        graphviz_pure_core::diag::agwarningf(
            "VPSC: too many cycle errors; ending the ipsep layout early\n",
        );
    }
    over
}

/// `stress_majorization_cola(graph, n, d_coords, nodes, dim, model, maxi, opt)`.
#[allow(clippy::too_many_arguments)]
pub fn stress_majorization_cola(
    g: &LGraph<'_>,
    params: &NeatoParams,
    graph: &[VtxData],
    n0: usize,
    d_coords: &mut [Vec<f64>],
    nodes: &[usize],
    dim: usize,
    model: i32,
    maxi: i64,
    opt: &mut IpsepOptions,
) -> Result<i64, SolverError> {
    let orig_n = n0;
    let mut n = n0;
    init_layout(g, n, dim, d_coords, nodes);
    if n == 1 {
        return Ok(0);
    }
    let mut max_edge_len = 0.0f32;
    for v in graph.iter().take(n) {
        let ew = v.ewgts.as_ref().unwrap();
        for j in 1..v.nedges() {
            max_edge_len = if ew[j] > max_edge_len {
                ew[j]
            } else {
                max_edge_len
            };
        }
    }
    if maxi == 0 {
        return Ok(0);
    }
    use graphviz_pure_core::diag::{elapsed_sec, fmt_f, fprintf_stderr, start_timer};
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
    let mut lap2 = dij.unwrap_or_else(|| {
        graphviz_pure_core::verbose_msg!("Calculating shortest paths");
        compute_apsp_packed(graph, n)
    });
    graphviz_pure_core::diag::verbose_phase("Setting initial positions");

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
    for c in d_coords.iter_mut().take(dim) {
        orthog1(&mut c[..n]);
    }
    let y_0 = d_coords[1][0];
    for v in d_coords[1].iter_mut().take(n) {
        *v -= y_0;
    }
    graphviz_pure_core::verbose_msg!(": {} sec", fmt_f(elapsed_sec(), 2));

    let mut lap_length = n + n * (n - 1) / 2;
    square_vec(&mut lap2[..lap_length]);
    invert_vec(&mut lap2[..lap_length]);

    let ncl = opt.clusters.nclusters();
    if ncl > 0 {
        let nn = n + ncl * 2;
        let clap_length = nn + nn * (nn - 1) / 2;
        let mut clap = Vec::with_capacity(clap_length);
        let mut c0 = 0;
        for i in 0..nn {
            for j in 0..nn - i {
                let v = if i < n && j < n - i {
                    let v = lap2[c0];
                    c0 += 1;
                    v
                } else if j == 1 && i % 2 == 1 {
                    let mut v = max_edge_len;
                    v *= v;
                    if v > 0.01f32 {
                        v = 1.0f32 / v;
                    }
                    v
                } else {
                    0.0
                };
                clap.push(v);
            }
        }
        lap2 = clap;
        n = nn;
        lap_length = clap_length;
    }
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
    let mut coords: Vec<Vec<f32>> = (0..dim)
        .map(|i| {
            (0..n)
                .map(|j| {
                    if j < orig_n {
                        d_coords[i][j] as f32
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    let constant_term = (n * (n - 1) / 2) as f32;
    let mut b: Vec<Vec<f32>> = vec![vec![0.0f32; n]; dim];
    let mut tmp_coords = vec![0.0f32; n];
    let mut dist_accumulator = vec![0.0f32; n];
    let mut old_stress = f64::MAX;

    let finish = |coords: &Vec<Vec<f32>>, d_coords: &mut [Vec<f64>]| {
        for i in 0..dim {
            for j in 0..orig_n {
                d_coords[i][j] = f64::from(coords[i][j]);
            }
        }
    };
    let Some(mut hor) = init_cmaj_vpsc_full(n, Some(&lap2), graph, opt, 0) else {
        finish(&coords, d_coords);
        return Err(SolverError);
    };
    let diredges = opt.diredges;
    let Some(mut vrt) = init_cmaj_vpsc_full(n, Some(&lap2), graph, opt, diredges) else {
        finish(&coords, d_coords);
        return Err(SolverError);
    };
    let mut lap1 = vec![0.0f32; lap_length];
    let mut nsize_scale = 0.0f64;
    let mut converged = false;
    let mut iterations = 0i64;
    let mut last_stress = 0.0f64;
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
        new_stress += f64::from(constant_term);
        for k in 0..dim {
            right_mult_with_vector_ff(&lap2, n, &coords[k], &mut tmp_coords);
            new_stress -= vectors_inner_productf(n, &coords[k], &tmp_coords);
        }
        last_stress = new_stress;
        if verbose {
            let nl = if iterations % 10 == 0 { "\n" } else { "" };
            fprintf_stderr(&format!("{} {nl}", fmt_f(new_stress, 3)));
        }
        converged = new_stress < old_stress
            && (new_stress - old_stress).abs() / (old_stress + 1e-10).abs() < params.epsilon;
        old_stress = new_stress;

        // gradually scale up the node sizes in the non-overlap constraints
        if (iterations >= maxi - 1 || converged) && opt.noverlap == 1 && nsize_scale < 0.999 {
            nsize_scale += 0.1;
            graphviz_pure_core::verbose_msg!(
                "nsizescale={},iterations={iterations}\n",
                fmt_f(nsize_scale, 6)
            );
            iterations = 0;
            converged = false;
        }
        // Not in C: where C's `Rectangle` asserts on a non-finite position, stop iterating.
        if opt.noverlap == 1
            && nsize_scale > 0.001
            && !generate_nonoverlap_constraints(
                &mut hor,
                nsize_scale as f32,
                &coords,
                0,
                nsize_scale >= 0.5,
                opt,
            )
        {
            break;
        }
        if hor.m > 0 {
            hor.constrained_majorization_vpsc(&b[0], &mut coords[0], LOCAL_CONSTR_MAJOR_ITERATIONS);
        } else if conjugate_gradient_mkernel(&lap2, &mut coords[0], &mut b[0], n, TOLERANCE_CG, n)
            .is_err()
        {
            finish(&coords, d_coords);
            return Err(SolverError);
        }
        if too_many_cycle_errors(&hor, &vrt) {
            break;
        }
        if opt.noverlap == 1
            && nsize_scale > 0.001
            && !generate_nonoverlap_constraints(
                &mut vrt,
                nsize_scale as f32,
                &coords,
                1,
                false,
                opt,
            )
        {
            break;
        }
        if vrt.m > 0 {
            vrt.constrained_majorization_vpsc(&b[1], &mut coords[1], LOCAL_CONSTR_MAJOR_ITERATIONS);
        } else {
            let _ =
                conjugate_gradient_mkernel(&lap2, &mut coords[1], &mut b[1], n, TOLERANCE_CG, n);
        }
        iterations += 1;
        if too_many_cycle_errors(&hor, &vrt) {
            break;
        }
    }
    if verbose {
        fprintf_stderr(&format!(
            "\nfinal e = {} {iterations} iterations {} sec\n",
            fmt_f(last_stress, 6),
            fmt_f(elapsed_sec(), 2)
        ));
    }
    if opt.noverlap == 2 {
        // C passes only the real nodes, and `initCMajVPSC` still treats the
        // last `2 * nclusters` of them as the cluster boundary variables. With
        // fewer real nodes than that, C's `nv` goes negative and
        // `generateNonoverlapConstraints` segfaults (#185); the port passes
        // the cluster dummy variables too there, as `hor` and `vrt` get.
        let nvars = if orig_n < 2 * ncl { n } else { orig_n };
        removeoverlaps(nvars, &mut coords, opt);
    }
    finish(&coords, d_coords);
    Ok(iterations)
}
