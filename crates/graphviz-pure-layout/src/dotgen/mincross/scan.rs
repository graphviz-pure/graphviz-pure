//! Exact shortcuts for `reorder` and `transpose`.
//!
//! The functions in `mincross.rs` mirror `mincross.c` and stay the reference. The code here
//! computes what they compute with less work: `settle` writes the order that `reorder`'s bubble
//! passes end in without running them, and `scan_rank` remembers each adjacent pair's crossing
//! counts until an exchange can change them. Both make the same exchanges, read the same counts
//! and leave the same state. `docs/perf/mincross-scans.md` is the argument. Under `cfg(test)` or
//! the `mincross-check` feature, every use also runs the reference and compares (`check.rs`).

use super::*;
use std::cmp::Ordering;

/// Scratch space of the shortcuts; capacity is reused between components and clusters. Its size
/// is fixed by the ranks before any scan runs (`reserve`) and no scan makes it grow: the memo
/// is one record per rank position, not a table keyed by pair, so it forgets a pair's counts
/// when the pair stops being adjacent.
#[derive(Default)]
pub(super) struct ScanStore {
    items: Vec<Medianed>,
    /// The target positions of `items`, then the tree and the waits of `rotate_ties`.
    work: Vec<u32>,
    ranks: Vec<ScanRank>,
    pairs: Vec<RankPair>,
    #[cfg(any(test, feature = "mincross-check"))]
    pub(super) reads: check::Reads,
    #[cfg(any(test, feature = "mincross-check"))]
    pub(super) largest: check::Largest,
    /// A test's way to break `forget_pairs`, proving that the cross-check sees it.
    #[cfg(test)]
    pub(super) keep_stale_pairs: bool,
    /// A test's way to break `rotate_ties`, proving that the cross-check sees it.
    #[cfg(test)]
    pub(super) keep_ties: bool,
    /// A test's way to skip `windows_hold_their_orders`, proving that the check is needed.
    #[cfg(test)]
    pub(super) ignore_stale_ties: bool,
}

impl ScanStore {
    /// Room for a view of `rows` ranks and `positions` rank positions, the widest with `widest`.
    pub(super) fn reserve(&mut self, rows: usize, positions: usize, widest: usize) {
        #[cfg(any(test, feature = "mincross-check"))]
        self.largest.note(rows, positions, widest);
        // Cleared first: `reserve_exact` counts from the length, and the last view's records would
        // otherwise add to this one's capacity.
        self.items.clear();
        self.work.clear();
        self.ranks.clear();
        self.pairs.clear();
        self.items.reserve_exact(widest);
        self.work
            .reserve_exact(widest.saturating_mul(3).saturating_add(1));
        self.ranks.reserve_exact(rows);
        self.pairs.reserve_exact(positions);
    }
}

/// A node with a nonnegative median, in `reorder`'s subsequence.
#[derive(Clone, Copy)]
struct Medianed {
    mval: f64,
    /// Position in the subsequence before the passes.
    seq: u32,
    slot: u32,
}

/// One position of a rank: the node there and the counts of the pair it starts.
#[derive(Clone, Copy)]
struct RankPair {
    slot: u32,
    /// `c0` and `c1` are what C's `transpose_step` would compute now.
    known: bool,
    c0: i64,
    c1: i64,
}

/// The transpose being run.
#[derive(Clone, Copy)]
struct ScanView {
    g: G,
    reverse: bool,
    minr: i32,
    maxr: i32,
}

#[derive(Clone, Copy, Default)]
struct ScanRank {
    /// Index of the rank's first position in `pairs`.
    start: usize,
    n: i32,
    /// `ND_order` of the rank's first position: a cluster's rank is a window of the root's.
    base: i32,
    root: RankWindow,
    has_out: bool,
    moved: bool,
}

