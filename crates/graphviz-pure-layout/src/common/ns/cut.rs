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
//! them; that order is recomputed from the parent pointers, and only for tied candidates. A
//! pivot then costs about |cut set of e| × |tree cycle of f| list operations.
//!
//! The proof that this picks the same entering edge at every pivot and leaves the same ranks,
//! cut values, tree lists and parent pointers is `docs/perf/ns-cut-sets.md`; each function
//! below names the invariant it keeps. [`begin`] picks this path per graph from a sampled cost
//! estimate. Where C's scan is not an argmin (a candidate with negative slack, only after
//! wrapped arithmetic), the rest of the run goes back to the functions that mirror `ns.c`.
//! Under `cfg(test)` or the `ns-check` feature, every pivot also runs C's search, `rerank`,
//! `treeupdate` and `dfs_range`, and asserts that both agree ([`check`]).

use super::{Ctx, NsGraph, dfs_range_impl, exchange_tree_edges, seq, slack, tree_edge};

/// Selector: graphs with fewer nodes than this keep C's search. Their subtree walks are short,
/// and building the cut sets would cost more than it saves.
pub(super) const CUT_SETS_MIN_NODES: usize = 1000;

/// Selector: the largest estimated cost ratio, (mean tree-cycle length × mean cut-set size) /
/// node count, for which the cut sets are faster than C's walks. Tuned on the corpus
/// (`docs/perf/ns-cut-sets.md`, "The selector").
const CUT_SETS_MAX_COST: f64 = 2.0;

/// Selector: how many non-tree edges are sampled to estimate the mean tree-cycle length.
const CUT_SETS_SAMPLE: usize = 64;

/// Cut-set entry flag: the edge crosses from head(t)'s side of the cut to tail(t)'s side.
const AGAINST: u32 = 0x8000_0000;
/// Tree-path entry flag: the cycle of `f`, oriented along `f`, runs this tree edge forwards.
const FORWARD: u32 = 0x8000_0000;
/// The edge index in an entry.
const ID: u32 = 0x7fff_ffff;

/// Per-edge state, indexed by edge.
#[derive(Clone, Copy, Default)]
struct EdgeCut {
    /// `rank(head) - rank(tail) - minlen`, wrapping, as C would compute it now.
    slack: i32,
    /// Position of the edge in `ND_in(head)` and `ND_out(tail)`.
    in_pos: u32,
    out_pos: u32,
    /// While a cut set is edited: `stamp` equals the edit's stamp when the edge is in it, at
    /// index `slot`.
    stamp: u32,
    slot: u32,
    /// For a tree edge, its cut set: `pool[start..start + len]`, with room for `cap`.
    start: u32,
    len: u32,
    cap: u32,
}

/// Reusable storage for the cut-set pivots. [`NsGraph::take_cut_store`] hands it to
/// [`rank2`](super::rank2), which gives it back when the pivots end.
#[derive(Default)]
pub struct CutStore {
    edges: Vec<EdgeCut>,
    /// Per node: the LCA walk's marks.
    marks: Vec<u32>,
    /// Every cut set, each a segment (`EdgeCut::start`).
    pool: Vec<u32>,
    /// Pool entries no cut set uses since it moved to a bigger segment.
    garbage: usize,
    /// The tree cycle of the entering edge: edge | [`FORWARD`].
    path: Vec<u32>,
    /// The tied candidates; also the stack of [`CutStore::write_ranks`].
    list: Vec<u32>,
    /// Both ancestor paths of a tie-break.
    walk: Vec<u32>,
    mark_gen: u32,
    stamp_gen: u32,
    /// The root's rank: what it was when the pivots began, moved by C's `rerank` of the
    /// whole tree (from the root, ns.c:697). The other ranks follow from the slacks.
    root_rank: i32,
}

/// The candidates include a negative slack: C's pruned scan is not an argmin there.
struct Fallback;

