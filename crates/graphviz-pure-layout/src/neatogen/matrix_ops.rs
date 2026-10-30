//! Dense and packed vector/matrix kernels from `lib/neatogen/matrix_ops.c`.
//!
//! Floating-point operation order follows the C source as compiled by clang
//! with `-ffp-contract=on`: `x += a * b` within one expression becomes
//! `fma(a, b, x)` (both `f32` and `f64`), while a `float * float` product added
//! to a `double` accumulator is rounded to `float` first (no contraction
//! across the conversion).
//!
//! `right_mult_with_vector_ff` blocks four rows and splits each row's scatter
//! from its running dot. Each output float keeps the operation sequence
//! documented on that function.

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

/// C: lib/neatogen/matrix_ops.c:right_mult_with_vector_ff
///
/// `result = A * vector` for a packed symmetric matrix (upper triangle,
/// row-major).
///
/// Four rows run together, so their dot-product chains are independent, and
/// each row's scatter is its own loop. For every `result[j]` the operations
/// stay, in order: `fma(A[i, j], vector[i], result[j])` for `i = 0 .. j-1`,
/// then one plain add of row `j`'s left-to-right dot product. That dot product
/// is `fma(A[j, k], vector[k], …)` from the diagonal through column `n - 1`,
/// starting from `+0`.
pub fn right_mult_with_vector_ff(packed: &[f32], n: usize, vector: &[f32], result: &mut [f32]) {
    // One-shot callers stay here. The parallel gather needs a lower-triangle
    // copy built once per solve; conjugate gradient and stress majorization
    // own that copy and call `matvec_parallel` directly.
    let mut vector = &vector[..n];
    let mut result = &mut result[..n];
    result.fill(0.0);
    let mut packed = packed;
    while vector.len() >= 4 {
        let (packed_rest, result_tail) = right_mult_ff_block4(packed, vector, result);
        packed = packed_rest;
        result = result_tail;
        vector = vector.split_at(4).1;
    }
    while !vector.is_empty() {
        let (packed_rest, result_tail) = right_mult_ff_row(packed, vector, result);
        packed = packed_rest;
        result = result_tail;
        vector = vector.split_at(1).1;
    }
}