/// `p1 > p2`, then rank order. No two items compare equal, so an unstable sort gives the stable
/// result without allocating. The caller has excluded NaN; -0.0 and 0.0 tie, as they do in C.
#[inline]
fn by_mval(a: &Medianed, b: &Medianed) -> Ordering {
    if a.mval < b.mval {
        Ordering::Less
    } else if a.mval > b.mval {
        Ordering::Greater
    } else {
        a.seq.cmp(&b.seq)
    }
}

/// How many items with a smaller `seq` are in the tree (a Fenwick tree over `seq`).
#[inline]
fn tree_before(tree: &[u32], seq: u32) -> u32 {
    let mut i = seq as usize;
    let mut total = 0u32;
    while let Some(&count) = tree.get(i).filter(|_| i > 0) {
        total = total.wrapping_add(count);
        i &= i.wrapping_sub(1);
    }
    total
}

#[inline]
fn tree_add(tree: &mut [u32], seq: u32) {
    let mut i = (seq as usize).wrapping_add(1);
    while let Some(count) = tree.get_mut(i) {
        *count = count.wrapping_add(1);
        i = i.wrapping_add(i & i.wrapping_neg());
    }
}

#[inline]
fn rotate_prefix(class: &mut [Medianed], active: usize, passes: u32) {
    if let Some(prefix) = class.get_mut(..active)
        && let Some(by) = (passes as usize).checked_rem(active)
    {
        prefix.rotate_left(by);
    }
}

/// The order within one class of equal medians after the passes. `waits[i]` is the pass from
/// which `class[i]` has no larger median before it; from then on it is active. Each pass moves
/// the first active item behind the other active ones: a left rotation of the active prefix.
fn rotate_class(class: &mut [Medianed], waits: &[u32], passes: u32) {
    let mut time = 0u32;
    let mut active = 0usize;
    while let Some(&next) = waits.get(active) {
        rotate_prefix(class, active, next.wrapping_sub(time));
        time = next;
        while waits.get(active) == Some(&time) {
            active = active.wrapping_add(1);
        }
    }
    rotate_prefix(class, active, passes.wrapping_sub(time));
}

/// `items` holds the settled values, each class in rank order. Puts each class in the order the
/// `>=` passes leave it, walking from the largest median down so that the tree holds exactly the
/// strictly larger items.
fn rotate_ties(items: &mut [Medianed], passes: u32, work: &mut [u32]) {
    let (tree, waits) = work.split_at_mut(items.len().wrapping_add(1).min(work.len()));
    tree.fill(0);
    let mut rest = items;
    while let Some(mval) = rest.last().map(|i| i.mval) {
        let start = rest
            .iter()
            .rposition(|i| i.mval != mval)
            .map_or(0, |p| p.wrapping_add(1));
        let (head, class) = rest.split_at_mut(start);
        if class.len() > 1 {
            for (item, wait) in class.iter().zip(waits.iter_mut()) {
                *wait = tree_before(tree, item.seq);
            }
            if let Some(waits) = waits.get(..class.len()) {
                rotate_class(class, waits, passes);
            }
        }
        for item in class.iter() {
            tree_add(tree, item.seq);
        }
        rest = head;
    }
}

/// C: lib/dotgen/mincross.c:reorder, on the nodes with a nonnegative median, in rank order, when
/// no pair is constrained. Leaves `items` in the order those nodes have after the passes, and
/// returns whether any pass exchanged anything. `passes` is the rank's size.
fn settle(items: &mut [Medianed], passes: u32, reverse: bool, work: &mut Vec<u32>) -> bool {
    let changed = items
        .iter()
        .zip(items.iter().skip(1))
        .any(|(a, b)| a.mval > b.mval || (reverse && a.mval >= b.mval));
    if !changed {
        return false;
    }
    items.sort_unstable_by(by_mval);
    if reverse
        && items
            .iter()
            .zip(items.iter().skip(1))
            .any(|(a, b)| a.mval == b.mval)
    {
        // The caller's target positions stay in front of the scratch space.
        let (kept, len) = (work.len(), items.len());
        work.resize(kept.wrapping_add(len.wrapping_mul(2)).wrapping_add(1), 0);
        if let Some(scratch) = work.get_mut(kept..) {
            rotate_ties(items, passes, scratch);
        }
        work.truncate(kept);
    }
    true
}

