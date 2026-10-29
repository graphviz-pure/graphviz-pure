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
//! Equal `j` counts are equal work. Threads run eight consecutive outputs
//! together: the chains for `j` through `j + 7` advance over the same `i`,
//! and each chain stays in increasing `i`. Those rows of the lower copy are
//! adjacent, so the loads stream. The in-block tail (row `j + k` has `k`
//! extra gather terms, and `8 - k` dot terms before the shared tail) is the
//! same shape as the blocked kernel's triangle.
//!
//! Eight chains, not four. One dependent FMA chain retires about one FMA
//! every four cycles, and four chains were still short of two FMA pipes.
//! `GRAPHVIZ_PURE_THREADS=1` stays on the sequential kernel. Unset, the count
//! is [`DEFAULT_THREADS`], and no count exceeds `available_parallelism`.
//! Conjugate gradient keeps one worker set for the solve; spawning on every
//! mat-vec was most of the system time. A spawn the OS refuses runs on the
//! caller.

use std::ops::Range;
use std::sync::{Arc, OnceLock, mpsc};

/// Sequential below this.
///
/// Two threads of the 8-wide gather beat the blocked kernel at n = 1300
/// (0.29 ms against 0.32 ms) and at every larger width that was timed. At
/// n = 1200 the edge was about 3%, at n = 1100 the two matched, and at
/// n = 1000 two threads lost. Four threads win from about n = 1000. The
/// threshold follows the two-thread win.
pub(super) const MIN_N: usize = 1300;

/// Sequential above this. The lower copy is `n*(n+1)/2` extra `f32`s:
/// 8_004_000 bytes at `n = 2000`, 134_250_496 bytes at this cap, and
/// 200_020_000 bytes at `n = 10000`, which is over the cap and stays
/// sequential.
pub(super) const MAX_N: usize = 8192;

/// Eight independent chains, one per output. Four left the FMA pipes short.
const BLOCK: usize = 8;

/// Threads used when `GRAPHVIZ_PURE_THREADS` is unset.
///
/// Four beat eight on rnd2000-as-neato (4.35 s against 4.58 s median wall).
/// An explicit count may be higher, and both are capped by
/// `available_parallelism`.
const DEFAULT_THREADS: usize = 4;

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

/// One worker set for many mat-vecs of the same matrix.
///
/// `right_mult_prepared` spawns and joins on every call. A conjugate-gradient
/// solve does that thousands of times; the threads here stay up for the solve
/// and wait on a channel between mat-vecs. The main thread owns chunk 0 and
/// the workers own the rest. Each worker writes a private buffer, and the main
/// thread copies it back after the barrier, so the output floats are the same
/// ones `owned_rows` produced.
pub(super) struct MatvecTeam<'a> {
    upper: &'a [f32],
    lower: &'a [f32],
    n: usize,
    ranges: Vec<Range<usize>>,
    /// Worker chunks that actually spawned, parallel to `txs`.
    worker_ranges: Vec<Range<usize>>,
    /// Worker chunks the OS would not spawn. The main thread runs these.
    inline: Vec<Range<usize>>,
    txs: Vec<mpsc::Sender<Job>>,
    dones: Vec<mpsc::Receiver<Vec<f32>>>,
    slots: Vec<Vec<f32>>,
    vector_buf: Vec<f32>,
}

enum Job {
    Run {
        j0: usize,
        count: usize,
        vector: Arc<Vec<f32>>,
        slot: Vec<f32>,
    },
    Stop,
}