/// Four packed rows starting at `vector[0]`. `vector` and `result` have the
/// same length, at least 4. The four `res` values are independent left-to-right
/// chains; scatters into a single `result[j]` stay in increasing row order.
fn right_mult_ff_block4<'p, 'r>(
    packed: &'p [f32],
    vector: &[f32],
    result: &'r mut [f32],
) -> (&'p [f32], &'r mut [f32]) {
    let n = vector.len();
    debug_assert!(n >= 4);
    debug_assert_eq!(result.len(), n);

    let (row0, packed) = packed.split_at(n);
    let (row1, packed) = packed.split_at(n.wrapping_sub(1));
    let (row2, packed) = packed.split_at(n.wrapping_sub(2));
    let (row3, packed_rest) = packed.split_at(n.wrapping_sub(3));

    let (v_head, v_tail) = vector.split_at(4);
    let mut lanes = [0.0f32; 4];
    for (lane, &value) in lanes.iter_mut().zip(v_head) {
        *lane = value;
    }
    let [v0, v1, v2, v3] = lanes;

    let (diag0, row0) = row0.split_at(1);
    let (a01, row0) = row0.split_at(1);
    let (a02, row0) = row0.split_at(1);
    let (a03, a0_tail) = row0.split_at(1);
    let (diag1, row1) = row1.split_at(1);
    let (a12, row1) = row1.split_at(1);
    let (a13, a1_tail) = row1.split_at(1);
    let (diag2, row2) = row2.split_at(1);
    let (a23, a2_tail) = row2.split_at(1);
    let (diag3, a3_tail) = row3.split_at(1);

    let d0 = lane_f32(diag0);
    let d1 = lane_f32(diag1);
    let d2 = lane_f32(diag2);
    let d3 = lane_f32(diag3);
    let p01 = lane_f32(a01);
    let p02 = lane_f32(a02);
    let p03 = lane_f32(a03);
    let p12 = lane_f32(a12);
    let p13 = lane_f32(a13);
    let p23 = lane_f32(a23);

    // Each chain is that row's dot product, column by column, one accumulator.
    let mut res0 = d0.mul_add(v0, 0.0f32);
    res0 = p01.mul_add(v1, res0);
    res0 = p02.mul_add(v2, res0);
    res0 = p03.mul_add(v3, res0);
    let mut res1 = d1.mul_add(v1, 0.0f32);
    res1 = p12.mul_add(v2, res1);
    res1 = p13.mul_add(v3, res1);
    let mut res2 = d2.mul_add(v2, 0.0f32);
    res2 = p23.mul_add(v3, res2);
    let mut res3 = d3.mul_add(v3, 0.0f32);

    // The four tails have the same length by the splits above. Naming that
    // length lets the four dot-product chains compile as one loop: each step
    // is `fma(A[row, j], vector[j], res)` for rows 0..3, in that order, which
    // is the same left-to-right order as one row at a time.
    debug_assert_eq!(a0_tail.len(), v_tail.len());
    debug_assert_eq!(a1_tail.len(), v_tail.len());
    debug_assert_eq!(a2_tail.len(), v_tail.len());
    debug_assert_eq!(a3_tail.len(), v_tail.len());
    let ntail = a0_tail.len();
    let a1_tail = &a1_tail[..ntail];
    let a2_tail = &a2_tail[..ntail];
    let a3_tail = &a3_tail[..ntail];
    let v_tail = &v_tail[..ntail];
    for ((((&p0, &p1), &p2), &p3), &vj) in a0_tail
        .iter()
        .zip(a1_tail)
        .zip(a2_tail)
        .zip(a3_tail)
        .zip(v_tail)
    {
        res0 = p0.mul_add(vj, res0);
        res1 = p1.mul_add(vj, res1);
        res2 = p2.mul_add(vj, res2);
        res3 = p3.mul_add(vj, res3);
    }

    let (head, result_tail) = result.split_at_mut(4);
    let (slot0, head) = head.split_at_mut(1);
    let (slot1, head) = head.split_at_mut(1);
    let (slot2, slot3) = head.split_at_mut(1);

    // Row i's term is applied before row i+1's, and both before `+= res`.
    fma_slot(slot1, p01, v0);
    fma_slot(slot2, p02, v0);
    fma_slot(slot2, p12, v1);
    fma_slot(slot3, p03, v0);
    fma_slot(slot3, p13, v1);
    fma_slot(slot3, p23, v2);
    scatter_fma(result_tail, a0_tail, v0);
    scatter_fma(result_tail, a1_tail, v1);
    scatter_fma(result_tail, a2_tail, v2);
    scatter_fma(result_tail, a3_tail, v3);
    add_slot(slot0, res0);
    add_slot(slot1, res1);
    add_slot(slot2, res2);
    add_slot(slot3, res3);

    (packed_rest, result_tail)
}

/// One packed row. The running dot stays a single left-to-right chain, and the
/// scatter of `factor * row` into later `result` entries is a separate loop.
fn right_mult_ff_row<'p, 'r>(
    packed: &'p [f32],
    vector: &[f32],
    result: &'r mut [f32],
) -> (&'p [f32], &'r mut [f32]) {
    let n = vector.len();
    debug_assert!(n >= 1);
    debug_assert_eq!(result.len(), n);
    let (row, packed_rest) = packed.split_at(n);
    let (diag, off) = row.split_at(1);
    let vi = lane_f32(vector);
    let v_tail = vector.split_at(1).1;
    let mut res = lane_f32(diag).mul_add(vi, 0.0f32);
    for (&coefficient, &vj) in off.iter().zip(v_tail) {
        res = coefficient.mul_add(vj, res);
    }
    let (slot, result_tail) = result.split_at_mut(1);
    scatter_fma(result_tail, off, vi);
    add_slot(slot, res);
    (packed_rest, result_tail)
}

/// First element. Callers pass a non-empty slice (`split_at` already checked).
fn lane_f32(values: &[f32]) -> f32 {
    // Unreachable: every caller splits a diagonal or vector head of length >= 1.
    debug_assert!(!values.is_empty());
    values.first().copied().unwrap_or(0.0)
}

