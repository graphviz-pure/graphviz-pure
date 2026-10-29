//! Precomputed quadtree repulsion for `spring_electrical_embedding`.
//!
//! The node loop is Gauss–Seidel. Attraction reads neighbours' live
//! positions, and `move_node` writes `x[i]` before node `i + 1`, so the
//! whole force loop is not a parallel kernel. `repulsive_all` (graphs
//! smaller than the quadtree) reads every other live position and stays in
//! that loop. `spring_electrical_spring_embedding` has the same repulsive
//! term, plus spring terms that read live positions. Default smoothing is
//! none, so that embedding stays sequential.
//!
//! With a quadtree, node `i`'s repulsive inputs are the tree built at the
//! start of the iteration and `x[i]`. Nothing writes `x[i]` until node
//! `i`'s own move, so it is still the value from the rebuild.
//! `get_supernodes` reads the tree and that query point. The term
//!
//! `w * KP * (x[i][k] - center[k]) / pow(max(dist, MINDIST), 1-p)`
//!
//! does not read any other live position. Workers store one `dim`-vector
//! of those terms per supernode, in supernode order. The sequential loop
//! then runs as before: attraction first, then each term added into `f` in
//! that order, then the move. `counts` and `nsuper` are added in node
//! order in that same loop.
//!
//! The stored terms are a window of nodes that have not moved yet. The
//! first window is at most 64 nodes, sized so that `n` supernodes per node
//! fit in 32 MiB. Later windows use twice the most supernodes any node has
//! needed so far, and that plan also fits in 32 MiB. A window that stores
//! more than the budget is dropped and retried shorter. One node is never
//! split.

use super::{BH, attractive, move_node};
use crate::neatogen::par;
use crate::sparse::quad_tree::{QuadTree, Supernodes};
use crate::sparse::{MINDIST, SparseMatrix};
use graphviz_pure_core::cnum::c_max;
use graphviz_pure_core::math;
#[cfg(test)]
use std::cell::Cell;
use std::ops::Range;

/// Smallest level that precomputes repulsive terms.
///
/// Each level is gated on its own `n`. Four threads at 2000 nodes were
/// 1.04× one thread, the samples overlapped, and the run used about 28%
/// more CPU. The gate is 5000. Fewer than four threads stay on the
/// sequential term: two threads lost on the 20000-node graph. Below
/// [`super::QUADTREE_SIZE`] there is no tree.
pub(super) const MIN_N: usize = 5000;

/// Two threads lost on the 20000-node graph. Four is the default count.
const MIN_THREADS: usize = 4;

/// Resident precomputed terms. `4_000_000` `f64`s is 32 MiB.
const TERM_BUDGET_F64S: usize = 4_000_000;

/// First window, before any supernode count has been observed.
const PROBE_NODES: usize = 64;

/// Later windows reserve this multiple of the most supernodes seen so far.
const SUPERNODE_MARGIN: usize = 2;