impl Dot<'_> {
    /// C: lib/dotgen/mincross.c:reorder, when the rank's view has no constraint. Returns false
    /// if the rank needs C's loop instead: its orders are not a permutation of the window, or a
    /// median is NaN.
    pub(super) fn reorder_settled(
        &mut self,
        g: G,
        r: i32,
        reverse: bool,
    ) -> Result<bool, &'static str> {
        let mut scan = std::mem::take(&mut self.mincross_store.scan);
        let result = self.reorder_with(g, r, reverse, &mut scan);
        self.mincross_store.scan = scan;
        result
    }

    fn reorder_with(
        &mut self,
        g: G,
        r: i32,
        reverse: bool,
        scan: &mut ScanStore,
    ) -> Result<bool, &'static str> {
        let rank = self.mincross_rank(g, r);
        if rank.n <= 0 {
            return Ok(false);
        }
        let Some(root) = self.positioned_rank(g, r, rank) else {
            return Ok(false);
        };
        let Some(base) = rank
            .offset
            .checked_sub(root.offset)
            .and_then(|o| i32::try_from(o).ok())
        else {
            return Ok(false);
        };
        scan.items.clear();
        scan.work.clear();
        for i in 0..rank.n {
            let Some(slot) = rank.node(self, i).and_then(|n| self.mincross_slot(n)) else {
                return Ok(false);
            };
            let Some(&mval) = self.mincross_store.mval.get(slot) else {
                return Ok(false);
            };
            if mval.is_nan() {
                return Ok(false);
            }
            if mval >= 0.0 {
                scan.items.push(Medianed {
                    mval,
                    seq: scan.items.len() as u32,
                    slot: slot as u32,
                });
                scan.work.push(base.wrapping_add(i) as u32);
            }
        }
        if !settle(&mut scan.items, rank.n as u32, reverse, &mut scan.work) {
            return Ok(true);
        }
        #[cfg(test)]
        if scan.keep_ties {
            scan.items.sort_unstable_by(by_mval);
        }
        let store = &mut self.mincross_store;
        let buf = self
            .bufs
            .get_mut(root.buf)
            .ok_or("mincross: missing exchange buffer")?;
        for (item, &place) in scan.items.iter().zip(&scan.work) {
            let slot = item.slot as usize;
            *store
                .order
                .get_mut(slot)
                .ok_or("mincross: missing exchange order")? = place as i32;
            let &node = store.nodes.get(slot).ok_or("mincross: missing node")?;
            *buf.get_mut(root.offset.saturating_add(place as usize))
                .ok_or("mincross: missing exchange slot")? = Some(node);
        }
        self.write_crossing_orders(rank)?;
        self.invalidate_crossing_rank(g, r, false);
        self.rk_mut(0, r).valid = false;
        if r > 0 {
            self.rk_mut(0, r.wrapping_sub(1)).valid = false;
        }
        Ok(true)
    }

    /// C: lib/dotgen/mincross.c:transpose, after it set the candidates, when every rank of g is
    /// a validated permutation of its window whose edges all reach the next ranks. Returns false
    /// if some rank is not, and C's loop must run: nothing has changed then.
    pub(super) fn transpose_scans(&mut self, g: G, reverse: bool) -> Result<bool, &'static str> {
        if self.mincross_store.graph != Some(g) {
            return Ok(false);
        }
        let Some(graph) = self.graphs.get(g) else {
            return Ok(false);
        };
        let view = ScanView {
            g,
            reverse,
            minr: graph.minrank,
            maxr: graph.maxrank,
        };
        let mut scan = std::mem::take(&mut self.mincross_store.scan);
        if !self.build_scan(view, &mut scan) {
            self.mincross_store.scan = scan;
            return Ok(false);
        }
        // Nothing below reads the store through `self`.
        let mut store = std::mem::take(&mut self.mincross_store);
        let result = if store.constrained {
            self.run_scans::<true>(view, &mut store, &mut scan)
        } else {
            self.run_scans::<false>(view, &mut store, &mut scan)
        };
        store.scan = scan;
        self.mincross_store = store;
        result.map(|()| true)
    }

    fn build_scan(&mut self, view: ScanView, scan: &mut ScanStore) -> bool {
        let g = view.g;
        scan.ranks.clear();
        scan.pairs.clear();
        for r in view.minr..=view.maxr {
            let rank = self.mincross_rank(g, r);
            let mut info = ScanRank {
                start: scan.pairs.len(),
                n: rank.n,
                has_out: self.rank_n(g, r.wrapping_add(1)) > 0,
                ..ScanRank::default()
            };
            if rank.n > 0 {
                let Some(root) = self.positioned_rank(g, r, rank) else {
                    return false;
                };
                let Some(base) = rank
                    .offset
                    .checked_sub(root.offset)
                    .and_then(|o| i32::try_from(o).ok())
                else {
                    return false;
                };
                info.base = base;
                info.root = root;
                for i in 0..rank.n {
                    let Some(slot) = rank.node(self, i).and_then(|n| self.mincross_slot(n)) else {
                        return false;
                    };
                    let Some(meta) = self.mincross_store.meta.get(slot) else {
                        return false;
                    };
                    if meta.rank != r || !meta.snapshot_valid || !meta.snapshot_adjacent {
                        return false;
                    }
                    scan.pairs.push(RankPair {
                        slot: slot as u32,
                        known: false,
                        c0: 0,
                        c1: 0,
                    });
                }
            }
            scan.ranks.push(info);
        }
        #[cfg(test)]
        if scan.ignore_stale_ties {
            return true;
        }
        self.windows_hold_their_orders(view, scan)
    }

    /// (W) of docs/perf/mincross-scans.md §3.1: a node of the view that is not in its rank's
    /// window has an order outside that window. A cluster's rank is a window of the root's, and
    /// a neighbour outside it can hold a stale order. If that order tied with a window node, an
    /// exchange could change a comparison `forget_pairs` does not look at.
    fn windows_hold_their_orders(&self, view: ScanView, scan: &ScanStore) -> bool {
        let store = &self.mincross_store;
        let mut slots = store.meta.iter().zip(&store.order).enumerate();
        slots.all(|(slot, (meta, &order))| {
            let row = meta
                .rank
                .checked_sub(view.minr)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| scan.ranks.get(i));
            let Some(info) = row else {
                return true;
            };
            let place = order.wrapping_sub(info.base);
            place < 0
                || place >= info.n
                || scan
                    .pairs
                    .get(info.start.wrapping_add(place as usize))
                    .is_some_and(|pair| pair.slot as usize == slot)
        })
    }

    fn run_scans<const CONSTRAINED: bool>(
        &mut self,
        view: ScanView,
        store: &mut MinCrossStore,
        scan: &mut ScanStore,
    ) -> Result<(), &'static str> {
        loop {
            let mut delta: i64 = 0;
            for r in view.minr..=view.maxr {
                if self.rk(view.g, r).candidate {
                    let rv = self.scan_rank::<CONSTRAINED>(view, r, store, scan)?;
                    delta = delta.wrapping_add(rv);
                }
            }
            if delta < 1 {
                break;
            }
        }
        self.publish_scans(view, store, scan)
    }

    /// The exchanges so far live in the store and in `pairs`: nothing reads the arena orders or
    /// the rank buffers during a transpose, so they are written once, here. The crossing copies
    /// were not kept up to date either, so no rank's snapshot stands any more.
    fn publish_scans(
        &mut self,
        view: ScanView,
        store: &MinCrossStore,
        scan: &ScanStore,
    ) -> Result<(), &'static str> {
        let mut any = false;
        for info in scan.ranks.iter().filter(|info| info.moved) {
            any = true;
            let pairs = scan
                .pairs
                .get(info.start..info.start.saturating_add(info.n as usize))
                .ok_or("mincross: missing scan rank")?;
            let at = info.root.offset.saturating_add(info.base as usize);
            let window = self
                .bufs
                .get_mut(info.root.buf)
                .and_then(|b| b.get_mut(at..at.saturating_add(info.n as usize)))
                .ok_or("mincross: missing exchange slot")?;
            for (pair, cell) in pairs.iter().zip(window) {
                *cell = store.nodes.get(pair.slot as usize).copied();
            }
            for (place, pair) in (info.base..).zip(pairs) {
                let &node = store
                    .nodes
                    .get(pair.slot as usize)
                    .ok_or("mincross: missing node")?;
                self.nodes
                    .get_mut(node)
                    .ok_or("mincross: missing node")?
                    .order = place;
            }
        }
        if any {
            for r in view.minr..=view.maxr {
                self.rk_mut(view.g, r).mincross_snapshot = false;
            }
        }
        Ok(())
    }

    /// C: lib/dotgen/mincross.c:transpose_step. A scan's exchanges only read and write the
    /// store; the flags, which nothing reads until the scan ends, are set at its end.
    fn scan_rank<const CONSTRAINED: bool>(
        &mut self,
        view: ScanView,
        r: i32,
        store: &mut MinCrossStore,
        scan: &mut ScanStore,
    ) -> Result<i64, &'static str> {
        let ScanView {
            g,
            reverse,
            minr,
            maxr,
        } = view;
        self.rk_mut(g, r).candidate = false;
        let index = r.wrapping_sub(minr) as usize;
        let info = *scan.ranks.get(index).ok_or("mincross: missing scan rank")?;
        let last = info.n.wrapping_sub(1);
        if last <= 0 {
            return Ok(0);
        }
        let mut rv: i64 = 0;
        let mut exchanged = false;
        for i in 0..last {
            let at = info.start.wrapping_add(i as usize);
            let (Some(&pair), Some(&next)) =
                (scan.pairs.get(at), scan.pairs.get(at.wrapping_add(1)))
            else {
                return Err("mincross: missing scan rank");
            };
            let (v, w) = (pair.slot as usize, next.slot as usize);
            if CONSTRAINED
                && let (Some(mv), Some(mw)) = (store.meta.get(v), store.meta.get(w))
                && left2right(self, g, mv, mw)
            {
                continue;
            }
            let (c0, c1) = if pair.known {
                (pair.c0, pair.c1)
            } else {
                let mut c0: i64 = 0;
                let mut c1: i64 = 0;
                if r > 0 {
                    let (a, b) = scan_counts(store, v, w, false)?;
                    c0 = c0.wrapping_add(a);
                    c1 = c1.wrapping_add(b);
                }
                if info.has_out {
                    let (a, b) = scan_counts(store, v, w, true)?;
                    c0 = c0.wrapping_add(a);
                    c1 = c1.wrapping_add(b);
                }
                (c0, c1)
            };
            #[cfg(any(test, feature = "mincross-check"))]
            scan.reads.compare(check::Read { r, i, c0, c1 })?;
            if c1 < c0 || (c0 > 0 && reverse && c1 == c0) {
                let place = info.base.wrapping_add(i);
                *store.order.get_mut(v).ok_or("mincross: missing order")? = place.wrapping_add(1);
                *store.order.get_mut(w).ok_or("mincross: missing order")? = place;
                // The neighbours' orders did not move, so the exchanged pair reads the
                // same two counts the other way round.
                if let Some(cell) = scan.pairs.get_mut(at) {
                    *cell = RankPair {
                        slot: next.slot,
                        known: true,
                        c0: c1,
                        c1: c0,
                    };
                }
                if let Some(cell) = scan.pairs.get_mut(at.wrapping_add(1)) {
                    cell.slot = pair.slot;
                    cell.known = false;
                }
                if i > 0
                    && let Some(cell) = scan.pairs.get_mut(at.wrapping_sub(1))
                {
                    cell.known = false;
                }
                let (Some(Some([vin, vout])), Some(Some([win, wout]))) =
                    (store.adjacency.get(v), store.adjacency.get(w))
                else {
                    return Err("mincross: missing adjacency");
                };
                #[cfg(test)]
                let forget = !scan.keep_stale_pairs;
                #[cfg(not(test))]
                let forget = true;
                if forget && r > minr {
                    forget_pairs(store, scan, index.wrapping_sub(1), *vin, *win);
                }
                if forget && r < maxr {
                    forget_pairs(store, scan, index.wrapping_add(1), *vout, *wout);
                }
                rv = rv.wrapping_add(c0.wrapping_sub(c1));
                exchanged = true;
            } else if !pair.known
                && let Some(cell) = scan.pairs.get_mut(at)
            {
                cell.known = true;
                cell.c0 = c0;
                cell.c1 = c1;
            }
        }
        if exchanged {
            if let Some(info) = scan.ranks.get_mut(index) {
                info.moved = true;
            }
            self.rk_mut(0, r).valid = false;
            self.rk_mut(g, r).candidate = true;
            if r > minr {
                let above = r.wrapping_sub(1);
                self.rk_mut(0, above).valid = false;
                self.rk_mut(g, above).candidate = true;
            }
            if r < maxr {
                let below = r.wrapping_add(1);
                self.rk_mut(0, below).valid = false;
                self.rk_mut(g, below).candidate = true;
            }
        }
        Ok(rv)
    }
}

