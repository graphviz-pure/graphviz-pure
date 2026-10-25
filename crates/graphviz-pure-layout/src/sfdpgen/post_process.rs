//! `StressMajorizationSmoother_smooth` (`lib/sfdpgen/post_process.c`) and
//! `SparseMatrix_solve` (`lib/sfdpgen/sparse_solve.c`).

use super::spring_electrical::SpringElectricalControl;
use crate::sparse::{SparseMatrix, distance, drand, vector_product};
use graphviz_pure_core::cnum::is_exactly_equal;
use graphviz_pure_core::math;

/// `StressMajorizationSmoother_struct` (normal scheme, no edge-label data).
pub struct StressMajorizationSmoother {
    pub lw: SparseMatrix,
    pub lwd: SparseMatrix,
    pub lambda: Vec<f64>,
    pub tol_cg: f64,
    pub maxit_cg: f64,
}

fn diag_precon_new(a: &SparseMatrix) -> Vec<f64> {
    let v = a.a.as_ref().unwrap();
    let mut diag = vec![1.0f64; a.m];
    for (i, d) in diag.iter_mut().enumerate() {
        for j in a.ia[i]..a.ia[i + 1] {
            let j = j as usize;
            if a.ja[j] == i as i32 && v[j].abs() > 0.0 {
                *d = 1.0 / v[j];
            }
        }
    }
    diag
}

fn conjugate_gradient(
    a: &SparseMatrix,
    precon: &[f64],
    n: usize,
    x: &mut [f64],
    rhs: &[f64],
    tol: f64,
    maxit: f64,
) -> f64 {
    let mut z = vec![0.0f64; n];
    let mut r = vec![0.0f64; n];
    let mut p = vec![0.0f64; n];
    let mut q = vec![0.0f64; n];
    a.multiply_vector(x, &mut r);
    for i in 0..n {
        r[i] = rhs[i] - r[i];
    }
    let res0 = vector_product(n, &r, &r).sqrt() / n as f64;
    let mut res = res0;
    let mut rho_old = 1.0f64;
    let mut iter = 0i64;
    loop {
        let cont = (iter as f64) < maxit && res > tol * res0;
        iter += 1;
        if !cont {
            break;
        }
        for i in 0..n {
            z[i] = r[i] * precon[i];
        }
        let rho = vector_product(n, &r, &z);
        if iter > 1 {
            let beta = rho / rho_old;
            for i in 0..n {
                p[i] = beta.mul_add(p[i], z[i]);
            }
        } else {
            p.copy_from_slice(&z);
        }
        a.multiply_vector(&p, &mut q);
        let alpha = rho / vector_product(n, &p, &q);
        for i in 0..n {
            x[i] = alpha.mul_add(p[i], x[i]);
        }
        for i in 0..n {
            r[i] = (-alpha).mul_add(q[i], r[i]);
        }
        res = vector_product(n, &r, &r).sqrt() / n as f64;
        rho_old = rho;
    }
    res
}

/// `SparseMatrix_solve(A, dim, x0, rhs, tol, maxit)`: the solution replaces `rhs`.
pub fn sparse_matrix_solve(
    a: &SparseMatrix,
    dim: usize,
    x0: &[f64],
    rhs: &mut [f64],
    tol: f64,
    maxit: f64,
) -> f64 {
    let n = a.m;
    let precon = diag_precon_new(a);
    let mut res = 0.0;
    let mut x = vec![0.0f64; n];
    let mut b = vec![0.0f64; n];
    for k in 0..dim {
        for i in 0..n {
            x[i] = x0[i * dim + k];
            b[i] = rhs[i * dim + k];
        }
        res += conjugate_gradient(a, &precon, n, &mut x, &b, tol, maxit);
        for i in 0..n {
            rhs[i * dim + k] = x[i];
        }
    }
    res
}

fn total_distance(m: usize, dim: usize, x: &[f64], y: &[f64]) -> f64 {
    let mut total = 0.0;
    for i in 0..m {
        let mut d = 0.0f64;
        for j in 0..dim {
            let t = y[i * dim + j] - x[i * dim + j];
            d = t.mul_add(t, d);
        }
        total += d.sqrt();
    }
    total
}

