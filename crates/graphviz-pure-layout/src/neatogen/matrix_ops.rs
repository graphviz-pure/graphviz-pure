//! Dense and packed vector/matrix kernels from `lib/neatogen/matrix_ops.c`.
//!
//! Floating-point operation order follows the C source as compiled by clang
//! with `-ffp-contract=on`: `x += a * b` within one expression becomes
//! `fma(a, b, x)` (both `f32` and `f64`), while a `float * float` product added
//! to a `double` accumulator is rounded to `float` first (no contraction
//! across the conversion).

use super::graphdata::VtxData;
use graphviz_pure_core::math;

/// `orthog1`: subtract the mean.
pub fn orthog1(vec: &mut [f64]) {
    let n = vec.len();
    let mut sum = 0.0;
    for v in vec.iter() {
        sum += *v;
    }
    sum /= n as f64;
    for v in vec.iter_mut() {
        *v -= sum;
    }
}

/// `orthog1f`.
pub fn orthog1f(vec: &mut [f32]) {
    let n = vec.len();
    let mut sum = 0.0f32;
    for v in vec.iter() {
        sum += *v;
    }
    sum /= n as f32;
    for v in vec.iter_mut() {
        *v -= sum;
    }
}

/// `right_mult_with_vector_ff`: `result = A * vector` for a packed symmetric
/// matrix (upper triangle, row-major).
pub fn right_mult_with_vector_ff(packed: &[f32], n: usize, vector: &[f32], result: &mut [f32]) {
    for r in result.iter_mut().take(n) {
        *r = 0.0;
    }
    let mut index = 0;
    for i in 0..n {
        let vector_i = vector[i];
        let mut res = 0.0f32;
        res = packed[index].mul_add(vector_i, res);
        index += 1;
        for j in (i + 1)..n {
            res = packed[index].mul_add(vector[j], res);
            result[j] = packed[index].mul_add(vector_i, result[j]);
            index += 1;
        }
        result[i] += res;
    }
}

/// `vectors_inner_productf`: `float` products accumulated in `double`.
pub fn vectors_inner_productf(n: usize, v1: &[f32], v2: &[f32]) -> f64 {
    let mut result = 0.0f64;
    for i in 0..n {
        result += f64::from(v1[i] * v2[i]);
    }
    result
}

/// `vectors_inner_product` (double).
pub fn vectors_inner_product(n: usize, v1: &[f64], v2: &[f64]) -> f64 {
    let mut result = 0.0f64;
    for i in 0..n {
        result = math::fma(v1[i], v2[i], result);
    }
    result
}

/// `vectors_mult_additionf`: `v1 += alpha * v2`.
pub fn vectors_mult_additionf(n: usize, v1: &mut [f32], alpha: f32, v2: &[f32]) {
    for i in 0..n {
        v1[i] = alpha.mul_add(v2[i], v1[i]);
    }
}

/// `max_absf`.
pub fn max_absf(n: usize, v: &[f32]) -> f64 {
    let mut m = -1e30f32;
    for x in v.iter().take(n) {
        m = m.max(x.abs());
    }
    f64::from(m)
}

/// `max_abs`.
pub fn max_abs(n: usize, v: &[f64]) -> f64 {
    let mut m = -1e50f64;
    for x in v.iter().take(n) {
        m = m.max(x.abs());
    }
    m
}

/// `square_vec`.
pub fn square_vec(vec: &mut [f32]) {
    for v in vec {
        *v *= *v;
    }
}

/// `invert_vec`.
pub fn invert_vec(vec: &mut [f32]) {
    for v in vec {
        if *v != 0.0 {
            *v = 1.0f32 / *v;
        }
    }
}

/// `sqrt_vecf`.
pub fn sqrt_vecf(source: &[f32], target: &mut [f32]) {
    for (s, t) in source.iter().zip(target.iter_mut()) {
        if *s >= 0.0 {
            *t = s.sqrt();
        }
    }
}

/// `invert_sqrt_vec`.
pub fn invert_sqrt_vec(vec: &mut [f32]) {
    for v in vec {
        if *v > 0.0 {
            *v = 1.0f32 / v.sqrt();
        }
    }
}

/// `right_mult_with_vector` (sparse, double).
pub fn right_mult_with_vector(matrix: &[VtxData], n: usize, vector: &[f64], result: &mut [f64]) {
    for i in 0..n {
        let mut res = 0.0f64;
        let w = matrix[i].ewgts.as_ref().expect("weights");
        for (j, &e) in matrix[i].edges.iter().enumerate() {
            res = math::fma(f64::from(w[j]), vector[e as usize], res);
        }
        result[i] = res;
    }
}

/// `right_mult_with_vector_f`: dense `float` matrix times double vector.
pub fn right_mult_with_vector_f(matrix: &[Vec<f32>], n: usize, vector: &[f64], result: &mut [f64]) {
    for i in 0..n {
        let mut res = 0.0f64;
        for j in 0..n {
            res = math::fma(f64::from(matrix[i][j]), vector[j], res);
        }
        result[i] = res;
    }
}

/// `right_mult_with_vector_transpose`: `result = matrix^T * vector` where
/// `matrix` is `dim2 x dim1`.
pub fn right_mult_with_vector_transpose(
    matrix: &[Vec<f64>],
    dim1: usize,
    dim2: usize,
    vector: &[f64],
    result: &mut [f64],
) {
    for i in 0..dim1 {
        let mut res = 0.0f64;
        for j in 0..dim2 {
            res = math::fma(matrix[j][i], vector[j], res);
        }
        result[i] = res;
    }
}

/// `right_mult_with_vector_d`: `result = matrix * vector`, `matrix` is `dim1 x dim2`.
pub fn right_mult_with_vector_d(
    matrix: &[Vec<f64>],
    dim1: usize,
    dim2: usize,
    vector: &[f64],
    result: &mut [f64],
) {
    for i in 0..dim1 {
        let mut res = 0.0f64;
        for j in 0..dim2 {
            res = math::fma(matrix[i][j], vector[j], res);
        }
        result[i] = res;
    }
}

/// `scadd`: `vec1[0..=end] += fac * vec2`.
pub fn scadd(vec1: &mut [f64], end: usize, fac: f64, vec2: &[f64]) {
    for i in 0..=end {
        vec1[i] = math::fma(fac, vec2[i], vec1[i]);
    }
}

/// `norm`: 2-norm over `vec[0..=end]`.
pub fn norm(vec: &[f64], end: usize) -> f64 {
    vectors_inner_product(end + 1, vec, vec).sqrt()
}
