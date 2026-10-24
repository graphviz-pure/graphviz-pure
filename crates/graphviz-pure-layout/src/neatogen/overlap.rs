//! Prism overlap removal, ported from `lib/neatogen/overlap.c`
//! (`remove_overlap`, `OverlapSmoother_new`, `get_overlap_graph`,
//! `ideal_distance_avoid_overlap`, `overlap_scaling`) and `fdpAdjust`
//! (`lib/neatogen/adjust.c`), as built with GTS and SFDP.

use super::adjust::{AdjustData, DFLT_MARGIN, sep_factor};
use super::delaunay::delaunay_tri;
use crate::common::types::{LGraph, Sub};
use crate::common::utils::map_bool;
use crate::cutil::qsort_by;
use crate::sfdpgen::post_process::{RelativePositionConstraints, StressMajorizationSmoother};
use crate::sparse::{Format, MACHINEACC, SparseMatrix, distance};
use graphviz_pure_core::diag::{fmt_f, fmt_g, fprintf_stderr, verbose_on};
use graphviz_pure_core::math;

/// `call_tri(n, x)`: Delaunay graph plus the diagonal, symmetrized.
pub fn call_tri(n: usize, x: &[f64]) -> SparseMatrix {
    let xv: Vec<f64> = (0..n).map(|i| x[2 * i]).collect();
    let yv: Vec<f64> = (0..n).map(|i| x[2 * i + 1]).collect();
    let edges = if n > 2 {
        delaunay_tri(&xv, &yv)
    } else {
        Vec::new()
    };
    let mut a = SparseMatrix::new(n, n, true, Format::Coord);
    for (i, j) in edges {
        a.coordinate_form_add_entry(i as i32, j as i32, 1.0);
    }
    if n == 2 {
        a.coordinate_form_add_entry(0, 1, 1.0);
    }
    for i in 0..n {
        a.coordinate_form_add_entry(i as i32, i as i32, 1.0);
    }
    let mut b = a.from_coordinate_format();
    b.symmetrize(false)
}

#[derive(Clone, Copy)]
struct ScanPoint {
    node: usize,
    x: f64,
    close: bool,
}

fn comp_scan(p: &ScanPoint, q: &ScanPoint) -> i32 {
    if p.x > q.x {
        1
    } else if p.x < q.x {
        -1
    } else if p.node > q.node {
        1
    } else if p.node < q.node {
        -1
    } else {
        0
    }
}

/// `get_overlap_graph(dim, n, x, width, check_overlap_only)`.
fn get_overlap_graph(
    dim: usize,
    n: usize,
    x: &[f64],
    width: &[f64],
    check_only: bool,
) -> SparseMatrix {
    let mut a = SparseMatrix::new(n, n, true, Format::Coord);
    let mut sx: Vec<ScanPoint> = Vec::with_capacity(2 * n);
    for i in 0..n {
        sx.push(ScanPoint {
            node: i,
            x: x[i * dim] - width[i * dim],
            close: false,
        });
        sx.push(ScanPoint {
            node: i + n,
            x: x[i * dim] + width[i * dim],
            close: true,
        });
    }
    qsort_by(&mut sx, comp_scan);
    let mut sy: Vec<ScanPoint> = Vec::with_capacity(2 * n);
    for i in 0..n {
        sy.push(ScanPoint {
            node: i,
            x: x[i * dim + 1] - width[i * dim + 1],
            close: false,
        });
    }
    for i in 0..n {
        sy.push(ScanPoint {
            node: i,
            x: x[i * dim + 1] + width[i * dim + 1],
            close: true,
        });
    }
    // Red-black tree of y scan points, ordered by (x, node); equal keys are
    // inserted to the right. Stored as indices into `sy`.
    let mut tree: Vec<usize> = Vec::new();
    'scan: for p in &sx {
        let k = p.node % n;
        if !p.close {
            for idx in [k, k + n] {
                let pos = tree.partition_point(|&t| comp_scan(&sy[t], &sy[idx]) <= 0);
                tree.insert(pos, idx);
            }
        } else {
            let Some(pos) = tree.iter().position(|&t| t == k + n) else {
                continue;
            };
            let ii = sy[tree[pos]].node;
            let (bsta, bsto) = (sy[ii].x, sy[ii + n].x);
            for q in (0..pos).rev() {
                let neighbor = sy[tree[q]].node % n;
                let (bbsta, bbsto) = (sy[neighbor].x, sy[neighbor + n].x);
                if neighbor != k
                    && (0.5 * (bsta + bsto) - 0.5 * (bbsta + bbsto)).abs()
                        < 0.5f64.mul_add(bsto - bsta, 0.5 * (bbsto - bbsta))
                {
                    a.coordinate_form_add_entry(neighbor as i32, k as i32, 1.0);
                    if check_only {
                        break 'scan;
                    }
                }
            }
            tree.remove(pos);
        }
    }
    let mut b = a.from_coordinate_format();
    let c = b.symmetrize(false);
    graphviz_pure_core::verbose_msg!("found {} clashes\n", c.nz);
    c
}