impl StressMajorizationSmoother {
    /// `StressMajorizationSmoother_smooth(sm, dim, x, maxit_sm)`.
    pub fn smooth(&mut self, dim: usize, x: &mut [f64], maxit_sm: i32) -> f64 {
        self.smooth_with(dim, x, maxit_sm, None)
    }

    /// `StressMajorizationSmoother_smooth` with `scheme == SM_SCHEME_NORMAL_ELABEL`
    /// when `constraints` is given (edge-label nodes kept near their neighbours).
    pub fn smooth_with(
        &mut self,
        dim: usize,
        x: &mut [f64],
        maxit_sm: i32,
        constraints: Option<&RelativePositionConstraints>,
    ) -> f64 {
        let mut lwdd = self.lwd.clone();
        let m = self.lw.m;
        let x0 = x[..dim * m].to_vec();
        let mut y = vec![0.0f64; dim * m];
        let (id, jd) = (&self.lwd.ia, &self.lwd.ja);
        let d = self.lwd.a.as_ref().unwrap();
        let w = self.lw.a.as_ref().unwrap();
        let mut diff = 1.0f64;
        let tol = 0.001;
        let mut x00: Option<Vec<f64>> = None;
        let lw_el;
        let mut lw = &self.lw;
        if let Some(data) = constraints {
            let (lc, rhs) = get_edge_label_matrix(data, m, dim, x);
            x00 = rhs;
            if let Some(lc) = lc {
                lw_el = self.lw.add(&lc);
                lw = &lw_el;
            }
        }
        let mut iter = 0;
        loop {
            let cont = iter < maxit_sm && diff > tol;
            iter += 1;
            if !cont {
                break;
            }
            {
                let dd = lwdd.a.as_mut().unwrap();
                for i in 0..m {
                    let mut idiag = None;
                    let mut diag = 0.0;
                    for j in id[i]..id[i + 1] {
                        let j = j as usize;
                        let c = jd[j] as usize;
                        if c == i {
                            idiag = Some(j);
                            continue;
                        }
                        let mut dist = distance(x, dim, i, c);
                        if d[j] == 0.0 {
                            dd[j] = 0.0;
                        } else {
                            if dist == 0.0 {
                                let dij = d[j] / w[j];
                                for k in 0..dim {
                                    let r = drand();
                                    x[c * dim + k] =
                                        (0.0001 * (r + 0.0001)).mul_add(dij, x[c * dim + k]);
                                }
                                dist = distance(x, dim, i, c);
                            }
                            dd[j] = d[j] / dist;
                        }
                        diag += dd[j];
                    }
                    dd[idiag.expect("diagonal entry")] = -diag;
                }
            }
            lwdd.multiply_dense(x, &mut y, dim);
            for i in 0..m {
                for j in 0..dim {
                    y[i * dim + j] = self.lambda[i].mul_add(x0[i * dim + j], y[i * dim + j]);
                }
            }
            if let Some(x00) = x00.as_ref().filter(|_| constraints.is_some()) {
                for i in 0..m * dim {
                    y[i] += x00[i];
                }
            }
            sparse_matrix_solve(lw, dim, x, &mut y, self.tol_cg, self.maxit_cg);
            diff = total_distance(m, dim, x, &y) / vector_product(m * dim, x, x).sqrt();
            x[..m * dim].copy_from_slice(&y);
        }
        diff
    }
}

/// `relative_position_constraints_struct` (`lib/neatogen/overlap.h`): edge-label
/// nodes that should sit between their neighbours.
#[derive(Debug, Clone)]
pub struct RelativePositionConstraints {
    pub constr_penalty: f64,
    pub edge_labeling_scheme: i32,
    pub constr_nodes: Vec<usize>,
    pub a_constr: SparseMatrix,
}

