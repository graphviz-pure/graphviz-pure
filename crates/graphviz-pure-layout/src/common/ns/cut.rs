//! Network simplex pivots on cut sets: the entering edge of `ns.c`, without its subtree walks.
//!
//! C's pivot (`enter_edge` and `update`, ns.c:282 and ns.c:707) walks the subtree under the
//! leaving tree edge `e` three times: `dfs_enter_outedge`/`dfs_enter_inedge` scan every edge
//! list in it, `rerank` shifts every rank in it, and `dfs_range` renumbers `lim`/`low` under
//! the LCA. On dot's large graphs that subtree is a large part of the graph at every pivot, so
//! network simplex is quadratic. This module keeps instead:
//!
//! - **cut sets**: for every tree edge `t`, the non-tree edges whose fundamental cycle
//!   contains `t`, which are the non-tree edges crossing t's cut. Each entry carries one bit,
//!   [`AGAINST`]: set when the edge crosses from head(t)'s side to tail(t)'s side;
//! - **slacks** of every edge, kept current in place of the ranks, which are written once,
//!   when the pivots end;
//! - **parent pointers**, re-hung at each pivot. `lim`/`low` are not maintained.
//!
//! C's candidates for `enter_edge(e)` are exactly the members of e's cut set that cross
//! against `e`. C returns the first of minimum slack in the order its stack-based scan meets
//! them; that order is recomputed from the parent pointers, only between tied candidates. A
//! pivot then costs about |cut set of e| × |tree cycle of f| list operations.
//!
//! The proof that this picks the same entering edge at every pivot and leaves the same ranks,
//! cut values, tree lists and parent pointers is `docs/perf/ns-cut-sets.md`; the functions
//! below name the invariant they keep. [`begin`] picks this path per graph from a sampled cost
//! estimate. Where C's scan is not an argmin (a candidate with a negative slack, only after
//! wrapped arithmetic), where a pivot's check phase finds the cut sets inconsistent, or where a
//! pivot could take the pool past its budget ([`CUT_SETS_MAX_WORDS`]), the rest of the run goes
//! back to the functions that mirror `ns.c`, and C's search picks the entering edge.
//! Under `cfg(test)` or the `ns-check` feature, every pivot also runs C's search, `rerank`,
//! `treeupdate` and `dfs_range`, and asserts that both agree (`check.rs`).
//!
//! Storage, all reused across calls through [`NsGraph::take_cut_store`]: the edges of the
//! graph by a local index (their order in `GD_nlist`/`ND_out`), the tree edges by their
//! `ED_tree_index` (the slot in `Tree_edge` that the entering edge takes over from the leaving
//! one), the cut sets as segments of one pool, and one work list.

use super::{Ctx, NsGraph, dfs_range_impl, exchange_tree_edges, seq, slack, tree_edge};

/// Selector: graphs with fewer nodes than this keep C's search. Their subtree walks are short,
/// and the cut sets' storage would not pay for itself.
const CUT_SETS_MIN_NODES: usize = 2500;

/// Selector: the largest estimated cost ratio, (mean tree-cycle length × mean cut-set size) /
/// node count, for which the cut sets are faster than C's walks. Tuned on the corpus
/// (`docs/perf/ns-cut-sets.md`, "The selector").
const CUT_SETS_MAX_COST: f64 = 1.0;

/// Selector: the most cut-set entries per node and non-tree edge, estimated as l·m / (n + m).
/// Each entry is four bytes on top of a graph that takes a few hundred bytes per node and
/// edge, so this bounds the extra memory to a few percent. It also turns away graphs whose
/// tree cycles are long compared with their size, where a pivot's cut-set edit can cost more
/// than C's walk even when the time estimate says otherwise (`docs/perf/ns-cut-sets.md`).
const CUT_SETS_MAX_ENTRIES: f64 = 10.0;

/// Selector: how many non-tree edges are sampled to estimate the mean tree-cycle length.
const CUT_SETS_SAMPLE: usize = 64;

/// The cut sets' pool may hold at most this many four-byte words per node and non-tree edge.
/// The selector's estimate is a sample of the initial tree; this is the bound that holds
/// during the run. A build that needs more keeps C's functions, and a pivot that could take
/// the pool past it hands the rest of the run to them, before it changes anything.
const CUT_SETS_MAX_WORDS: usize = 24;

/// Storage kept between runs, in four-byte words: a store that grew past this is released
/// when its run ends, so a large graph's cut sets do not stay allocated for the rest of the
/// layout. Smaller ones are kept and reused by the next run.
const CUT_SETS_KEEP_WORDS: usize = 1 << 18;

/// Cut-set entry flag: the edge crosses from head(t)'s side of the cut to tail(t)'s side.
const AGAINST: u32 = 0x8000_0000;
/// Path entry flag: the cycle of `f`, oriented along `f`, runs this tree edge forwards.
const FORWARD: u32 = 0x8000_0000;
/// The index in an entry.
const ID: u32 = 0x7fff_ffff;
/// Header words before each cut set in the pool: its tree slot and its capacity.
const HEADER: u32 = 2;

/// The capacity a cut set of `len` entries gets when it is laid out.
fn headroom(len: u32) -> u32 {
    len.saturating_add(len.wrapping_div(8)).saturating_add(2)
}

/// An edge of the graph, by local index.
#[derive(Clone, Copy, Default)]
struct EdgeSlot {
    /// The edge.
    edge: u32,
    /// `rank(head) - rank(tail) - minlen`, wrapping, as C would compute it now.
    slack: i32,
    /// While a cut set is edited: `stamp` equals the edit's number when the edge is in it,
    /// at index `slot`.
    stamp: u32,
    slot: u32,
}

/// A tree edge, by its `ED_tree_index`.
#[derive(Clone, Copy, Default)]
struct TreeSlot {
    /// The tree edge's local index.
    edge: u32,
    /// Its cut set: `pool[start..start + len]`, with room for `cap`.
    start: u32,
    len: u32,
    cap: u32,
    /// The LCA walk's mark.
    mark: u32,
}

