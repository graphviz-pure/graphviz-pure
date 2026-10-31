//! The shortcuts against the functions that mirror C, on random ranks. Under `cfg(test)` every
//! shortcut also runs its reference and asserts, so these tests only need to reach them.

use super::*;
use graphviz_pure_core::{Graph, GraphKind};

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

/// `reorder`'s loops from mincross.c on plain arrays: the node at each position, and whether
/// any exchange happened.
fn bubble(mval: &[f64], reverse: bool, hasfixed: bool) -> (Vec<u32>, bool) {
    let n = mval.len() as i32;
    let mut at: Vec<u32> = (0..n as u32).collect();
    let value = |at: &[u32], i: i32| mval[at[i as usize] as usize];
    let mut changed = false;
    let mut ep = n;
    let mut nelt = n - 1;
    while nelt >= 0 {
        let mut lp = 0;
        while lp < ep {
            while lp < ep && value(&at, lp) < 0.0 {
                lp += 1;
            }
            if lp >= ep {
                break;
            }
            let mut rp = lp + 1;
            while rp < ep && value(&at, rp) < 0.0 {
                rp += 1;
            }
            if rp >= ep {
                break;
            }
            let (p1, p2) = (value(&at, lp), value(&at, rp));
            if p1 > p2 || (p1 >= p2 && reverse) {
                at.swap(lp as usize, rp as usize);
                changed = true;
            }
            lp = rp;
        }
        if !hasfixed && !reverse {
            ep -= 1;
        }
        nelt -= 1;
    }
    (at, changed)
}

fn settled(mval: &[f64], reverse: bool) -> (Vec<u32>, bool) {
    let mut items = Vec::new();
    let mut places = Vec::new();
    for (i, &m) in mval.iter().enumerate() {
        if m >= 0.0 {
            items.push(Medianed {
                mval: m,
                seq: items.len() as u32,
                slot: i as u32,
            });
            places.push(i as u32);
        }
    }
    let mut work = places.clone();
    let changed = settle(&mut items, mval.len() as u32, reverse, &mut work);
    assert_eq!(work, places, "settle keeps the target positions");
    let mut at: Vec<u32> = (0..mval.len() as u32).collect();
    if changed {
        for (item, &place) in items.iter().zip(&places) {
            at[place as usize] = item.slot;
        }
    }
    (at, changed)
}

#[test]
fn settled_order_equals_the_bubble_passes() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..200_000u32 {
        let n = 1 + rng.below(if case % 16 == 0 { 120 } else { 24 }) as usize;
        // A small value range makes ties common; -0.0 ties with 0.0 and is not negative.
        let distinct = 1 + rng.below(if case % 4 == 0 { 2 } else { 12 });
        let negatives = rng.below(4);
        let mval: Vec<f64> = (0..n)
            .map(|_| {
                if rng.below(8) < negatives {
                    -1.0
                } else {
                    match rng.below(distinct) {
                        0 if rng.below(2) == 0 => -0.0,
                        v => v as f64,
                    }
                }
            })
            .collect();
        for reverse in [false, true] {
            for hasfixed in [false, true] {
                assert_eq!(
                    settled(&mval, reverse),
                    bubble(&mval, reverse, hasfixed),
                    "case {case}: {mval:?}, reverse {reverse}, hasfixed {hasfixed}"
                );
            }
        }
    }
}

#[test]
fn infinite_medians_sort_like_any_other() {
    let mval = [
        f64::INFINITY,
        2.0,
        f64::NEG_INFINITY,
        2.0,
        f64::INFINITY,
        0.0,
    ];
    for reverse in [false, true] {
        assert_eq!(settled(&mval, reverse), bubble(&mval, reverse, false));
    }
}

/// How the random ranks are drawn.
#[derive(Clone, Copy, Default)]
struct Shape {
    /// Penalties whose products wrap `out_cross`'s i32 sum.
    wrapping: bool,
    /// Clusters and a flat-edge matrix, so that `left2right` answers.
    constrained: bool,
    /// Nodes outside graph 1's windows whose stale orders tie with nodes inside them.
    stale_ties: bool,
}

