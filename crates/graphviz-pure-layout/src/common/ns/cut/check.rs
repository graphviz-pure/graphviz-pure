//! Check mode (`cfg(test)` or the `ns-check` feature): the cut-set pivots run on the graph as
//! shipped, and beside them C's functions run on a copy of it, and every step asserts that
//! both agree.
//!
//! The graph keeps what production keeps: the cut sets, the slacks, and parent pointers
//! re-hung at each pivot. Its ranks and its `lim`/`low` stay as they were when the pivots
//! began, which is stale from the first pivot on. The copy ([`Reference`]) runs C's whole
//! `update` at every pivot, with C's `rerank`, `invalidate_path` and `dfs_range`, so its ranks
//! are real and its `lim`/`low` current, and C's own search runs on it. A cut-set search that
//! read a stale interval would therefore answer differently from C's, and the comparison
//! fails (`docs/perf/ns-cut-sets.md` §8). [`super::begin`] takes the cut-set path on every
//! graph in this mode, whatever the selector would say, stored and computed in turn.

use super::{AGAINST, CutStore, FORWARD, ID, Kind, NsGraph, tree_slot};
use crate::common::ns::{Ctx, NsData, NsEdge, NsNode, leave, seq, slack};

/// C's side of the comparison: a copy of the graph, taken when the pivots begin, on which
/// C's functions run at every pivot.
pub(super) struct Reference {
    g: NsData,
    ctx: Ctx,
}

impl Reference {
    fn copy<G: NsGraph + ?Sized>(g: &G, ctx: &Ctx) -> Self {
        let mut nodes = 0usize;
        let mut edges = 0usize;
        for &u in g.nlist() {
            nodes = nodes.max(u + 1);
            for &x in g.in_edges(u).iter().chain(g.out_edges(u)) {
                edges = edges.max(x + 1);
                nodes = nodes.max(g.tail(x) + 1).max(g.head(x) + 1);
            }
        }
        let mut copy = NsData {
            nodes: vec![NsNode::default(); nodes],
            edges: vec![NsEdge::default(); edges],
            nlist: g.nlist().to_vec(),
        };
        for &u in g.nlist() {
            if let Some(n) = copy.nodes.get_mut(u) {
                *n = NsNode {
                    rank: g.rank(u),
                    mark: false,
                    priority: g.priority(u),
                    low: g.low(u),
                    lim: g.lim(u),
                    par: g.par(u),
                    subtree: None,
                    in_: g.in_edges(u).to_vec(),
                    out: g.out_edges(u).to_vec(),
                    tree_in: g.tree_in(u).to_vec(),
                    tree_out: g.tree_out(u).to_vec(),
                };
            }
            for &x in g.in_edges(u).iter().chain(g.out_edges(u)) {
                if let Some(c) = copy.edges.get_mut(x) {
                    *c = NsEdge {
                        tail: g.tail(x),
                        head: g.head(x),
                        minlen: g.minlen(x),
                        weight: g.weight(x),
                        cutvalue: g.cutvalue(x),
                        tree_index: g.tree_index(x),
                    };
                }
            }
        }
        let tree_edge = ctx.tree_edge.clone();
        let neg = leave::NegSlots::build(&tree_edge, |e| g.cutvalue(e) < 0);
        Self {
            g: copy,
            ctx: Ctx {
                tree_edge,
                s_i: ctx.s_i,
                search_size: ctx.search_size,
                neg,
                subtrees: Vec::new(),
                enter_stack: Vec::new(),
                rerank_stack: Vec::new(),
                range_stack: Vec::new(),
            },
        }
    }
}

/// Starts the comparison: the copy of the graph that C's functions will run on. Not kept in
/// a test's shipped mode ([`SHIPPED`]), where nothing is compared.
pub(super) fn begin<G: NsGraph + ?Sized>(g: &G, ctx: &Ctx, store: &mut CutStore) {
    store.reference = (!shipped()).then(|| Box::new(Reference::copy(g, ctx)));
}

/// Ends it, when the ranks are written.
pub(super) fn finish(store: &mut CutStore) {
    store.reference = None;
}

/// A test's bug: the cut-set search takes e's child from `lim`, as C's does, instead of from
/// the parent pointers. `lim` is stale there, which only the comparison with C's search on
/// the renumbered copy can see.
pub(super) fn reads_numbering<G: NsGraph + ?Sized>(g: &G, e: usize) -> Option<bool> {
    READS_NUMBERING
        .with(std::cell::Cell::get)
        .then(|| g.lim(g.tail(e)) < g.lim(g.head(e)))
}

