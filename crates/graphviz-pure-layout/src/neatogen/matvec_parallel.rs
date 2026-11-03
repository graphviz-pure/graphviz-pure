//! Parallel `right_mult_with_vector_ff` by output range.
//!
//! After the sequential 8-wide kernel (#705), a lower-triangle gather is
//! slower than one thread even at n = 3000: that gather rebuilds each output
//! from a second copy of the triangle, while the sequential kernel streams
//! the packed upper rows it already holds. Workers therefore run that same
//! kernel on a contiguous range of output rows.
//!
//! A worker that owns `result[j0..j1]` first scatters rows `0..j0` into that
//! slice (`fma(A[i, j], v[i], result[j])` for increasing `i`, then increasing
//! `j` inside each row). It then runs the 8-wide, 4-wide, and 1-wide blocks
//! from row `j0`, with scatters clipped to `j1`. For each owned `result[j]`
//! the IEEE sequence is still increasing `i` through `j-1`, then a plain add
//! of row `j`'s left-to-right dot from the diagonal through column `n-1`.
//! The first chunk has an empty prefix and is the sequential kernel on those
//! rows. There is no extra triangle copy.
//!
//! Each output does `n` FMAs, so equal `j` counts are equal work.
//! `GRAPHVIZ_PURE_THREADS=1` stays on the sequential kernel. Unset, the count
//! is [`super::par::DEFAULT_THREADS`], and no count exceeds `available_parallelism`.
//! Conjugate gradient keeps one worker set for each solve; spawning on every
//! mat-vec was most of the system time. A spawn the OS refuses runs on the
//! caller.

use std::ops::Range;
use std::sync::{Arc, mpsc};

use super::matrix_ops::{add_slot, dot_from_zero, fma_slot, lane_at, lane_f32, scatter_fma};

/// Sequential below this.
///
/// After #705 the sequential 8-wide kernel is as fast as one thread of the
/// old lower-triangle gather. Workers run that kernel on an output range,
/// with no extra triangle copy. On the shipped musl x86-64-v3 PGO binary,
/// four threads were slower than one at every size timed (pinned 1300 and
/// 1400, sparse 2000): sparse_2000 was 5.18 s at one thread and 10.80 s at
/// four, with CPU 3.7×. Sharing `owned_rows` so PGO trains the sequential
/// path still left prefix scatter and clipped 8-wide blocks cold — training
/// never reaches this gate. A gnu v3 build without PGO had a modest win;
/// that is not the shipped shape. Until four threads beat one there, stay
/// sequential. Unit tests still run the owned kernel at every thread count.
pub(super) const MIN_N: usize = 1_000_000;

pub(super) fn use_parallel(n: usize) -> bool {
    n >= MIN_N && super::par::configured_threads() > 1
}