/// Random ranks, with random edges between adjacent ranks. Graph 1 is a window of graph 0, as a
/// cluster's ranks are of the root's.
fn random_ranks<'a>(rng: &mut Rng, input: &'a Graph, shape: Shape) -> Dot<'a> {
    let mut d = Dot::empty(input);
    let ranks = 2 + rng.below(6) as i32;
    let mut levels: Vec<Vec<N>> = Vec::new();
    for r in 0..ranks {
        let width = 1 + rng.below(14) as usize;
        let mut level = Vec::new();
        for order in 0..width {
            let v = d.nodes.len();
            d.nodes.push(NodeInfo::default());
            d.nodes[v].rank = r;
            d.nodes[v].order = order as i32;
            d.nodes[v].low = order as i32;
            if shape.constrained && rng.below(4) == 0 {
                d.nodes[v].clust = Some(1 + rng.below(2) as usize);
            }
            level.push(v);
        }
        d.bufs.push(level.iter().copied().map(Some).collect());
        levels.push(level);
    }
    d.graphs.push(GraphInfo {
        minrank: 0,
        maxrank: ranks - 1,
        rank: vec![Rank::default(); ranks as usize + 3],
        ..GraphInfo::default()
    });
    for r in 0..ranks {
        *d.rk_mut(0, r) = Rank {
            vbuf: r as usize,
            n: levels[r as usize].len() as i32,
            ..Rank::default()
        };
    }
    // Graph 1: the tail of each rank.
    d.graphs.push(d.graphs[0].clone());
    for r in 0..ranks {
        let n = levels[r as usize].len();
        let skip = rng.below(n as u64) as usize;
        if shape.stale_ties {
            for &v in &levels[r as usize][..skip] {
                if rng.below(2) == 0 {
                    d.nodes[v].order = (skip + rng.below((n - skip) as u64) as usize) as i32;
                }
            }
        }
        let row = d.rk_mut(1, r);
        row.voff = skip;
        row.n = (n - skip) as i32;
    }
    if shape.constrained {
        let r = rng.below(ranks as u64) as i32;
        let n = levels[r as usize].len();
        let mut matrix = AdjMatrix::new(n, n);
        for _ in 0..n {
            let (a, b) = (rng.below(n as u64) as usize, rng.below(n as u64) as usize);
            if a < b {
                matrix.set(a, b);
            }
        }
        d.rk_mut(0, r).flat = Some(matrix.clone());
        d.rk_mut(1, r).flat = Some(matrix);
    }
    let density = 1 + rng.below(3);
    for r in 0..(ranks - 1) as usize {
        for &tail in &levels[r] {
            for _ in 0..rng.below(density + 1) {
                let head = levels[r + 1][rng.below(levels[r + 1].len() as u64) as usize];
                let e = d.edges.len();
                let mut edge = EdgeInfo {
                    tail,
                    head,
                    xpenalty: if shape.wrapping && rng.below(4) == 0 {
                        (1 << 16) + rng.below(1 << 16) as i32
                    } else {
                        rng.below(5) as i32 - 1
                    },
                    ..EdgeInfo::default()
                };
                edge.tail_port.p.x = rng.below(3) as f64;
                edge.head_port.p.x = rng.below(3) as f64;
                d.edges.push(edge);
                d.nodes[tail].out.push(e);
                d.nodes[head].in_.push(e);
            }
        }
    }
    d
}

/// A short mincross on graph g: every `reorder_settled` and `transpose_scans` asserts inside.
fn run_passes(d: &mut Dot<'_>, g: G, rng: &mut Rng) -> Result<(), &'static str> {
    d.prepare_mincross(g)?;
    for pass in 0..6 {
        d.mincross_step(g, pass + rng.below(4) as i32)?;
        d.ncross();
    }
    d.transpose(g, false)
}

fn run_case(seed: u64, shape: Shape) -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    for g in [0, 1] {
        let mut d = random_ranks(&mut rng, &input, shape);
        run_passes(&mut d, g, &mut rng)?;
    }
    Ok(())
}

#[test]
fn scans_equal_c_on_random_ranks() -> Result<(), &'static str> {
    for seed in 1..=1_500 {
        run_case(seed, Shape::default())?;
    }
    Ok(())
}

/// Counts that wrap are not a potential that falls, so C's loop may never end on them. Check
/// mode stops such a run at `READS_MAX`, with every read that far compared.
#[test]
fn scans_equal_c_when_the_counts_wrap() -> Result<(), &'static str> {
    let shape = Shape {
        wrapping: true,
        ..Shape::default()
    };
    let (mut converged, mut endless) = (0, 0);
    for seed in 1..=300 {
        match run_case(seed, shape) {
            Ok(()) => converged += 1,
            Err(check::NOT_CONVERGED) => endless += 1,
            Err(error) => return Err(error),
        }
    }
    assert!(converged >= 100, "{converged} converged, {endless} did not");
    Ok(())
}

/// A second view must not add to the first one's capacity: the store is reused.
#[test]
fn a_later_view_does_not_add_to_the_memo() -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    let mut rng = Rng(23);
    let mut d = random_ranks(&mut rng, &input, Shape::default());
    run_passes(&mut d, 0, &mut rng)?;
    let root = d.mincross_store.scan.pairs.capacity();
    run_passes(&mut d, 1, &mut rng)?;
    assert_eq!(d.mincross_store.scan.pairs.capacity(), root);
    Ok(())
}