/// `slot[0] = fma(factor, scale, slot[0])`.
fn fma_slot(slot: &mut [f32], factor: f32, scale: f32) {
    // Unreachable: the slot is one element of a head `split_at_mut` just checked.
    debug_assert!(!slot.is_empty());
    if let Some(slot) = slot.first_mut() {
        *slot = factor.mul_add(scale, *slot);
    }
}

/// `slot[0] += term`, a plain add, not an FMA.
fn add_slot(slot: &mut [f32], term: f32) {
    // Unreachable: the slot is one element of a head `split_at_mut` just checked.
    debug_assert!(!slot.is_empty());
    if let Some(slot) = slot.first_mut() {
        *slot += term;
    }
}

/// `dst[j] = fma(row[j], factor, dst[j])`. Each `j` is its own chain, so this
/// loop may vectorize without reordering any sum.
#[inline(always)]
fn scatter_fma(dst: &mut [f32], row: &[f32], factor: f32) {
    debug_assert_eq!(dst.len(), row.len());
    for (dst, &coefficient) in dst.iter_mut().zip(row) {
        *dst = coefficient.mul_add(factor, *dst);
    }
}

/// C: lib/neatogen/matrix_ops.c:vectors_inner_productf
///
/// `float` products accumulated in `double`, left to right. One accumulator:
/// a second one would reorder the sum.
pub fn vectors_inner_productf(n: usize, v1: &[f32], v2: &[f32]) -> f64 {
    let mut result = 0.0f64;
    for (&a, &b) in v1[..n].iter().zip(&v2[..n]) {
        result += f64::from(a * b);
    }
    result
}

/// C: lib/neatogen/matrix_ops.c:vectors_inner_product
///
/// Double dot product, one left-to-right `fma` chain.
pub fn vectors_inner_product(n: usize, v1: &[f64], v2: &[f64]) -> f64 {
    let mut result = 0.0f64;
    for (&a, &b) in v1[..n].iter().zip(&v2[..n]) {
        result = math::fma(a, b, result);
    }
    result
}