pub(super) const ON: bool = true;

/// Events the tests want to see happen at least once.
pub(super) type Counter = std::thread::LocalKey<std::cell::Cell<u32>>;

thread_local! {
    /// Runs that fell back to C's functions.
    pub(super) static FALLBACKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// C's `rerank` from the root, which moves the whole tree (ns.c:697, `ND_par(root)` is NULL).
    pub(super) static WHOLE_TREE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Candidates of the same slack, ordered by C's scan.
    pub(super) static TIES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Ties that the climbs could not settle within their budget.
    pub(super) static TIE_SCANS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Cut sets moved to a bigger segment.
    pub(super) static RELOCATIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Pool compactions.
    pub(super) static COMPACTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// `LR_balance` runs that went to C's functions when the walks passed their allowance.
    pub(super) static BALANCE_HANDED_OVER: std::cell::Cell<u32> =
        const { std::cell::Cell::new(0) };
    /// The tests' allowance for `LR_balance`, in nodes, in place of the shipped one.
    pub(super) static BALANCE_SPARE: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// `LR_balance` queries that found the cut sets broken.
    pub(super) static BALANCE_FALLBACKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Pivots whose entering edge C's search picked again after `update` found them broken.
    pub(super) static RESELECTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The tests' corruption: of which kind, and after how many more chances.
    pub(super) static CORRUPTION: std::cell::Cell<Option<(Corrupt, u32)>> =
        const { std::cell::Cell::new(None) };
    /// The tests' pool budget: this many words above what the build lays out.
    pub(super) static BUDGET_SLACK: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// The tests' budget for climbing between tied candidates, in place of `TIE_CLIMBS`.
    pub(super) static CLIMBS: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    /// A test's way to run as shipped: no copy is kept and nothing is compared, which is what
    /// [`super::fall_back`] has to repair the graph for.
    pub(super) static SHIPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The tests' injected bug: see [`reads_numbering`].
    pub(super) static READS_NUMBERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The tests' replay of the old check, which hid the injected bug: after each pivot the
    /// graph's own `lim`/`low` are copied from the copy's.
    pub(super) static RENUMBER_GRAPH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The tests' choice of cut sets; without one, runs take turns.
    pub(super) static KIND: std::cell::Cell<Option<Kind>> = const { std::cell::Cell::new(None) };
    static TURN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The tests' budget for the climbs, if they set one.
pub(super) fn climbs() -> Option<u32> {
    CLIMBS.with(std::cell::Cell::get)
}

/// The tests' allowance for `LR_balance`, if they set one.
pub(super) fn balance_spare() -> Option<usize> {
    BALANCE_SPARE.with(std::cell::Cell::get)
}

fn shipped() -> bool {
    SHIPPED.with(std::cell::Cell::get)
}

/// Which cut sets this run uses.
pub(super) fn kind() -> Kind {
    KIND.with(std::cell::Cell::get).unwrap_or_else(|| {
        let computed = TURN.with(|t| t.replace(!t.get()));
        if computed { Kind::Computed } else { Kind::Pool }
    })
}

/// The computed path's links are the tree of the parent pointers, with C's subtree sizes
/// (from the copy's `lim`/`low`: the graph's are stale).
pub(super) fn links(store: &CutStore) {
    if let (Kind::Computed, Some(r)) = (store.kind, &store.reference) {
        store.computed.assert_links(&r.g);
        assert_eq!(store.computed.cut_of, None, "a cut set outlived its tree");
    }
}

pub(super) fn count(c: &'static Counter) {
    c.with(|n| n.set(n.get() + 1));
}

pub(super) fn fell_back() {
    count(&FALLBACKS);
}

/// Where a test breaks the cut sets: after `enter_edge` picks a pivot's edge (before its check
/// phase), just before `update` (forgetting the checked pivot), or after an `LR_balance` query.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Corrupt {
    Enter,
    Update,
    Balance,
    /// The computed path's walk stops short of a node's children: not a fallback, the
    /// cross-check itself must see it.
    Links,
}