/// Reusable storage for the cut-set pivots. [`NsGraph::take_cut_store`] hands it to
/// [`rank2`](super::rank2), which gives it back when the pivots end.
#[derive(Default)]
pub struct CutStore {
    edges: Vec<EdgeSlot>,
    tree: Vec<TreeSlot>,
    /// Every cut set, each a segment: two header words (its tree slot, its capacity), then
    /// the entries ([`TreeSlot::start`] points past the header). The headers let
    /// [`CutStore::compact`] walk the pool in place.
    pool: Vec<u32>,
    /// Pool words that no cut set uses since theirs moved to a bigger segment.
    garbage: usize,
    /// One phase at a time: the ancestor paths of a tie-break, the tree cycle of the entering
    /// edge (tree slot | [`FORWARD`]), the order of a compaction, the stack of
    /// [`CutStore::write_ranks`].
    work: Vec<u32>,
    mark_gen: u32,
    stamp_gen: u32,
    /// The root's rank: what it was when the pivots began, moved by C's `rerank` of the
    /// whole tree (from the root, ns.c:697). The other ranks follow from the slacks.
    root_rank: i32,
    /// The most words the pool may hold ([`CUT_SETS_MAX_WORDS`]).
    budget: usize,
    /// The pivot [`CutStore::prepare`] checked, its cycle in `work`, for `update`.
    prepared: Option<Prepared>,
}

/// A pivot whose cut sets, LCA, tree cycle and pool budget were checked.
#[derive(Clone, Copy)]
struct Prepared {
    e: usize,
    f: usize,
    se: usize,
    kf: u32,
    lca: usize,
}

/// C's functions take over: a candidate has a negative slack (C's pruned scan is not an
/// argmin there), the tree is not what the cut sets describe, or the pool would outgrow its
/// budget.
struct Fallback;

fn other<G: NsGraph + ?Sized>(g: &G, e: usize, u: usize) -> usize {
    if g.tail(e) == u { g.head(e) } else { g.tail(e) }
}

fn parent<G: NsGraph + ?Sized>(g: &G, u: usize) -> Option<usize> {
    g.par(u).map(|e| other(g, e, u))
}

/// The tree slot of tree edge `e`: its `ED_tree_index`.
fn tree_slot<G: NsGraph + ?Sized>(g: &G, e: usize) -> Option<usize> {
    usize::try_from(g.tree_index(e)).ok()
}

/// Whether `a` comes before `b` in `list` (C's `ND_in`/`ND_out`/tree lists).
fn first_of(list: &[usize], a: usize, b: usize) -> bool {
    list.iter().find(|&&x| x == a || x == b) == Some(&a)
}

/// Visits the tree path between the ends of non-tree edge `f`, as `(t, against)` where
/// `against` says that `f` crosses t's cut from head(t)'s side to tail(t)'s side. Needs valid
/// `lim`/`low`. `false` if the tree is not what `lim`/`low` describe.
fn tree_path<G: NsGraph + ?Sized>(g: &G, f: usize, mut visit: impl FnMut(usize, bool)) -> bool {
    let (a, b) = (g.tail(f), g.head(f));
    // from tail(f) up to the LCA: tail(f) is on u's side of par(u)
    let mut u = a;
    while !seq(g, b, g.low(u), g.lim(u)) {
        let Some(t) = g.par(u) else {
            return false;
        };
        visit(t, g.tail(t) != u);
        u = other(g, t, u);
    }
    // from head(f) up to the LCA: tail(f) is on the other side
    let mut u = b;
    while !seq(g, a, g.low(u), g.lim(u)) {
        let Some(t) = g.par(u) else {
            return false;
        };
        visit(t, g.tail(t) == u);
        u = other(g, t, u);
    }
    true
}

/// Every edge, nodes in `GD_nlist` order and each node's `ND_out` in order: the local order.
fn all_edges<G: NsGraph + ?Sized>(g: &G) -> impl Iterator<Item = usize> + '_ {
    g.nlist()
        .iter()
        .flat_map(move |&u| g.out_edges(u).iter().copied())
}

/// Whether the cut sets are expected to be faster than C's walks on this graph, within a
/// small memory budget. The mean tree-cycle length `l` is estimated from up to
/// [`CUT_SETS_SAMPLE`] evenly spaced non-tree edges; for `m` non-tree edges and `n` nodes the
/// cut sets then hold about `l × m` entries, at most [`CUT_SETS_MAX_ENTRIES`] per node and
/// non-tree edge, and a pivot costs about `l × (l × m / n)` against C's walks of a fraction of
/// `n`.
fn worth_it<G: NsGraph + ?Sized>(g: &G) -> bool {
    let n = g.nlist().len();
    if n < CUT_SETS_MIN_NODES {
        return false;
    }
    let nontree = || all_edges(g).filter(|&e| !tree_edge(g, e));
    let m = nontree().count();
    if m == 0 {
        return false;
    }
    let stride = m.wrapping_div(CUT_SETS_SAMPLE).max(1);
    let mut sampled = 0usize;
    let mut length = 0usize;
    for f in nontree().step_by(stride).take(CUT_SETS_SAMPLE) {
        if !tree_path(g, f, |_, _| length = length.wrapping_add(1)) {
            return false;
        }
        sampled = sampled.wrapping_add(1);
    }
    let l = length as f64 / sampled.max(1) as f64;
    let entries = l * m as f64;
    let cost = l * (entries / n as f64) / n as f64;
    entries <= CUT_SETS_MAX_ENTRIES * (n as f64 + m as f64) && cost <= CUT_SETS_MAX_COST
}

/// Starts the cut-set pivots after `feasible_tree` (`lim`/`low` valid), when [`worth_it`]
/// (always in check mode); `None` keeps C's functions.
pub(super) fn begin<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx) -> Option<CutStore> {
    if !check::ON && !worth_it(g) {
        return None;
    }
    let mut store = g.take_cut_store();
    if store.build(g, ctx) {
        Some(store)
    } else {
        g.keep_cut_store(store);
        None
    }
}

impl CutStore {
    fn edge(&self, k: u32) -> EdgeSlot {
        self.edges.get(k as usize).copied().unwrap_or_default()
    }