fn other<G: NsGraph + ?Sized>(g: &G, e: usize, u: usize) -> usize {
    if g.tail(e) == u { g.head(e) } else { g.tail(e) }
}

fn parent<G: NsGraph + ?Sized>(g: &G, u: usize) -> Option<usize> {
    g.par(u).map(|e| other(g, e, u))
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

/// The non-tree edges, in `ND_out` order of the nodes in `GD_nlist` order.
fn nontree_edges<G: NsGraph + ?Sized>(g: &G) -> impl Iterator<Item = usize> + '_ {
    g.nlist()
        .iter()
        .flat_map(move |&u| g.out_edges(u).iter().copied())
        .filter(move |&e| !tree_edge(g, e))
}

/// Whether the cut sets are expected to be faster than C's walks on this graph: the mean
/// tree-cycle length `l` is estimated from up to [`CUT_SETS_SAMPLE`] non-tree edges, the
/// cut sets then hold about `l × m` entries for `m` non-tree edges, and a pivot costs about
/// `l × (l × m / n)` against C's walks of a fraction of `n`.
fn worth_it<G: NsGraph + ?Sized>(g: &G) -> bool {
    let n = g.nlist().len();
    if n < CUT_SETS_MIN_NODES {
        return false;
    }
    let m = nontree_edges(g).count();
    if m == 0 {
        return false;
    }
    let stride = m.wrapping_div(CUT_SETS_SAMPLE).max(1);
    let mut sampled = 0usize;
    let mut length = 0usize;
    for f in nontree_edges(g).step_by(stride).take(CUT_SETS_SAMPLE) {
        if !tree_path(g, f, |_, _| length = length.wrapping_add(1)) {
            return false;
        }
        sampled = sampled.wrapping_add(1);
    }
    let l = length as f64 / sampled.max(1) as f64;
    let cost = l * (l * m as f64 / n as f64) / n as f64;
    cost <= CUT_SETS_MAX_COST
}