/// After v and w exchange, the only comparisons that change are between an edge at v and an edge
/// at w. So a pair of a neighbouring rank keeps its counts unless one of its nodes has an edge
/// to v and the other an edge to w. Such a pair sits at two adjacent positions.
#[inline]
fn forget_pairs(
    store: &MinCrossStore,
    scan: &mut ScanStore,
    rank: usize,
    v: CrossingSpan,
    w: CrossingSpan,
) {
    let Some(&ScanRank { start, n, base, .. }) = scan.ranks.get(rank) else {
        return;
    };
    let ((a, b), (c, d)) = (v.range(), w.range());
    let (Some(xs), Some(ys)) = (store.edges.get(a..b), store.edges.get(c..d)) else {
        return;
    };
    for x in xs {
        let Some(&ox) = store.order.get(x.other) else {
            continue;
        };
        for y in ys {
            let Some(&oy) = store.order.get(y.other) else {
                continue;
            };
            let first = match ox.wrapping_sub(oy) {
                1 => oy,
                -1 => ox,
                _ => continue,
            };
            // Outside the window nothing is remembered.
            let i = first.wrapping_sub(base);
            if i >= 0
                && i < n.wrapping_sub(1)
                && let Some(cell) = scan.pairs.get_mut(start.wrapping_add(i as usize))
            {
                cell.known = false;
            }
        }
    }
}