    fn edge_mut(&mut self, k: u32) -> Option<&mut EdgeSlot> {
        self.edges.get_mut(k as usize)
    }

    fn tree(&self, s: usize) -> TreeSlot {
        self.tree.get(s).copied().unwrap_or_default()
    }

    fn tree_mut(&mut self, s: usize) -> Option<&mut TreeSlot> {
        self.tree.get_mut(s)
    }

    fn entry(&self, i: u32) -> u32 {
        self.pool.get(i as usize).copied().unwrap_or(0)
    }

    fn set_entry(&mut self, i: u32, x: u32) {
        if let Some(slot) = self.pool.get_mut(i as usize) {
            *slot = x;
        } else {
            debug_assert!(false, "cut-set entry out of the pool");
        }
    }

    /// Writes the header of the segment whose entries start at `start`.
    fn set_header(&mut self, start: u32, s: u32, cap: u32) {
        self.set_entry(start.wrapping_sub(2), s);
        self.set_entry(start.wrapping_sub(1), cap);
    }

    /// Check mode's corruption: takes `f` out of tree slot `s`'s cut set.
    #[cfg(any(test, feature = "ns-check"))]
    fn remove_member(&mut self, s: usize, f: usize) {
        let set = self.tree(s);
        let at = (0..set.len)
            .find(|&i| self.edge(self.entry(set.start.wrapping_add(i)) & ID).edge as usize == f);
        if let Some(i) = at {
            self.remove_at(s, i);
        }
    }

    /// The local index of `f` among the members of tree slot `s`'s cut set.
    fn member(&self, s: usize, f: usize) -> Option<u32> {
        let set = self.tree(s);
        (0..set.len)
            .map(|i| self.entry(set.start.wrapping_add(i)) & ID)
            .find(|&k| self.edge(k).edge as usize == f)
    }

    /// The slack C would compute for `f`, a member of e's cut set; `None` if it is not one.
    fn entering_slack<G: NsGraph + ?Sized>(&self, g: &G, e: usize, f: usize) -> Option<i32> {
        tree_slot(g, e)
            .and_then(|s| self.member(s, f))
            .map(|k| self.edge(k).slack)
    }

    /// Releases the storage once it is larger than [`CUT_SETS_KEEP_WORDS`].
    fn trim(&mut self) {
        let words = self
            .pool
            .capacity()
            .saturating_add(self.work.capacity())
            .saturating_add(self.edges.capacity().saturating_mul(4))
            .saturating_add(self.tree.capacity().saturating_mul(5));
        if words > CUT_SETS_KEEP_WORDS {
            *self = CutStore::default();
        }
    }

    /// The slack of tree edge `t`.
    fn tree_slack<G: NsGraph + ?Sized>(&self, g: &G, t: usize) -> i32 {
        let Some(s) = tree_slot(g, t) else {
            debug_assert!(false, "a tree edge has a tree slot");
            return slack(g, t);
        };
        self.edge(self.tree(s).edge).slack
    }

    /// The cut sets, slacks and local indices of the current tree. `false` (C's functions
    /// then run) when an index does not fit an entry or the tree disagrees with `lim`/`low`.
    fn build<G: NsGraph + ?Sized>(&mut self, g: &G, ctx: &Ctx) -> bool {
        let count = all_edges(g).count();
        if count > ID as usize {
            return false;
        }
        self.edges.clear();
        self.edges.reserve(count);
        self.tree.clear();
        self.tree.resize(ctx.tree_edge.len(), TreeSlot::default());
        self.mark_gen = 0;
        self.stamp_gen = 0;
        self.garbage = 0;
        self.prepared = None;
        self.root_rank = g.nlist().first().map_or(0, |&r| g.rank(r));
        let nontree = all_edges(g).filter(|&e| !tree_edge(g, e)).count();
        self.budget = CUT_SETS_MAX_WORDS.saturating_mul(g.nlist().len().saturating_add(nontree));
        for e in all_edges(g) {
            let Ok(edge) = u32::try_from(e) else {
                return false;
            };
            let k = self.edges.len() as u32;
            self.edges.push(EdgeSlot {
                edge,
                slack: slack(g, e),
                stamp: 0,
                slot: 0,
            });
            if let Some(s) = tree_slot(g, e) {
                let Some(ts) = self.tree.get_mut(s) else {
                    return false;
                };
                ts.edge = k;
            }
        }
        // count, lay the segments out with room to grow, then fill
        for e in all_edges(g).filter(|&e| !tree_edge(g, e)) {
            let tree = &mut self.tree;
            let walked = tree_path(g, e, |t, _| {
                if let Some(ts) = tree_slot(g, t).and_then(|s| tree.get_mut(s)) {
                    ts.len = ts.len.wrapping_add(1);
                }
            });
            if !walked {
                return false;
            }
        }
        let mut size = 0u32;
        for ts in &mut self.tree {
            ts.cap = headroom(ts.len);
            ts.len = 0;
            let Some(next) = size
                .checked_add(HEADER)
                .and_then(|n| n.checked_add(ts.cap))
                .filter(|&n| n <= ID)
            else {
                return false;
            };
            ts.start = size.wrapping_add(HEADER);
            size = next;
        }
        self.budget = check::budget(size as usize).unwrap_or(self.budget);
        if size as usize > self.budget {
            return false;
        }
        self.pool.clear();
        let room = (size as usize).saturating_add((size as usize).wrapping_div(8));
        self.pool.reserve_exact(room.min(self.budget));
        self.pool.resize(size as usize, 0);
        for s in 0..self.tree.len() {
            let ts = self.tree(s);
            self.set_header(ts.start, s as u32, ts.cap);
        }
        for (k, e) in all_edges(g).enumerate() {
            if tree_edge(g, e) {
                continue;
            }
            let (tree, pool) = (&mut self.tree, &mut self.pool);
            tree_path(g, e, |t, against| {
                if let Some(ts) = tree_slot(g, t).and_then(|s| tree.get_mut(s))
                    && ts.len < ts.cap
                    && let Some(x) = pool.get_mut(ts.start.wrapping_add(ts.len) as usize)
                {
                    *x = k as u32 | if against { AGAINST } else { 0 };
                    ts.len = ts.len.wrapping_add(1);
                }
            });
        }
        true
    }

