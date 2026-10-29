//! Lower-triangle gather for `right_mult_with_vector_ff`.
//!
//! The packed upper triangle stores `A[i][j]` (`i <= j`) in row `i`. A worker
//! that owns column `j` used to walk every earlier row to read that one entry,
//! so each thread reread the triangle. The symmetric matrix passed to
//! conjugate gradient is constant for the solve, and in
//! `stress_majorization_kd_mkernel` the Laplacian `lap2` is also constant
//! across the outer iterations (`sqrt_vecf` writes `lap1`, not `lap2`).
//! Callers that own such a matrix build one packed lower triangle and reuse
//! it. Row `j` is `A[0][j] .. A[j][j]`, contiguous, diagonal last.
//!
//! `result[j]` then gathers `i = 0 .. j-1` from that row, in increasing `i`,
//! then adds row `j`'s existing left-to-right dot (diagonal through column
//! `n - 1`, starting at `+0`). The diagonal stays in the dot. Folding it into
//! the gather would change the IEEE sequence.
//!
//! Each output does `j` gather FMAs and `n - j` dot FMAs, so `n` in total.
//! Equal `j` counts are equal work. A split by lower-triangle area gives the
//! first of four threads about half the rows, and that was slower. Several
//! outputs run together so their chains stay independent; nothing is
//! reordered inside one chain.
//!
//! The gather is experimental. It is a scalar chain per output, and it reads
//! both triangles, so it does more work than the blocked kernel. On
//! rnd2000-as-neato, one thread took 6.5 s and four threads took 20.0 s, with
//! user cycles rising from 20.5e9 to 53.6e9. `GRAPHVIZ_PURE_THREADS=1` stays
//! on the sequential kernel.

use std::sync::OnceLock;

/// Sequential below this.
///
/// Two threads did not beat one thread at any timed width (256 through 4000
/// in the kernel, and the 1000- and 2000-node graphs end to end). Below 1000
/// the kernel loss was larger still, so those graphs stay on the blocked
/// kernel. This is not a measured crossover.
pub(super) const MIN_N: usize = 1000;

/// Sequential above this. The lower copy is `n*(n+1)/2` extra `f32`s:
/// 8_004_000 bytes at `n = 2000`, 134_250_496 bytes at this cap, and
/// 200_020_000 bytes at `n = 10000`, which is over the cap and stays
/// sequential.
pub(super) const MAX_N: usize = 8192;

/// Four independent chains. Wider blocks raised the cycle count.
const BLOCK: usize = 4;

pub(super) fn use_parallel(n: usize) -> bool {
    (MIN_N..=MAX_N).contains(&n) && configured_threads() > 1
}

/// Packed lower triangle, including the diagonal. Same length as `upper`.
pub(super) fn pack_lower(upper: &[f32], n: usize) -> Vec<f32> {
    let mut lower = vec![0.0f32; n * (n + 1) / 2];
    for j in 0..n {
        let dest = tri(j);
        for i in 0..=j {
            lower[dest + i] = upper[upper_row(n, i) + (j - i)];
        }
    }
    lower
}

pub(super) fn right_mult_prepared(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
) {
    right_mult_with_lower(upper, lower, n, vector, result, configured_threads());
}

/// `threads == 1` still runs the gather, so tests can compare that kernel
/// with the sequential one without the environment.
#[cfg(test)]
pub(super) fn right_mult_ff_with_threads(
    upper: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
    threads: usize,
) {
    if n == 0 {
        return;
    }
    let lower = pack_lower(upper, n);
    right_mult_with_lower(upper, &lower, n, vector, result, threads);
}

fn right_mult_with_lower(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
    threads: usize,
) {
    if n == 0 {
        return;
    }
    debug_assert!(lower.len() >= n * (n + 1) / 2);
    debug_assert!(upper.len() >= n * (n + 1) / 2);
    let vector = &vector[..n];
    let result = &mut result[..n];
    graphviz_pure_core::par::for_each_chunk_mut(result, threads, |range, slot| {
        owned_rows(upper, lower, n, vector, slot, range.start);
    });
}

