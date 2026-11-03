//! Both paths of `rank2` on the same graphs: C's functions alone, and the cut-set pivots
//! (which, under `cfg(test)`, also assert at every pivot that they agree with C's).

use super::Kind;
use crate::common::ns::{
    Ctx, NsData, NsEdge, NsGraph, NsNode, feasible_tree, init_graph, init_rank, rank2_with,
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

/// Runs `f` with this kind of cut sets.
fn with_kind<R>(kind: Kind, f: impl FnOnce() -> R) -> R {
    super::check::KIND.with(|k| k.set(Some(kind)));
    let r = f();
    super::check::KIND.with(|k| k.set(None));
    r
}

const KINDS: [Kind; 2] = [Kind::Pool, Kind::Computed];

fn same_both_ways(g: &NsData, balance: i32, maxiter: i32, search: i32, case: &str) {
    let c = run(g, balance, maxiter, search, false);
    for kind in KINDS {
        let cut = with_kind(kind, || run(g, balance, maxiter, search, true));
        assert_eq!(
            cut, c,
            "{case}: {kind:?}, balance {balance} maxiter {maxiter} search {search}"
        );
    }
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
    for kind in KINDS {
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
            let cut = with_kind(kind, || run(&g, 0, i32::MAX, 30, true));
            assert_eq!(cut, c, "{kind:?}, wrapped case {case}");
            if super::check::FALLBACKS.with(|f| f.replace(0)) > 0 {
                fell_back += 1;
            }
        }
        assert!(fell_back > 0, "{kind:?}: no case reached the fallback");
    }
}

fn ctx() -> Ctx {
    Ctx {
        tree_edge: Vec::new(),
        s_i: 0,
        search_size: 30,
        neg: Default::default(),
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
    // dot_dag_20000 of scripts/bench/synth.py, as dot ranks it: each node points to one or
    // two of the next 40, so tree cycles are short (about 27 edges) and the cut sets small
    // (about 9 entries per node and non-tree edge)
    struct Lcg(u32);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as usize % n
        }
    }
    let n = 20_000;
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
    assert_eq!(super::choose(&g), Some(Kind::Pool));
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
    assert_eq!(super::choose(&g), Some(Kind::Computed));
}

/// The cut-set path with a broken cut set or a pool budget, as the tests arm them.
fn run_armed(
    g: &NsData,
    balance: i32,
    corruption: Option<(super::check::Corrupt, u32)>,
    budget_slack: Option<usize>,
) -> Outcome {
    super::check::CORRUPTION.with(|c| c.set(corruption));
    super::check::BUDGET_SLACK.with(|b| b.set(budget_slack));
    let outcome = run(g, balance, i32::MAX, 30, true);
    super::check::CORRUPTION.with(|c| c.set(None));
    super::check::BUDGET_SLACK.with(|b| b.set(None));
    outcome
}

fn fallbacks() -> u32 {
    super::check::FALLBACKS.with(|f| f.get())
}

#[test]
fn broken_stored_cut_sets_hand_the_pivot_to_c() {
    with_kind(Kind::Pool, broken_cut_sets_hand_the_pivot_to_c);
}

#[test]
fn broken_computed_cut_sets_hand_the_pivot_to_c() {
    with_kind(Kind::Computed, broken_cut_sets_hand_the_pivot_to_c);
}

fn broken_cut_sets_hand_the_pivot_to_c() {
    // f taken out of e's cut set: after enter_edge picks f (the pivot's check phase finds
    // it missing), just before update (which has to check the pivot again), or after an
    // LR_balance query. Each time C's search picks the edge again and C's functions finish
    // the run with C's results.
    use super::check::Corrupt;
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    let kinds = [Corrupt::Enter, Corrupt::Update, Corrupt::Balance];
    let mut fell = [0u32; 3];
    for case in 0..300 {
        let n = 8 + rng.below(60) as usize;
        let extra = rng.below(2 * n as u64 + 1) as usize;
        let g = random_dag(&mut rng, n, extra, 3);
        let c = run(&g, 2, i32::MAX, 30, false);
        for (i, kind) in kinds.into_iter().enumerate() {
            let at = rng.below(6) as u32;
            let before = fallbacks();
            let cut = run_armed(&g, 2, Some((kind, at)), None);
            assert_eq!(cut, c, "case {case}, corruption {i} at {at}");
            if fallbacks() > before {
                fell[i] += 1;
            }
        }
    }
    assert!(
        fell.iter().all(|&n| n > 0),
        "every corruption reached its fallback: {fell:?}"
    );
    assert!(super::check::BALANCE_FALLBACKS.with(|n| n.get()) > 0);
    assert!(super::check::RESELECTIONS.with(|n| n.get()) > 0);
}