    /// C: lib/common/ns.c:enter_edge, dfs_enter_outedge, dfs_enter_inedge
    ///
    /// C's candidates are the non-tree edges crossing e's cut against `e` (from head(e)'s
    /// side to tail(e)'s side): the `AGAINST` members of e's cut set. C keeps the first of
    /// minimum slack in its scan order, which [`CutStore::scanned_before`] decides between
    /// two tied candidates. That is C's answer only while no candidate has a negative slack:
    /// below 0, C's `Slack > 0` pruning can stop its scan before the minimum ([`Fallback`]).
    /// A pivot with no candidate at all asks C's search too ([`Fallback`]).
    ///
    /// For a pivot (`pivot`), the pivot is then checked ([`CutStore::prepare`]) before the
    /// edge is returned: if that fails, C's search picks the edge instead.
    fn enter_edge<G: NsGraph + ?Sized>(
        &mut self,
        g: &G,
        e: usize,
        pivot: bool,
    ) -> Result<Option<usize>, Fallback> {
        self.prepared = None;
        let Some(s) = tree_slot(g, e) else {
            return Err(Fallback);
        };
        let tail_is_child = g.par(g.tail(e)) == Some(e);
        let v = if tail_is_child { g.tail(e) } else { g.head(e) };
        let set = self.tree(s);
        let mut best: Option<(u32, i32)> = None;
        for i in 0..set.len {
            let x = self.entry(set.start.wrapping_add(i));
            if x & AGAINST == 0 {
                continue;
            }
            let k = x & ID;
            let sk = self.edge(k).slack;
            if sk < 0 {
                return Err(Fallback);
            }
            let better = match best {
                None => true,
                Some((w, sw)) if sk == sw => {
                    check::count(&check::TIES);
                    self.scanned_before(g, tail_is_child, v, k, w)
                }
                Some((_, sw)) => sk < sw,
            };
            if better {
                best = Some((k, sk));
            }
        }
        // no candidate: for a pivot, C's search is asked (`rank2` reports it if C finds none
        // either); `LR_balance` skips such edges, as C does
        let Some((k, _)) = best else {
            return if pivot { Err(Fallback) } else { Ok(None) };
        };
        let f = self.edge(k).edge as usize;
        if pivot {
            check::corrupt(self, check::Corrupt::Enter, s, f);
            self.prepare(g, e, f)?;
        }
        Ok(Some(f))
    }

    /// The check phase of a pivot, before anything changes: e's tree slot, `f` in e's cut set,
    /// the LCA of f's ends, f's tree cycle with a tree slot for every edge (left in `work`),
    /// and room in the pool for the most this pivot can add to it. Any failure is a
    /// [`Fallback`]; nothing has been changed by then.
    fn prepare<G: NsGraph + ?Sized>(&mut self, g: &G, e: usize, f: usize) -> Result<(), Fallback> {
        self.prepared = None;
        let (Some(se), Some(kf)) = (
            tree_slot(g, e),
            tree_slot(g, e).and_then(|s| self.member(s, f)),
        ) else {
            return Err(Fallback);
        };
        let (ft, fh) = (g.tail(f), g.head(f));
        let Some(lca) = self.lca(g, ft, fh) else {
            return Err(Fallback);
        };
        // f's tree cycle, oriented along f: from head(f) up to the LCA, then down to tail(f)
        let len_e = self.tree(se).len as usize;
        let mut growth = 0usize;
        self.work.clear();
        for (start, down) in [(ft, true), (fh, false)] {
            let mut u = start;
            while u != lca {
                let (Some(t), Some(s)) = (g.par(u), g.par(u).and_then(|t| tree_slot(g, t))) else {
                    return Err(Fallback);
                };
                let forward = if down { g.head(t) == u } else { g.tail(t) == u };
                self.work.push(s as u32 | if forward { FORWARD } else { 0 });
                // t's set ends this pivot with at most `len` entries. Each move to the end of
                // the pool doubles its capacity (2·cap + 4), so the moved segments add up to
                // under twice the last one, which is at most 2·len + 4; a u32 capacity
                // doubles at most 32 times, each move with its header.
                let len = (self.tree(s).len as usize)
                    .saturating_add(len_e)
                    .saturating_add(1);
                let most = len
                    .saturating_mul(4)
                    .saturating_add(8usize.saturating_add(32usize.saturating_mul(HEADER as usize)));
                growth = growth.saturating_add(most);
                u = other(g, t, u);
            }
        }
        if self.pool.len().saturating_add(growth) > self.budget && self.garbage > 0 {
            self.compact();
        }
        if self.pool.len().saturating_add(growth) > self.budget {
            return Err(Fallback);
        }
        self.prepared = Some(Prepared { e, f, se, kf, lca });
        Ok(())
    }

    /// Whether candidate `c` comes before `w` (local indices) in C's scan of the subtree
    /// under `v`.
    ///
    /// `dfs_enter_inedge` (tail of `e` below it) pops a node, scans its `ND_in` list, and
    /// pushes its children: those joined by an in-edge in `ND_in` order, then those joined by
    /// an out-tree edge in `ND_tree_out` order; `dfs_enter_outedge` the same with in and out
    /// swapped. So nodes come in pre-order, children in the reverse of their push order, and
    /// edges in list order within a node.
    fn scanned_before<G: NsGraph + ?Sized>(
        &mut self,
        g: &G,
        insearch: bool,
        v: usize,
        c: u32,
        w: u32,
    ) -> bool {
        let (c, w) = (self.edge(c).edge as usize, self.edge(w).edge as usize);
        let end = |e: usize| if insearch { g.head(e) } else { g.tail(e) };
        let (sc, sw) = (end(c), end(w));
        if sc == sw {
            let list = if insearch {
                g.in_edges(sc)
            } else {
                g.out_edges(sc)
            };
            return first_of(list, c, w);
        }
        // both ancestor paths up to v, one after the other
        let mut walk = std::mem::take(&mut self.work);
        walk.clear();
        let climb = |walk: &mut Vec<u32>, mut u: usize| {
            walk.push(u as u32);
            while u != v {
                let Some(p) = parent(g, u) else {
                    debug_assert!(false, "candidate end outside the searched subtree");
                    return;
                };
                u = p;
                walk.push(u as u32);
            }
        };
        climb(&mut walk, sc);
        let split = walk.len();
        climb(&mut walk, sw);
        let (a, b) = walk.split_at(split);
        // the common part, from v down to the LCA
        let common = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let at = |p: &[u32], from_end: usize| {
            p.len()
                .checked_sub(from_end)
                .and_then(|i| p.get(i).copied())
                .map(|u| u as usize)
        };
        let before = match (at(a, common.wrapping_add(1)), at(b, common.wrapping_add(1))) {
            // c's end is an ancestor of w's, or the other way round: pre-order
            (None, _) => true,
            (_, None) => false,
            (Some(ca), Some(cb)) => {
                let lca = at(a, common).unwrap_or(v);
                push_rank(g, insearch, lca, ca) > push_rank(g, insearch, lca, cb)
            }
        };
        self.work = walk;
        before
    }