/// `get_edge_label_matrix(data, m, dim, x, &Lc, &rhs)`.
fn get_edge_label_matrix(
    data: &RelativePositionConstraints,
    m: usize,
    dim: usize,
    x: &[f64],
) -> (Option<SparseMatrix>, Option<Vec<f64>>) {
    use super::spring_electrical::{
        ELSCHEME_PENALTY, ELSCHEME_PENALTY2, ELSCHEME_STRAIGHTLINE_PENALTY,
        ELSCHEME_STRAIGHTLINE_PENALTY2,
    };
    let scheme = data.edge_labeling_scheme;
    let (ia, ja) = (&data.a_constr.ia, &data.a_constr.ja);
    let cp = data.constr_penalty;
    let (mut irn, mut jcn, mut val) = (Vec::new(), Vec::new(), Vec::new());
    if scheme == ELSCHEME_PENALTY || scheme == ELSCHEME_STRAIGHTLINE_PENALTY {
        for &ii in &data.constr_nodes {
            let jj = ja[ia[ii] as usize] as usize;
            let ll = ja[ia[ii] as usize + 1] as usize;
            if jj == ll {
                continue;
            }
            let mut dist = crate::sparse::distance_cropped(x, dim, jj, ll);
            dist *= dist;
            let mut k = f64::from(ia[ii + 1] - ia[ii]);
            let mut kk = k * k;
            irn.push(ii as i32);
            jcn.push(ii as i32);
            val.push(cp / dist);
            k = cp / (k * dist);
            kk = cp / (kk * dist);
            for j in ia[ii]..ia[ii + 1] {
                irn.push(ii as i32);
                jcn.push(ja[j as usize]);
                val.push(-k);
            }
            for j in ia[ii]..ia[ii + 1] {
                let jj = ja[j as usize];
                irn.push(jj);
                jcn.push(ii as i32);
                val.push(-k);
                for l in ia[ii]..ia[ii + 1] {
                    irn.push(jj);
                    jcn.push(ja[l as usize]);
                    val.push(kk);
                }
            }
        }
        let lc =
            crate::sparse::from_coordinate_arrays(irn.len(), m, m, &irn, &jcn, Some(&val), true);
        (Some(lc), None)
    } else if scheme == ELSCHEME_PENALTY2 || scheme == ELSCHEME_STRAIGHTLINE_PENALTY2 {
        let mut x00 = vec![0.0f64; m * dim];
        for &ii in &data.constr_nodes {
            let jj = ja[ia[ii] as usize] as usize;
            let ll = ja[ia[ii] as usize + 1] as usize;
            let dist = crate::sparse::distance_cropped(x, dim, jj, ll);
            irn.push(ii as i32);
            jcn.push(ii as i32);
            val.push(cp / dist);
            for j in ia[ii]..ia[ii + 1] {
                let jj = ja[j as usize] as usize;
                for l in 0..dim {
                    x00[ii * dim + l] += x[jj * dim + l];
                }
            }
            for l in 0..dim {
                x00[ii * dim + l] *= cp / dist / f64::from(ia[ii + 1] - ia[ii]);
            }
        }
        let lc =
            crate::sparse::from_coordinate_arrays(irn.len(), m, m, &irn, &jcn, Some(&val), true);
        (Some(lc), Some(x00))
    } else {
        (None, None)
    }
}

/// `IDEAL_GRAPH_DIST`, `IDEAL_AVG_DIST`, `IDEAL_POWER_DIST`.
pub const IDEAL_GRAPH_DIST: i32 = 0;
pub const IDEAL_AVG_DIST: i32 = 1;
pub const IDEAL_POWER_DIST: i32 = 2;

fn node_degree(a: &SparseMatrix, i: usize) -> i32 {
    a.ia[i + 1] - a.ia[i]
}