pub(super) fn with_team<R>(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    body: impl FnOnce(&mut MatvecTeam<'_>) -> R,
) -> R {
    let threads = configured_threads().max(1).min(n.max(1));
    if n == 0 || threads == 1 {
        let mut team = MatvecTeam {
            upper,
            lower,
            n,
            ranges: Vec::new(),
            worker_ranges: Vec::new(),
            inline: Vec::new(),
            txs: Vec::new(),
            dones: Vec::new(),
            slots: Vec::new(),
            vector_buf: Vec::new(),
        };
        return body(&mut team);
    }
    let ranges = chunk_ranges(n, threads);
    std::thread::scope(|scope| {
        let mut joins = Vec::new();
        let mut txs = Vec::new();
        let mut dones = Vec::new();
        let mut slots = Vec::new();
        let mut worker_ranges = Vec::new();
        let mut inline = Vec::new();
        for range in ranges.iter().skip(1).cloned() {
            let (tx, rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let handle = super::par::try_spawn(scope, move || {
                matvec_worker(rx, done_tx, upper, lower, n);
            });
            if let Some(handle) = handle {
                joins.push(handle);
                slots.push(vec![0.0f32; range.len()]);
                worker_ranges.push(range);
                txs.push(tx);
                dones.push(done_rx);
            } else {
                inline.push(range);
            }
        }
        let mut team = MatvecTeam {
            upper,
            lower,
            n,
            ranges,
            worker_ranges,
            inline,
            txs,
            dones,
            slots,
            vector_buf: vec![0.0f32; n],
        };
        let result = body(&mut team);
        for tx in &team.txs {
            let _ = tx.send(Job::Stop);
        }
        // Join after Stop. Dropping a handle earlier would wait out the worker
        // before the solve had sent it any work.
        drop(joins);
        result
    })
}

impl MatvecTeam<'_> {
    pub(super) fn multiply(&mut self, vector: &[f32], result: &mut [f32]) {
        let n = self.n;
        if n == 0 {
            return;
        }
        debug_assert!(vector.len() >= n && result.len() >= n);
        if self.txs.is_empty() {
            owned_rows(self.upper, self.lower, n, vector, &mut result[..n], 0);
            return;
        }
        self.vector_buf[..n].copy_from_slice(&vector[..n]);
        let shared = Arc::new(std::mem::take(&mut self.vector_buf));
        let slots = std::mem::take(&mut self.slots);
        let worker_ranges = self.worker_ranges.clone();
        let inline = self.inline.clone();
        for (tx, (range, slot)) in self.txs.iter().zip(worker_ranges.iter().zip(slots)) {
            tx.send(Job::Run {
                j0: range.start,
                count: range.len(),
                vector: Arc::clone(&shared),
                slot,
            })
            .expect("worker stopped");
        }
        let upper = self.upper;
        let lower = self.lower;
        let main = self.ranges[0].clone();
        owned_rows(
            upper,
            lower,
            n,
            shared.as_slice(),
            &mut result[main.start..main.end],
            main.start,
        );
        for range in inline {
            owned_rows(
                upper,
                lower,
                n,
                shared.as_slice(),
                &mut result[range.start..range.end],
                range.start,
            );
        }
        let workers = worker_ranges.len();
        for i in 0..workers {
            let slot = self.dones[i].recv().expect("worker stopped");
            let range = worker_ranges[i].clone();
            result[range.start..range.end].copy_from_slice(&slot[..range.len()]);
            self.slots.push(slot);
        }
        self.vector_buf = match Arc::try_unwrap(shared) {
            Ok(buf) => buf,
            Err(arc) => (*arc).clone(),
        };
    }
}

fn matvec_worker(
    rx: mpsc::Receiver<Job>,
    done: mpsc::Sender<Vec<f32>>,
    upper: &[f32],
    lower: &[f32],
    n: usize,
) {
    while let Ok(job) = rx.recv() {
        match job {
            Job::Stop => break,
            Job::Run {
                j0,
                count,
                vector,
                mut slot,
            } => {
                owned_rows(upper, lower, n, vector.as_slice(), &mut slot[..count], j0);
                drop(vector);
                if done.send(slot).is_err() {
                    break;
                }
            }
        }
    }
}

/// Same split as [`super::par`]. The earliest chunk takes the remainder, so
/// `n = 10`, `threads = 3` is `4 + 3 + 3`.
fn chunk_ranges(n: usize, threads: usize) -> Vec<Range<usize>> {
    let threads = threads.max(1).min(n.max(1));
    let mut ranges = Vec::with_capacity(threads);
    let mut base = 0;
    let mut left = n;
    let mut remaining = threads;
    while remaining > 0 && left > 0 {
        let len = left / remaining + usize::from(!left.is_multiple_of(remaining));
        ranges.push(base..base + len);
        base += len;
        left -= len;
        remaining -= 1;
    }
    ranges
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
    super::par::for_each_chunk_mut(result, threads, |range, slot| {
        owned_rows(upper, lower, n, vector, slot, range.start);
    });
}

fn owned_rows(upper: &[f32], lower: &[f32], n: usize, vector: &[f32], slot: &mut [f32], j0: usize) {
    let mut local = 0;
    let mut j = j0;
    while local + BLOCK <= slot.len() {
        gather_block8(upper, lower, n, vector, &mut slot[local..], j);
        local += BLOCK;
        j += BLOCK;
    }
    while local < slot.len() {
        gather_row(upper, lower, n, vector, &mut slot[local..], j);
        local += 1;
        j += 1;
    }
}

/// Eight outputs starting at `j`. Gather from the lower copy, then the upper
/// dot, each chain in increasing column order.
fn gather_block8(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    vector: &[f32],
    slot: &mut [f32],
    j: usize,
) {
    debug_assert!(slot.len() >= BLOCK);
    debug_assert!(j + BLOCK <= n);
    let (acc, dot) = chains8(upper, lower, n, vector, j);
    for k in 0..BLOCK {
        // Store the gather, then add the dot in another function. Doing both
        // in one function lets LLVM turn them into one commutative `fadd`.
        if let Some(cell) = slot[k..].first_mut() {
            *cell = acc[k];
        }
        add_dot(&mut slot[k..], dot[k]);
    }
}

fn chains8(
    upper: &[f32],
    lower: &[f32],
    n: usize,
    vector: &[f32],
    j: usize,
) -> ([f32; BLOCK], [f32; BLOCK]) {
    let mut rows: [&[f32]; BLOCK] = [&[]; BLOCK];
    let mut cursor = &lower[tri(j)..];
    for (k, row_slot) in rows.iter_mut().enumerate() {
        let (row, rest) = cursor.split_at(j + 1 + k);
        *row_slot = row;
        cursor = rest;
    }
    let mut pre: [&[f32]; BLOCK] = [&[]; BLOCK];
    let mut tail: [&[f32]; BLOCK] = [&[]; BLOCK];
    for (k, row) in rows.iter().enumerate() {
        let (prefix, rest) = row.split_at(j);
        // `rest` is the gather tail plus the diagonal. The diagonal stays in
        // the dot, so the tail length is `k`.
        pre[k] = prefix;
        tail[k] = &rest[..k];
    }
    let v_pre = &vector[..j];
    let mut acc = [0.0f32; BLOCK];
    for i in 0..v_pre.len() {
        let vi = v_pre[i];
        acc[0] = pre[0][i].mul_add(vi, acc[0]);
        acc[1] = pre[1][i].mul_add(vi, acc[1]);
        acc[2] = pre[2][i].mul_add(vi, acc[2]);
        acc[3] = pre[3][i].mul_add(vi, acc[3]);
        acc[4] = pre[4][i].mul_add(vi, acc[4]);
        acc[5] = pre[5][i].mul_add(vi, acc[5]);
        acc[6] = pre[6][i].mul_add(vi, acc[6]);
        acc[7] = pre[7][i].mul_add(vi, acc[7]);
    }
    for k in 1..BLOCK {
        for t in 0..k {
            acc[k] = tail[k][t].mul_add(vector[j + t], acc[k]);
        }
    }

    let mut urows: [&[f32]; BLOCK] = [&[]; BLOCK];
    let mut cursor = &upper[upper_row(n, j)..];
    let n_from = n - j;
    for (k, row_slot) in urows.iter_mut().enumerate() {
        let (row, rest) = cursor.split_at(n_from - k);
        *row_slot = row;
        cursor = rest;
    }
    let mut heads: [&[f32]; BLOCK] = [&[]; BLOCK];
    let mut dtails: [&[f32]; BLOCK] = [&[]; BLOCK];
    for (k, row) in urows.iter().enumerate() {
        // Columns `j+k .. j+7`, diagonal first. The shared tail then starts
        // at column `j+8` for every row.
        let (head, rest) = row.split_at(BLOCK - k);
        heads[k] = head;
        dtails[k] = rest;
    }
    let mut dot = [0.0f32; BLOCK];
    for k in 0..BLOCK {
        let mut res = heads[k][0].mul_add(vector[j + k], 0.0);
        for t in 1..heads[k].len() {
            res = heads[k][t].mul_add(vector[j + k + t], res);
        }
        dot[k] = res;
    }
    let ntail = dtails[0].len();
    let v_tail = &vector[j + BLOCK..j + BLOCK + ntail];
    for t in 0..ntail {
        let vj = v_tail[t];
        dot[0] = dtails[0][t].mul_add(vj, dot[0]);
        dot[1] = dtails[1][t].mul_add(vj, dot[1]);
        dot[2] = dtails[2][t].mul_add(vj, dot[2]);
        dot[3] = dtails[3][t].mul_add(vj, dot[3]);
        dot[4] = dtails[4][t].mul_add(vj, dot[4]);
        dot[5] = dtails[5][t].mul_add(vj, dot[5]);
        dot[6] = dtails[6][t].mul_add(vj, dot[6]);
        dot[7] = dtails[7][t].mul_add(vj, dot[7]);
    }
    (acc, dot)
}

fn gather_row(upper: &[f32], lower: &[f32], n: usize, vector: &[f32], slot: &mut [f32], j: usize) {
    debug_assert!(!slot.is_empty());
    debug_assert!(j < n);
    let mut acc = 0.0f32;
    let grow = &lower[tri(j)..tri(j) + j];
    for (i, &coefficient) in grow.iter().enumerate() {
        acc = coefficient.mul_add(vector[i], acc);
    }
    let urow = &upper[upper_row(n, j)..upper_row(n, j) + (n - j)];
    let mut dot = urow[0].mul_add(vector[j], 0.0);
    for (t, &coefficient) in urow[1..].iter().enumerate() {
        dot = coefficient.mul_add(vector[j + 1 + t], dot);
    }
    if let Some(cell) = slot.first_mut() {
        *cell = acc;
    }
    add_dot(slot, dot);
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

/// `None` is [`DEFAULT_THREADS`], capped by `available`. An explicit count is
/// capped by `available` too. `0` and any other unreadable value are
/// sequential, so a bad setting does not spawn.
fn parse_threads(value: Option<&str>, available: usize) -> usize {
    let available = available.max(1);
    match value {
        None => DEFAULT_THREADS.min(available),
        Some(text) => match text.trim().parse::<usize>() {
            Ok(count) if count >= 1 => count.min(available),
            _ => 1,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_threads_treats_one_as_sequential_and_rejects_garbage() {
        assert_eq!(parse_threads(None, 8), DEFAULT_THREADS);
        assert_eq!(parse_threads(None, 2), 2);
        assert_eq!(parse_threads(None, 0), 1);
        assert_eq!(parse_threads(Some("1"), 8), 1);
        assert_eq!(parse_threads(Some("4"), 8), 4);
        assert_eq!(parse_threads(Some("8"), 4), 4);
        assert_eq!(parse_threads(Some("100000"), 8), 8);
        assert_eq!(parse_threads(Some("  3  "), 8), 3);
        assert_eq!(parse_threads(Some("0"), 8), 1);
        assert_eq!(parse_threads(Some(""), 8), 1);
        assert_eq!(parse_threads(Some("nope"), 8), 1);
    }

    #[test]
    fn team_matches_the_direct_gather_including_a_nan() {
        let n = 37;
        let mut state = 0x51u32;
        let len = n * (n + 1) / 2;
        let mut upper: Vec<f32> = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits(state | 0x3f80_0000)
            })
            .collect();
        upper[10] = f32::from_bits(0x7fc0_0001);
        let mut vector: Vec<f32> = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits(state | 0x3f80_0000)
            })
            .collect();
        vector[4] = f32::from_bits(0x7fc0_0000);
        let lower = pack_lower(&upper, n);
        let mut direct = vec![1.0f32; n + 2];
        let mut via_team = direct.clone();
        right_mult_ff_with_threads(&upper, n, &vector, &mut direct, 3);
        with_team(&upper, &lower, n, |team| {
            team.multiply(&vector, &mut via_team);
        });
        for (i, (got, expect)) in via_team.iter().zip(&direct).enumerate() {
            assert_eq!(got.to_bits(), expect.to_bits(), "i={i}");
        }
    }

    #[test]
    fn refused_spawn_matches_the_gather_including_a_nan() {
        let n = 40;
        let mut state = 0x51u32;
        let len = n * (n + 1) / 2;
        let mut upper: Vec<f32> = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits(state | 0x3f80_0000)
            })
            .collect();
        upper[10] = f32::from_bits(0x7fc0_0001);
        let mut vector: Vec<f32> = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits(state | 0x3f80_0000)
            })
            .collect();
        vector[4] = f32::from_bits(0x7fc0_0000);
        let lower = pack_lower(&upper, n);
        let mut sequential = vec![1.0f32; n];
        right_mult_ff_with_threads(&upper, n, &vector, &mut sequential, 1);
        for limit in [0usize, 1] {
            let _limit = super::super::par::SpawnLimit::at_most(limit);
            let mut got = vec![1.0f32; n];
            right_mult_ff_with_threads(&upper, n, &vector, &mut got, 4);
            for (i, (got, expect)) in got.iter().zip(&sequential).enumerate() {
                assert_eq!(
                    got.to_bits(),
                    expect.to_bits(),
                    "gather limit={limit} i={i}"
                );
            }
            let mut via_team = vec![1.0f32; n];
            with_team(&upper, &lower, n, |team| {
                team.multiply(&vector, &mut via_team);
            });
            for (i, (got, expect)) in via_team.iter().zip(&sequential).enumerate() {
                assert_eq!(got.to_bits(), expect.to_bits(), "team limit={limit} i={i}");
            }
        }
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