    /// C's `rerank` of e's child side by `-shift`, on the slacks: only `e` and the members
    /// of e's cut set have exactly one end on that side.
    fn shift(&mut self, s: usize, tail_is_child: bool, shift: i32) {
        // slack = rank(head) - rank(tail) - minlen
        let moved = |sl: i32, tail_inside: bool| {
            if tail_inside {
                sl.wrapping_sub(shift)
            } else {
                sl.wrapping_add(shift)
            }
        };
        let set = self.tree(s);
        for i in 0..set.len {
            let x = self.entry(set.start.wrapping_add(i));
            // AGAINST clear: tail(g) is on tail(e)'s side, which moves when tail(e) is the child
            let tail_inside = (x & AGAINST == 0) == tail_is_child;
            if let Some(es) = self.edge_mut(x & ID) {
                es.slack = moved(es.slack, tail_inside);
            }
        }
        if let Some(es) = self.edge_mut(set.edge) {
            es.slack = moved(es.slack, tail_is_child);
        }
    }

    /// The LCA of `a` and `b`. Both climb one tree edge at a time, marking it; a climber
    /// that reaches an edge the other marked, or both reaching the root, stops at the LCA.
    /// Costs at most twice the longer of the two paths.
    fn lca<G: NsGraph + ?Sized>(&mut self, g: &G, a: usize, b: usize) -> Option<usize> {
        if a == b {
            return Some(a);
        }
        if self.mark_gen >= u32::MAX.wrapping_sub(2) {
            for ts in &mut self.tree {
                ts.mark = 0;
            }
            self.mark_gen = 0;
        }
        self.mark_gen = self.mark_gen.wrapping_add(2);
        let marks = [self.mark_gen, self.mark_gen.wrapping_add(1)];
        let mut at = [a, b];
        let mut parked = [false, false];
        loop {
            for i in 0..2 {
                let (Some(&u), Some(&mine), Some(&theirs)) =
                    (at.get(i), marks.get(i), marks.get(i.wrapping_add(1) & 1))
                else {
                    return None;
                };
                let Some(t) = g.par(u) else {
                    if let Some(p) = parked.get_mut(i) {
                        *p = true;
                    }
                    continue;
                };
                let ts = tree_slot(g, t).and_then(|s| self.tree.get_mut(s))?;
                if ts.mark == theirs {
                    return Some(u);
                }
                ts.mark = mine;
                if let Some(x) = at.get_mut(i) {
                    *x = other(g, t, u);
                }
            }
            if parked == [true, true] {
                // both at the root
                return at.first().copied();
            }
        }
    }

    fn next_stamp(&mut self) -> u32 {
        if self.stamp_gen == u32::MAX {
            for es in &mut self.edges {
                es.stamp = 0;
            }
            self.stamp_gen = 0;
        }
        self.stamp_gen = self.stamp_gen.wrapping_add(1);
        self.stamp_gen
    }

    /// Appends `x` to tree slot `s`'s cut set. A full set moves to a twice bigger segment at the
    /// end of the pool; when the pool has no room left, it is compacted if that frees enough,
    /// and grows by an eighth otherwise.
    fn push(&mut self, s: usize, x: u32) {
        let mut set = self.tree(s);
        if set.len >= set.cap {
            let cap = set.len.saturating_mul(2).saturating_add(4);
            let need = cap.saturating_add(HEADER) as usize;
            if self.pool.capacity().saturating_sub(self.pool.len()) < need {
                if self.garbage >= need.saturating_mul(2) {
                    self.compact();
                    set = self.tree(s);
                } else {
                    let more = need.max(self.pool.len().wrapping_div(8));
                    let room = self.budget.saturating_sub(self.pool.len());
                    self.pool.reserve_exact(more.min(room).max(need));
                }
            }
            let (from, to) = (set.start as usize, set.start.wrapping_add(set.len) as usize);
            let header = self.pool.len();
            let start = header.saturating_add(HEADER as usize);
            if to > header || start.saturating_add(cap as usize) > ID as usize {
                debug_assert!(false, "cut-set pool out of range");
                return;
            }
            self.pool.resize(start, 0);
            self.pool.extend_from_within(from..to);
            self.pool.resize(start.wrapping_add(cap as usize), 0);
            self.garbage = self
                .garbage
                .wrapping_add(set.cap.saturating_add(HEADER) as usize);
            set.start = start as u32;
            set.cap = cap;
            self.set_header(set.start, s as u32, cap);
            check::count(&check::RELOCATIONS);
        }
        self.set_entry(set.start.wrapping_add(set.len), x);
        let len = set.len;
        if let Some(es) = self.edge_mut(x & ID) {
            es.slot = len;
        }
        if let Some(ts) = self.tree_mut(s) {
            ts.start = set.start;
            ts.len = len.wrapping_add(1);
            ts.cap = set.cap;
        }
    }