/// Takes `f` out of tree slot `s`'s cut set when the armed corruption of this kind is due.
pub(super) fn corrupt(store: &mut CutStore, kind: Corrupt, s: usize, f: usize) {
    let due = CORRUPTION.with(|c| match c.get() {
        Some((k, 0)) if k == kind => {
            c.set(None);
            true
        }
        Some((k, n)) if k == kind => {
            c.set(Some((k, n - 1)));
            false
        }
        _ => false,
    });
    if due {
        if kind == Corrupt::Update {
            store.prepared = None;
        }
        if kind == Corrupt::Links {
            store.computed.cut_children(s);
        } else {
            store.remove_member(s, f);
        }
    }
}

/// The pool budget in check mode: none, so that every graph runs on the cut sets as with the
/// selector, unless a test sets one.
pub(super) fn budget(size: usize) -> Option<usize> {
    Some(BUDGET_SLACK.with(|b| b.get().map_or(usize::MAX, |slack| size + slack)))
}

/// The parent pointers the cut-set pivot set, to compare with `dfs_range`'s.
pub(super) type Rehung = Vec<(usize, usize)>;

/// The cut set of `e` is exactly the non-tree edges crossing e's cut, with their direction;
/// its signed weights add up to e's cut value; every slack is C's; and the entering edge `f`
/// is the one C's search returns on the copy, whose numbering is current.
pub(super) fn enter_edge<G: NsGraph + ?Sized>(
    g: &mut G,
    store: &mut CutStore,
    e: usize,
    f: Option<usize>,
) {
    let Some(mut r) = store.reference.take() else {
        return;
    };
    let rg = &r.g;
    let (t, h) = (rg.tail(e), rg.head(e));
    let child = if rg.lim(t) < rg.lim(h) { t } else { h };
    assert_eq!(g.par(child), Some(e), "parent pointer of e's child");
    let (low, lim) = (rg.low(child), rg.lim(child));
    let s = tree_slot(g, e).expect("a tree edge has a tree index");
    let set = store.tree(s);
    assert_eq!(
        store.edge(set.edge).edge as usize,
        e,
        "tree slot of edge {e}"
    );
    let mut members: Vec<(usize, bool)> = (0..store.cut_len(s))
        .map(|i| store.cut_entry(s, i))
        .map(|x| (store.edge(x & ID).edge as usize, x & AGAINST != 0))
        .collect();
    members.sort_unstable();
    let mut crossing = Vec::new();
    for &u in rg.nlist() {
        for &x in rg.out_edges(u) {
            if rg.tree_index(x) >= 0 {
                continue;
            }
            let tail_in = seq(rg, rg.tail(x), low, lim);
            let head_in = seq(rg, rg.head(x), low, lim);
            if tail_in != head_in {
                // against e: tail(x) on head(e)'s side
                let tail_side_of_e = tail_in == (child == t);
                crossing.push((x, !tail_side_of_e));
            }
        }
    }
    crossing.sort_unstable();
    assert_eq!(members, crossing, "cut set of tree edge {e}");
    let mut cutvalue = g.weight(e);
    for &(x, against) in &members {
        cutvalue = if against {
            cutvalue.wrapping_sub(g.weight(x))
        } else {
            cutvalue.wrapping_add(g.weight(x))
        };
    }
    assert_eq!(cutvalue, g.cutvalue(e), "cut value of tree edge {e}");
    for (k, es) in store.edges.iter().enumerate() {
        let x = es.edge as usize;
        assert_eq!(es.slack, slack(rg, x), "slack of edge {x} (local {k})");
    }
    let c = crate::common::ns::enter_edge(&r.g, &mut r.ctx, e);
    assert_eq!(f, c, "entering edge for leaving edge {e}");
    store.reference = Some(r);
}

/// C's `rerank` on the copy, so that its ranks stay real (`LR_balance`; a pivot's is part of
/// [`pivot`]).
pub(super) fn rerank(store: &mut CutStore, node: usize, delta: i32) {
    if let Some(r) = store.reference.as_mut() {
        crate::common::ns::rerank(&mut r.g, &mut r.ctx, node, delta);
    }
}

