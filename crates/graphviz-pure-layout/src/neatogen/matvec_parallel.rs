//! Column-range `right_mult_with_vector_ff` for the `parallel` feature.
//!
//! A packed column gather is strided: each `A[i][j]` sits a row apart, and
//! turning the upper triangle into columns made every reduction a scalar
//! chain. That lost the vectorized scatter and was several times slower than
//! the blocked row kernel, including after a transpose built once. Workers
//! therefore own disjoint column ranges and still walk the packed rows.
//!
//! For `result[j]` with `j` in `[a, b)` the operations stay, in order:
//! `fma(A[i, j], vector[i], result[j])` for `i = 0 .. j-1`, then one plain
//! add of row `j`'s dot product. That dot is `fma(A[j, k], vector[k], …)`
//! from the diagonal through column `n - 1`, starting from `+0`. Rows before
//! `a` only scatter into the owned columns (a contiguous slice of each packed
//! row). The owned rows use the same four-row blocking as the sequential
//! kernel; scatter stops at column `b`, and the dot reads the rest of the row.

use std::sync::OnceLock;

/// Sequential below this. A mat-vec of a few hundred nodes is shorter than
/// starting workers. Above it the column split runs when more than one
/// thread is requested. On the measured graphs that split was slower than
/// one thread: each worker rereads the packed triangle. `GRAPHVIZ_PURE_THREADS=1`
/// stays on the sequential kernel at every size.
const MIN_N: usize = 512;

pub(super) fn use_parallel(n: usize) -> bool {
    n >= MIN_N && configured_threads() > 1
}

pub(super) fn right_mult_ff(packed: &[f32], n: usize, vector: &[f32], result: &mut [f32]) {
    right_mult_ff_with_threads(packed, n, vector, result, configured_threads());
}

/// `threads == 1` still runs the column-range kernel, so tests can compare
/// that kernel with the sequential one without depending on the environment.
pub(super) fn right_mult_ff_with_threads(
    packed: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
    threads: usize,
) {
    if n == 0 {
        return;
    }
    let vector = &vector[..n];
    let result = &mut result[..n];
    graphviz_pure_core::par::for_each_chunk_mut(result, threads, |range, slot| {
        owned_columns(packed, n, vector, slot, range.start);
    });
}

fn configured_threads() -> usize {
    static CACHED: OnceLock<usize> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let available = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        match std::env::var("GRAPHVIZ_PURE_THREADS") {
            Ok(value) => parse_threads(Some(&value), available),
            Err(std::env::VarError::NotPresent) => available.max(1),
            Err(std::env::VarError::NotUnicode(_)) => 1,
        }
    })
}

/// `None` uses `available`. `1` is the sequential path. `0` and any other
/// unreadable value are sequential too, so a bad setting does not spawn.
fn parse_threads(value: Option<&str>, available: usize) -> usize {
    match value {
        None => available.max(1),
        Some(text) => match text.trim().parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => 1,
        },
    }
}

fn owned_columns(packed: &[f32], n: usize, vector: &[f32], slot: &mut [f32], a: usize) {
    let m = slot.len();
    debug_assert!(m > 0);
    debug_assert!(a + m <= n);
    slot.fill(0.0);
    let mut rows = packed;
    for i in 0..a {
        let (row, rest) = rows.split_at(n - i);
        rows = rest;
        let off = a - i;
        // Columns `[a, b)` of this row are contiguous.
        scatter_fma(slot, &row[off..off + m], vector[i]);
    }
    let mut slot = slot;
    let mut vector = &vector[a..];
    let mut row_len = n - a;
    while slot.len() >= 4 {
        let next = block4_window(rows, vector, slot, row_len);
        rows = next.0;
        slot = next.1;
        vector = next.2;
        row_len = next.3;
    }
    while !slot.is_empty() {
        let next = row_window(rows, vector, slot, row_len);
        rows = next.0;
        slot = next.1;
        vector = next.2;
        row_len = next.3;
    }
}