/// As shipped, with nothing to keep `lim`/`low` and the ranks current: a fallback has to write
/// the ranks and renumber the tree, or C's functions decide on stale numbers. Check mode's own
/// `dfs_range` would hide that.
#[test]
fn a_fallback_renumbers_the_tree_it_hands_to_c() {
    use super::check::{Corrupt, SHIPPED};
    SHIPPED.with(|s| s.set(true));
    for kind in KINDS {
        let mut rng = Rng(0x0a0b_0c0d_0e0f_1011);
        let before = fallbacks();
        for case in 0..400 {
            let n = 8 + rng.below(80) as usize;
            let extra = rng.below(2 * n as u64 + 1) as usize;
            let g = random_dag(&mut rng, n, extra, 3);
            let c = run(&g, 2, i32::MAX, 30, false);
            // and with nothing broken
            let clean = with_kind(kind, || run(&g, 2, i32::MAX, 30, true));
            assert_eq!(clean, c, "{kind:?}, case {case}");
            for corruption in [Corrupt::Enter, Corrupt::Update, Corrupt::Balance] {
                // late, so that the numbers from before the pivots have gone stale
                let at = 2 + rng.below(12) as u32;
                let cut = with_kind(kind, || run_armed(&g, 2, Some((corruption, at)), None));
                assert_eq!(cut, c, "{kind:?}, case {case}, after {at}");
            }
        }
        assert!(fallbacks() > before, "{kind:?}: no run fell back");
    }
    // and without a corruption: a wrapped slack is the fallback's real cause
    for kind in KINDS {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
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
            let cut = with_kind(kind, || run(&g, 0, i32::MAX, 30, true));
            assert_eq!(cut, c, "{kind:?}, wrapped case {case}");
        }
    }
    SHIPPED.with(|s| s.set(false));
}

