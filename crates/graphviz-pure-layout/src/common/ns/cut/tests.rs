//! Both paths of `rank2` on the same graphs: C's functions alone, and the cut-set pivots
//! (which, under `cfg(test)`, also assert at every pivot that they agree with C's).

use crate::common::ns::{
    Ctx, NsData, NsEdge, NsNode, feasible_tree, init_graph, init_rank, rank2_with,
};

/// xorshift64*: a fixed sequence, so a failure names its case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random DAG on `n` nodes: every node but the first hangs from an earlier one (so the
/// graph is connected), plus `extra` random forward edges, some of them parallel.
fn random_dag(rng: &mut Rng, n: usize, extra: usize, minlen: i32) -> NsData {
    let mut g = NsData {
        nodes: vec![NsNode::default(); n],
        ..NsData::default()
    };
    let add = |g: &mut NsData, t: usize, h: usize, rng: &mut Rng| {
        let e = g.edges.len();
        g.edges.push(NsEdge {
            tail: t,
            head: h,
            minlen: rng.below(minlen as u64 + 1) as i32,
            weight: rng.below(4) as i32,
            ..NsEdge::default()
        });
        g.nodes[t].out.push(e);
        g.nodes[h].in_.push(e);
    };
    for h in 1..n {
        let t = rng.below(h as u64) as usize;
        add(&mut g, t, h, rng);
    }
    for _ in 0..extra {
        let a = rng.below(n as u64) as usize;
        let b = rng.below(n as u64) as usize;
        if a != b {
            add(&mut g, a.min(b), a.max(b), rng);
        }
    }
    // GD_nlist order, and with it the root, is not the node order
    g.nlist = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        g.nlist.swap(i, j);
    }
    g
}

/// Everything `rank2` leaves behind that anything reads, apart from `lim`/`low` (see
/// `docs/perf/ns-cut-sets.md`, "What is not the same").
#[derive(Debug, PartialEq)]
struct Outcome {
    result: Result<i32, String>,
    rank: Vec<i32>,
    par: Vec<Option<usize>>,
    mark: Vec<bool>,
    tree_in: Vec<Vec<usize>>,
    tree_out: Vec<Vec<usize>>,
    cutvalue: Vec<i32>,
    tree_index: Vec<i32>,
}

fn run(g: &NsData, balance: i32, maxiter: i32, search: i32, cut_sets: bool) -> Outcome {
    let mut g = g.clone();
    let result = rank2_with(&mut g, balance, maxiter, search, cut_sets);
    Outcome {
        result,
        rank: g.nodes.iter().map(|n| n.rank).collect(),
        par: g.nodes.iter().map(|n| n.par).collect(),
        mark: g.nodes.iter().map(|n| n.mark).collect(),
        tree_in: g.nodes.iter().map(|n| n.tree_in.clone()).collect(),
        tree_out: g.nodes.iter().map(|n| n.tree_out.clone()).collect(),
        cutvalue: g.edges.iter().map(|e| e.cutvalue).collect(),
        tree_index: g.edges.iter().map(|e| e.tree_index).collect(),
    }
}

fn same_both_ways(g: &NsData, balance: i32, maxiter: i32, search: i32, case: &str) {
    let c = run(g, balance, maxiter, search, false);
    let cut = run(g, balance, maxiter, search, true);
    assert_eq!(
        cut, c,
        "{case}: balance {balance} maxiter {maxiter} search {search}"
    );
}

#[test]
fn random_dags_rank_the_same_on_both_paths() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for counter in [
        &super::check::WHOLE_TREE,
        &super::check::TIES,
        &super::check::RELOCATIONS,
        &super::check::COMPACTIONS,
    ] {
        counter.with(|n| n.set(0));
    }
    for case in 0..3000 {
        let n = 2 + rng.below(70) as usize;
        let extra = rng.below(2 * n as u64 + 1) as usize;
        let g = random_dag(&mut rng, n, extra, 3);
        let balance = rng.below(3) as i32;
        let search = [30, 1, 2, 5, -1][rng.below(5) as usize];
        same_both_ways(&g, balance, i32::MAX, search, &format!("case {case}"));
        // stopped part way: the ranks are written where the pivots stop
        let maxiter = 1 + rng.below(8) as i32;
        same_both_ways(
            &g,
            balance,
            maxiter,
            search,
            &format!("case {case}, cut short"),
        );
    }
    // the cases reach C's quirks, not only the plain pivots
    assert!(
        super::check::WHOLE_TREE.with(|n| n.get()) > 0,
        "no rerank from the root"
    );
    assert!(
        super::check::TIES.with(|n| n.get()) > 0,
        "no tied entering edges"
    );
}