/// A singleton's payload with its remote order read now. The store's copy is not kept current.
#[inline(always)]
fn live(
    store: &MinCrossStore,
    index: usize,
    value: ResolvedCrossing,
) -> Result<ResolvedCrossing, &'static str> {
    let edge = store
        .edges
        .get(index)
        .ok_or("mincross: missing singleton edge")?;
    let order = *store
        .order
        .get(edge.other)
        .ok_or("mincross: missing order")?;
    Ok(ResolvedCrossing { order, ..value })
}

/// Brings one node's crossing orders up to date, for the loops that stream them as slices.
#[inline]
fn refresh(store: &mut MinCrossStore, start: usize, end: usize) -> Result<(), &'static str> {
    let edges = store
        .edges
        .get(start..end)
        .ok_or("mincross: missing adjacency range")?;
    let orders = store
        .crossings
        .order
        .get_mut(start..end)
        .ok_or("mincross: missing crossing order")?;
    for (edge, order) in edges.iter().zip(orders) {
        *order = *store
            .order
            .get(edge.other)
            .ok_or("mincross: missing order")?;
    }
    Ok(())
}

/// C's `in_cross` or `out_cross` of (v, w) and of (w, v), with the same functions as the
/// reference and the same inputs, read from the live orders.
#[inline(always)]
fn scan_counts(
    store: &mut MinCrossStore,
    v: usize,
    w: usize,
    out: bool,
) -> Result<(i64, i64), &'static str> {
    #[inline(always)]
    fn count<
        V: Iterator<Item = ResolvedCrossing> + Clone,
        W: Iterator<Item = ResolvedCrossing> + Clone,
    >(
        v: V,
        w: W,
        out: bool,
    ) -> (i64, i64) {
        if out {
            (out_cross(v.clone(), w.clone()), out_cross(w, v))
        } else {
            (in_cross(v.clone(), w.clone()), in_cross(w, v))
        }
    }
    let span = |store: &MinCrossStore, slot: usize| {
        store
            .adjacency
            .get(slot)
            .and_then(|a| a.as_ref())
            .and_then(|a| a.get(usize::from(out)).copied())
            .ok_or("mincross: missing adjacency")
    };
    let (vs, ws) = (span(store, v)?, span(store, w)?);
    Ok(match (vs, ws) {
        (CrossingSpan::Empty, _) | (_, CrossingSpan::Empty) => (0, 0),
        (CrossingSpan::One { index: a, value: x }, CrossingSpan::One { index: b, value: y }) => {
            let (x, y) = (live(store, a, x)?, live(store, b, y)?);
            count(std::iter::once(x), std::iter::once(y), out)
        }
        (CrossingSpan::One { index, value }, CrossingSpan::Many { start, end }) => {
            let x = live(store, index, value)?;
            refresh(store, start, end)?;
            count(
                std::iter::once(x),
                store.crossings.view(start, end)?.iter(),
                out,
            )
        }
        (CrossingSpan::Many { start, end }, CrossingSpan::One { index, value }) => {
            let y = live(store, index, value)?;
            refresh(store, start, end)?;
            count(
                store.crossings.view(start, end)?.iter(),
                std::iter::once(y),
                out,
            )
        }
        (CrossingSpan::Many { start: a, end: b }, CrossingSpan::Many { start: c, end: d }) => {
            refresh(store, a, b)?;
            refresh(store, c, d)?;
            Dot::count_pair(
                store.crossings.view(a, b)?,
                store.crossings.view(c, d)?,
                out,
            )
        }
    })
}

#[cfg(any(test, feature = "mincross-check"))]
pub(super) mod check;

/// Check mode off: the hooks in `mincross.rs` compile to nothing.
#[cfg(not(any(test, feature = "mincross-check")))]
pub(super) mod check {
    use super::*;

    pub(in super::super) struct Before;

    #[inline(always)]
    pub(in super::super) fn before_reorder(_: &Dot<'_>, _: G) -> Before {
        Before
    }

    #[inline(always)]
    pub(in super::super) fn after_reorder(
        _: &mut Dot<'_>,
        _: Before,
        _: G,
        _: i32,
        _: bool,
        _: bool,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    #[inline(always)]
    pub(in super::super) fn before_transpose(_: &mut Dot<'_>, _: G, _: bool) -> Before {
        Before
    }

    #[inline(always)]
    pub(in super::super) fn after_transpose(_: &mut Dot<'_>, _: Before, _: G) {}

    #[inline(always)]
    pub(in super::super) fn before_fallback(_: &mut Dot<'_>) {}
}

#[cfg(test)]
mod tests;