/// `ideal_distance_avoid_overlap(dim, A, x, width, ideal, &tmax, &tmin)`.
fn ideal_distance_avoid_overlap(
    dim: usize,
    a: &SparseMatrix,
    x: &[f64],
    width: &[f64],
    ideal: &mut [f64],
) -> (f64, f64) {
    let (expandmax, expandmin) = (1.5f64, 1.0f64);
    let mut tmax = 0.0f64;
    let mut tmin = 1.0e10f64;
    for i in 0..a.m {
        for j in a.ia[i]..a.ia[i + 1] {
            let j = j as usize;
            let jj = a.ja[j] as usize;
            if jj == i {
                continue;
            }
            let dist = distance(x, dim, i, jj);
            let dx = (x[i * dim] - x[jj * dim]).abs();
            let dy = (x[i * dim + 1] - x[jj * dim + 1]).abs();
            let wx = width[i * dim] + width[jj * dim];
            let wy = width[i * dim + 1] + width[jj * dim + 1];
            if dx < MACHINEACC * wx && dy < MACHINEACC * wy {
                ideal[j] = math::hypot(wx, wy);
                tmax = 2.0;
            } else {
                let mut t = if dx < MACHINEACC * wx {
                    wy / dy
                } else if dy < MACHINEACC * wy {
                    wx / dx
                } else {
                    (wx / dx).min(wy / dy)
                };
                if t > 1.0 {
                    t = t.max(1.001);
                }
                tmax = tmax.max(t);
                tmin = tmin.min(t);
                t = expandmax.min(t);
                t = expandmin.max(t);
                ideal[j] = if t > 1.0 { t * dist } else { -t * dist };
            }
        }
    }
    (tmax, tmin)
}

fn scale_coord(x: &mut [f64], scale: f64) {
    for v in x.iter_mut() {
        *v *= scale;
    }
}

fn has_overlap(dim: usize, m: usize, x: &[f64], width: &[f64]) -> bool {
    get_overlap_graph(dim, m, x, width, true).nz > 0
}

/// `overlap_scaling(dim, m, x, width, scale_sta, scale_sto, epsilon, maxiter)`.
fn overlap_scaling(
    dim: usize,
    m: usize,
    x: &mut [f64],
    width: &[f64],
    mut scale_sta: f64,
    mut scale_sto: f64,
    epsilon: f64,
    maxiter: i32,
) {
    let n = dim * m;
    if scale_sta <= 0.0 {
        scale_sta = 0.0;
    } else {
        scale_coord(&mut x[..n], scale_sta);
        if !has_overlap(dim, m, x, width) {
            graphviz_pure_core::verbose_msg!(" shrinking with {} works\n", fmt_f(scale_sta, 6));
            return;
        }
        scale_coord(&mut x[..n], 1.0 / scale_sta);
    }
    if scale_sto < 0.0 {
        scale_sto = if scale_sta == 0.0 { epsilon } else { scale_sta };
        scale_coord(&mut x[..n], scale_sto);
        loop {
            scale_sto *= 2.0;
            scale_coord(&mut x[..n], 2.0);
            if !has_overlap(dim, m, x, width) {
                break;
            }
        }
        scale_coord(&mut x[..n], 1.0 / scale_sto);
    }
    let mut scale_best = scale_sto;
    let mut iter = 0;
    loop {
        let cont = iter < maxiter && scale_sto - scale_sta > epsilon;
        iter += 1;
        if !cont {
            break;
        }
        graphviz_pure_core::verbose_msg!(
            "in overlap_scaling iter={iter}, maxiter={maxiter}, scaling bracket: {{{},{}}}\n",
            fmt_f(scale_sta, 6),
            fmt_f(scale_sto, 6)
        );
        let scale = 0.5 * (scale_sta + scale_sto);
        scale_coord(&mut x[..n], scale);
        let overlap = has_overlap(dim, m, x, width);
        scale_coord(&mut x[..n], 1.0 / scale);
        if overlap {
            scale_sta = scale;
        } else {
            scale_best = scale;
            scale_sto = scale;
        }
    }
    scale_coord(&mut x[..n], scale_best);
}