fn owned_rows(upper: &[f32], lower: &[f32], n: usize, vector: &[f32], slot: &mut [f32], j0: usize) {
    let mut local = 0;
    let mut j = j0;
    while local + BLOCK <= slot.len() {
        gather_block::<BLOCK>(upper, lower, n, vector, &mut slot[local..], j);
        local += BLOCK;
        j += BLOCK;
    }
    while local < slot.len() {
        gather_block::<1>(upper, lower, n, vector, &mut slot[local..], j);
        local += 1;
        j += 1;
    }
}

#[inline(always)]
fn gather_block<const W: usize>(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    vector: &[f32],
    slot: &mut [f32],
    j0: usize,
) {
    debug_assert!(slot.len() >= W);
    debug_assert!(j0 + W <= n);
    let mut acc = [0.0f32; W];
    let mut lower_at = [0usize; W];
    let mut upper_at = [0usize; W];
    for k in 0..W {
        let col = j0 + k;
        lower_at[k] = tri(col);
        upper_at[k] = upper_row(n, col);
    }
    // Common gather prefix. Column `j0 + k` still has `k` entries after this.
    for i in 0..j0 {
        let scale = vector[i];
        for k in 0..W {
            acc[k] = lower[lower_at[k] + i].mul_add(scale, acc[k]);
        }
    }
    for k in 1..W {
        for i in j0..(j0 + k) {
            acc[k] = lower[lower_at[k] + i].mul_add(vector[i], acc[k]);
        }
    }
    let mut res = [0.0f32; W];
    for k in 0..W {
        res[k] = upper[upper_at[k]].mul_add(vector[j0 + k], 0.0);
    }
    // Shortest remaining dot, then the longer rows finish alone.
    let short_tail = n - j0 - W;
    for t in 0..short_tail {
        for k in 0..W {
            res[k] = upper[upper_at[k] + 1 + t].mul_add(vector[j0 + k + 1 + t], res[k]);
        }
    }
    for k in 0..W {
        let tail = n - (j0 + k) - 1;
        for t in short_tail..tail {
            res[k] = upper[upper_at[k] + 1 + t].mul_add(vector[j0 + k + 1 + t], res[k]);
        }
    }
    for k in 0..W {
        // Store the gather, then add the dot in another function. Doing both
        // here lets LLVM turn them into one commutative `fadd`.
        if let Some(cell) = slot[k..].first_mut() {
            *cell = acc[k];
        }
        add_dot(&mut slot[k..], res[k]);
    }
}

/// `slot[0] += term`, a plain add, not an FMA.
///
/// Outlined on purpose, and it must not also store `acc`. An inlined `+=` is a
/// commutative `fadd`, and LLVM may swap the operands. When both are NaN, x86
/// `addss` keeps the source operand's payload, so the swap changes bits. The
/// sequential kernel's `result[i] += res` does not swap. A call keeps that
/// order at every opt level.
#[inline(never)]
fn add_dot(slot: &mut [f32], term: f32) {
    debug_assert!(!slot.is_empty());
    if let Some(slot) = slot.first_mut() {
        *slot += term;
    }
}

fn tri(j: usize) -> usize {
    j * (j + 1) / 2
}

fn upper_row(n: usize, i: usize) -> usize {
    if i == 0 { 0 } else { i * n - i * (i - 1) / 2 }
}

fn configured_threads() -> usize {
    static CACHED: OnceLock<usize> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let available = std::thread::available_parallelism()
            .map(|count| count.get())
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
            Ok(count) if count >= 1 => count,
            _ => 1,
        },
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

    #[test]
    fn lower_row_is_the_upper_column_plus_diagonal() {
        let n = 6;
        let upper: Vec<f32> = (0..n * (n + 1) / 2).map(|i| i as f32).collect();
        let lower = pack_lower(&upper, n);
        assert_eq!(lower.len(), upper.len());
        for j in 0..n {
            for i in 0..=j {
                assert_eq!(
                    lower[tri(j) + i],
                    upper[upper_row(n, i) + (j - i)],
                    "i={i} j={j}"
                );
            }
        }
    }
}