/// C: lib/neatogen/matrix_ops.c:vectors_mult_additionf
///
/// `v1[i] = fma(alpha, v2[i], v1[i])`. The lanes are independent.
pub fn vectors_mult_additionf(n: usize, v1: &mut [f32], alpha: f32, v2: &[f32]) {
    for (dst, &src) in v1[..n].iter_mut().zip(&v2[..n]) {
        *dst = alpha.mul_add(src, *dst);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-T8 loop, kept so a reassociated sum fails this file's tests.
    fn scalar_right_mult_with_vector_ff(
        packed: &[f32],
        n: usize,
        vector: &[f32],
        result: &mut [f32],
    ) {
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

    fn scalar_inner_productf(n: usize, v1: &[f32], v2: &[f32]) -> f64 {
        let mut result = 0.0f64;
        for i in 0..n {
            result += f64::from(v1[i] * v2[i]);
        }
        result
    }

    fn scalar_inner_product(n: usize, v1: &[f64], v2: &[f64]) -> f64 {
        let mut result = 0.0f64;
        for i in 0..n {
            result = math::fma(v1[i], v2[i], result);
        }
        result
    }

    fn scalar_mult_additionf(n: usize, v1: &mut [f32], alpha: f32, v2: &[f32]) {
        for i in 0..n {
            v1[i] = alpha.mul_add(v2[i], v1[i]);
        }
    }

    fn next_u32(state: &mut u32) -> u32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *state
    }

    fn next_f32(state: &mut u32) -> f32 {
        let bits = next_u32(state);
        let sign = bits & 0x8000_0000;
        let frac = bits & 0x007f_ffff;
        // `% 24` visits every residue. `& 23` is a mask and kept only 16
        // exponents, all normal. Three windows: zero and subnormals, the
        // old modest band, and magnitudes whose products overflow.
        let exp = match (bits >> 28) % 3 {
            0 => (bits >> 23) % 24,
            1 => 117 + ((bits >> 23) % 24),
            _ => 231 + ((bits >> 23) % 24),
        };
        f32::from_bits(sign | (exp << 23) | frac)
    }

    fn edge_values() -> [f32; 13] {
        [
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MIN,
            f32::MAX,
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::from_bits(0x7fc0_0001),
            f32::from_bits(0x8000_0001),
        ]
    }

    /// Bit equality, except that any NaN matches any NaN.
    ///
    /// Rust does not specify the payload or the sign of an FMA's NaN, and
    /// other targets do not keep the x86 payload. The sign is not checked:
    /// it was not shown to be stable off x86. `+0`, `-0`, and subnormals
    /// are not NaNs, so `to_bits` still distinguishes them.
    fn assert_f32_bits(got: &[f32], expect: &[f32], what: &str) {
        assert_eq!(got.len(), expect.len(), "{what}");
        for (i, (got, expect)) in got.iter().zip(expect).enumerate() {
            if got.is_nan() && expect.is_nan() {
                continue;
            }
            assert_eq!(
                got.to_bits(),
                expect.to_bits(),
                "{what} [{i}]: {got:?} vs {expect:?}"
            );
        }
    }

    #[test]
    fn f32_bits_accept_any_nan_and_keep_signed_zero_and_subnormals() {
        assert_f32_bits(
            &[
                f32::from_bits(0x7fc0_0001),
                f32::from_bits(0xffc0_0001),
                0.0,
                -0.0,
                f32::from_bits(1),
            ],
            &[
                f32::from_bits(0x7fc0_0000),
                f32::from_bits(0x7f80_0001),
                0.0,
                -0.0,
                f32::from_bits(1),
            ],
            "nan, zero, subnormal",
        );
    }

    fn check_matvec(packed: &[f32], n: usize, vector: &[f32]) {
        let mut got = vec![1.0f32; n + 2];
        let mut expect = got.clone();
        let mut packed_ext = packed.to_vec();
        packed_ext.push(123.0);
        let mut vector_ext = vector.to_vec();
        vector_ext.extend([7.0, 9.0]);
        right_mult_with_vector_ff(&packed_ext, n, &vector_ext, &mut got);
        scalar_right_mult_with_vector_ff(&packed_ext, n, &vector_ext, &mut expect);
        assert_f32_bits(&got, &expect, &format!("n={n}"));
    }

    #[test]
    fn right_mult_matches_scalar_chain_on_every_width() {
        // Widths that are not a multiple of the 4-row block, including the
        // empty and single-row edges.
        let widths = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33, 37, 63, 64, 65, 127, 128,
        ];
        for &n in &widths {
            for seed in [1u32, 0x1234_5678, 0xa5a5_5a5a] {
                let mut state = seed.wrapping_add(n as u32);
                let packed_len = n * (n + 1) / 2;
                let packed: Vec<f32> = (0..packed_len).map(|_| next_f32(&mut state)).collect();
                let vector: Vec<f32> = (0..n).map(|_| next_f32(&mut state)).collect();
                check_matvec(&packed, n, &vector);
            }
        }
    }

    #[test]
    fn right_mult_matches_scalar_chain_on_edge_values() {
        let specials = edge_values();
        for n in 0..specials.len() {
            let packed_len = n * (n + 1) / 2;
            let packed: Vec<f32> = (0..packed_len)
                .map(|i| specials[i % specials.len()])
                .collect();
            let vector: Vec<f32> = (0..n)
                .map(|i| specials[(i * 3 + 1) % specials.len()])
                .collect();
            check_matvec(&packed, n, &vector);
        }
    }

    #[test]
    fn right_mult_matches_scalar_chain_on_large_mixed_inputs() {
        // Widths past the 4-row block. Every 11th matrix entry and every 5th
        // vector entry is an edge value (zero, subnormal, overflow, inf, NaN)
        // so those patterns are not left to a rare generator draw.
        let specials = edge_values();
        for &n in &[37usize, 64] {
            for seed in [1u32, 0x1234_5678, 0xa5a5_5a5a] {
                let mut state = seed.wrapping_add(n as u32);
                let packed_len = n * (n + 1) / 2;
                let packed: Vec<f32> = (0..packed_len)
                    .map(|i| {
                        if i % 11 == 0 {
                            specials[i % specials.len()]
                        } else {
                            next_f32(&mut state)
                        }
                    })
                    .collect();
                let vector: Vec<f32> = (0..n)
                    .map(|i| {
                        if i % 5 == 0 {
                            specials[(i * 3) % specials.len()]
                        } else {
                            next_f32(&mut state)
                        }
                    })
                    .collect();
                check_matvec(&packed, n, &vector);
            }
        }
    }

    #[test]
    fn vector_kernels_match_their_scalar_chains() {
        let mut state = 0x0c0ffeeu32;
        for n in [0, 1, 2, 3, 7, 8, 15, 16, 17, 64] {
            let v1: Vec<f32> = (0..n).map(|_| next_f32(&mut state)).collect();
            let v2: Vec<f32> = (0..n).map(|_| next_f32(&mut state)).collect();
            let got = vectors_inner_productf(n, &v1, &v2);
            let expect = scalar_inner_productf(n, &v1, &v2);
            assert_eq!(got.to_bits(), expect.to_bits(), "innerf n={n}");

            let d1: Vec<f64> = v1.iter().map(|v| f64::from(*v)).collect();
            let d2: Vec<f64> = v2.iter().map(|v| f64::from(*v)).collect();
            let got = vectors_inner_product(n, &d1, &d2);
            let expect = scalar_inner_product(n, &d1, &d2);
            assert_eq!(got.to_bits(), expect.to_bits(), "inner n={n}");

            let alpha = next_f32(&mut state);
            let mut got = v1.clone();
            let mut expect = v1.clone();
            vectors_mult_additionf(n, &mut got, alpha, &v2);
            scalar_mult_additionf(n, &mut expect, alpha, &v2);
            assert_f32_bits(&got, &expect, &format!("axpy n={n}"));
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_right_mult_matches_scalar_chain_for_every_thread_count() {
        // Odd widths, and widths that are not a multiple of the thread count
        // or of the 8-wide gather. `threads == 1` still runs the gather, not
        // the sequential kernel.
        let widths = [0, 1, 2, 3, 5, 7, 8, 9, 17, 37, 63, 64, 65, 127, 128];
        let threads = [1usize, 2, 3, 8];
        for &n in &widths {
            for &thread_count in &threads {
                for seed in [1u32, 0x1234_5678, 0xa5a5_5a5a] {
                    let mut state = seed
                        .wrapping_add(n as u32)
                        .wrapping_add(thread_count as u32);
                    let packed_len = n * (n + 1) / 2;
                    let packed: Vec<f32> = (0..packed_len).map(|_| next_f32(&mut state)).collect();
                    let vector: Vec<f32> = (0..n).map(|_| next_f32(&mut state)).collect();
                    let mut got = vec![1.0f32; n + 2];
                    let mut expect = got.clone();
                    let mut packed_ext = packed.clone();
                    packed_ext.push(123.0);
                    let mut vector_ext = vector.clone();
                    vector_ext.extend([7.0, 9.0]);
                    super::super::matvec_parallel::right_mult_ff_with_threads(
                        &packed_ext,
                        n,
                        &vector_ext,
                        &mut got,
                        thread_count,
                    );
                    scalar_right_mult_with_vector_ff(&packed_ext, n, &vector_ext, &mut expect);
                    assert_f32_bits(
                        &got,
                        &expect,
                        &format!("n={n} threads={thread_count} seed={seed}"),
                    );
                }
            }
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_right_mult_matches_scalar_chain_on_large_odd_n() {
        let specials = edge_values();
        for &n in &[37usize, 65] {
            for &thread_count in &[1usize, 2, 3, 8] {
                let mut state = 0x0a11_u32.wrapping_add(n as u32);
                let packed_len = n * (n + 1) / 2;
                let packed: Vec<f32> = (0..packed_len)
                    .map(|i| {
                        if i % 11 == 0 {
                            specials[i % specials.len()]
                        } else {
                            next_f32(&mut state)
                        }
                    })
                    .collect();
                let vector: Vec<f32> = (0..n)
                    .map(|i| {
                        if i % 5 == 0 {
                            specials[(i * 3) % specials.len()]
                        } else {
                            next_f32(&mut state)
                        }
                    })
                    .collect();
                let mut got = vec![1.0f32; n + 2];
                let mut expect = got.clone();
                super::super::matvec_parallel::right_mult_ff_with_threads(
                    &packed,
                    n,
                    &vector,
                    &mut got,
                    thread_count,
                );
                scalar_right_mult_with_vector_ff(&packed, n, &vector, &mut expect);
                assert_f32_bits(
                    &got,
                    &expect,
                    &format!("mixed n={n} threads={thread_count}"),
                );
            }
        }
    }
}