/// `OverlapSmoother_new(...)`; `None` when the layout was only rescaled.
fn overlap_smoother_new(
    a: &SparseMatrix,
    m: usize,
    dim: usize,
    x: &mut [f64],
    width: &[f64],
    neighborhood_only: bool,
    max_overlap: &mut f64,
    min_overlap: &mut f64,
    shrink: bool,
) -> StressMajorizationSmoother {
    let mut b = call_tri(m, x);
    if !neighborhood_only {
        let c = get_overlap_graph(dim, m, x, width, false);
        b = b.add(&c);
    }
    let mut lwd = b.clone();
    let mut ideal = lwd.a.take().expect("real matrix");
    let (tmax, tmin) = ideal_distance_avoid_overlap(dim, &b, x, width, &mut ideal);
    lwd.a = Some(ideal);
    *max_overlap = tmax;
    *min_overlap = tmin;
    let mut sm = StressMajorizationSmoother {
        lw: b,
        lwd,
        lambda: vec![0.0; m],
        tol_cg: 0.01,
        maxit_cg: (a.m as f64).sqrt().floor(),
    };
    if *max_overlap < 1.0 && shrink {
        let scale_sta = (*max_overlap * 1.0001).min(1.0);
        graphviz_pure_core::verbose_msg!(
            " no overlap (overlap = {}), rescale to shrink\n",
            fmt_f(*max_overlap - 1.0, 6)
        );
        overlap_scaling(dim, m, x, width, scale_sta, 1.0, 0.0001, 15);
        *max_overlap = 1.0;
        return sm;
    }
    let (iw, jw) = (sm.lw.ia.clone(), sm.lw.ja.clone());
    let w = sm.lw.a.as_mut().unwrap();
    let d = sm.lwd.a.as_mut().unwrap();
    for i in 0..m {
        let mut diag_d = 0.0;
        let mut diag_w = 0.0;
        let mut jdiag = None;
        for j in iw[i]..iw[i + 1] {
            let j = j as usize;
            if jw[j] as usize == i {
                jdiag = Some(j);
                continue;
            }
            if d[j] > 0.0 {
                w[j] = -100.0 / d[j] / d[j];
            } else {
                w[j] = -1.0 / d[j] / d[j];
                d[j] = -d[j];
            }
            let dist = d[j];
            diag_w += w[j];
            d[j] = w[j] * dist;
            diag_d += d[j];
        }
        let jd = jdiag.expect("diagonal");
        w[jd] = -diag_w;
        d[jd] = -diag_d;
    }
    sm
}

fn average_edge_length(a: &mut SparseMatrix, dim: usize, coord: &[f64]) -> f64 {
    if a.ia[a.m] == 0 {
        return 1.0;
    }
    let mut dist = 0.0;
    for i in 0..a.m {
        for j in a.ia[i]..a.ia[i + 1] {
            let c = a.ja[j as usize] as usize;
            let mut d = 0.0f64;
            for k in 0..dim {
                // C indexes coord[dim * ja[j]] without `+ k`.
                let t = coord[dim * i + k] - coord[dim * c];
                d = t.mul_add(t, d);
            }
            dist += d.sqrt();
        }
    }
    dist / f64::from(a.ia[a.m])
}