/// The bound that keeps these structures from growing with the work done: they hold one record
/// per rank position, and a run's many exchanges add none.
#[test]
fn the_memo_is_one_record_per_rank_position() -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut d = random_ranks(&mut rng, &input, Shape::default());
        d.prepare_mincross(0)?;
        let capacity = |d: &Dot<'_>| {
            let scan = &d.mincross_store.scan;
            (
                scan.items.capacity(),
                scan.work.capacity(),
                scan.ranks.capacity(),
                scan.pairs.capacity(),
            )
        };
        let reserved = capacity(&d);
        run_passes(&mut d, 0, &mut rng)?;
        assert_eq!(capacity(&d), reserved, "seed {seed}");
        let positions = (0..=d.graphs[0].maxrank)
            .map(|r| d.rank_n(0, r) as usize)
            .sum::<usize>();
        assert_eq!(d.mincross_store.scan.pairs.len(), positions, "seed {seed}");
        assert_eq!(reserved.3, positions, "seed {seed}");
    }
    Ok(())
}

#[test]
fn scans_equal_c_under_constraints() -> Result<(), &'static str> {
    let shape = Shape {
        constrained: true,
        ..Shape::default()
    };
    for seed in 1..=1_000 {
        run_case(seed, shape)?;
    }
    Ok(())
}

/// An exchange can change a comparison with a tied stale order, so the crossings are not a
/// potential that falls and C's loop may not end here either, even on small penalties.
#[test]
fn scans_equal_c_with_stale_orders_outside_the_window() -> Result<(), &'static str> {
    let shape = Shape {
        stale_ties: true,
        ..Shape::default()
    };
    let (mut converged, mut endless) = (0, 0);
    for seed in 1..=300 {
        match run_case(seed, shape) {
            Ok(()) => converged += 1,
            Err(check::NOT_CONVERGED) => endless += 1,
            Err(error) => return Err(error),
        }
    }
    assert!(converged >= 100, "{converged} converged, {endless} did not");
    Ok(())
}

/// Whether a node of graph 1's view outside its rank's window has an order inside that window.
fn has_stale_tie(d: &Dot<'_>) -> bool {
    let store = &d.mincross_store;
    store.nodes.iter().enumerate().any(|(slot, &v)| {
        let r = store.meta[slot].rank;
        let (base, n) = (d.rk(1, r).voff as i32, d.rk(1, r).n);
        let inside = (base..base + n).contains(&store.order[slot]);
        inside && !(0..n).any(|i| d.rv(1, r, i) == Some(v))
    })
}

#[test]
fn a_stale_order_in_a_window_hands_the_transpose_to_c() -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    let shape = Shape {
        stale_ties: true,
        ..Shape::default()
    };
    let (mut ties, mut clean) = (0, 0);
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut d = random_ranks(&mut rng, &input, shape);
        d.prepare_mincross(1)?;
        let tie = has_stale_tie(&d);
        assert_eq!(d.transpose_scans(1, false)?, !tie, "seed {seed}");
        if tie {
            ties += 1;
        } else {
            clean += 1;
        }
    }
    assert!(ties > 0 && clean > 0, "{ties} ties, {clean} clean");
    Ok(())
}

#[test]
#[should_panic(expected = "transpose: ")]
fn the_cross_check_sees_a_stale_order_tie() {
    let input = Graph::new("G", GraphKind::Directed, false);
    let shape = Shape {
        stale_ties: true,
        ..Shape::default()
    };
    for seed in 1..=2_000u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut d = random_ranks(&mut rng, &input, shape);
        d.mincross_store.scan.ignore_stale_ties = true;
        let _ = run_passes(&mut d, 1, &mut rng);
    }
}