    /// Removes the entry at index `i` of tree slot `s`'s cut set; the last entry takes its
    /// place.
    fn remove_at(&mut self, s: usize, i: u32) {
        let set = self.tree(s);
        let Some(last) = set.len.checked_sub(1) else {
            debug_assert!(false, "removal from an empty cut set");
            return;
        };
        let moved = self.entry(set.start.wrapping_add(last));
        self.set_entry(set.start.wrapping_add(i), moved);
        if let Some(es) = self.edge_mut(moved & ID) {
            es.slot = i;
        }
        if let Some(ts) = self.tree_mut(s) {
            ts.len = last;
        }
    }

    /// The cut sets once `e` (local `ke`, tree slot `se`) leaves the tree and `f` (local `kf`)
    /// enters it in the same slot; `self.work` holds f's tree cycle.
    ///
    /// A non-tree edge's fundamental cycle changes only when it contains `e`, that is for the
    /// members of e's cut set. For such a `g`, the new cycle is the old one minus a multiple
    /// of f's, so its membership toggles on every tree edge `t ≠ e` of f's cycle; where it
    /// joins, it crosses t's cut against `t` exactly when its crossing of `e` and f's
    /// orientation of `t` (`FORWARD`) differ. `f` leaves those cut sets and `e` joins them
    /// (against `t` where `FORWARD`). The cut of `f` is the cut `e` had, so f's set is e's
    /// with every bit flipped and `f` replaced by `e` (against `f`), in place.
    fn move_cut_sets(&mut self, se: usize, ke: u32, kf: u32) {
        let set_e = self.tree(se);
        // e's members after the path in `work`: pushes may move e's segment in the pool
        let steps = self.work.len();
        for i in 0..set_e.len {
            let x = self.entry(set_e.start.wrapping_add(i));
            self.work.push(x);
        }
        for p in 0..steps {
            let Some(&step) = self.work.get(p) else {
                break;
            };
            let st = (step & ID) as usize;
            let forward = step & FORWARD != 0;
            if st == se {
                continue;
            }
            // stamp t's members, with their index
            let stamp = self.next_stamp();
            let set_t = self.tree(st);
            for i in 0..set_t.len {
                let k = self.entry(set_t.start.wrapping_add(i)) & ID;
                if let Some(es) = self.edge_mut(k) {
                    es.stamp = stamp;
                    es.slot = i;
                }
            }
            for q in steps..self.work.len() {
                let Some(&x) = self.work.get(q) else {
                    break;
                };
                let k = x & ID;
                if k == kf {
                    continue;
                }
                let member = self.edge(k);
                if member.stamp == stamp {
                    self.remove_at(st, member.slot);
                } else {
                    let against = (x & AGAINST != 0) != forward;
                    self.push(st, k | if against { AGAINST } else { 0 });
                }
            }
            let ef = self.edge(kf);
            if ef.stamp == stamp {
                self.remove_at(st, ef.slot);
            } else {
                debug_assert!(false, "f is in the cut set of every edge of its cycle");
            }
            self.push(st, ke | if forward { AGAINST } else { 0 });
        }
        let mut found = false;
        let start_e = self.tree(se).start;
        for i in 0..set_e.len {
            let at = start_e.wrapping_add(i);
            let x = self.entry(at);
            if x & ID == kf {
                self.set_entry(at, ke | AGAINST);
                found = true;
            } else {
                self.set_entry(at, x ^ AGAINST);
            }
        }
        debug_assert!(found, "f is in e's cut set");
        if let Some(ts) = self.tree_mut(se) {
            ts.edge = kf;
        }
    }

    /// Packs the cut sets to the front of the pool, in place: walking the segments in pool
    /// order by their headers, each live one (its tree slot still points at it) slides down
    /// and keeps at most [`headroom`] of its capacity, so it never reaches a segment not yet
    /// moved.
    fn compact(&mut self) {
        let (mut from, mut to) = (0usize, 0usize);
        let end = self.pool.len();
        while from < end {
            let s = self.entry(from as u32) as usize;
            let cap = self.entry(from.wrapping_add(1) as u32) as usize;
            let start = from.wrapping_add(HEADER as usize);
            let next = start.saturating_add(cap);
            let set = self.tree(s);
            if set.start as usize == start && next <= end {
                let keep = headroom(set.len).min(set.cap);
                let len = set.len as usize;
                self.pool.copy_within(from..start.wrapping_add(len), to);
                let moved = to.wrapping_add(HEADER as usize) as u32;
                self.set_header(moved, s as u32, keep);
                if let Some(ts) = self.tree_mut(s) {
                    ts.start = moved;
                    ts.cap = keep;
                }
                to = (moved as usize).wrapping_add(keep as usize);
            }
            from = next;
        }
        self.pool.truncate(to);
        self.garbage = 0;
        check::count(&check::COMPACTIONS);
    }