/// Tied candidates come out in C's scan order, however far apart they are. In each gadget the
/// tree edge `u -> v` has a negative cut value, because the chains under `v` all end in an edge to
/// `t` that crosses back, and each of those has slack 1. The climbs between them are as long as
/// the chains, which makes this the shape that used to be quadratic.
#[test]
fn ties_on_long_chains_stay_c_s_choice() {
    let mut rng = Rng(0x3333_1111_2222_4444);
    for counter in [
        &super::check::TIES,
        &super::check::TIE_SCANS,
        &super::check::FALLBACKS,
    ] {
        counter.with(|n| n.set(0));
    }
    let mut spent_one = 0;
    for case in 0..40 {
        let mut g = NsData::default();
        let add_node = |g: &mut NsData| {
            g.nodes.push(NsNode::default());
            g.nlist.push(g.nodes.len() - 1);
            g.nodes.len() - 1
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
        let mut most = 0;
        let mut join = None;
        for _ in 0..1 + rng.below(3) {
            let (u, v, t) = (add_node(&mut g), add_node(&mut g), add_node(&mut g));
            if let Some(j) = join {
                add(&mut g, j, u, 1);
            }
            join = Some(t);
            add(&mut g, u, v, 1);
            let lengths: Vec<i32> = (0..3 + rng.below(8))
                .map(|_| 5 + rng.below(30) as i32)
                .collect();
            let longest = lengths.iter().copied().max().unwrap_or(0);
            most = most.max(longest as u32);
            let span = longest + 3;
            add(&mut g, u, t, span);
            for length in lengths {
                let mut last = v;
                for _ in 0..length {
                    let next = add_node(&mut g);
                    add(&mut g, last, next, 1);
                    // a branch that the scan has to go through, or to leave for later
                    if rng.below(4) == 0 {
                        let leaf = add_node(&mut g);
                        add(&mut g, next, leaf, 1);
                    }
                    last = next;
                }
                // slack 1 whatever the chain's length
                add(&mut g, last, t, span - length - 2);
            }
        }
        for climbs in [None, Some(0), Some(1), Some(most), Some(4 * most)] {
            super::check::CLIMBS.with(|c| c.set(climbs));
            let scans = super::check::TIE_SCANS.with(|n| n.get());
            for balance in [1, 2] {
                same_both_ways(
                    &g,
                    balance,
                    i32::MAX,
                    30,
                    &format!("chains {case} {climbs:?}"),
                );
            }
            // every chain is longer than one step, so the climbs run out
            if climbs == Some(1) {
                spent_one += super::check::TIE_SCANS.with(|n| n.get()) - scans;
            }
        }
    }
    super::check::CLIMBS.with(|c| c.set(None));
    assert!(super::check::TIES.with(|n| n.get()) > 0, "no tie");
    assert!(spent_one > 0, "one step of climbing was enough");
    assert_eq!(
        super::check::FALLBACKS.with(|f| f.get()),
        0,
        "the scan did not find its edge"
    );
}

/// The answer does not depend on how far the climbs may go: with no climbs at all, every tie
/// takes C's scan order, and with a few they run out part way through a query.
#[test]
fn ties_agree_at_any_budget_for_the_climbs() {
    let mut rng = Rng(0x1111_3333_4444_2222);
    for counter in [
        &super::check::TIES,
        &super::check::TIE_SCANS,
        &super::check::FALLBACKS,
    ] {
        counter.with(|n| n.set(0));
    }
    for case in 0..1500 {
        let n = 2 + rng.below(70) as usize;
        let extra = rng.below(2 * n as u64 + 1) as usize;
        let g = random_dag(&mut rng, n, extra, 3);
        let climbs = [0, 1, 2, 3, 5, 8, 13, 21][rng.below(8) as usize];
        super::check::CLIMBS.with(|c| c.set(Some(climbs)));
        let balance = rng.below(3) as i32;
        same_both_ways(
            &g,
            balance,
            i32::MAX,
            30,
            &format!("case {case}, {climbs} climbs"),
        );
    }
    super::check::CLIMBS.with(|c| c.set(None));
    assert!(super::check::TIES.with(|n| n.get()) > 0, "no tie");
    assert!(super::check::TIE_SCANS.with(|n| n.get()) > 0, "no scan");
    assert_eq!(
        super::check::FALLBACKS.with(|f| f.get()),
        0,
        "the scan did not find its edge"
    );
}

/// `LR_balance` on computed cuts goes to C's functions when its walks pass their allowance, at
/// whatever point, as shipped or beside C's functions, and only then. The output cannot show when:
/// that is what the counter is for.
#[test]
fn lr_balance_hands_over_when_its_walks_pass_the_allowance() {
    use super::check::{BALANCE_HANDED_OVER, BALANCE_SPARE, SHIPPED};
    let mut rng = Rng(0x5555_1234_0000_7777);
    for shipped in [false, true] {
        SHIPPED.with(|s| s.set(shipped));
        let mut handed = [0u32; 4];
        for case in 0..200 {
            let n = 4 + rng.below(60) as usize;
            let g = random_dag(&mut rng, n, 2 * n, 3);
            let c = run(&g, 2, i32::MAX, 30, false);
            // none, a little, a little more, as much as it takes
            for (i, spare) in [0, 1, 1 + rng.below(40) as usize, usize::MAX]
                .into_iter()
                .enumerate()
            {
                BALANCE_SPARE.with(|b| b.set(Some(spare)));
                let before = BALANCE_HANDED_OVER.with(|n| n.get());
                let cut = with_kind(Kind::Computed, || run(&g, 2, i32::MAX, 30, true));
                BALANCE_SPARE.with(|b| b.set(None));
                assert_eq!(cut, c, "shipped {shipped}, case {case}, spare {spare}");
                handed[i] += BALANCE_HANDED_OVER.with(|n| n.get()) - before;
                // stored cut sets have no walks to count
                BALANCE_SPARE.with(|b| b.set(Some(spare)));
                let before = BALANCE_HANDED_OVER.with(|n| n.get());
                let cut = with_kind(Kind::Pool, || run(&g, 2, i32::MAX, 30, true));
                BALANCE_SPARE.with(|b| b.set(None));
                assert_eq!(cut, c, "shipped {shipped}, stored, case {case}");
                assert_eq!(BALANCE_HANDED_OVER.with(|n| n.get()), before);
            }
        }
        assert!(handed[0] > 0 && handed[1] > 0, "{handed:?}");
        assert_eq!(handed[3], 0, "an allowance that cannot run out ran out");
    }
    SHIPPED.with(|s| s.set(false));
}

/// The smallest run that leaves C's functions.
#[test]
fn the_selector_starts_at_64_nodes() {
    for (n, picked) in [(63, false), (64, true)] {
        let mut rng = Rng(7);
        let mut g = random_dag(&mut rng, n, n, 2);
        if !init_graph(&mut g) {
            init_rank(&mut g);
        }
        assert_eq!(feasible_tree(&mut g, &mut ctx()), Ok(0));
        assert_eq!(super::choose(&g).is_some(), picked, "{n} nodes");
    }
}

#[test]
fn a_full_pool_hands_the_run_to_c() {
    // a budget of the build's own size (no pivot fits) or a little more (some pivots fit):
    // the run falls back before a pivot that could exceed it, with C's results
    let mut rng = Rng(0x0fed_cba9_8765_4321);
    let before = fallbacks();
    for case in 0..200 {
        let n = 8 + rng.below(60) as usize;
        let extra = rng.below(2 * n as u64 + 1) as usize;
        let g = random_dag(&mut rng, n, extra, 3);
        let balance = rng.below(3) as i32;
        let c = run(&g, balance, i32::MAX, 30, false);
        for slack in [0, 64, 256] {
            let cut = with_kind(Kind::Pool, || run_armed(&g, balance, None, Some(slack)));
            assert_eq!(cut, c, "case {case}, budget slack {slack}");
        }
    }
    assert!(fallbacks() > before, "no run outgrew its budget");
}

#[test]
fn a_computed_cut_set_answers_only_for_its_own_edge() {
    let mut rng = Rng(0x1111_2222_3333_4444);
    let mut g = random_dag(&mut rng, 40, 30, 2);
    if !init_graph(&mut g) {
        init_rank(&mut g);
    }
    let mut c = ctx();
    assert_eq!(feasible_tree(&mut g, &mut c), Ok(0));
    let mut store = super::CutStore {
        kind: Kind::Computed,
        ..super::CutStore::default()
    };
    assert!(store.build(&g, &c));
    let (a, b) = (c.tree_edge[0], c.tree_edge[1]);
    let (sa, sb) = (
        super::tree_slot(&g, a).unwrap(),
        super::tree_slot(&g, b).unwrap(),
    );
    let va = if g.par(g.tail(a)) == Some(a) {
        g.tail(a)
    } else {
        g.head(a)
    };
    store.compute_cut(sa, va, g.par(g.tail(a)) == Some(a));
    // an edge in a's cut set is not in another slot's answer, even one that holds it
    let k = store.cut_entry(sa, 0) & super::ID;
    let member = store.edge(k).edge as usize;
    assert_eq!(store.member(sa, member), Some(k));
    assert_eq!(store.member(sb, member), None);
}

#[test]
fn a_tree_that_disagrees_with_lim_is_refused() {
    let mut rng = Rng(0x5555_6666_7777_8888);
    let mut g = random_dag(&mut rng, 40, 30, 2);
    if !init_graph(&mut g) {
        init_rank(&mut g);
    }
    let mut c = ctx();
    assert_eq!(feasible_tree(&mut g, &mut c), Ok(0));
    let mut store = super::CutStore {
        kind: Kind::Computed,
        ..super::CutStore::default()
    };
    assert!(store.build(&g, &c));
    let root = g.nlist[0];
    g.nodes[root].low += 1;
    assert!(!store.build(&g, &c));
}

#[test]
#[should_panic(expected = "cut set of tree edge")]
fn the_cross_check_sees_a_computed_cut_that_misses_nodes() {
    use super::check::Corrupt;
    let mut rng = Rng(0x1234_5678_9abc_def1);
    with_kind(Kind::Computed, || {
        for _ in 0..200 {
            let n = 8 + rng.below(60) as usize;
            let extra = rng.below(2 * n as u64 + 1) as usize;
            let g = random_dag(&mut rng, n, extra, 3);
            let _ = run_armed(&g, 2, Some((Corrupt::Links, 0)), None);
        }
    });
}

/// The chain with chords of `the_selector_keeps_c_when_the_cut_sets_would_be_large`, smaller.
fn chords(n: usize, span: usize, every: usize) -> NsData {
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
    for i in 1..n {
        add(&mut g, i - 1, i);
    }
    for i in (0..n - span).step_by(every) {
        add(&mut g, i, i + span);
    }
    g
}

#[test]
fn the_selector_keeps_small_runs_on_c_and_computes_the_rest() {
    let mut small = chords(super::COMPUTED_MIN_NODES - 1, 4, 3);
    let mut mid = chords(super::COMPUTED_MIN_NODES, 4, 3);
    for g in [&mut small, &mut mid] {
        if !init_graph(g) {
            init_rank(g);
        }
        assert_eq!(feasible_tree(g, &mut ctx()), Ok(0));
    }
    assert_eq!(super::choose(&small), None);
    assert_eq!(super::choose(&mid), Some(Kind::Computed));
}