/// The ranks of #682: 40 parallel edges of penalty 1,000 merge into one of 40,000 when i + j is
/// even, and a single edge has 1,000. Two such products add up past `out_cross`'s `i32`.
fn issue_682_ranks<'a>(input: &'a Graph, top: [usize; 4], bottom: [usize; 4]) -> Dot<'a> {
    let mut d = Dot::empty(input);
    d.nodes.resize_with(8, NodeInfo::default);
    for (order, (&a, &b)) in top.iter().zip(&bottom).enumerate() {
        d.nodes[a].order = order as i32;
        d.nodes[4 + b].rank = 1;
        d.nodes[4 + b].order = order as i32;
    }
    d.bufs.push(top.iter().map(|&a| Some(a)).collect());
    d.bufs.push(bottom.iter().map(|&b| Some(4 + b)).collect());
    d.graphs.push(GraphInfo {
        minrank: 0,
        maxrank: 1,
        rank: vec![Rank::default(); 4],
        ..GraphInfo::default()
    });
    for r in 0..2 {
        *d.rk_mut(0, r) = Rank {
            vbuf: r as usize,
            n: 4,
            ..Rank::default()
        };
    }
    for a in 0..4 {
        for b in 0..4 {
            let e = d.edges.len();
            d.edges.push(EdgeInfo {
                tail: a,
                head: 4 + b,
                xpenalty: if (a + b) % 2 == 0 { 40_000 } else { 1_000 },
                ..EdgeInfo::default()
            });
            d.nodes[a].out.push(e);
            d.nodes[4 + b].in_.push(e);
        }
    }
    d
}

/// Some start of #682's ranks that C's loop does not leave within the read limit.
fn issue_682_endless<'a>(input: &'a Graph) -> Option<(Dot<'a>, bool)> {
    const ORDERS: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [0, 2, 1, 3],
        [1, 0, 2, 3],
        [3, 1, 2, 0],
        [0, 3, 2, 1],
        [2, 0, 3, 1],
    ];
    for top in ORDERS {
        for bottom in ORDERS {
            for reverse in [false, true] {
                let mut d = issue_682_ranks(input, top, bottom);
                if d.transpose(0, reverse) == Err(check::NOT_CONVERGED) {
                    return Some((issue_682_ranks(input, top, bottom), reverse));
                }
            }
        }
    }
    None
}

/// `transpose` does not finish on #682's ranks, as in C, and check mode stops it at the limit.
#[test]
fn the_ranks_of_issue_682_do_not_converge() {
    let input = Graph::new("G", GraphKind::Directed, false);
    assert!(issue_682_endless(&input).is_some());
}

/// C's loop as the fallback gets the read limit too, or check mode would spin forever here.
#[test]
fn a_fallback_stops_at_the_read_limit() -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    let (mut d, reverse) = issue_682_endless(&input).ok_or("mincross: no endless start")?;
    d.prepare_mincross(0)?;
    d.rk_mut(0, 0).mincross_position = Some(false);
    assert_eq!(d.transpose(0, reverse), Err(check::NOT_CONVERGED));
    Ok(())
}

#[test]
fn shortcuts_run_on_the_random_ranks() -> Result<(), &'static str> {
    // These tests prove nothing if everything falls back to C's loop.
    let input = Graph::new("G", GraphKind::Directed, false);
    let mut rng = Rng(7);
    let mut d = random_ranks(&mut rng, &input, Shape::default());
    d.prepare_mincross(0)?;
    assert!(d.transpose_scans(0, false)?);
    assert!(d.reorder_settled(0, 0, false)?);
    Ok(())
}

#[test]
fn nan_medians_and_broken_windows_use_c_loops() -> Result<(), &'static str> {
    let input = Graph::new("G", GraphKind::Directed, false);
    let mut rng = Rng(11);
    let mut d = random_ranks(&mut rng, &input, Shape::default());
    d.prepare_mincross(0)?;
    let v = d.rv(0, 0, 0).ok_or("mincross: missing node")?;
    d.mc_set_mval(d.mc_node(v), f64::NAN);
    assert!(!d.reorder_settled(0, 0, false)?);
    // A window that is not a permutation: the proof of its positions fails.
    let slot = d.mc_node(v).slot;
    d.mincross_store.order[slot] = 999;
    d.rk_mut(0, 0).mincross_position = None;
    assert!(!d.transpose_scans(0, false)?);
    assert!(!d.reorder_settled(0, 0, false)?);
    Ok(())
}

#[test]
#[should_panic(expected = "transpose: counts differ")]
fn the_cross_check_sees_a_missed_invalidation() {
    let input = Graph::new("G", GraphKind::Directed, false);
    for seed in 1..=2_000 {
        let mut rng = Rng((seed as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut d = random_ranks(&mut rng, &input, Shape::default());
        d.mincross_store.scan.keep_stale_pairs = true;
        let _ = run_passes(&mut d, 0, &mut rng);
    }
}

#[test]
#[should_panic(expected = "reorder: settled order")]
fn the_cross_check_sees_a_wrong_tie_order() {
    let input = Graph::new("G", GraphKind::Directed, false);
    for seed in 1..=2_000 {
        let mut rng = Rng((seed as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut d = random_ranks(&mut rng, &input, Shape::default());
        d.mincross_store.scan.keep_ties = true;
        let _ = run_passes(&mut d, 0, &mut rng);
    }
}