    /// C: lib/common/ns.c:update, treeupdate, rerank, exchange_tree_edges, dfs_range
    ///
    /// C's `update` on the cut sets: the same `rerank` choice applied to the slacks (or, for
    /// the whole tree, to the root's rank), the same cut-value changes along f's tree cycle,
    /// the same `exchange_tree_edges`. Instead of `dfs_range`, only the parent pointers on the
    /// path from f's end in the moved subtree up to e's child are reversed, which is what
    /// `dfs_range` assigns them.
    fn update<G: NsGraph + ?Sized>(
        &mut self,
        g: &mut G,
        ctx: &mut Ctx,
        e: usize,
        f: usize,
    ) -> Result<i32, Fallback> {
        // the check phase ran when `enter_edge` picked f, and nothing has changed since; it is
        // redone only if this is not that pivot
        let p = match self.prepared.take() {
            Some(p) if p.e == e && p.f == f => p,
            _ => {
                self.prepare(g, e, f)?;
                self.prepared.take().ok_or(Fallback)?
            }
        };
        let Prepared { se, kf, lca, .. } = p;
        let (ft, fh) = (g.tail(f), g.head(f));
        let ke = self.tree(se).edge;
        let delta = self.edge(kf).slack;
        let (et, eh) = (g.tail(e), g.head(e));
        let tail_is_child = g.par(et) == Some(e);
        if delta > 0 {
            // ns.c:710: which node `rerank` starts from, and by how much
            let degree = |u: usize| g.tree_in(u).len().saturating_add(g.tree_out(u).len());
            let (node, d) = if degree(et) == 1 {
                (et, delta)
            } else if degree(eh) == 1 {
                (eh, delta.wrapping_neg())
            } else if tail_is_child {
                (et, delta)
            } else {
                (eh, delta.wrapping_neg())
            };
            if g.par(node).is_none() {
                // the root: `rerank` follows every tree edge and moves the whole tree
                self.root_rank = self.root_rank.wrapping_sub(d);
                check::count(&check::WHOLE_TREE);
            } else {
                // a node below e (a leaf is its own subtree): e's child side moves by -d
                self.shift(se, tail_is_child, d.wrapping_neg());
            }
            check::rerank(g, ctx, node, d);
        }
        let lca_low = check::path(g, self, e, f, lca);
        // treeupdate: each edge of the cycle gains cutvalue(e) when the cycle runs it
        // backwards and loses it when forwards
        let cutvalue = g.cutvalue(e);
        for &step in &self.work {
            let t = self.edge(self.tree((step & ID) as usize).edge).edge as usize;
            let cv = g.cutvalue(t);
            let next = if step & FORWARD != 0 {
                cv.wrapping_sub(cutvalue)
            } else {
                cv.wrapping_add(cutvalue)
            };
            g.set_cutvalue(t, next);
        }
        self.move_cut_sets(se, ke, kf);
        // re-hang the subtree below e from f: f's end in it is the new top
        let v = if tail_is_child { et } else { eh };
        let mut u = if tail_is_child { fh } else { ft };
        let mut above = f;
        let mut rehung = check::rehung_start();
        loop {
            let old = g.par(u);
            g.set_par(u, Some(above));
            check::rehung_push(&mut rehung, u, above);
            if u == v {
                break;
            }
            let Some(pe) = old else {
                debug_assert!(false, "f's end lies below e");
                break;
            };
            let next = other(g, pe, u);
            above = pe;
            u = next;
        }
        g.set_cutvalue(f, cutvalue.wrapping_neg());
        g.set_cutvalue(e, 0);
        exchange_tree_edges(g, ctx, e, f);
        check::dfs_range(g, ctx, lca, lca_low, &rehung);
        check::slacks(g, self, e, f);
        if self.garbage > self.pool.len().wrapping_div(2) {
            self.compact();
        }
        Ok(0)
    }

    /// Writes every rank: the root's, plus `minlen + slack` down each tree edge.
    fn write_ranks<G: NsGraph + ?Sized>(&mut self, g: &mut G) {
        let Some(&root) = g.nlist().first() else {
            return;
        };
        let r = self.root_rank;
        check::rank(g, root, r);
        g.set_rank(root, r);
        let mut stack = std::mem::take(&mut self.work);
        stack.clear();
        stack.push(root as u32);
        while let Some(u) = stack.pop() {
            let u = u as usize;
            let (ru, par) = (g.rank(u), g.par(u));
            for i in 0..g.tree_out(u).len() {
                let Some(&t) = g.tree_out(u).get(i) else {
                    break;
                };
                if Some(t) != par {
                    let c = g.head(t);
                    let rc = ru
                        .wrapping_add(g.minlen(t))
                        .wrapping_add(self.tree_slack(g, t));
                    check::rank(g, c, rc);
                    g.set_rank(c, rc);
                    stack.push(c as u32);
                }
            }
            for i in 0..g.tree_in(u).len() {
                let Some(&t) = g.tree_in(u).get(i) else {
                    break;
                };
                if Some(t) != par {
                    let c = g.tail(t);
                    let rc = ru
                        .wrapping_sub(g.minlen(t))
                        .wrapping_sub(self.tree_slack(g, t));
                    check::rank(g, c, rc);
                    g.set_rank(c, rc);
                    stack.push(c as u32);
                }
            }
        }
        self.work = stack;
    }
}

/// The order in which C's scan pushes `child` while it pops `u`: first the children joined by
/// an edge of the list it scans (`ND_in` for the in-search), in that list's order, then those
/// joined by a tree edge of the other direction, in the tree list's order.
fn push_rank<G: NsGraph + ?Sized>(g: &G, insearch: bool, u: usize, child: usize) -> (u8, usize) {
    let Some(pe) = g.par(child) else {
        debug_assert!(false, "a child has a parent edge");
        return (0, 0);
    };
    let index = |list: &[usize]| list.iter().position(|&x| x == pe).unwrap_or(0);
    if insearch {
        if g.head(pe) == u {
            (0, index(g.in_edges(u)))
        } else {
            (1, index(g.tree_out(u)))
        }
    } else if g.tail(pe) == u {
        (0, index(g.out_edges(u)))
    } else {
        (1, index(g.tree_in(u)))
    }
}

/// `enter_edge(e)`: on the cut sets while they run, else C's search. For a pivot (`pivot`,
/// as opposed to `LR_balance`'s queries), the pivot's check phase runs before the edge is
/// used. A [`Fallback`] gives the rest of the run to C's functions, and C's search picks the
/// edge: the cut sets' choice is not used once a check has failed.
pub(super) fn enter_edge<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    cut: &mut Option<CutStore>,
    e: usize,
    pivot: bool,
) -> Option<usize> {
    let Some(store) = cut.as_mut() else {
        return super::enter_edge(g, ctx, e);
    };
    match store.enter_edge(g, e, pivot) {
        Ok(f) => {
            check::enter_edge(g, ctx, store, e, f);
            f
        }
        Err(Fallback) => {
            fall_back(g, ctx, cut);
            super::enter_edge(g, ctx, e)
        }
    }
}

/// `update(e, f)`: on the cut sets while they run, else C's. `enter_edge` already ran this
/// pivot's check phase and nothing has changed since, so a [`Fallback`] here means `update`
/// was not called for the pivot `enter_edge` checked. Then C's functions take this pivot and
/// the rest of the run, with the entering edge picked again by C's search. `None`: C's search
/// found no entering edge.
pub(super) fn update<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    cut: &mut Option<CutStore>,
    e: usize,
    f: usize,
) -> Option<i32> {
    let Some(store) = cut.as_mut() else {
        return Some(super::update(g, ctx, e, f));
    };
    check::corrupt(
        store,
        check::Corrupt::Update,
        tree_slot(g, e).unwrap_or(usize::MAX),
        f,
    );
    match store.update(g, ctx, e, f) {
        Ok(r) => Some(r),
        Err(Fallback) => {
            fall_back(g, ctx, cut);
            check::count(&check::RESELECTIONS);
            let f = super::enter_edge(g, ctx, e)?;
            Some(super::update(g, ctx, e, f))
        }
    }
}