/// `ideal_distance_matrix(A, dim, x)`: ideal edge lengths
/// `|N[i] ∪ N[j]| - |N[i] ∩ N[j]|`, scaled to the current average edge length.
fn ideal_distance_matrix(a: &SparseMatrix, dim: usize, x: &[f64]) -> SparseMatrix {
    let mut dm = a.clone();
    if dm.a.is_none() {
        dm.a = Some(vec![0.0; dm.nz]);
    }
    let (ia, ja) = (&a.ia, &a.ja);
    let m = a.m;
    let d = dm.a.as_mut().unwrap();
    let mut mask = vec![usize::MAX; m];
    for i in 0..m {
        let di = f64::from(node_degree(a, i));
        mask[i] = i;
        for j in ia[i]..ia[i + 1] {
            let c = ja[j as usize] as usize;
            if c == i {
                continue;
            }
            mask[c] = i;
        }
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            if k == i {
                continue;
            }
            let mut len = di + f64::from(node_degree(a, k));
            for l in ia[k]..ia[k + 1] {
                if mask[ja[l as usize] as usize] == i {
                    len -= 1.0;
                }
            }
            d[j as usize] = len;
        }
    }
    let mut sum = 0.0f64;
    let mut sumd = 0.0f64;
    let mut nz = 0i32;
    for i in 0..m {
        for j in ia[i]..ia[i + 1] {
            let c = ja[j as usize] as usize;
            if c == i {
                continue;
            }
            nz += 1;
            sum += distance(x, dim, i, c);
            sumd += d[j as usize];
        }
    }
    sum /= f64::from(nz);
    sumd /= f64::from(nz);
    sum /= sumd;
    for i in 0..m {
        for j in ia[i]..ia[i + 1] {
            if ja[j as usize] as usize == i {
                continue;
            }
            d[j as usize] *= sum;
        }
    }
    dm
}

fn distance_cropped(x: &[f64], dim: usize, i: usize, j: usize) -> f64 {
    crate::sparse::distance_cropped(x, dim, i, j)
}

/// `StressMajorizationSmoother2_new(A, dim, lambda0, x, ideal_dist_scheme)`:
/// stress majorization over distance-1 and distance-2 neighbours.
/// Returns the smoother and its `scaling`, or `None` when a node has no
/// neighbour: C asserts there, and the caller keeps `x` unsmoothed.
pub fn stress_majorization_smoother2_new(
    a: &SparseMatrix,
    dim: usize,
    lambda0: f64,
    x: &[f64],
    ideal_dist_scheme: i32,
) -> Option<(StressMajorizationSmoother, f64)> {
    let (ia, ja) = (&a.ia, &a.ja);
    let m = a.m;
    // C computes (and frees) the ideal distance matrix without using it.
    let _ = ideal_distance_matrix(a, dim, x);
    let mut lambda = vec![lambda0; m];
    let mut avg_dist = vec![0.0f64; m];
    for i in 0..m {
        let mut nz = 0;
        for j in ia[i]..ia[i + 1] {
            let c = ja[j as usize] as usize;
            if c == i {
                continue;
            }
            avg_dist[i] += distance(x, dim, i, c);
            nz += 1;
        }
        // `StressMajorizationSmoother2_new` (`lib/sfdpgen/post_process.c`)
        // asserts `nz > 0`. A node with no neighbour (a self-edge does not
        // count) makes C abort with no output. Keep the unsmoothed positions
        // instead of dividing by zero.
        if nz == 0 {
            return None;
        }
        avg_dist[i] /= f64::from(nz);
    }
    let mut mask = vec![usize::MAX; m];
    let mut iw = vec![0i32; m + 1];
    let mut jw: Vec<i32> = Vec::new();
    let mut w: Vec<f64> = Vec::new();
    let mut jd: Vec<i32> = Vec::new();
    let mut d: Vec<f64> = Vec::new();
    let (mut stop, mut sbot) = (0.0f64, 0.0f64);
    let mut nz = 0usize;
    for i in 0..m {
        mask[i] = i + m;
        let (mut diag_d, mut diag_w) = (0.0f64, 0.0f64);
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            if mask[k] != i + m {
                mask[k] = i + m;
                jw.push(k as i32);
                let dist = match ideal_dist_scheme {
                    IDEAL_GRAPH_DIST => 1.0,
                    IDEAL_AVG_DIST => (avg_dist[i] + avg_dist[k]) * 0.5,
                    _ => math::powf(distance_cropped(x, dim, i, k), 0.4),
                };
                let wv = -1.0 / (dist * dist);
                w.push(wv);
                diag_w += wv;
                jd.push(k as i32);
                let dv = wv * dist;
                d.push(dv);
                stop = dv.mul_add(distance(x, dim, i, k), stop);
                sbot = dv.mul_add(dist, sbot);
                diag_d += dv;
                nz += 1;
            }
        }
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            for l in ia[k]..ia[k + 1] {
                let kl = ja[l as usize] as usize;
                if mask[kl] != i + m {
                    mask[kl] = i + m;
                    let dist = match ideal_dist_scheme {
                        IDEAL_GRAPH_DIST => 2.0,
                        IDEAL_AVG_DIST => {
                            (2.0f64.mul_add(avg_dist[k], avg_dist[i]) + avg_dist[kl]) * 0.5
                        }
                        _ => math::powf(distance_cropped(x, dim, i, kl), 0.4),
                    };
                    jw.push(kl as i32);
                    let wv = -1.0 / (dist * dist);
                    w.push(wv);
                    diag_w += wv;
                    jd.push(kl as i32);
                    let dv = wv * dist;
                    d.push(dv);
                    stop = dv.mul_add(distance(x, dim, kl, k), stop);
                    sbot = dv.mul_add(dist, sbot);
                    diag_d += dv;
                    nz += 1;
                }
            }
        }
        jw.push(i as i32);
        lambda[i] *= -diag_w;
        w.push(-diag_w + lambda[i]);
        jd.push(i as i32);
        d.push(-diag_d);
        nz += 1;
        iw[i + 1] = nz as i32;
    }
    let s = stop / sbot;
    for v in d.iter_mut() {
        *v *= s;
    }
    let lw = SparseMatrix::from_csr(m, m, iw.clone(), jw, Some(w));
    let lwd = SparseMatrix::from_csr(m, m, iw, jd, Some(d));
    Some((
        StressMajorizationSmoother {
            lw,
            lwd,
            lambda,
            tol_cg: 0.01,
            maxit_cg: (m as f64).sqrt().floor(),
        },
        s,
    ))
}