pub(super) fn right_mult_prepared(upper: &[f32], n: usize, vector: &[f32], result: &mut [f32]) {
    right_mult_with_threads(upper, n, vector, result, super::par::configured_threads());
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
    n: usize,
    body: impl FnOnce(&mut MatvecTeam<'_>) -> R,
) -> R {
    let threads = super::par::configured_threads().max(1).min(n.max(1));
    if n == 0 || threads == 1 {
        let mut team = MatvecTeam {
            upper,
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
                matvec_worker(rx, done_tx, upper, n);
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
            owned_rows(self.upper, n, vector, &mut result[..n], 0);
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
        let main = self.ranges[0].clone();
        owned_rows(
            upper,
            n,
            shared.as_slice(),
            &mut result[main.start..main.end],
            main.start,
        );
        for range in inline {
            owned_rows(
                upper,
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

fn matvec_worker(rx: mpsc::Receiver<Job>, done: mpsc::Sender<Vec<f32>>, upper: &[f32], n: usize) {
    while let Ok(job) = rx.recv() {
        match job {
            Job::Stop => break,
            Job::Run {
                j0,
                count,
                vector,
                mut slot,
            } => {
                owned_rows(upper, n, vector.as_slice(), &mut slot[..count], j0);
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
    super::par::split_ranges(n, threads)
}

/// `threads == 1` still runs the owned kernel, so tests can compare it with
/// the sequential one without the environment.
#[cfg(test)]
pub(super) fn right_mult_ff_with_threads(
    upper: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
    threads: usize,
) {
    right_mult_with_threads(upper, n, vector, result, threads);
}

fn right_mult_with_threads(
    upper: &[f32],
    n: usize,
    vector: &[f32],
    result: &mut [f32],
    threads: usize,
) {
    if n == 0 {
        return;
    }
    debug_assert!(upper.len() >= n * (n + 1) / 2);
    let vector = &vector[..n];
    let result = &mut result[..n];
    super::par::for_each_chunk_mut(result, threads, |range, slot| {
        owned_rows(upper, n, vector, slot, range.start);
    });
}

pub(super) fn owned_rows(upper: &[f32], n: usize, vector: &[f32], slot: &mut [f32], j0: usize) {
    debug_assert!(j0 + slot.len() <= n);
    slot.fill(0.0);
    prefix_scatter(upper, n, vector, slot, j0);
    let mut packed = &upper[upper_row(n, j0)..];
    let mut vector = &vector[j0..n];
    let mut slot = slot;
    while slot.len() >= 8 && vector.len() >= 8 {
        let (packed_rest, slot_rest) = block8_owned(packed, vector, slot);
        packed = packed_rest;
        slot = slot_rest;
        vector = vector.split_at(8).1;
    }
    while slot.len() >= 4 && vector.len() >= 4 {
        let (packed_rest, slot_rest) = block4_owned(packed, vector, slot);
        packed = packed_rest;
        slot = slot_rest;
        vector = vector.split_at(4).1;
    }
    while !slot.is_empty() && !vector.is_empty() {
        let (packed_rest, slot_rest) = row_owned(packed, vector, slot);
        packed = packed_rest;
        slot = slot_rest;
        vector = vector.split_at(1).1;
    }
}

/// Rows `0..j0` scatter into `slot` (`result[j0..j1]`). Increasing `i`, then
/// increasing `j` inside each row, which is increasing `i` for each output.
fn prefix_scatter(upper: &[f32], n: usize, vector: &[f32], slot: &mut [f32], j0: usize) {
    if j0 == 0 || slot.is_empty() {
        return;
    }
    let width = slot.len();
    for i in 0..j0 {
        let start = upper_row(n, i) + (j0 - i);
        scatter_fma(slot, &upper[start..start + width], vector[i]);
    }
}

fn scatter_clip(dst: &mut [f32], row: &[f32], factor: f32) {
    let n = dst.len().min(row.len());
    scatter_fma(&mut dst[..n], &row[..n], factor);
}

/// Sequential 8-wide block on an owned prefix of `result`. Dots use the full
/// remaining columns; scatters stop at `result.len()`.
fn block8_owned<'p, 'r>(
    packed: &'p [f32],
    vector: &[f32],
    result: &'r mut [f32],
) -> (&'p [f32], &'r mut [f32]) {
    let n = vector.len();
    debug_assert!(n >= 8);
    debug_assert!(result.len() >= 8);
    debug_assert!(result.len() <= n);

    let (row0, packed) = packed.split_at(n);
    let (row1, packed) = packed.split_at(n.wrapping_sub(1));
    let (row2, packed) = packed.split_at(n.wrapping_sub(2));
    let (row3, packed) = packed.split_at(n.wrapping_sub(3));
    let (row4, packed) = packed.split_at(n.wrapping_sub(4));
    let (row5, packed) = packed.split_at(n.wrapping_sub(5));
    let (row6, packed) = packed.split_at(n.wrapping_sub(6));
    let (row7, packed_rest) = packed.split_at(n.wrapping_sub(7));

    let (v_head, v_tail) = vector.split_at(8);
    let mut lanes = [0.0f32; 8];
    for (lane, &value) in lanes.iter_mut().zip(v_head) {
        *lane = value;
    }
    let [v0, v1, v2, v3, v4, v5, v6, v7] = lanes;

    let (pre0, a0_tail) = row0.split_at(8);
    let (pre1, a1_tail) = row1.split_at(7);
    let (pre2, a2_tail) = row2.split_at(6);
    let (pre3, a3_tail) = row3.split_at(5);
    let (pre4, a4_tail) = row4.split_at(4);
    let (pre5, a5_tail) = row5.split_at(3);
    let (pre6, a6_tail) = row6.split_at(2);
    let (pre7, a7_tail) = row7.split_at(1);

    let mut v_prefix = v_head;
    let mut res0 = dot_from_zero(pre0, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res1 = dot_from_zero(pre1, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res2 = dot_from_zero(pre2, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res3 = dot_from_zero(pre3, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res4 = dot_from_zero(pre4, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res5 = dot_from_zero(pre5, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res6 = dot_from_zero(pre6, v_prefix);
    v_prefix = v_prefix.split_at(1).1;
    let mut res7 = dot_from_zero(pre7, v_prefix);

    let (head, result_tail) = result.split_at_mut(8);
    let (slot0, head) = head.split_at_mut(1);
    let (slot1, head) = head.split_at_mut(1);
    let (slot2, head) = head.split_at_mut(1);
    let (slot3, head) = head.split_at_mut(1);
    let (slot4, head) = head.split_at_mut(1);
    let (slot5, head) = head.split_at_mut(1);
    let (slot6, slot7) = head.split_at_mut(1);

    fma_slot(slot1, lane_at(pre0, 1), v0);
    fma_slot(slot2, lane_at(pre0, 2), v0);
    fma_slot(slot2, lane_at(pre1, 1), v1);
    fma_slot(slot3, lane_at(pre0, 3), v0);
    fma_slot(slot3, lane_at(pre1, 2), v1);
    fma_slot(slot3, lane_at(pre2, 1), v2);
    fma_slot(slot4, lane_at(pre0, 4), v0);
    fma_slot(slot4, lane_at(pre1, 3), v1);
    fma_slot(slot4, lane_at(pre2, 2), v2);
    fma_slot(slot4, lane_at(pre3, 1), v3);
    fma_slot(slot5, lane_at(pre0, 5), v0);
    fma_slot(slot5, lane_at(pre1, 4), v1);
    fma_slot(slot5, lane_at(pre2, 3), v2);
    fma_slot(slot5, lane_at(pre3, 2), v3);
    fma_slot(slot5, lane_at(pre4, 1), v4);
    fma_slot(slot6, lane_at(pre0, 6), v0);
    fma_slot(slot6, lane_at(pre1, 5), v1);
    fma_slot(slot6, lane_at(pre2, 4), v2);
    fma_slot(slot6, lane_at(pre3, 3), v3);
    fma_slot(slot6, lane_at(pre4, 2), v4);
    fma_slot(slot6, lane_at(pre5, 1), v5);
    fma_slot(slot7, lane_at(pre0, 7), v0);
    fma_slot(slot7, lane_at(pre1, 6), v1);
    fma_slot(slot7, lane_at(pre2, 5), v2);
    fma_slot(slot7, lane_at(pre3, 4), v3);
    fma_slot(slot7, lane_at(pre4, 3), v4);
    fma_slot(slot7, lane_at(pre5, 2), v5);
    fma_slot(slot7, lane_at(pre6, 1), v6);

    debug_assert_eq!(a0_tail.len(), v_tail.len());
    let ntail = a0_tail.len();
    let a1_tail = &a1_tail[..ntail];
    let a2_tail = &a2_tail[..ntail];
    let a3_tail = &a3_tail[..ntail];
    let a4_tail = &a4_tail[..ntail];
    let a5_tail = &a5_tail[..ntail];
    let a6_tail = &a6_tail[..ntail];
    let a7_tail = &a7_tail[..ntail];
    let v_tail = &v_tail[..ntail];
    for ((((((((&p0, &p1), &p2), &p3), &p4), &p5), &p6), &p7), &vj) in a0_tail
        .iter()
        .zip(a1_tail)
        .zip(a2_tail)
        .zip(a3_tail)
        .zip(a4_tail)
        .zip(a5_tail)
        .zip(a6_tail)
        .zip(a7_tail)
        .zip(v_tail)
    {
        res0 = p0.mul_add(vj, res0);
        res1 = p1.mul_add(vj, res1);
        res2 = p2.mul_add(vj, res2);
        res3 = p3.mul_add(vj, res3);
        res4 = p4.mul_add(vj, res4);
        res5 = p5.mul_add(vj, res5);
        res6 = p6.mul_add(vj, res6);
        res7 = p7.mul_add(vj, res7);
    }

    scatter_clip(result_tail, a0_tail, v0);
    scatter_clip(result_tail, a1_tail, v1);
    scatter_clip(result_tail, a2_tail, v2);
    scatter_clip(result_tail, a3_tail, v3);
    scatter_clip(result_tail, a4_tail, v4);
    scatter_clip(result_tail, a5_tail, v5);
    scatter_clip(result_tail, a6_tail, v6);
    scatter_clip(result_tail, a7_tail, v7);
    add_slot(slot0, res0);
    add_slot(slot1, res1);
    add_slot(slot2, res2);
    add_slot(slot3, res3);
    add_slot(slot4, res4);
    add_slot(slot5, res5);
    add_slot(slot6, res6);
    add_slot(slot7, res7);

    (packed_rest, result_tail)
}

fn block4_owned<'p, 'r>(
    packed: &'p [f32],
    vector: &[f32],
    result: &'r mut [f32],
) -> (&'p [f32], &'r mut [f32]) {
    let n = vector.len();
    debug_assert!(n >= 4);
    debug_assert!(result.len() >= 4);
    debug_assert!(result.len() <= n);

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

    debug_assert_eq!(a0_tail.len(), v_tail.len());
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

    fma_slot(slot1, p01, v0);
    fma_slot(slot2, p02, v0);
    fma_slot(slot2, p12, v1);
    fma_slot(slot3, p03, v0);
    fma_slot(slot3, p13, v1);
    fma_slot(slot3, p23, v2);
    scatter_clip(result_tail, a0_tail, v0);
    scatter_clip(result_tail, a1_tail, v1);
    scatter_clip(result_tail, a2_tail, v2);
    scatter_clip(result_tail, a3_tail, v3);
    add_slot(slot0, res0);
    add_slot(slot1, res1);
    add_slot(slot2, res2);
    add_slot(slot3, res3);

    (packed_rest, result_tail)
}

fn row_owned<'p, 'r>(
    packed: &'p [f32],
    vector: &[f32],
    result: &'r mut [f32],
) -> (&'p [f32], &'r mut [f32]) {
    let n = vector.len();
    debug_assert!(n >= 1);
    debug_assert!(!result.is_empty());
    debug_assert!(result.len() <= n);
    let (row, packed_rest) = packed.split_at(n);
    let (diag, off) = row.split_at(1);
    let vi = lane_f32(vector);
    let v_tail = vector.split_at(1).1;
    let mut res = lane_f32(diag).mul_add(vi, 0.0f32);
    for (&coefficient, &vj) in off.iter().zip(v_tail) {
        res = coefficient.mul_add(vj, res);
    }
    let (slot, result_tail) = result.split_at_mut(1);
    scatter_clip(result_tail, off, vi);
    add_slot(slot, res);
    (packed_rest, result_tail)
}

fn upper_row(n: usize, i: usize) -> usize {
    if i == 0 { 0 } else { i * n - i * (i - 1) / 2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_threads_treats_one_as_sequential_and_rejects_garbage() {
        use super::super::par::{DEFAULT_THREADS, parse_threads};
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
    fn team_matches_the_owned_kernel_including_a_nan() {
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
        let mut direct = vec![1.0f32; n + 2];
        let mut via_team = direct.clone();
        right_mult_ff_with_threads(&upper, n, &vector, &mut direct, 3);
        with_team(&upper, n, |team| {
            team.multiply(&vector, &mut via_team);
        });
        for (i, (got, expect)) in via_team.iter().zip(&direct).enumerate() {
            if got.is_nan() && expect.is_nan() {
                continue;
            }
            assert_eq!(got.to_bits(), expect.to_bits(), "i={i}");
        }
    }

    #[test]
    fn refused_spawn_matches_the_owned_kernel_including_a_nan() {
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
        let mut sequential = vec![1.0f32; n];
        right_mult_ff_with_threads(&upper, n, &vector, &mut sequential, 1);
        for limit in [0usize, 1] {
            let _limit = super::super::par::SpawnLimit::at_most(limit);
            let mut got = vec![1.0f32; n];
            right_mult_ff_with_threads(&upper, n, &vector, &mut got, 4);
            for (i, (got, expect)) in got.iter().zip(&sequential).enumerate() {
                if got.is_nan() && expect.is_nan() {
                    continue;
                }
                assert_eq!(got.to_bits(), expect.to_bits(), "owned limit={limit} i={i}");
            }
            let mut via_team = vec![1.0f32; n];
            with_team(&upper, n, |team| {
                team.multiply(&vector, &mut via_team);
            });
            for (i, (got, expect)) in via_team.iter().zip(&sequential).enumerate() {
                if got.is_nan() && expect.is_nan() {
                    continue;
                }
                assert_eq!(got.to_bits(), expect.to_bits(), "team limit={limit} i={i}");
            }
        }
    }

    #[test]
    fn owned_chunk_matches_sequential_bits() {
        use super::super::matrix_ops::right_mult_with_vector_ff;
        let n = 37;
        let mut state = 0x51u32;
        let len = n * (n + 1) / 2;
        let upper: Vec<f32> = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits((state >> 9) | 0x3f80_0000)
            })
            .collect();
        let vector: Vec<f32> = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits((state >> 9) | 0x3f80_0000)
            })
            .collect();
        let mut sequential = vec![0.0f32; n];
        right_mult_with_vector_ff(&upper, n, &vector, &mut sequential);
        let mut owned = vec![0.0f32; n];
        owned_rows(&upper, n, &vector, &mut owned, 0);
        for (i, (got, expect)) in owned.iter().zip(&sequential).enumerate() {
            assert_eq!(got.to_bits(), expect.to_bits(), "full i={i}");
        }
        for threads in [2usize, 3, 4, 8] {
            let mut split = vec![0.0f32; n];
            right_mult_ff_with_threads(&upper, n, &vector, &mut split, threads);
            for (i, (got, expect)) in split.iter().zip(&sequential).enumerate() {
                assert_eq!(got.to_bits(), expect.to_bits(), "threads={threads} i={i}");
            }
        }
    }
}