/// scale_to_edge_length's `-v` line.
fn verbose_edge_length(dist: f64, avg_label_size: f64) {
    graphviz_pure_core::verbose_msg!(
        "avg edge len={} avg_label-size= {}\n",
        fmt_f(dist, 6),
        fmt_f(avg_label_size, 6)
    );
}

/// `print_bounding_box(n, dim, x)`.
fn print_bounding_box(n: usize, dim: usize, x: &[f64]) {
    let mut xmin: Vec<f64> = x.iter().take(dim).copied().collect();
    let mut xmax = xmin.clone();
    for p in x.chunks(dim.max(1)).take(n) {
        for ((lo, hi), &v) in xmin.iter_mut().zip(xmax.iter_mut()).zip(p) {
            *lo = lo.min(v);
            *hi = hi.max(v);
        }
    }
    let mut m = String::from("bounding box = \n");
    for (lo, hi) in xmin.iter().zip(&xmax) {
        m.push_str(&format!("{{{},{}}}, ", fmt_f(*lo, 6), fmt_f(*hi, 6)));
    }
    m.push('\n');
    fprintf_stderr(&m);
}

/// `remove_overlap(dim, A, x, label_sizes, ntry, initial_scaling, ELSCHEME_NONE, ...)`.
pub fn remove_overlap(
    dim: usize,
    a: &mut SparseMatrix,
    x: &mut [f64],
    label_sizes: &[f64],
    ntry: i32,
    initial_scaling: f64,
    do_shrinking: bool,
) {
    remove_overlap_el(
        dim,
        a,
        x,
        label_sizes,
        ntry,
        initial_scaling,
        None,
        do_shrinking,
    );
}

/// `remove_overlap(dim, A, x, label_sizes, ntry, initial_scaling,
/// edge_labeling_scheme, n_constr_nodes, constr_nodes, A_constr, do_shrinking)`;
/// `constraints` carries the edge-labeling scheme, nodes and `A_constr`.
pub fn remove_overlap_el(
    dim: usize,
    a: &mut SparseMatrix,
    x: &mut [f64],
    label_sizes: &[f64],
    ntry: i32,
    initial_scaling: f64,
    constraints: Option<&RelativePositionConstraints>,
    do_shrinking: bool,
) {
    let epsilon = 0.005;
    let large = 100_000.0;
    let mut res = large;
    let mut max_overlap = 0.0;
    let mut min_overlap = 999.0;
    let mut neighborhood_only = true;
    let mut shrink = false;
    let m = a.m;
    if initial_scaling < 0.0 {
        let mut avg = 0.0;
        for i in 0..m {
            avg += label_sizes[i * dim] + label_sizes[i * dim + 1];
        }
        avg /= m as f64;
        let target = -initial_scaling * avg;
        let dist = average_edge_length(a, dim, x);
        verbose_edge_length(dist, target);
        let s = target / dist.max(MACHINEACC);
        for v in x.iter_mut().take(dim * m) {
            *v *= s;
        }
    } else if initial_scaling > 0.0 {
        let dist = average_edge_length(a, dim, x);
        verbose_edge_length(dist, initial_scaling);
        let s = initial_scaling / dist.max(MACHINEACC);
        for v in x.iter_mut().take(dim * m) {
            *v *= s;
        }
    }
    if ntry == 0 {
        return;
    }
    let constraints =
        constraints.filter(|c| c.edge_labeling_scheme != 0 && !c.constr_nodes.is_empty());
    let has_penalty_terms = constraints.is_some();
    let mut i = 0;
    let report = |i: i32, neighborhood_only: bool, max_overlap: f64, min_overlap: f64| {
        graphviz_pure_core::verbose_msg!(
            "overlap removal neighbors only?= {} iter -- {i}, overlap factor = {} underlap factor = {}\n",
            i32::from(neighborhood_only),
            fmt_g(max_overlap - 1.0, 6),
            fmt_g(min_overlap, 6)
        );
    };
    while i < ntry {
        if verbose_on() {
            print_bounding_box(m, dim, x);
        }
        let mut sm = overlap_smoother_new(
            a,
            m,
            dim,
            x,
            label_sizes,
            neighborhood_only,
            &mut max_overlap,
            &mut min_overlap,
            shrink,
        );
        report(i, neighborhood_only, max_overlap, min_overlap);
        // check_convergence(max_overlap, res, has_penalty_terms, epsilon)
        let converged = if has_penalty_terms {
            res < epsilon
        } else {
            max_overlap <= 1.0
        };
        if converged {
            if !neighborhood_only {
                break;
            }
            res = large;
            neighborhood_only = false;
            if do_shrinking {
                shrink = true;
            }
            i += 1;
            continue;
        }
        res = sm.smooth_with(dim, x, 1, constraints);
        graphviz_pure_core::verbose_msg!("res = {}\n", fmt_f(res, 6));
        i += 1;
    }
    report(i, neighborhood_only, max_overlap, min_overlap);
    if has_penalty_terms {
        remove_overlap_el(dim, a, x, label_sizes, ntry, 0.0, None, do_shrinking);
    }
}

