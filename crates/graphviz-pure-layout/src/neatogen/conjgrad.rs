//! Conjugate-gradient solvers from `lib/neatogen/conjgrad.c`.

use super::graphdata::VtxData;
use super::matrix_ops::*;

/// `conjugate_gradient_mkernel(A, x, b, n, tol, max_iterations)`: solves
/// `A x = b` for a packed symmetric `float` matrix, centring the vectors.
/// Returns `Err` on a zero-length residual, like C's non-zero return.
pub fn conjugate_gradient_mkernel(
    a: &[f32],
    x: &mut [f32],
    b: &mut [f32],
    n: usize,
    tol: f64,
    max_iterations: usize,
) -> Result<(), super::SolverError> {
    let mut r = vec![0.0f32; n];
    let mut p = vec![0.0f32; n];
    let mut ap = vec![0.0f32; n];
    let mut ax = vec![0.0f32; n];
    orthog1f(&mut x[..n]);
    orthog1f(&mut b[..n]);
    right_mult_with_vector_ff(a, n, x, &mut ax);
    orthog1f(&mut ax);
    // One subtraction per lane, same operands as `b[i] - ax[i]`.
    for ((ri, &bi), &axi) in r.iter_mut().zip(&b[..n]).zip(ax.iter()) {
        *ri = bi - axi;
    }
    p.copy_from_slice(&r);
    let mut r_r = vectors_inner_productf(n, &r, &r);
    let mut i = 0;
    while i < max_iterations && max_absf(n, &r) > tol {
        orthog1f(&mut p);
        orthog1f(&mut x[..n]);
        orthog1f(&mut r);
        right_mult_with_vector_ff(a, n, &p, &mut ap);
        orthog1f(&mut ap);
        let p_ap = vectors_inner_productf(n, &p, &ap);
        if p_ap == 0.0 {
            break;
        }
        let alpha = r_r / p_ap;
        vectors_mult_additionf(n, x, alpha as f32, &p);
        if i + 1 < max_iterations {
            vectors_mult_additionf(n, &mut r, (-alpha) as f32, &ap);
            let r_r_new = vectors_inner_productf(n, &r, &r);
            if r_r == 0.0 {
                return Err(super::SolverError);
            }
            let beta = r_r_new / r_r;
            r_r = r_r_new;
            let bf = beta as f32;
            // `p[j] = fma(beta, p[j], r[j])`, one chain per lane.
            for (pj, &rj) in p.iter_mut().zip(r.iter()) {
                *pj = bf.mul_add(*pj, rj);
            }
        }
        i += 1;
    }
    Ok(())
}

/// `conjugate_gradient_f(A, x, b, n, tol, max_iterations, ortho1)`: dense
/// `float` matrix, double vectors.
pub fn conjugate_gradient_f(
    a: &[Vec<f32>],
    x: &mut [f64],
    b: &[f64],
    n: usize,
    tol: f64,
    max_iterations: usize,
    ortho1: bool,
) -> Result<(), super::SolverError> {
    let mut r = vec![0.0f64; n];
    let mut p = vec![0.0f64; n];
    let mut ap = vec![0.0f64; n];
    let mut ax = vec![0.0f64; n];
    let mut alphap = vec![0.0f64; n];
    let mut orth_b = b[..n].to_vec();
    if ortho1 {
        orthog1(&mut orth_b);
        orthog1(&mut x[..n]);
    }
    right_mult_with_vector_f(a, n, x, &mut ax);
    for i in 0..n {
        r[i] = orth_b[i] - ax[i];
    }
    p.copy_from_slice(&r);
    let mut r_r = vectors_inner_product(n, &r, &r);
    let mut i = 0;
    while i < max_iterations && max_abs(n, &r) > tol {
        right_mult_with_vector_f(a, n, &p, &mut ap);
        let p_ap = vectors_inner_product(n, &p, &ap);
        if p_ap == 0.0 {
            break;
        }
        let alpha = r_r / p_ap;
        for j in 0..n {
            alphap[j] = p[j] * alpha;
        }
        for j in 0..n {
            x[j] += alphap[j];
        }
        if i + 1 < max_iterations {
            for j in 0..n {
                ap[j] *= alpha;
            }
            for j in 0..n {
                r[j] -= ap[j];
            }
            let r_r_new = vectors_inner_product(n, &r, &r);
            if r_r == 0.0 {
                return Err(super::SolverError);
            }
            let beta = r_r_new / r_r;
            r_r = r_r_new;
            for j in 0..n {
                p[j] *= beta;
            }
            for j in 0..n {
                p[j] += r[j];
            }
        }
        i += 1;
    }
    Ok(())
}

/// `conjugate_gradient(A, x, b, n, tol, max_iterations)`: sparse matrix.
pub fn conjugate_gradient(
    a: &[VtxData],
    x: &mut [f64],
    b: &[f64],
    n: usize,
    tol: f64,
    max_iterations: usize,
) -> Result<(), super::SolverError> {
    let mut r = vec![0.0f64; n];
    let mut p = vec![0.0f64; n];
    let mut ap = vec![0.0f64; n];
    let mut ax = vec![0.0f64; n];
    let mut orth_b = b[..n].to_vec();
    orthog1(&mut orth_b);
    orthog1(&mut x[..n]);
    right_mult_with_vector(a, n, x, &mut ax);
    for i in 0..n {
        r[i] = orth_b[i] - ax[i];
    }
    p.copy_from_slice(&r);
    let mut r_r = vectors_inner_product(n, &r, &r);
    let mut i = 0;
    while i < max_iterations && max_abs(n, &r) > tol {
        right_mult_with_vector(a, n, &p, &mut ap);
        let p_ap = vectors_inner_product(n, &p, &ap);
        if p_ap == 0.0 {
            break;
        }
        let alpha = r_r / p_ap;
        for j in 0..n {
            x[j] += p[j] * alpha;
        }
        if i + 1 < max_iterations {
            for j in 0..n {
                ap[j] *= alpha;
            }
            for j in 0..n {
                r[j] -= ap[j];
            }
            let r_r_new = vectors_inner_product(n, &r, &r);
            if r_r == 0.0 {
                return Err(super::SolverError);
            }
            let beta = r_r_new / r_r;
            r_r = r_r_new;
            for j in 0..n {
                p[j] *= beta;
            }
            for j in 0..n {
                p[j] += r[j];
            }
        }
        i += 1;
    }
    Ok(())
}
