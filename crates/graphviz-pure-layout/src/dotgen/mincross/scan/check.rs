//! Check mode: every shortcut runs beside the reference and must leave the same state, having
//! read the same counts in the same order. Off in shipped builds; it costs C's full work on top.
//!
//! A transpose runs C's loop first and logs what it reads. The shortcut then compares each read
//! as it makes it, so a wrong count stops the run at once, where a wrong memo might otherwise
//! keep `transpose` from ever converging. Counts that wrap can keep C's own loop running, with
//! no end to its log: past `READS_MAX` reads both stop with `NOT_CONVERGED`, compared that far.

use super::super::*;

/// One comparison of `transpose_step`: the pair at position `i` of rank `r`, and its counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct Read {
    pub(in super::super) r: i32,
    pub(in super::super) i: i32,
    pub(in super::super) c0: i64,
    pub(in super::super) c1: i64,
}

/// The most reads one transpose may log: 24 B each, so 400 MB in check mode and 6 MB in a unit
/// test, whose graphs are small and whose endless runs should end soon.
#[cfg(not(test))]
const READS_MAX: usize = 1 << 24;
#[cfg(test)]
const READS_MAX: usize = 1 << 18;

/// What a transpose returns in check mode when it reaches `READS_MAX`.
pub(in super::super) const NOT_CONVERGED: &str = "mincross: transpose did not converge";

#[derive(Clone, Copy, Default, PartialEq)]
enum ReadMode {
    #[default]
    Off,
    Recording,
    Comparing,
}

/// The sizes `reserve` was asked for, largest first: what the scratch arrays may hold.
#[derive(Clone, Copy, Default)]
pub(in super::super) struct Largest {
    rows: usize,
    positions: usize,
    widest: usize,
}

impl Largest {
    pub(in super::super) fn note(&mut self, rows: usize, positions: usize, widest: usize) {
        self.rows = self.rows.max(rows);
        self.positions = self.positions.max(positions);
        self.widest = self.widest.max(widest);
    }
}

#[derive(Default)]
pub(in super::super) struct Reads {
    mode: ReadMode,
    log: Vec<Read>,
    next: usize,
}

impl Reads {
    /// From C's loop.
    #[inline]
    pub(in super::super) fn record(&mut self, read: Read) -> Result<(), &'static str> {
        if self.mode != ReadMode::Recording {
            return Ok(());
        }
        if self.log.len() >= READS_MAX {
            return Err(NOT_CONVERGED);
        }
        self.log.push(read);
        Ok(())
    }

    /// From the shortcut.
    #[inline]
    pub(in super::super) fn compare(&mut self, read: Read) -> Result<(), &'static str> {
        if self.mode != ReadMode::Comparing {
            return Ok(());
        }
        match self.log.get(self.next) {
            Some(expected) => assert_eq!(
                read, *expected,
                "transpose: counts differ from C's loop at read {}",
                self.next
            ),
            None if self.log.len() >= READS_MAX => return Err(NOT_CONVERGED),
            None => panic!(
                "transpose: counts differ from C's loop, which stopped after {} reads",
                self.next
            ),
        }
        self.next += 1;
        Ok(())
    }
}

/// Everything `reorder` and `transpose` can change, over the ranks of one view.
#[derive(Clone, PartialEq, Debug)]
pub(in super::super) struct Visible {
    /// A rank's buffer window, and its `valid` (root), `candidate` and `cache_nc` flags.
    ranks: Vec<(Vec<Option<N>>, bool, bool, i64)>,
    /// Per slot: compact order, arena order, compact median, arena median.
    slots: Vec<(i32, i32, u64, u64)>,
}

fn visible(d: &Dot<'_>, g: G) -> Visible {
    let (minr, maxr) = (d.graphs[g].minrank, d.graphs[g].maxrank);
    let ranks = (minr..=maxr)
        .map(|r| {
            let rank = d.mincross_rank(g, r);
            let window = (0..rank.n).map(|i| rank.node(d, i)).collect();
            (
                window,
                d.rk(0, r).valid,
                d.rk(g, r).candidate,
                d.rk(0, r).cache_nc,
            )
        })
        .collect();
    let store = &d.mincross_store;
    let slots = store
        .nodes
        .iter()
        .zip(store.order.iter().zip(&store.mval))
        .map(|(&n, (&order, &mval))| {
            let node = &d.nodes[n];
            (order, node.order, mval.to_bits(), node.mval.to_bits())
        })
        .collect();
    Visible { ranks, slots }
}

/// What the reference needs back to start from the same point. Its crossing copies count: the
/// shortcut refreshes some of them and a rank's snapshot flag vouches for all of them.
pub(in super::super) struct Before {
    visible: Visible,
    flags: Vec<(bool, Option<bool>)>,
    crossing_orders: Vec<i32>,
    adjacency: Vec<Option<[CrossingSpan; 2]>>,
}

fn before(d: &Dot<'_>, g: G) -> Before {
    let (minr, maxr) = (d.graphs[g].minrank, d.graphs[g].maxrank);
    Before {
        visible: visible(d, g),
        flags: (minr..=maxr)
            .map(|r| (d.rk(g, r).mincross_snapshot, d.rk(g, r).mincross_position))
            .collect(),
        crossing_orders: d.mincross_store.crossings.order.clone(),
        adjacency: d.mincross_store.adjacency.clone(),
    }
}