/// f's tree cycle (in `store.work`, as tree slots) and LCA are what C's two `treeupdate`
/// walks visit on the copy, in the same order and with the same sign.
pub(super) fn path(store: &CutStore, e: usize, f: usize, lca: usize) {
    let Some(r) = &store.reference else {
        return;
    };
    let rg = &r.g;
    let (ft, fh) = (rg.tail(f), rg.head(f));
    let path: Vec<u32> = store
        .work
        .iter()
        .map(|&step| {
            let t = store.edge(store.tree((step & ID) as usize).edge).edge;
            t | (step & FORWARD)
        })
        .collect();
    let mut walked = Vec::new();
    let mut tops = Vec::new();
    for (start, other, dir) in [(ft, fh, true), (fh, ft, false)] {
        let mut v = start;
        while !seq(rg, other, rg.low(v), rg.lim(v)) {
            let t = rg.par(v).expect("treeupdate climbs a parent edge");
            let d = if v == rg.tail(t) { dir } else { !dir };
            // C adds cutvalue(e) when d; the cut-set path subtracts it when FORWARD
            walked.push(t as u32 | if d { 0 } else { FORWARD });
            v = if rg.lim(rg.tail(t)) > rg.lim(rg.head(t)) {
                rg.tail(t)
            } else {
                rg.head(t)
            };
        }
        tops.push(v);
    }
    assert_eq!(tops, [lca, lca], "LCA of the entering edge {f}");
    assert_eq!(walked, path, "tree cycle of the entering edge {f}");
    assert!(
        path.contains(&(e as u32 | FORWARD)),
        "the entering edge crosses e against it"
    );
}

pub(super) fn rehung_start() -> Rehung {
    Vec::new()
}

pub(super) fn rehung_push(rehung: &mut Rehung, u: usize, par: usize) {
    rehung.push((u, par));
}

/// C's whole `update(e, f)` on the copy (`rerank`, `treeupdate`, `invalidate_path`,
/// `exchange_tree_edges` and `dfs_range`), after which the graph holds what C's pivot leaves
/// in everything but its ranks and its numbering: the parent pointers the cut-set pivot
/// re-hung (`rehung`), every node's parent and tree lists, every edge's cut value and tree
/// index, and the tree-edge list.
pub(super) fn pivot<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &Ctx,
    store: &mut CutStore,
    e: usize,
    f: usize,
    rehung: &Rehung,
) {
    let Some(r) = store.reference.as_mut() else {
        return;
    };
    let err = crate::common::ns::update(&mut r.g, &mut r.ctx, e, f);
    assert_eq!(err, 0, "C's update for pivot {e} -> {f}");
    let rg = &r.g;
    for &(u, p) in rehung {
        assert_eq!(g.par(u), Some(p), "parent edge of node {u}");
        assert_eq!(rg.par(u), Some(p), "C's parent edge of node {u}");
    }
    for &u in g.nlist() {
        let at = format!("node {u} after pivot {e} -> {f}");
        assert_eq!(g.par(u), rg.par(u), "parent edge of {at}");
        assert_eq!(g.tree_in(u), rg.tree_in(u), "tree in-edges of {at}");
        assert_eq!(g.tree_out(u), rg.tree_out(u), "tree out-edges of {at}");
        for &x in g.out_edges(u) {
            let at = format!("edge {x} after pivot {e} -> {f}");
            assert_eq!(g.cutvalue(x), rg.cutvalue(x), "cut value of {at}");
            assert_eq!(g.tree_index(x), rg.tree_index(x), "tree index of {at}");
        }
    }
    assert_eq!(
        ctx.tree_edge, r.ctx.tree_edge,
        "tree edges after pivot {e} -> {f}"
    );
    if RENUMBER_GRAPH.with(std::cell::Cell::get) {
        for &u in rg.nlist() {
            g.set_low(u, rg.low(u));
            g.set_lim(u, rg.lim(u));
        }
    }
}

/// After a pivot, every slack is C's (the copy's), and every tree slot holds its tree edge.
pub(super) fn slacks<G: NsGraph + ?Sized>(g: &G, store: &CutStore, e: usize, f: usize) {
    let Some(r) = &store.reference else {
        return;
    };
    for es in &store.edges {
        let x = es.edge as usize;
        assert_eq!(
            es.slack,
            slack(&r.g, x),
            "slack of edge {x} after pivot {e} -> {f}"
        );
    }
    for (s, ts) in store.tree.iter().enumerate() {
        let t = store.edge(ts.edge).edge as usize;
        assert_eq!(
            g.tree_index(t),
            s as i32,
            "tree slot {s} after pivot {e} -> {f}"
        );
    }
}

/// The rank written from the slacks is C's (the copy's).
pub(super) fn rank(store: &CutStore, u: usize, r: i32) {
    if let Some(c) = &store.reference {
        assert_eq!(r, c.g.rank(u), "rank of node {u}");
    }
}