/// C: lib/common/ns.c:LR_balance
///
/// C's `LR_balance` with its `enter_edge` and its `rerank` (by half the entering slack) on
/// the cut sets while they run. The tree does not change here, so the cut sets stay valid.
pub(super) fn lr_balance<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    cut: &mut Option<CutStore>,
) {
    if cut.is_none() {
        super::lr_balance(g, ctx);
        return;
    }
    for i in 0..ctx.tree_edge.len() {
        let Some(&e) = ctx.tree_edge.get(i) else {
            break;
        };
        if g.cutvalue(e) == 0 {
            let Some(mut f) = enter_edge(g, ctx, cut, e, false) else {
                continue;
            };
            if let Some(store) = cut.as_mut() {
                check::corrupt(
                    store,
                    check::Corrupt::Balance,
                    tree_slot(g, e).unwrap_or(usize::MAX),
                    f,
                );
            }
            // f's slack from the cut sets; if f is not in e's, they are broken: C's functions
            // finish the run, starting with C's search for this entering edge
            let known = cut
                .as_ref()
                .and_then(|store| store.entering_slack(g, e, f).zip(tree_slot(g, e)));
            if cut.is_some() && known.is_none() {
                check::count(&check::BALANCE_FALLBACKS);
                fall_back(g, ctx, cut);
                let Some(c) = super::enter_edge(g, ctx, e) else {
                    continue;
                };
                f = c;
            }
            let delta = known.map_or_else(|| slack(g, f), |(d, _)| d);
            if delta <= 1 {
                continue;
            }
            let (t, h) = (g.tail(e), g.head(e));
            // `par` is right on either path; `lim`/`low` only on C's
            let tail_is_child = g.par(t) == Some(e);
            debug_assert!(cut.is_some() || tail_is_child == (g.lim(t) < g.lim(h)));
            let (node, d) = if tail_is_child {
                (t, delta.wrapping_div(2))
            } else {
                (h, delta.wrapping_neg().wrapping_div(2))
            };
            match (cut.as_mut(), known) {
                (Some(store), Some((_, s))) => {
                    store.shift(s, tail_is_child, d.wrapping_neg());
                    check::rerank(g, ctx, node, d);
                }
                _ => super::rerank(g, ctx, node, d),
            }
        }
    }
    end(g, cut);
    super::free_tree_list(g, ctx);
}

/// Ends the cut-set pivots: writes the ranks and gives the storage back to the graph.
pub(super) fn end<G: NsGraph + ?Sized>(g: &mut G, cut: &mut Option<CutStore>) {
    if let Some(mut store) = cut.take() {
        store.write_ranks(g);
        store.trim();
        g.keep_cut_store(store);
    }
}

/// Hands the rest of the run to C's functions: ranks written, `lim`/`low` renumbered from the
/// root (any valid numbering serves: C reads them only as subtree membership).
fn fall_back<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, cut: &mut Option<CutStore>) {
    check::fell_back();
    end(g, cut);
    if let Some(&root) = g.nlist().first() {
        dfs_range_impl(g, ctx, root, None, 1, false);
    }
}

#[cfg(any(test, feature = "ns-check"))]
mod check;

/// Check mode is off: these do nothing.
#[cfg(not(any(test, feature = "ns-check")))]
mod check {
    use super::{CutStore, NsGraph};
    use crate::common::ns::Ctx;

    pub(super) const ON: bool = false;
    pub(super) struct Rehung;
    pub(super) struct Counter;
    pub(super) static WHOLE_TREE: Counter = Counter;
    pub(super) static TIES: Counter = Counter;
    pub(super) static RELOCATIONS: Counter = Counter;
    pub(super) static COMPACTIONS: Counter = Counter;
    pub(super) static BALANCE_FALLBACKS: Counter = Counter;
    pub(super) static RESELECTIONS: Counter = Counter;

    pub(super) enum Corrupt {
        Enter,
        Update,
        Balance,
    }

    #[inline(always)]
    pub(super) fn corrupt(_: &mut CutStore, _: Corrupt, _: usize, _: usize) {}
    #[inline(always)]
    pub(super) fn budget(_: usize) -> Option<usize> {
        None
    }

    #[inline(always)]
    pub(super) fn count(_: &Counter) {}
    #[inline(always)]
    pub(super) fn fell_back() {}
    #[inline(always)]
    pub(super) fn enter_edge<G: NsGraph + ?Sized>(
        _: &mut G,
        _: &mut Ctx,
        _: &CutStore,
        _: usize,
        _: Option<usize>,
    ) {
    }
    #[inline(always)]
    pub(super) fn rerank<G: NsGraph + ?Sized>(_: &mut G, _: &mut Ctx, _: usize, _: i32) {}
    #[inline(always)]
    pub(super) fn path<G: NsGraph + ?Sized>(
        _: &mut G,
        _: &CutStore,
        _: usize,
        _: usize,
        _: usize,
    ) -> i32 {
        0
    }
    #[inline(always)]
    pub(super) fn rehung_start() -> Rehung {
        Rehung
    }
    #[inline(always)]
    pub(super) fn rehung_push(_: &mut Rehung, _: usize, _: usize) {}
    #[inline(always)]
    pub(super) fn dfs_range<G: NsGraph + ?Sized>(
        _: &mut G,
        _: &mut Ctx,
        _: usize,
        _: i32,
        _: &Rehung,
    ) {
    }
    #[inline(always)]
    pub(super) fn slacks<G: NsGraph + ?Sized>(_: &G, _: &CutStore, _: usize, _: usize) {}
    #[inline(always)]
    pub(super) fn rank<G: NsGraph + ?Sized>(_: &G, _: usize, _: i32) {}
}

#[cfg(test)]
mod tests;