fn restore(d: &mut Dot<'_>, g: G, before: Before) {
    let (minr, maxr) = (d.graphs[g].minrank, d.graphs[g].maxrank);
    for ((r, (window, valid, candidate, cache)), (snapshot, position)) in
        (minr..=maxr).zip(before.visible.ranks).zip(before.flags)
    {
        let rank = d.mincross_rank(g, r);
        for (i, node) in window.into_iter().enumerate() {
            d.bufs[rank.buf][rank.offset + i] = node;
        }
        let root = d.rk_mut(0, r);
        root.valid = valid;
        root.cache_nc = cache;
        let row = d.rk_mut(g, r);
        row.candidate = candidate;
        row.mincross_snapshot = snapshot;
        row.mincross_position = position;
    }
    let nodes = std::mem::take(&mut d.mincross_store.nodes);
    for (slot, (&n, (order, arena, mval, arena_mval))) in
        nodes.iter().zip(before.visible.slots).enumerate()
    {
        d.mincross_store.order[slot] = order;
        d.mincross_store.mval[slot] = f64::from_bits(mval);
        d.nodes[n].order = arena;
        d.nodes[n].mval = f64::from_bits(arena_mval);
    }
    d.mincross_store.nodes = nodes;
    d.mincross_store.crossings.order = before.crossing_orders;
    d.mincross_store.adjacency = before.adjacency;
}

/// A snapshot flag that stands vouches for the rank's copies: the reference trusts them.
fn assert_snapshots_stand(d: &Dot<'_>, g: G) {
    let (minr, maxr) = (d.graphs[g].minrank, d.graphs[g].maxrank);
    for r in minr..=maxr {
        if !d.rk(g, r).mincross_snapshot {
            continue;
        }
        let rank = d.mincross_rank(g, r);
        for i in 0..rank.n {
            let v = d.mc_rank_node(rank.node(d, i).unwrap());
            for span in d.mincross_store.adjacency[v.slot].unwrap() {
                let (start, end) = span.range();
                for at in start..end {
                    let live = d.mincross_store.order[d.mincross_store.edges[at].other];
                    match span {
                        CrossingSpan::One { value, .. } => assert_eq!(value.order, live),
                        _ => assert_eq!(d.mincross_store.crossings.order[at], live),
                    }
                }
            }
        }
    }
}

#[inline]
pub(in super::super) fn before_reorder(d: &Dot<'_>, g: G) -> Before {
    before(d, g)
}

/// `reorder_settled` ran: compare with C's full loop, every constraint test included.
pub(in super::super) fn after_reorder(
    d: &mut Dot<'_>,
    state: Before,
    g: G,
    r: i32,
    reverse: bool,
    hasfixed: bool,
) -> Result<(), &'static str> {
    let settled = visible(d, g);
    restore(d, g, state);
    d.reorder::<true, true>(g, r, reverse, hasfixed)?;
    assert_eq!(
        settled,
        visible(d, g),
        "reorder: settled order and C's passes differ, rank {r}, reverse {reverse}, hasfixed {hasfixed}"
    );
    assert_snapshots_stand(d, g);
    assert_scratch_fits(d);
    Ok(())
}

/// C's loop runs first, then its state is put back for the shortcut, which the log now guards.
/// `None` if C's loop stopped with an error: the shortcut must stop with one too.
pub(in super::super) fn before_transpose(d: &mut Dot<'_>, g: G, reverse: bool) -> Option<Visible> {
    let state = before(d, g);
    let reads = &mut d.mincross_store.scan.reads;
    reads.log.clear();
    reads.next = 0;
    reads.mode = ReadMode::Recording;
    let (minr, maxr) = (d.graphs[g].minrank, d.graphs[g].maxrank);
    let result = d.transpose_loop(g, reverse, minr, maxr);
    let reference = result.is_ok().then(|| visible(d, g));
    restore(d, g, state);
    d.mincross_store.scan.reads.mode = ReadMode::Comparing;
    reference
}

/// `transpose_scans` finished: it must have read all that C's loop read, and leave its state.
pub(in super::super) fn after_transpose(d: &mut Dot<'_>, reference: Option<Visible>, g: G) {
    let reads = &mut d.mincross_store.scan.reads;
    reads.mode = ReadMode::Off;
    assert_eq!(
        reads.next,
        reads.log.len(),
        "transpose: counts differ from C's loop, which read more"
    );
    let reference = reference.expect("transpose: C's loop stopped with an error, the shortcut not");
    assert_eq!(
        visible(d, g),
        reference,
        "transpose: state differs from C's loop, graph {g}"
    );
    assert_snapshots_stand(d, g);
    assert_scratch_fits(d);
}

/// `transpose_scans` declined, so C's loop runs for real. It gets the same cap: it may not end.
pub(in super::super) fn before_fallback(d: &mut Dot<'_>) {
    let reads = &mut d.mincross_store.scan.reads;
    reads.log.clear();
    reads.next = 0;
    reads.mode = ReadMode::Recording;
}

/// The scratch arrays' sizes are fixed by the views: nothing a scan does may have grown them.
fn assert_scratch_fits(d: &Dot<'_>) {
    let scan = &d.mincross_store.scan;
    let most = scan.largest;
    for (name, capacity, limit) in [
        ("items", scan.items.capacity(), most.widest),
        ("work", scan.work.capacity(), most.widest * 3 + 1),
        ("ranks", scan.ranks.capacity(), most.rows),
        ("pairs", scan.pairs.capacity(), most.positions),
    ] {
        assert!(
            capacity <= limit,
            "mincross: {name} outgrew its views: capacity {capacity}, limit {limit}"
        );
    }
}