fn block4_window<'p, 'r>(
    packed: &'p [f32],
    vector: &'p [f32],
    result: &'r mut [f32],
    row_len: usize,
) -> (&'p [f32], &'r mut [f32], &'p [f32], usize) {
    debug_assert!(row_len >= 4);
    debug_assert!(result.len() >= 4);
    debug_assert!(vector.len() >= row_len);
    let (row0, packed) = packed.split_at(row_len);
    let (row1, packed) = packed.split_at(row_len - 1);
    let (row2, packed) = packed.split_at(row_len - 2);
    let (row3, packed_rest) = packed.split_at(row_len - 3);
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
    let mut res0 = d0.mul_add(v0, 0.0);
    res0 = p01.mul_add(v1, res0);
    res0 = p02.mul_add(v2, res0);
    res0 = p03.mul_add(v3, res0);
    let mut res1 = d1.mul_add(v1, 0.0);
    res1 = p12.mul_add(v2, res1);
    res1 = p13.mul_add(v3, res1);
    let mut res2 = d2.mul_add(v2, 0.0);
    res2 = p23.mul_add(v3, res2);
    let mut res3 = d3.mul_add(v3, 0.0);
    let (head, result_tail) = result.split_at_mut(4);
    let scatter = result_tail.len();
    // Row i's scatter before row i+1's. The dot then reads the whole tail,
    // including columns past this worker's range, which are not scattered.
    scatter_fma(result_tail, &a0_tail[..scatter], v0);
    scatter_fma(result_tail, &a1_tail[..scatter], v1);
    scatter_fma(result_tail, &a2_tail[..scatter], v2);
    scatter_fma(result_tail, &a3_tail[..scatter], v3);
    res0 = dot_tail(res0, a0_tail, v_tail);
    res1 = dot_tail(res1, a1_tail, v_tail);
    res2 = dot_tail(res2, a2_tail, v_tail);
    res3 = dot_tail(res3, a3_tail, v_tail);
    let (slot0, head) = head.split_at_mut(1);
    let (slot1, head) = head.split_at_mut(1);
    let (slot2, slot3) = head.split_at_mut(1);
    fma_slot(slot1, p01, v0);
    fma_slot(slot2, p02, v0);
    fma_slot(slot2, p12, v1);
    fma_slot(slot3, p03, v0);
    fma_slot(slot3, p13, v1);
    fma_slot(slot3, p23, v2);
    add_slot(slot0, res0);
    add_slot(slot1, res1);
    add_slot(slot2, res2);
    add_slot(slot3, res3);
    (packed_rest, result_tail, v_tail, row_len - 4)
}

fn row_window<'p, 'r>(
    packed: &'p [f32],
    vector: &'p [f32],
    result: &'r mut [f32],
    row_len: usize,
) -> (&'p [f32], &'r mut [f32], &'p [f32], usize) {
    debug_assert!(row_len >= 1);
    debug_assert!(!result.is_empty());
    debug_assert!(vector.len() >= row_len);
    let (row, packed_rest) = packed.split_at(row_len);
    let (diag, off) = row.split_at(1);
    let vi = lane_f32(vector);
    let scatter = result.len() - 1;
    let mut res = lane_f32(diag).mul_add(vi, 0.0);
    let (owned, _past) = off.split_at(scatter);
    let (slot, result_tail) = result.split_at_mut(1);
    scatter_fma(result_tail, owned, vi);
    res = dot_tail(res, off, &vector[1..row_len]);
    add_slot(slot, res);
    let v_tail = vector.split_at(1).1;
    (packed_rest, result_tail, v_tail, row_len - 1)
}

fn dot_tail(mut res: f32, tail: &[f32], v_tail: &[f32]) -> f32 {
    for (&coefficient, &vj) in tail.iter().zip(v_tail) {
        res = coefficient.mul_add(vj, res);
    }
    res
}

#[inline(always)]
fn scatter_fma(dst: &mut [f32], row: &[f32], factor: f32) {
    debug_assert_eq!(dst.len(), row.len());
    for (dst, &coefficient) in dst.iter_mut().zip(row) {
        *dst = coefficient.mul_add(factor, *dst);
    }
}

fn lane_f32(values: &[f32]) -> f32 {
    // Unreachable: every caller splits a piece of length >= 1.
    debug_assert!(!values.is_empty());
    values.first().copied().unwrap_or(0.0)
}

fn fma_slot(slot: &mut [f32], factor: f32, scale: f32) {
    // Unreachable: the slot is one element of a head `split_at_mut` just checked.
    debug_assert!(!slot.is_empty());
    if let Some(slot) = slot.first_mut() {
        *slot = factor.mul_add(scale, *slot);
    }
}

/// `slot[0] += term`, a plain add, not an FMA.
///
/// Outlined on purpose. An inlined `+=` is a commutative `fadd`, and LLVM may
/// swap the operands. When both are NaN, x86 `addss` keeps the source operand's
/// payload, so the swap changes bits. The sequential kernel's `result[i] += res`
/// does not swap. A call keeps that order at every opt level.
#[inline(never)]
fn add_slot(slot: &mut [f32], term: f32) {
    // Unreachable: the slot is one element of a head `split_at_mut` just checked.
    debug_assert!(!slot.is_empty());
    if let Some(slot) = slot.first_mut() {
        *slot += term;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_threads_treats_one_as_sequential_and_rejects_garbage() {
        assert_eq!(parse_threads(None, 8), 8);
        assert_eq!(parse_threads(None, 0), 1);
        assert_eq!(parse_threads(Some("1"), 8), 1);
        assert_eq!(parse_threads(Some("4"), 8), 4);
        assert_eq!(parse_threads(Some("  3  "), 8), 3);
        assert_eq!(parse_threads(Some("0"), 8), 1);
        assert_eq!(parse_threads(Some(""), 8), 1);
        assert_eq!(parse_threads(Some("nope"), 8), 1);
    }
}