/// Starts the cut-set pivots after `feasible_tree` (`lim`/`low` valid), when [`worth_it`]
/// (always under check mode); `None` keeps C's functions.
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
    fn edge(&self, e: usize) -> EdgeCut {
        self.edges.get(e).copied().unwrap_or_default()
    }

    fn edge_mut(&mut self, e: usize) -> Option<&mut EdgeCut> {
        self.edges.get_mut(e)
    }

    fn slack(&self, e: usize) -> i32 {
        self.edge(e).slack
    }

    fn entry(&self, i: usize) -> u32 {
        self.pool.get(i).copied().unwrap_or(0)
    }

    fn set_entry(&mut self, i: usize, x: u32) {
        if let Some(slot) = self.pool.get_mut(i) {
            *slot = x;
        } else {
            debug_assert!(false, "cut-set entry out of the pool");
        }
    }

    /// The cut sets, slacks and edge positions of the current tree. `false` (C's functions
    /// then run) when an index does not fit an entry or the tree disagrees with `lim`/`low`.
    fn build<G: NsGraph + ?Sized>(&mut self, g: &G, ctx: &Ctx) -> bool {
        let nl = g.nlist();
        let nodes = nl.iter().copied().max().map_or(0, |m| m.wrapping_add(1));
        let mut edges = 0usize;
        for &u in nl {
            for &e in g.out_edges(u) {
                edges = edges.max(e.wrapping_add(1));
            }
        }
        if edges > ID as usize || nodes > u32::MAX as usize {
            return false;
        }
        self.edges.clear();
        self.edges.resize(edges, EdgeCut::default());
        self.marks.clear();
        self.marks.resize(nodes, 0);
        self.mark_gen = 0;
        self.stamp_gen = 0;
        self.root_rank = nl.first().map_or(0, |&r| g.rank(r));
        self.garbage = 0;
        for &u in nl {
            for (i, &e) in g.in_edges(u).iter().enumerate() {
                if let Some(ec) = self.edges.get_mut(e) {
                    ec.in_pos = i as u32;
                }
            }
            for (i, &e) in g.out_edges(u).iter().enumerate() {
                let s = slack(g, e);
                if let Some(ec) = self.edges.get_mut(e) {
                    ec.out_pos = i as u32;
                    ec.slack = s;
                }
            }
        }
        // count, lay the segments out with room to grow, then fill
        for f in nontree_edges(g) {
            let edges = &mut self.edges;
            let walked = tree_path(g, f, |t, _| {
                if let Some(ec) = edges.get_mut(t) {
                    ec.len = ec.len.wrapping_add(1);
                }
            });
            if !walked {
                return false;
            }
        }
        let mut size = 0u32;
        for &t in &ctx.tree_edge {
            let Some(ec) = self.edges.get_mut(t) else {
                return false;
            };
            ec.start = size;
            ec.cap = ec
                .len
                .saturating_add(ec.len.wrapping_div(2))
                .saturating_add(2);
            ec.len = 0;
            let Some(next) = size.checked_add(ec.cap) else {
                return false;
            };
            size = next;
        }
        self.pool.clear();
        self.pool.resize(size as usize, 0);
        for f in nontree_edges(g) {
            let id = f as u32;
            let pool = &mut self.pool;
            let edges = &mut self.edges;
            tree_path(g, f, |t, against| {
                if let Some(ec) = edges.get_mut(t)
                    && ec.len < ec.cap
                    && let Some(slot) = pool.get_mut(ec.start.wrapping_add(ec.len) as usize)
                {
                    *slot = id | if against { AGAINST } else { 0 };
                    ec.len = ec.len.wrapping_add(1);
                }
            });
        }
        true
    }

    /// C: lib/common/ns.c:enter_edge, dfs_enter_outedge, dfs_enter_inedge
    ///
    /// C's candidates are the non-tree edges crossing e's cut against `e` (from head(e)'s
    /// side to tail(e)'s side): the `AGAINST` members of e's cut set. C keeps the first of
    /// minimum slack in its scan order, which [`CutStore::scanned_before`] recomputes for
    /// ties. That is C's answer only while no candidate has a negative slack: below 0, C's
    /// `Slack > 0` pruning cuts its scan short before the minimum ([`Fallback`]).
    fn enter_edge<G: NsGraph + ?Sized>(
        &mut self,
        g: &G,
        e: usize,
    ) -> Result<Option<usize>, Fallback> {
        let tail_is_child = g.par(g.tail(e)) == Some(e);
        let v = if tail_is_child { g.tail(e) } else { g.head(e) };
        let set = self.edge(e);
        let mut tied = std::mem::take(&mut self.list);
        tied.clear();
        let mut best = i32::MAX;
        for i in 0..set.len {
            let x = self.entry(set.start.wrapping_add(i) as usize);
            if x & AGAINST == 0 {
                continue;
            }
            let f = x & ID;
            let s = self.slack(f as usize);
            if s < 0 {
                self.list = tied;
                return Err(Fallback);
            }
            if tied.is_empty() || s < best {
                best = s;
                tied.clear();
                tied.push(f);
            } else if s == best {
                tied.push(f);
            }
        }
        let mut win = tied.first().copied();
        if tied.len() > 1 {
            check::count(&check::TIES);
        }
        for &c in tied.iter().skip(1) {
            if let Some(w) = win
                && self.scanned_before(g, tail_is_child, v, c as usize, w as usize)
            {
                win = Some(c);
            }
        }
        self.list = tied;
        Ok(win.map(|w| w as usize))
    }

    /// Whether candidate `c` comes before `w` in C's scan of the subtree under `v`.
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
        c: usize,
        w: usize,
    ) -> bool {
        let end = |e: usize| if insearch { g.head(e) } else { g.tail(e) };
        let (sc, sw) = (end(c), end(w));
        if sc == sw {
            let (pc, pw) = (self.edge(c), self.edge(w));
            return if insearch {
                pc.in_pos < pw.in_pos
            } else {
                pc.out_pos < pw.out_pos
            };
        }
        // both ancestor paths up to v, one after the other
        let mut walk = std::mem::take(&mut self.walk);
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
        // drop the common part from v down; what is left starts below the LCA
        let common = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let below = |p: &[u32]| {
            p.len()
                .checked_sub(common)
                .and_then(|k| k.checked_sub(1))
                .and_then(|k| p.get(k).copied())
        };
        let before = match (below(a), below(b)) {
            // c's end is an ancestor of w's: pre-order
            (None, _) => true,
            (_, None) => false,
            (Some(ca), Some(cb)) => {
                let lca = a
                    .len()
                    .checked_sub(common)
                    .and_then(|k| a.get(k))
                    .copied()
                    .unwrap_or_default() as usize;
                self.push_rank(g, insearch, lca, ca as usize)
                    > self.push_rank(g, insearch, lca, cb as usize)
            }
        };
        self.walk = walk;
        before
    }

    /// The order in which C's scan pushes `child` while it pops `u`.
    fn push_rank<G: NsGraph + ?Sized>(
        &self,
        g: &G,
        insearch: bool,
        u: usize,
        child: usize,
    ) -> (u32, u32) {
        let Some(pe) = g.par(child) else {
            debug_assert!(false, "a child has a parent edge");
            return (0, 0);
        };
        let ec = self.edge(pe);
        let index = |list: &[usize]| list.iter().position(|&x| x == pe).unwrap_or(0) as u32;
        if insearch {
            if g.head(pe) == u {
                (0, ec.in_pos)
            } else {
                (1, index(g.tree_out(u)))
            }
        } else if g.tail(pe) == u {
            (0, ec.out_pos)
        } else {
            (1, index(g.tree_in(u)))
        }
    }

    /// C's `rerank` of e's child side by `-shift`, on the slacks: only `e` and the edges in
    /// e's cut set have exactly one end on that side.
    fn shift(&mut self, e: usize, tail_is_child: bool, shift: i32) {
        // slack = rank(head) - rank(tail) - minlen
        let moved = |s: i32, tail_inside: bool| {
            if tail_inside {
                s.wrapping_sub(shift)
            } else {
                s.wrapping_add(shift)
            }
        };
        let set = self.edge(e);
        for i in 0..set.len {
            let x = self.entry(set.start.wrapping_add(i) as usize);
            // AGAINST clear: tail(g) is on tail(e)'s side, which moves when tail(e) is the child
            let tail_inside = (x & AGAINST == 0) == tail_is_child;
            if let Some(ec) = self.edge_mut((x & ID) as usize) {
                ec.slack = moved(ec.slack, tail_inside);
            }
        }
        if let Some(ec) = self.edge_mut(e) {
            ec.slack = moved(ec.slack, tail_is_child);
        }
    }

    /// The LCA of `a` and `b`: both climb one step at a time, marking, until one reaches a
    /// node the other marked. Costs at most twice the longer of the two paths.
    fn lca<G: NsGraph + ?Sized>(&mut self, g: &G, a: usize, b: usize) -> Option<usize> {
        if a == b {
            return Some(a);
        }
        if self.mark_gen >= u32::MAX.wrapping_sub(2) {
            self.marks.fill(0);
            self.mark_gen = 0;
        }
        self.mark_gen = self.mark_gen.wrapping_add(2);
        let (ma, mb) = (self.mark_gen, self.mark_gen.wrapping_add(1));
        let mut mark = |u: usize, m: u32| {
            if let Some(x) = self.marks.get_mut(u) {
                *x = m;
            }
        };
        mark(a, ma);
        mark(b, mb);
        let (mut x, mut y) = (Some(a), Some(b));
        while x.is_some() || y.is_some() {
            for (cur, mine, theirs) in [(&mut x, ma, mb), (&mut y, mb, ma)] {
                let Some(u) = *cur else {
                    continue;
                };
                *cur = parent(g, u);
                if let Some(p) = *cur {
                    match self.marks.get_mut(p) {
                        Some(m) if *m == theirs => return Some(p),
                        Some(m) => *m = mine,
                        None => return None,
                    }
                }
            }
        }
        None
    }

    fn next_stamp(&mut self) -> u32 {
        if self.stamp_gen == u32::MAX {
            for ec in &mut self.edges {
                ec.stamp = 0;
            }
            self.stamp_gen = 0;
        }
        self.stamp_gen = self.stamp_gen.wrapping_add(1);
        self.stamp_gen
    }

    /// Appends `x` to t's cut set, moving the set to the end of the pool when it is full.
    fn push(&mut self, t: usize, x: u32) {
        let mut set = self.edge(t);
        if set.len >= set.cap {
            let cap = set.len.saturating_mul(2).saturating_add(4);
            let (from, to) = (set.start as usize, set.start.wrapping_add(set.len) as usize);
            let start = self.pool.len();
            if to > start || start.saturating_add(cap as usize) > ID as usize {
                debug_assert!(false, "cut-set pool out of range");
                return;
            }
            self.pool.extend_from_within(from..to);
            self.pool.resize(start.wrapping_add(cap as usize), 0);
            self.garbage = self.garbage.wrapping_add(set.cap as usize);
            set.start = start as u32;
            set.cap = cap;
        }
        self.set_entry(set.start.wrapping_add(set.len) as usize, x);
        if let Some(ec) = self.edge_mut((x & ID) as usize) {
            ec.slot = set.len;
        }
        set.len = set.len.wrapping_add(1);
        if let Some(ec) = self.edge_mut(t) {
            ec.start = set.start;
            ec.len = set.len;
            ec.cap = set.cap;
        }
    }

    /// Removes the entry at index `i` of t's cut set; the last entry takes its place.
    fn remove_at(&mut self, t: usize, i: u32) {
        let set = self.edge(t);
        let Some(last) = set.len.checked_sub(1) else {
            debug_assert!(false, "removal from an empty cut set");
            return;
        };
        let moved = self.entry(set.start.wrapping_add(last) as usize);
        self.set_entry(set.start.wrapping_add(i) as usize, moved);
        if let Some(ec) = self.edge_mut((moved & ID) as usize) {
            ec.slot = i;
        }
        if let Some(ec) = self.edge_mut(t) {
            ec.len = last;
        }
    }

    /// The cut sets after `e` leaves the tree and `f` enters it.
    ///
    /// A non-tree edge's fundamental cycle changes only when it contains `e`, that is for the
    /// members of e's cut set. For such a `g`, the new cycle is the old one minus the cycle of
    /// `f` (as oriented edge sums), so its membership toggles on every tree edge `t` of f's
    /// cycle except `e`; where it joins, it crosses t's cut against `t` exactly when its
    /// crossing of `e` and f's orientation of `t` (`FORWARD`) differ. `f` leaves those cut
    /// sets and `e` joins them (against `t` where `FORWARD`). The cut of `f` is the cut `e`
    /// had, so f's set is e's, with every orientation flipped and `f` replaced by `e`
    /// (against `f`); it takes over e's segment.
    fn move_cut_sets(&mut self, e: usize, f: usize) {
        let set_e = self.edge(e);
        for k in 0..self.path.len() {
            let Some(&p) = self.path.get(k) else {
                break;
            };
            let t = (p & ID) as usize;
            let forward = p & FORWARD != 0;
            if t == e {
                continue;
            }
            // stamp t's members, with their index
            let stamp = self.next_stamp();
            let set_t = self.edge(t);
            for i in 0..set_t.len {
                let x = self.entry(set_t.start.wrapping_add(i) as usize);
                if let Some(ec) = self.edge_mut((x & ID) as usize) {
                    ec.stamp = stamp;
                    ec.slot = i;
                }
            }
            for i in 0..set_e.len {
                let x = self.entry(set_e.start.wrapping_add(i) as usize);
                let id = x & ID;
                if id as usize == f {
                    continue;
                }
                let member = self.edge(id as usize);
                if member.stamp == stamp {
                    self.remove_at(t, member.slot);
                } else {
                    let against = (x & AGAINST != 0) != forward;
                    self.push(t, id | if against { AGAINST } else { 0 });
                }
            }
            let ef = self.edge(f);
            if ef.stamp == stamp {
                self.remove_at(t, ef.slot);
            } else {
                debug_assert!(false, "f is in the cut set of every edge of its cycle");
            }
            self.push(t, e as u32 | if forward { AGAINST } else { 0 });
        }
        // f's set: e's segment, orientations flipped, f replaced by e
        let mut found = false;
        for i in 0..set_e.len {
            let at = set_e.start.wrapping_add(i) as usize;
            let x = self.entry(at);
            if (x & ID) as usize == f {
                self.set_entry(at, e as u32 | AGAINST);
                found = true;
            } else {
                self.set_entry(at, x ^ AGAINST);
            }
        }
        debug_assert!(found, "f is in e's cut set");
        if let Some(ec) = self.edge_mut(f) {
            ec.start = set_e.start;
            ec.len = set_e.len;
            ec.cap = set_e.cap;
        }
        if let Some(ec) = self.edge_mut(e) {
            ec.start = 0;
            ec.len = 0;
            ec.cap = 0;
        }
    }

    /// Packs the cut sets to the front of the pool once more than half of it is garbage.
    fn compact(&mut self, tree_edges: &[usize]) {
        let used = self.pool.len().saturating_sub(self.garbage);
        if self.garbage <= used.saturating_add(4096) {
            return;
        }
        let mut order = std::mem::take(&mut self.list);
        order.clear();
        order.extend(tree_edges.iter().map(|&t| t as u32));
        order.sort_unstable_by_key(|&t| self.edge(t as usize).start);
        let mut to = 0u32;
        for &t in &order {
            let set = self.edge(t as usize);
            let from = set.start as usize;
            // in start order, to <= from: every earlier set moved down to its length
            self.pool
                .copy_within(from..from.wrapping_add(set.len as usize), to as usize);
            if let Some(ec) = self.edge_mut(t as usize) {
                ec.start = to;
                ec.cap = set.len;
            }
            to = to.wrapping_add(set.len);
        }
        self.pool.truncate(to as usize);
        self.garbage = 0;
        self.list = order;
    }

    /// C: lib/common/ns.c:update, treeupdate, rerank, exchange_tree_edges, dfs_range
    ///
    /// C's `update` on the cut sets: the same `rerank` choice applied to the slacks (or, for
    /// the whole tree, to the root's rank), the same cut-value changes along f's tree cycle, the
    /// same `exchange_tree_edges`. Instead of `dfs_range`, only the parent pointers on the path
    /// from f's end in the moved subtree up to e's child are reversed, which is what
    /// `dfs_range` assigns them.
    fn update<G: NsGraph + ?Sized>(&mut self, g: &mut G, ctx: &mut Ctx, e: usize, f: usize) -> i32 {
        let delta = self.slack(f);
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
                self.shift(e, tail_is_child, d.wrapping_neg());
            }
            check::rerank(g, ctx, node, d);
        }
        let (ft, fh) = (g.tail(f), g.head(f));
        let Some(lca) = self.lca(g, ft, fh) else {
            graphviz_pure_core::diag::agerrorf("update: mismatched lca in treeupdates\n");
            return 2;
        };
        // f's tree cycle, oriented along f: from head(f) up to the LCA, then down to tail(f)
        self.path.clear();
        for (start, down) in [(ft, true), (fh, false)] {
            let mut u = start;
            while u != lca {
                let Some(t) = g.par(u) else {
                    break;
                };
                let forward = if down { g.head(t) == u } else { g.tail(t) == u };
                self.path.push(t as u32 | if forward { FORWARD } else { 0 });
                u = other(g, t, u);
            }
        }
        let lca_low = check::path(g, e, f, lca, &self.path);
        // treeupdate: each edge of the cycle gains cutvalue(e) when the cycle runs it
        // backwards and loses it when forwards
        let cutvalue = g.cutvalue(e);
        for &p in &self.path {
            let t = (p & ID) as usize;
            let cv = g.cutvalue(t);
            let next = if p & FORWARD != 0 {
                cv.wrapping_sub(cutvalue)
            } else {
                cv.wrapping_add(cutvalue)
            };
            g.set_cutvalue(t, next);
        }
        self.move_cut_sets(e, f);
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
        self.compact(&ctx.tree_edge);
        0
    }

    /// Writes every rank: the root's, plus `minlen + slack` down each tree edge.
    fn write_ranks<G: NsGraph + ?Sized>(&mut self, g: &mut G) {
        let Some(&root) = g.nlist().first() else {
            return;
        };
        let r = self.root_rank;
        check::rank(g, root, r);
        g.set_rank(root, r);
        let mut stack = std::mem::take(&mut self.list);
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
                    let rc = ru.wrapping_add(g.minlen(t)).wrapping_add(self.slack(t));
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
                    let rc = ru.wrapping_sub(g.minlen(t)).wrapping_sub(self.slack(t));
                    check::rank(g, c, rc);
                    g.set_rank(c, rc);
                    stack.push(c as u32);
                }
            }
        }
        self.list = stack;
    }
}