/// `SparseStressMajorizationSmoother_new(A, dim, x)`: stress towards the
/// edge lengths in `A`. Returns the smoother and its `scaling`, or `None`
/// when the scaling is zero.
pub fn sparse_stress_majorization_smoother_new(
    a: &SparseMatrix,
    dim: usize,
    x: &mut [f64],
) -> Option<(StressMajorizationSmoother, f64)> {
    let m = a.m;
    if x[..m * dim].iter().all(|&v| is_exactly_equal(v, 0.0)) {
        for v in x[..m * dim].iter_mut() {
            *v = 72.0 * drand();
        }
    }
    let (ia, ja) = (&a.ia, &a.ja);
    let av = a.a.as_ref().expect("real matrix");
    let lambda = vec![0.0f64; m];
    let mut iw = vec![0i32; m + 1];
    let (mut jw, mut w, mut jd, mut d) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut stop, mut sbot) = (0.0f64, 0.0f64);
    let mut nz = 0usize;
    for i in 0..m {
        let (mut diag_d, mut diag_w) = (0.0f64, 0.0f64);
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            if k != i {
                jw.push(k as i32);
                let dist = av[j as usize];
                w.push(-1.0);
                diag_w += -1.0;
                jd.push(k as i32);
                let dv = -dist;
                d.push(dv);
                stop = dv.mul_add(distance(x, dim, i, k), stop);
                sbot = dv.mul_add(dist, sbot);
                diag_d += dv;
                nz += 1;
            }
        }
        jw.push(i as i32);
        // lambda[i] *= -diag_w with lambda[i] == 0
        w.push(-diag_w + lambda[i]);
        jd.push(i as i32);
        d.push(-diag_d);
        nz += 1;
        iw[i + 1] = nz as i32;
    }
    let s = stop / sbot;
    if s == 0.0 {
        return None;
    }
    for v in d.iter_mut() {
        *v *= s;
    }
    let lw = SparseMatrix::from_csr(m, m, iw.clone(), jw, Some(w));
    let lwd = SparseMatrix::from_csr(m, m, iw, jd, Some(d));
    Some((
        StressMajorizationSmoother {
            lw,
            lwd,
            lambda,
            tol_cg: 0.01,
            maxit_cg: (m as f64).sqrt().floor(),
        },
        s,
    ))
}