#[test]
fn larger_layered_dags_rank_the_same_on_both_paths() {
    let mut rng = Rng(0x5851_f42d_4c95_7f2d);
    for case in 0..6 {
        let n = 300 + rng.below(300) as usize;
        let g = random_dag(&mut rng, n, n / 2, 2);
        same_both_ways(&g, 2, i32::MAX, 30, &format!("layered case {case}"));
        same_both_ways(&g, 1, i32::MAX, 30, &format!("layered case {case}"));
    }
}

#[test]
fn wrapped_slacks_hand_the_run_back_to_c() {
    // minimum lengths near i32::MAX make some ranks wrap, so some candidates' slacks are
    // negative and C's scan stops being an argmin: the cut sets must fall back to C
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut fell_back = 0;
    for case in 0..300 {
        let n = 3 + rng.below(25) as usize;
        let extra = rng.below(2 * n as u64 + 1) as usize;
        let mut g = random_dag(&mut rng, n, extra, 2);
        for e in &mut g.edges {
            if rng.below(3) == 0 {
                e.minlen = i32::MAX / 2 - rng.below(4) as i32;
            }
        }
        let c = run(&g, 0, i32::MAX, 30, false);
        let cut = run(&g, 0, i32::MAX, 30, true);
        assert_eq!(cut, c, "wrapped case {case}");
        if super::check::FALLBACKS.with(|f| f.replace(0)) > 0 {
            fell_back += 1;
        }
    }
    assert!(fell_back > 0, "no case reached the fallback");
}

fn ctx() -> Ctx {
    Ctx {
        tree_edge: Vec::new(),
        s_i: 0,
        search_size: 30,
        subtrees: Vec::new(),
        enter_stack: Vec::new(),
        rerank_stack: Vec::new(),
        range_stack: Vec::new(),
    }
}

#[test]
fn the_selector_prefers_c_on_small_graphs() {
    let mut rng = Rng(7);
    let mut g = random_dag(&mut rng, 50, 60, 2);
    if !init_graph(&mut g) {
        init_rank(&mut g);
    }
    assert_eq!(feasible_tree(&mut g, &mut ctx()), Ok(0));
    assert!(!super::worth_it(&g));
}

#[test]
fn the_selector_prefers_cut_sets_on_dot_dags() {
    // dot_dag_5000 of scripts/bench/synth.py, as dot ranks it: each node points to one or
    // two of the next 40, so tree cycles are short (about 27 edges) and the cut sets small
    // (about 9 entries per node and non-tree edge)
    struct Lcg(u32);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as usize % n
        }
    }
    let n = 5000;
    let mut r = Lcg(n as u32);
    let mut g = NsData {
        nodes: vec![NsNode::default(); n],
        nlist: (0..n).collect(),
        ..NsData::default()
    };
    let add = |g: &mut NsData, t: usize, h: usize| {
        let e = g.edges.len();
        g.edges.push(NsEdge {
            tail: t,
            head: h,
            minlen: 1,
            weight: 1,
            ..NsEdge::default()
        });
        g.nodes[t].out.push(e);
        g.nodes[h].in_.push(e);
    };
    for i in 0..n - 1 {
        let h = i + 1 + r.below(40.min(n - 1 - i));
        add(&mut g, i, h);
        if r.below(2) == 1 {
            let h = i + 1 + r.below(40.min(n - 1 - i));
            add(&mut g, i, h);
        }
    }
    if !init_graph(&mut g) {
        init_rank(&mut g);
    }
    assert_eq!(feasible_tree(&mut g, &mut ctx()), Ok(0));
    assert!(super::worth_it(&g));
}

#[test]
fn the_selector_keeps_c_when_the_cut_sets_would_be_large() {
    // a 10,000-node chain with a chord over 100 links every 4 nodes: the tight tree is the
    // chain, so every chord's tree cycle has 100 edges. The time estimate passes
    // (100 × (100 × 2,475 / 10,000) / 10,000 ≈ 0.25) but the cut sets would hold about
    // 20 entries per node and non-tree edge, twice the limit
    let n = 10_000;
    let mut g = NsData {
        nodes: vec![NsNode::default(); n],
        nlist: (0..n).collect(),
        ..NsData::default()
    };
    let add = |g: &mut NsData, t: usize, h: usize, minlen: i32| {
        let e = g.edges.len();
        g.edges.push(NsEdge {
            tail: t,
            head: h,
            minlen,
            weight: 1,
            ..NsEdge::default()
        });
        g.nodes[t].out.push(e);
        g.nodes[h].in_.push(e);
    };
    for i in 1..n {
        add(&mut g, i - 1, i, 1);
    }
    for i in (0..n - 100).step_by(4) {
        add(&mut g, i, i + 100, 1);
    }
    if !init_graph(&mut g) {
        init_rank(&mut g);
    }
    assert_eq!(feasible_tree(&mut g, &mut ctx()), Ok(0));
    assert!(!super::worth_it(&g));
}
