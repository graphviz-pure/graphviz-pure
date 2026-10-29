//! Check mode (`cfg(test)` or the `ns-check` feature): the cut-set pivots run beside the
//! functions that mirror `ns.c`, on the same graph, and every step asserts that both agree.
//!
//! C's `rerank` keeps the real ranks alongside the slacks, and C's `invalidate_path` and
//! `dfs_range` keep `lim`/`low` alongside the parent pointers, so C's own search can run at
//! every pivot. [`super::begin`] takes the cut-set path on every graph in this mode, whatever
//! the selector would say.

use super::{AGAINST, CutStore, FORWARD, ID, NsGraph};
use crate::common::ns::{Ctx, invalidate_path, seq, slack};

pub(super) const ON: bool = true;

thread_local! {
    /// How many runs fell back to C's functions, for the tests.
    pub(super) static FALLBACKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(super) fn fell_back() {
    FALLBACKS.with(|f| f.set(f.get() + 1));
}

/// Events the tests want to see happen at least once.
pub(super) type Counter = std::thread::LocalKey<std::cell::Cell<u32>>;

thread_local! {
    /// C's `rerank` from the root, which moves the whole tree (ns.c:697, `ND_par(root)` is NULL).
    pub(super) static WHOLE_TREE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Entering edges chosen among several of the same slack.
    pub(super) static TIES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(super) fn count(c: &'static Counter) {
    c.with(|n| n.set(n.get() + 1));
}

/// The parent pointers the cut-set pivot set, to compare with `dfs_range`'s.
pub(super) type Rehung = Vec<(usize, usize)>;

/// The cut set of `e` is exactly the non-tree edges crossing e's cut, with their direction;
/// its signed weights add up to e's cut value; every slack is C's; and the entering edge `f`
/// is the one C's search returns.
pub(super) fn enter_edge<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    store: &CutStore,
    e: usize,
    f: Option<usize>,
) {
    let (t, h) = (g.tail(e), g.head(e));
    let child = if g.lim(t) < g.lim(h) { t } else { h };
    assert_eq!(g.par(child), Some(e), "parent pointer of e's child");
    let (low, lim) = (g.low(child), g.lim(child));
    let set = store.edge(e);
    let mut members: Vec<(usize, bool)> = (0..set.len)
        .map(|i| store.entry((set.start + i) as usize))
        .map(|x| ((x & ID) as usize, x & AGAINST != 0))
        .collect();
    members.sort_unstable();
    let mut crossing = Vec::new();
    for &u in g.nlist() {
        for &x in g.out_edges(u) {
            if x == e || g.tree_index(x) >= 0 {
                continue;
            }
            let tail_in = seq(g, g.tail(x), low, lim);
            let head_in = seq(g, g.head(x), low, lim);
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
        assert_eq!(store.slack(x), slack(g, x), "slack of edge {x}");
        cutvalue = if against {
            cutvalue.wrapping_sub(g.weight(x))
        } else {
            cutvalue.wrapping_add(g.weight(x))
        };
    }
    assert_eq!(cutvalue, g.cutvalue(e), "cut value of tree edge {e}");
    let c = crate::common::ns::enter_edge(g, ctx, e);
    assert_eq!(f, c, "entering edge for leaving edge {e}");
}

/// C's `rerank`, so the real ranks stay current.
pub(super) fn rerank<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, node: usize, delta: i32) {
    crate::common::ns::rerank(g, ctx, node, delta);
}

/// f's tree cycle and LCA are what C's two `treeupdate` walks visit, in the same order and
/// with the same sign; then C's `invalidate_path`. Returns `low(lca)` for `dfs_range`.
pub(super) fn path<G: NsGraph + ?Sized>(
    g: &mut G,
    e: usize,
    f: usize,
    lca: usize,
    path: &[u32],
) -> i32 {
    let (ft, fh) = (g.tail(f), g.head(f));
    let mut walked = Vec::new();
    let mut tops = Vec::new();
    for (start, other, dir) in [(ft, fh, true), (fh, ft, false)] {
        let mut v = start;
        while !seq(g, other, g.low(v), g.lim(v)) {
            let t = g.par(v).expect("treeupdate climbs a parent edge");
            let d = if v == g.tail(t) { dir } else { !dir };
            // C adds cutvalue(e) when d; the cut-set path subtracts it when FORWARD
            walked.push(t as u32 | if d { 0 } else { FORWARD });
            v = if g.lim(g.tail(t)) > g.lim(g.head(t)) {
                g.tail(t)
            } else {
                g.head(t)
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
    let lca_low = g.low(lca);
    invalidate_path(g, lca, fh);
    invalidate_path(g, lca, ft);
    lca_low
}

pub(super) fn rehung_start() -> Rehung {
    Vec::new()
}

pub(super) fn rehung_push(rehung: &mut Rehung, u: usize, par: usize) {
    rehung.push((u, par));
}

/// C's `dfs_range` from the LCA; the parent pointers it assigns are the ones the cut-set
/// pivot re-hung.
pub(super) fn dfs_range<G: NsGraph + ?Sized>(
    g: &mut G,
    ctx: &mut Ctx,
    lca: usize,
    lca_low: i32,
    rehung: &Rehung,
) {
    let par = g.par(lca);
    crate::common::ns::dfs_range(g, ctx, lca, par, lca_low);
    for &(u, p) in rehung {
        assert_eq!(g.par(u), Some(p), "parent edge of node {u}");
    }
}

/// The rank written from the slacks is C's.
pub(super) fn rank<G: NsGraph + ?Sized>(g: &G, u: usize, r: i32) {
    assert_eq!(r, g.rank(u), "rank of node {u}");
}

/// After a pivot, every slack the cut sets keep is C's.
pub(super) fn slacks<G: NsGraph + ?Sized>(g: &G, store: &CutStore, e: usize, f: usize) {
    for &u in g.nlist() {
        for &x in g.out_edges(u) {
            assert_eq!(
                store.slack(x),
                slack(g, x),
                "slack of edge {x} (tree: {}) after pivot {e} -> {f}",
                g.tree_index(x) >= 0
            );
        }
    }
}