/// `TriangleSmoother_new(A, dim, x, use_triangularization)`: stress over the
/// graph plus its Delaunay triangulation (or relative neighbourhood graph).
pub fn triangle_smoother_new(
    a: &SparseMatrix,
    dim: usize,
    x: &[f64],
    use_triangularization: bool,
) -> StressMajorizationSmoother {
    let m = a.m;
    let b = if m > 2 {
        if use_triangularization {
            crate::neatogen::overlap::call_tri(m, x)
        } else {
            crate::neatogen::call_tri::call_tri2(m, dim, x)
        }
    } else {
        a.clone()
    };
    let mut lw = a.add(&b);
    let mut lwd = lw.clone();
    let mut lambda = vec![0.0f64; m];
    let (iw, jw) = (lw.ia.clone(), lw.ja.clone());
    let w = lw.a.as_mut().unwrap();
    let d = lwd.a.as_mut().unwrap();
    let (mut stop, mut sbot) = (0.0f64, 0.0f64);
    for i in 0..m {
        let (mut diag_d, mut diag_w) = (0.0f64, 0.0f64);
        let mut jdiag = None;
        for j in iw[i]..iw[i + 1] {
            let j = j as usize;
            let k = jw[j] as usize;
            if k == i {
                jdiag = Some(j);
                continue;
            }
            let dist = math::powf(distance_cropped(x, dim, i, k), 0.6);
            w[j] = 1.0 / (dist * dist);
            diag_w += w[j];
            d[j] = w[j] * dist;
            stop = d[j].mul_add(distance(x, dim, i, k), stop);
            sbot = d[j].mul_add(dist, sbot);
            diag_d += d[j];
        }
        lambda[i] *= -diag_w;
        let jd = jdiag.expect("diagonal entry");
        w[jd] = -diag_w + lambda[i];
        d[jd] = -diag_d;
    }
    let s = stop / sbot;
    for v in d.iter_mut().take(iw[m] as usize) {
        *v *= s;
    }
    StressMajorizationSmoother {
        lw,
        lwd,
        lambda,
        tol_cg: 0.01,
        maxit_cg: (m as f64).sqrt().floor(),
    }
}

/// `SpringSmoother_struct`.
pub struct SpringSmoother {
    pub d: SparseMatrix,
    pub ctrl: SpringElectricalControl,
}

/// `SpringSmoother_new(A, dim, ctrl, x)`.
pub fn spring_smoother_new(
    a: &SparseMatrix,
    dim: usize,
    ctrl: &SpringElectricalControl,
    x: &[f64],
) -> SpringSmoother {
    let (ia, ja) = (&a.ia, &a.ja);
    let m = a.m;
    let id_m = ideal_distance_matrix(a, dim, x);
    let dd = id_m.a.as_ref().unwrap();
    let mut mask = vec![usize::MAX; m];
    let mut id = vec![0i32; m + 1];
    let mut jd: Vec<i32> = Vec::new();
    let mut d: Vec<f64> = Vec::new();
    for i in 0..m {
        mask[i] = i + m;
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            if mask[k] != i + m {
                mask[k] = i + m;
                jd.push(k as i32);
                d.push(dd[j as usize]);
            }
        }
        for j in ia[i]..ia[i + 1] {
            let k = ja[j as usize] as usize;
            for l in ia[k]..ia[k + 1] {
                let kl = ja[l as usize] as usize;
                if mask[kl] != i + m {
                    mask[kl] = i + m;
                    jd.push(kl as i32);
                    d.push(dd[j as usize] + dd[l as usize]);
                }
            }
        }
        id[i + 1] = jd.len() as i32;
    }
    let mut c = *ctrl;
    c.random_start = false;
    c.multilevels = 1;
    c.step /= 2.0;
    c.maxiter = 20;
    SpringSmoother {
        d: SparseMatrix::from_csr(m, m, id, jd, Some(d)),
        ctrl: c,
    }
}