#[cfg(test)]
thread_local! {
    static THREADS_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    static MIN_NODES_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    static TERM_BUDGET_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(super) struct ThreadsOverride {
    prev: Option<usize>,
}

#[cfg(test)]
impl ThreadsOverride {
    pub(super) fn set(n: usize) -> Self {
        let prev = THREADS_OVERRIDE.with(|cell| cell.replace(Some(n)));
        Self { prev }
    }
}

#[cfg(test)]
impl Drop for ThreadsOverride {
    fn drop(&mut self) {
        let prev = self.prev;
        THREADS_OVERRIDE.with(|cell| cell.set(prev));
    }
}

#[cfg(test)]
pub(super) struct MinNodesOverride {
    prev: Option<usize>,
}

#[cfg(test)]
impl MinNodesOverride {
    pub(super) fn set(n: usize) -> Self {
        let prev = MIN_NODES_OVERRIDE.with(|cell| cell.replace(Some(n)));
        Self { prev }
    }
}

#[cfg(test)]
impl Drop for MinNodesOverride {
    fn drop(&mut self) {
        let prev = self.prev;
        MIN_NODES_OVERRIDE.with(|cell| cell.set(prev));
    }
}

#[cfg(test)]
pub(super) struct TermBudgetOverride {
    prev: Option<usize>,
}

#[cfg(test)]
impl TermBudgetOverride {
    /// Cap stored term floats at `n` for this thread. `0` is one float.
    pub(super) fn set(n: usize) -> Self {
        let prev = TERM_BUDGET_OVERRIDE.with(|cell| cell.replace(Some(n.max(1))));
        Self { prev }
    }
}

#[cfg(test)]
impl Drop for TermBudgetOverride {
    fn drop(&mut self) {
        let prev = self.prev;
        TERM_BUDGET_OVERRIDE.with(|cell| cell.set(prev));
    }
}

fn run_threads() -> usize {
    #[cfg(test)]
    if let Some(n) = THREADS_OVERRIDE.with(Cell::get) {
        return n.max(1);
    }
    par::configured_threads()
}

fn min_nodes() -> usize {
    #[cfg(test)]
    if let Some(n) = MIN_NODES_OVERRIDE.with(Cell::get) {
        return n;
    }
    MIN_N
}

pub(super) fn use_parallel(n: usize) -> bool {
    n >= min_nodes() && run_threads() >= MIN_THREADS
}

struct NodeTerms {
    counts: f64,
    nsuper: usize,
}

#[derive(Default)]
struct Chunk {
    nodes: Vec<NodeTerms>,
    terms: Vec<f64>,
    sn: Supernodes,
}

/// Reused across the iterations of one embedding. The term buffers keep
/// their capacity, so a later iteration does not allocate them again.
#[derive(Default)]
pub(super) struct Scratch {
    chunks: Vec<Chunk>,
    #[cfg(test)]
    max_term_f64s: usize,
    #[cfg(test)]
    windows: usize,
}

impl Scratch {
    #[cfg(test)]
    fn resident_terms(&self) -> usize {
        self.max_term_f64s
    }
}

pub(super) fn accumulate_iteration(
    qt: &QuadTree,
    a: &SparseMatrix,
    dim: usize,
    x: &mut [f64],
    n: usize,
    kp: f64,
    p: f64,
    crk: f64,
    step: f64,
    f: &mut [f64],
    fnorm: &mut f64,
    counts_avg: &mut f64,
    nsuper_avg: &mut f64,
    scratch: &mut Scratch,
) {
    accumulate_with_threads(
        qt,
        a,
        dim,
        x,
        n,
        kp,
        p,
        crk,
        step,
        f,
        fnorm,
        counts_avg,
        nsuper_avg,
        run_threads(),
        scratch,
    );
}

fn accumulate_with_threads(
    qt: &QuadTree,
    a: &SparseMatrix,
    dim: usize,
    x: &mut [f64],
    n: usize,
    kp: f64,
    p: f64,
    crk: f64,
    step: f64,
    f: &mut [f64],
    fnorm: &mut f64,
    counts_avg: &mut f64,
    nsuper_avg: &mut f64,
    threads: usize,
    scratch: &mut Scratch,
) {
    if n == 0 || dim == 0 {
        return;
    }
    let mut start = 0;
    let mut max_nsuper = 0usize;
    let mut primed = false;
    while start < n {
        let mut nodes = planned_window(n - start, n, dim, primed.then_some(max_nsuper));
        loop {
            let end = start + nodes;
            precompute(qt, x, dim, start..end, threads, kp, p, scratch);
            let terms: usize = scratch.chunks.iter().map(|chunk| chunk.terms.len()).sum();
            max_nsuper = max_nsuper.max(observed_supernodes(&scratch.chunks));
            // A node past the margin makes this attempt too big. Drop it
            // and retry a shorter window. One node is kept even then.
            if terms > term_budget() && nodes > 1 {
                let fit = nodes.saturating_mul(term_budget()) / terms.max(1);
                nodes = fit.min(nodes - 1).max(1);
                release_terms(scratch);
                continue;
            }
            debug_assert!(
                terms <= term_budget() || nodes == 1,
                "window stored {terms} floats, budget {}",
                term_budget()
            );
            #[cfg(test)]
            {
                scratch.windows += 1;
                scratch.max_term_f64s = scratch.max_term_f64s.max(terms);
            }
            consume(
                a,
                dim,
                x,
                crk,
                step,
                f,
                fnorm,
                counts_avg,
                nsuper_avg,
                start,
                end,
                &scratch.chunks,
            );
            start = end;
            primed = true;
            break;
        }
    }
}

fn term_budget() -> usize {
    #[cfg(test)]
    if let Some(n) = TERM_BUDGET_OVERRIDE.with(Cell::get) {
        return n.max(1);
    }
    TERM_BUDGET_F64S
}

/// Nodes that fit in [`term_budget`] at `supernodes` stored vectors each.
fn nodes_for(supernodes: usize, dim: usize) -> usize {
    let per = supernodes.max(1).saturating_mul(dim.max(1));
    (term_budget() / per).max(1)
}

/// First window: at most [`PROBE_NODES`], and no more than the budget
/// allows when every node is its own supernode. Later windows: the budget
/// divided by [`SUPERNODE_MARGIN`] times the most supernodes seen so far.
fn planned_window(remaining: usize, n: usize, dim: usize, max_nsuper: Option<usize>) -> usize {
    let limit = match max_nsuper {
        None => nodes_for(n, dim).min(PROBE_NODES),
        Some(seen) => nodes_for(seen.max(1).saturating_mul(SUPERNODE_MARGIN), dim),
    };
    limit.clamp(1, remaining.max(1))
}

fn observed_supernodes(chunks: &[Chunk]) -> usize {
    chunks
        .iter()
        .flat_map(|chunk| chunk.nodes.iter())
        .map(|node| node.nsuper)
        .max()
        .unwrap_or(0)
}

fn release_terms(scratch: &mut Scratch) {
    for chunk in &mut scratch.chunks {
        chunk.nodes.clear();
        chunk.terms.clear();
        chunk.terms.shrink_to_fit();
    }
}

fn precompute(
    qt: &QuadTree,
    x: &[f64],
    dim: usize,
    range: Range<usize>,
    threads: usize,
    kp: f64,
    p: f64,
    scratch: &mut Scratch,
) {
    let len = range.end - range.start;
    if len == 0 {
        scratch.chunks.clear();
        return;
    }
    let threads = threads.max(1).min(len);
    let ranges = par::split_ranges(len, threads);
    if scratch.chunks.len() != ranges.len() {
        scratch.chunks.resize_with(ranges.len(), Chunk::default);
    }
    let base = range.start;
    par::for_each_chunk_mut(&mut scratch.chunks, threads, |slot_range, slots| {
        for (offset, chunk) in slots.iter_mut().enumerate() {
            let rel = ranges[slot_range.start + offset].clone();
            fill_chunk(chunk, qt, x, dim, base + rel.start..base + rel.end, kp, p);
        }
    });
}

fn fill_chunk(
    chunk: &mut Chunk,
    qt: &QuadTree,
    x: &[f64],
    dim: usize,
    range: Range<usize>,
    kp: f64,
    p: f64,
) {
    chunk.nodes.clear();
    chunk.terms.clear();
    chunk.nodes.reserve(range.len());
    for i in range {
        let counts = qt.get_supernodes(BH, &x[dim * i..dim * i + dim], i, &mut chunk.sn);
        let nsuper = chunk.sn.len();
        chunk.nodes.push(NodeTerms { counts, nsuper });
        // Same expression as `repulsive_supernodes`, detached from `f`.
        for j in 0..nsuper {
            let dist = c_max(chunk.sn.distances[j], MINDIST);
            let denom = math::powf(dist, 1.0 - p);
            let w = chunk.sn.wgts[j];
            for k in 0..dim {
                let term = w * kp * (x[i * dim + k] - chunk.sn.center[j * dim + k]) / denom;
                chunk.terms.push(term);
            }
        }
    }
}

fn consume(
    a: &SparseMatrix,
    dim: usize,
    x: &mut [f64],
    crk: f64,
    step: f64,
    f: &mut [f64],
    fnorm: &mut f64,
    counts_avg: &mut f64,
    nsuper_avg: &mut f64,
    start: usize,
    end: usize,
    chunks: &[Chunk],
) {
    let mut i = start;
    for chunk in chunks {
        let mut cursor = 0;
        for node in &chunk.nodes {
            f.iter_mut().for_each(|v| *v = 0.0);
            attractive(a, dim, x, i, crk, f);
            *counts_avg += node.counts;
            *nsuper_avg += node.nsuper as f64;
            for _j in 0..node.nsuper {
                for k in 0..dim {
                    f[k] += chunk.terms[cursor];
                    cursor += 1;
                }
            }
            *fnorm += move_node(dim, x, i, f, step);
            i += 1;
        }
        debug_assert_eq!(cursor, chunk.terms.len());
    }
    debug_assert_eq!(i, end);
}

#[cfg(test)]
mod tests {
    use super::super::{
        BH, QUAD_TREE_NORMAL, QUADTREE_SIZE, SMOOTHING_NONE, SpringElectricalControl, attractive,
        move_node, repulsive_supernodes, spring_electrical_embedding,
    };
    use super::*;
    use crate::sparse::SparseMatrix;
    use crate::sparse::quad_tree::{QuadTree, Supernodes};

    fn assert_bits(got: &[f64], expect: &[f64], label: &str) {
        assert_eq!(got.len(), expect.len(), "{label} len");
        for (i, (got, expect)) in got.iter().zip(expect).enumerate() {
            assert_eq!(
                got.to_bits(),
                expect.to_bits(),
                "{label} [{i}] got={got:?} expect={expect:?}"
            );
        }
    }

    fn path_graph(n: usize) -> SparseMatrix {
        let mut ia = vec![0i32; n + 1];
        let mut ja = Vec::new();
        for i in 0..n {
            if i > 0 {
                ja.push((i - 1) as i32);
            }
            if i + 1 < n {
                ja.push((i + 1) as i32);
            }
            if i + 5 < n {
                ja.push((i + 5) as i32);
            }
            ia[i + 1] = ja.len() as i32;
        }
        SparseMatrix::from_csr(n, n, ia, ja, None)
    }

    fn positions(n: usize, dim: usize, seed: u32) -> Vec<f64> {
        let mut state = seed;
        let mut x = Vec::with_capacity(n * dim);
        for _ in 0..n * dim {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let unit = f64::from(state >> 8) / f64::from(1u32 << 24);
            x.push(unit.mul_add(50.0, -10.0));
        }
        if x.len() > 4 {
            x[0] = -0.0;
            x[3] = f64::from_bits(1);
        }
        x
    }

    fn oracle(
        a: &SparseMatrix,
        dim: usize,
        x: &mut [f64],
        n: usize,
        qt: &QuadTree,
        kp: f64,
        p: f64,
        crk: f64,
        step: f64,
    ) -> (f64, f64, f64) {
        let mut f = vec![0.0; dim];
        let mut sn = Supernodes::default();
        let mut fnorm = 0.0;
        let mut counts_avg = 0.0;
        let mut nsuper_avg = 0.0;
        for i in 0..n {
            f.iter_mut().for_each(|v| *v = 0.0);
            attractive(a, dim, x, i, crk, &mut f);
            let counts = qt.get_supernodes(BH, &x[dim * i..dim * i + dim], i, &mut sn);
            counts_avg += counts;
            nsuper_avg += sn.len() as f64;
            repulsive_supernodes(&sn, dim, x, i, kp, p, &mut f);
            fnorm += move_node(dim, x, i, &mut f, step);
        }
        (fnorm, counts_avg, nsuper_avg)
    }

    fn parallel(
        a: &SparseMatrix,
        dim: usize,
        x: &mut [f64],
        n: usize,
        qt: &QuadTree,
        kp: f64,
        p: f64,
        crk: f64,
        step: f64,
        threads: usize,
    ) -> (f64, f64, f64, usize, usize) {
        let mut f = vec![0.0; dim];
        let mut fnorm = 0.0;
        let mut counts_avg = 0.0;
        let mut nsuper_avg = 0.0;
        let mut scratch = Scratch::default();
        accumulate_with_threads(
            qt,
            a,
            dim,
            x,
            n,
            kp,
            p,
            crk,
            step,
            &mut f,
            &mut fnorm,
            &mut counts_avg,
            &mut nsuper_avg,
            threads,
            &mut scratch,
        );
        (
            fnorm,
            counts_avg,
            nsuper_avg,
            scratch.resident_terms(),
            scratch.windows,
        )
    }

    fn assert_one_tree(
        a: &SparseMatrix,
        dim: usize,
        x0: &[f64],
        qt: &QuadTree,
        kp: f64,
        p: f64,
        threads: usize,
        label: &str,
    ) {
        let n = x0.len() / dim;
        let mut x_seq = x0.to_vec();
        let mut x_par = x0.to_vec();
        let (f_seq, c_seq, s_seq) = oracle(a, dim, &mut x_seq, n, qt, kp, p, 0.2, 0.1);
        let (f_par, c_par, s_par, resident, _) =
            parallel(a, dim, &mut x_par, n, qt, kp, p, 0.2, 0.1, threads);
        assert_eq!(f_seq.to_bits(), f_par.to_bits(), "fnorm {label}");
        assert_eq!(c_seq.to_bits(), c_par.to_bits(), "counts {label}");
        assert_eq!(s_seq.to_bits(), s_par.to_bits(), "nsuper {label}");
        assert_bits(&x_par, &x_seq, label);
        assert!(resident > 0, "{label} stored no terms");
    }

    fn assert_rebuilt(n: usize, dim: usize, p: f64, threads: usize, iters: usize) {
        let a = path_graph(n);
        let kp = 0.35;
        let mut x_seq = positions(n, dim, 0x51 + n as u32 + dim as u32);
        let mut x_par = x_seq.clone();
        for iter in 0..iters {
            let mut tree_seq = QuadTree::default();
            let mut tree_par = QuadTree::default();
            tree_seq.rebuild_from_point_list(dim, n, 10, &x_seq);
            tree_par.rebuild_from_point_list(dim, n, 10, &x_par);
            let (f_seq, c_seq, s_seq) = oracle(&a, dim, &mut x_seq, n, &tree_seq, kp, p, 0.2, 0.1);
            let (f_par, c_par, s_par, _, _) =
                parallel(&a, dim, &mut x_par, n, &tree_par, kp, p, 0.2, 0.1, threads);
            let label = format!("n={n} dim={dim} p={p} threads={threads} iter={iter}");
            assert_eq!(f_seq.to_bits(), f_par.to_bits(), "fnorm {label}");
            assert_eq!(c_seq.to_bits(), c_par.to_bits(), "counts {label}");
            assert_eq!(s_seq.to_bits(), s_par.to_bits(), "nsuper {label}");
            assert_bits(&x_par, &x_seq, &label);
        }
    }

    #[test]
    fn threshold_follows_min_nodes_and_the_thread_count() {
        {
            let _threads = ThreadsOverride::set(4);
            let _min = MinNodesOverride::set(100);
            assert!(!use_parallel(99));
            assert!(use_parallel(100));
        }
        {
            // Two threads lost on the 20000-node graph, so the gate is four.
            let _threads = ThreadsOverride::set(2);
            let _min = MinNodesOverride::set(100);
            assert!(!use_parallel(10_000));
        }
        {
            let _threads = ThreadsOverride::set(3);
            let _min = MinNodesOverride::set(100);
            assert!(!use_parallel(10_000));
        }
        let _threads = ThreadsOverride::set(1);
        let _min = MinNodesOverride::set(100);
        assert!(!use_parallel(1000));
        const {
            assert!(MIN_N >= QUADTREE_SIZE);
        }
        // `sfdp_bytes_6000` in scripts/bench/synth.py stays strictly above the gate.
        const {
            assert!(MIN_N < 6_000);
        }
    }

    /// A budget smaller than the level still matches. The first window is the
    /// worst case (10 nodes here). Later windows grow from the supernodes
    /// actually seen, and none of the kept windows pass the budget.
    #[test]
    fn a_short_budget_splits_the_level_and_stays_inside_it() {
        let n = 80;
        let dim = 2;
        let budget = n * dim * 10;
        let _budget = TermBudgetOverride::set(budget);
        let a = path_graph(n);
        let x = positions(n, dim, 0x51);
        let mut tree = QuadTree::default();
        tree.rebuild_from_point_list(dim, n, 10, &x);
        let mut x_seq = x.clone();
        let mut x_par = x.clone();
        let (f_seq, c_seq, s_seq) = oracle(&a, dim, &mut x_seq, n, &tree, 0.35, -1.0, 0.2, 0.1);
        let (f_par, c_par, s_par, resident, windows) =
            parallel(&a, dim, &mut x_par, n, &tree, 0.35, -1.0, 0.2, 0.1, 4);
        assert_eq!(f_seq.to_bits(), f_par.to_bits(), "fnorm");
        assert_eq!(c_seq.to_bits(), c_par.to_bits(), "counts");
        assert_eq!(s_seq.to_bits(), s_par.to_bits(), "nsuper");
        assert_bits(&x_par, &x_seq, "short budget");
        let worst_windows = n / 10;
        assert!(windows > 1, "windows={windows}");
        assert!(
            windows < worst_windows,
            "windows={windows} stayed at the worst-case size"
        );
        assert!(resident <= budget, "resident={resident} budget={budget}");
        assert!(resident > 0);
    }

    #[test]
    fn precomputed_terms_match_the_sequential_loop() {
        for &n in &[45usize, 46, 63, 64, 65, 80] {
            for &threads in &[1usize, 2, 3, 8] {
                assert_rebuilt(n, 2, -1.0, threads, 2);
            }
        }
        assert_rebuilt(48, 3, -1.0, 3, 1);
        assert_rebuilt(65, 2, -1.8, 3, 2);
        assert_rebuilt(70, 2, 0.5, 8, 1);
    }

    #[test]
    fn nan_query_matches_including_a_refused_spawn() {
        let n = 65;
        let dim = 2;
        let a = path_graph(n);
        let mut x = positions(n, dim, 9);
        let mut tree = QuadTree::default();
        tree.rebuild_from_point_list(dim, n, 10, &x);
        x[0] = f64::NAN;
        x[dim] = f64::from_bits(0x7ff8_0000_0000_0001);
        for &threads in &[1usize, 2, 3, 8] {
            assert_one_tree(
                &a,
                dim,
                &x,
                &tree,
                0.35,
                -1.0,
                threads,
                &format!("nan threads={threads}"),
            );
        }
        for limit in [0usize, 1, 2] {
            let _limit = par::SpawnLimit::at_most(limit);
            assert_one_tree(
                &a,
                dim,
                &x,
                &tree,
                0.35,
                -1.8,
                8,
                &format!("nan spawn limit={limit}"),
            );
        }
    }

    #[test]
    fn embedding_matches_at_one_and_several_threads() {
        for &threads in &[4usize, 8] {
            assert_embedding(
                48,
                threads,
                45,
                -1.0,
                false,
                2,
                &format!("hook threads={threads}"),
            );
        }
        assert_embedding(48, 2, 45, -1.0, false, 2, "two threads stay sequential");
        assert_embedding(48, 4, 10_000, -1.0, false, 2, "below threshold");
        assert_embedding(50, 4, 45, -1.8, false, 2, "exponent -1.8");
        assert_embedding(48, 4, 45, -1.0, true, 2, "random start");
        {
            let _limit = par::SpawnLimit::at_most(0);
            assert_embedding(64, 8, 45, -1.0, false, 2, "embedding spawn limit 0");
        }
        {
            let _limit = par::SpawnLimit::at_most(1);
            assert_embedding(64, 4, 45, -1.0, false, 1, "embedding spawn limit 1");
        }
    }

    fn assert_embedding(
        n: usize,
        threads: usize,
        min_nodes: usize,
        p: f64,
        random_start: bool,
        maxiter: i32,
        label: &str,
    ) {
        let x0 = positions(n, 2, 0x5f);
        let sequential = embed(n, &x0, 1, min_nodes, p, random_start, maxiter);
        let parallel = embed(n, &x0, threads, min_nodes, p, random_start, maxiter);
        assert_bits(&parallel, &sequential, label);
    }

    fn embed(
        n: usize,
        x0: &[f64],
        threads: usize,
        min_nodes: usize,
        p: f64,
        random_start: bool,
        maxiter: i32,
    ) -> Vec<f64> {
        let _threads = ThreadsOverride::set(threads);
        let _min = MinNodesOverride::set(min_nodes);
        let mut ctrl = SpringElectricalControl {
            p,
            k: 1.0,
            multilevels: 0,
            maxiter,
            random_start,
            adaptive_cooling: true,
            beautify_leaves: false,
            smoothing: SMOOTHING_NONE,
            tscheme: QUAD_TREE_NORMAL,
            random_seed: 123,
            ..SpringElectricalControl::default()
        };
        let a = path_graph(n);
        let mut x = x0.to_vec();
        let mut flag = 0;
        crate::cutil::in_fresh_run(|| {
            spring_electrical_embedding(2, &a, &mut ctrl, &mut x, &mut flag);
        });
        assert_eq!(flag, 0, "threads={threads}");
        x
    }
}