/// `makeMatrix(g)`: assigns `ND_id` in node order and returns the adjacency
/// matrix of `sub`, one entry per out-edge weighted by `weight` (default 1).
pub fn make_matrix(g: &mut LGraph<'_>, sub: &Sub) -> SparseMatrix {
    let n = sub.nodes.len();
    for (i, &v) in sub.nodes.iter().enumerate() {
        g.nodes[v].id = i;
    }
    let weight_declared = g.edge_attr_declared("weight");
    let (mut irn, mut jcn, mut val) = (Vec::new(), Vec::new(), Vec::new());
    for &v in &sub.nodes {
        for e in g.outs(sub, v) {
            irn.push(g.nodes[v].id as i32);
            jcn.push(g.nodes[g.edges[e].head].id as i32);
            let w = if weight_declared {
                g.eattr(e, "weight")
                    .and_then(graphviz_pure_core::strtod::scan_lf)
                    .map_or(1.0, |(v, _)| v)
            } else {
                1.0
            };
            val.push(w);
        }
    }
    crate::sparse::from_coordinate_arrays(irn.len(), n, n, &irn, &jcn, Some(&val), true)
}

/// `getSizes(g, pad, ...)`: half sizes in inches plus `pad`, indexed by `ND_id`.
pub fn get_sizes(g: &LGraph<'_>, sub: &Sub, pad: (f64, f64)) -> Vec<f64> {
    let dim = g.ndim;
    let mut sizes = vec![0.0f64; dim * sub.nodes.len()];
    for &v in &sub.nodes {
        let i = g.nodes[v].id;
        sizes[i * dim] = g.nodes[v].width * 0.5 + pad.0;
        sizes[i * dim + 1] = g.nodes[v].height * 0.5 + pad.1;
    }
    sizes
}

/// `fdpAdjust(g, am)`: prism overlap removal on the nodes of `sub`.
pub fn fdp_adjust(g: &mut LGraph<'_>, sub: &Sub, am: &AdjustData) {
    let n = sub.nodes.len();
    let dim = g.ndim;
    let a0 = make_matrix(g, sub);
    let sep = sep_factor(g);
    let pad = if sep.do_add {
        (sep.x / 72.0, sep.y / 72.0)
    } else {
        (DFLT_MARGIN / 72.0, DFLT_MARGIN / 72.0)
    };
    let sizes = get_sizes(g, sub, pad);
    let mut pos = vec![0.0f64; dim * n];
    for &v in &sub.nodes {
        let i = g.nodes[v].id;
        for k in 0..dim {
            pos[i * dim + k] = g.nodes[v].pos[k];
        }
    }
    let mut a0c = a0.clone();
    let mut a = if !a0c.is_symmetric(false) || !a0c.is_real() {
        a0.real_adjacency_symmetrized()
    } else {
        let mut t = a0c;
        t.remove_diagonal();
        t
    };
    let shrink = map_bool(g.gattr("overlap_shrink"), true);
    remove_overlap(dim, &mut a, &mut pos, &sizes, am.value, am.scaling, shrink);
    for &v in &sub.nodes {
        let i = g.nodes[v].id;
        for k in 0..dim {
            g.nodes[v].pos[k] = pos[i * dim + k];
        }
    }
}