impl SpringSmoother {
    /// `SpringSmoother_smooth(sm, A, dim, x)`.
    pub fn smooth(&mut self, a: &SparseMatrix, dim: usize, x: &mut [f64]) {
        super::spring_electrical::spring_electrical_spring_embedding(
            dim,
            a,
            &self.d,
            &mut self.ctrl,
            x,
        );
    }
}

/// `post_process_smoothing(dim, A, ctrl, x)`.
pub fn post_process_smoothing(
    dim: usize,
    a: &mut SparseMatrix,
    ctrl: &SpringElectricalControl,
    x: &mut [f64],
) {
    use super::spring_electrical::*;
    match ctrl.smoothing {
        SMOOTHING_RNG | SMOOTHING_TRIANGLE => {
            if a.m > 2 {
                let mut sm = triangle_smoother_new(a, dim, x, ctrl.smoothing != SMOOTHING_RNG);
                sm.smooth(dim, x, 50);
            }
        }
        SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST
        | SMOOTHING_STRESS_MAJORIZATION_POWER_DIST
        | SMOOTHING_STRESS_MAJORIZATION_AVG_DIST => {
            let scheme = match ctrl.smoothing {
                SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST => IDEAL_GRAPH_DIST,
                SMOOTHING_STRESS_MAJORIZATION_AVG_DIST => IDEAL_AVG_DIST,
                _ => IDEAL_POWER_DIST,
            };
            let Some((mut sm, _)) = stress_majorization_smoother2_new(a, dim, 0.05, x, scheme)
            else {
                // C asserted in `StressMajorizationSmoother2_new`. `x` is still
                // the unsmoothed layout.
                return;
            };
            sm.smooth(dim, x, 50);
        }
        SMOOTHING_SPRING => {
            let mut sm = spring_smoother_new(a, dim, ctrl, x);
            sm.smooth(a, dim, x);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::spring_electrical::{
        SMOOTHING_STRESS_MAJORIZATION_AVG_DIST, SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST,
        SMOOTHING_STRESS_MAJORIZATION_POWER_DIST, SpringElectricalControl,
    };
    use super::*;

    fn self_edge() -> SparseMatrix {
        SparseMatrix::from_csr(1, 1, vec![0, 1], vec![0], Some(vec![1.0]))
    }

    fn edge() -> SparseMatrix {
        SparseMatrix::from_csr(2, 2, vec![0, 1, 2], vec![1, 0], Some(vec![1.0, 1.0]))
    }

    /// Issue #453: `StressMajorizationSmoother2_new` asserts `nz > 0`.
    #[test]
    fn an_isolated_node_is_left_unsmoothed() {
        let x = [3.0, 4.0];
        for scheme in [IDEAL_GRAPH_DIST, IDEAL_AVG_DIST, IDEAL_POWER_DIST] {
            assert!(stress_majorization_smoother2_new(&self_edge(), 2, 0.05, &x, scheme).is_none());
        }
        let connected = [0.0, 0.0, 1.0, 0.0];
        assert!(
            stress_majorization_smoother2_new(&edge(), 2, 0.05, &connected, IDEAL_AVG_DIST)
                .is_some()
        );
        let mut ctrl = SpringElectricalControl::default();
        for mode in [
            SMOOTHING_STRESS_MAJORIZATION_GRAPH_DIST,
            SMOOTHING_STRESS_MAJORIZATION_AVG_DIST,
            SMOOTHING_STRESS_MAJORIZATION_POWER_DIST,
        ] {
            ctrl.smoothing = mode;
            let mut pos = x;
            post_process_smoothing(2, &mut self_edge(), &ctrl, &mut pos);
            assert_eq!(pos, x);
        }
    }
}