/// `enter_edge(e)`: on the cut sets while they run, else C's search. A [`Fallback`] gives
/// the rest of the run to C's functions.
pub(super) fn enter_edge<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    cut: &mut Option<CutStore>,
    e: usize,
) -> Option<usize> {
    let Some(store) = cut.as_mut() else {
        return super::enter_edge(g, ctx, e);
    };
    match store.enter_edge(g, e) {
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

/// `update(e, f)`: on the cut sets while they run, else C's.
pub(super) fn update<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    cut: &mut Option<CutStore>,
    e: usize,
    f: usize,
) -> i32 {
    match cut.as_mut() {
        Some(store) => store.update(g, ctx, e, f),
        None => super::update(g, ctx, e, f),
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
            let Some(f) = enter_edge(g, ctx, cut, e) else {
                continue;
            };
            let delta = match cut.as_ref() {
                Some(store) => store.slack(f),
                None => slack(g, f),
            };
            if delta <= 1 {
                continue;
            }
            let (t, h) = (g.tail(e), g.head(e));
            let tail_is_child = match cut {
                Some(_) => g.par(t) == Some(e),
                None => g.lim(t) < g.lim(h),
            };
            let (node, d) = if tail_is_child {
                (t, delta.wrapping_div(2))
            } else {
                (h, delta.wrapping_neg().wrapping_div(2))
            };
            match cut.as_mut() {
                Some(store) => {
                    store.shift(e, tail_is_child, d.wrapping_neg());
                    check::rerank(g, ctx, node, d);
                }
                None => super::rerank(g, ctx, node, d),
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

    #[inline(always)]
    pub(super) fn fell_back() {}
    pub(super) struct Counter;
    pub(super) static WHOLE_TREE: Counter = Counter;
    pub(super) static TIES: Counter = Counter;
    #[inline(always)]
    pub(super) fn count(_: &Counter) {}
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
        _: usize,
        _: usize,
        _: usize,
        _: &[u32],
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
    pub(super) fn rank<G: NsGraph + ?Sized>(_: &G, _: usize, _: i32) {}
    #[inline(always)]
    pub(super) fn slacks<G: NsGraph + ?Sized>(_: &G, _: &CutStore, _: usize, _: usize) {}
}

#[cfg(test)]
mod tests;
