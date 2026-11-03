//! `leave_edge` on an ordered index of the tree slots with a negative cut value.
//!
//! C's `leave_edge` (ns.c:`leave_edge`) walks `Tree_edge` cyclically from the cursor `S_i`,
//! remembers the most negative cut value it meets, and returns once it has seen `Search_size`
//! (30) negative ones, leaving `S_i` on the slot where it stopped. Late in a run few cut values
//! are negative, so the walk crosses most of the array at every pivot, and that array has one slot
//! per tree edge: on dot's position graphs the walk costs more than the rest of the pivot.
//!
//! [`NegSlots`] is the set of slots whose edge has a negative cut value, kept as a 64-ary tree of
//! bit words, so that "the first member at or above `p`" costs one word read per level (three, at
//! 200,000 slots). [`leave_edge`] visits only members, in the order the walk meets them, applies
//! C's rule to each (the first of the minimum stays: a later one replaces it only when strictly
//! smaller), stops at the same count, and leaves `S_i` where the walk leaves it, so it returns
//! the same edge and leaves the same cursor as C at every call.
//!
//! The index is right when each slot's bit is `cutvalue(Tree_edge[slot]) < 0`. Cut values change
//! in three places after the first pivot, and each calls [`NegSlots::set`]: `treeupdate` (and the
//! cut-set path's loop over the entering edge's cycle), which add to the cut value of every tree
//! edge on the cycle, and `exchange_tree_edges`, which puts the entering edge, with its new cut
//! value, in the leaving edge's slot. `x_cutval` sets the cut values before the index is built.
//!
//! The equality with C's walk, and the preservation of the index's invariant by point edits, is
//! proved in `proofs/GvpProofs/Ns/LeaveEdge.lean` (`leaveIdx_eq_leaveRef`, `run_idx_eq_run_ref`).
//! The proof takes the index as an ordered set with a successor query; that [`NegSlots`] is one,
//! and that the edit sites are complete, are checked at run time: under `cfg(test)` or the
//! `ns-check` feature, every call also runs C's walk from the same cursor and asserts the same
//! edge and the same cursor, and asserts the index against the cut values of every tree slot.

use super::{Ctx, NsGraph};

/// Runs whose `Tree_edge` has fewer slots than this keep C's walk: it is short, and building and
/// editing an index would not pay for itself. Check mode indexes every run.
#[cfg(not(any(test, feature = "ns-check")))]
const INDEX_MIN_SLOTS: usize = 4096;
#[cfg(any(test, feature = "ns-check"))]
const INDEX_MIN_SLOTS: usize = 1;

/// Words per level are 64 bits wide: 64^8 slots is more than a `usize` index can address on
/// the targets that matter, so eight levels never run out.
const MAX_LEVELS: usize = 8;

/// The set of tree slots with a negative cut value, or nothing (`!on`) for a run that keeps
/// C's walk.
#[derive(Default)]
pub(super) struct NegSlots {
    /// Level 0 (one bit per slot), then each summary level (one bit per nonzero word of the level
    /// below), in one allocation.
    words: Vec<u64>,
    /// Where each level starts in `words`.
    starts: [usize; MAX_LEVELS],
    /// How many words each level has.
    sizes: [usize; MAX_LEVELS],
    /// The levels in use; the last one is a single word.
    levels: usize,
    /// The number of slots: a slot at or above it is never a member.
    len: usize,
    on: bool,
}

impl NegSlots {
    /// The index of `tree_edge`'s slots whose edge `is_negative`, or an index that is off when
    /// the run is too small for it to pay off.
    pub(super) fn build(tree_edge: &[usize], is_negative: impl Fn(usize) -> bool) -> NegSlots {
        let len = tree_edge.len();
        if len < INDEX_MIN_SLOTS || len == 0 {
            return NegSlots::default();
        }
        let mut sizes = [0usize; MAX_LEVELS];
        let mut starts = [0usize; MAX_LEVELS];
        let mut levels = 0usize;
        let mut total = 0usize;
        let mut n = len;
        while levels < MAX_LEVELS {
            n = n.div_ceil(64);
            if let (Some(size), Some(start)) = (sizes.get_mut(levels), starts.get_mut(levels)) {
                *size = n;
                *start = total;
            }
            total = total.saturating_add(n);
            levels = levels.saturating_add(1);
            if n <= 1 {
                break;
            }
        }
        let mut index = NegSlots {
            words: vec![0u64; total],
            starts,
            sizes,
            levels,
            len,
            on: n <= 1,
        };
        if !index.on {
            return NegSlots::default();
        }
        for (slot, &e) in tree_edge.iter().enumerate() {
            if is_negative(e) {
                index.insert_bit(0, slot);
            }
        }
        // each summary bit: its word below is nonzero
        for level in 1..levels {
            let below = index.sizes.get(level.wrapping_sub(1)).copied().unwrap_or(0);
            for w in 0..below {
                if index.word(level.wrapping_sub(1), w) != 0 {
                    index.insert_bit(level, w);
                }
            }
        }
        index
    }

    /// Whether this run uses the index.
    #[inline]
    pub(super) fn is_on(&self) -> bool {
        self.on
    }

    #[inline]
    fn word(&self, level: usize, w: usize) -> u64 {
        match (self.starts.get(level), self.sizes.get(level)) {
            (Some(&start), Some(&size)) if w < size => {
                self.words.get(start.wrapping_add(w)).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Sets bit `pos` of `level` without touching the levels above.
    fn insert_bit(&mut self, level: usize, pos: usize) {
        let w = pos / 64;
        let bit = 1u64.wrapping_shl((pos % 64) as u32);
        if let (Some(&start), Some(&size)) = (self.starts.get(level), self.sizes.get(level))
            && w < size
            && let Some(word) = self.words.get_mut(start.wrapping_add(w))
        {
            *word |= bit;
        }
    }

    /// `Tree_edge[slot]` now has a negative cut value (`negative`) or does not.
    #[inline]
    pub(super) fn set(&mut self, slot: usize, negative: bool) {
        if !self.on || slot >= self.len {
            return;
        }
        let mut pos = slot;
        let mut on = negative;
        for level in 0..self.levels {
            let w = pos / 64;
            let bit = 1u64.wrapping_shl((pos % 64) as u32);
            let Some(&start) = self.starts.get(level) else {
                return;
            };
            let Some(word) = self.words.get_mut(start.wrapping_add(w)) else {
                return;
            };
            let was_empty = *word == 0;
            if on {
                *word |= bit;
            } else {
                *word &= !bit;
            }
            // the bit above changes only when this word goes between empty and not
            if was_empty == (*word == 0) {
                return;
            }
            on = *word != 0;
            pos = w;
        }
    }

    /// The least member at or above `from`.
    #[inline]
    pub(super) fn next_from(&self, from: usize) -> Option<usize> {
        let mut level = 0usize;
        let mut pos = from;
        loop {
            if level >= self.levels {
                return None;
            }
            let w = pos / 64;
            if w >= self.sizes.get(level).copied().unwrap_or(0) {
                return None;
            }
            let above = u64::MAX.wrapping_shl((pos % 64) as u32);
            let bits = self.word(level, w) & above;
            if bits != 0 {
                let mut found = w
                    .wrapping_mul(64)
                    .wrapping_add(bits.trailing_zeros() as usize);
                // `found` is a bit of this level, so a word of the level below: descend to its
                // least bit
                while level > 0 {
                    level = level.wrapping_sub(1);
                    let least = self.word(level, found).trailing_zeros() as usize;
                    found = found.wrapping_mul(64).wrapping_add(least);
                }
                return Some(found);
            }
            pos = w.wrapping_add(1);
            level = level.wrapping_add(1);
        }
    }
}

/// C: lib/common/ns.c:leave_edge
///
/// Runs that keep the index walk `Tree_edge` as C does ([`super::leave_edge`]); the others visit
/// the members of the index from the cursor, in C's order, and return the same edge and leave the
/// same cursor. `rv` is the edge, as in C.
pub(super) fn leave_edge<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx) -> Option<usize> {
    if !ctx.neg.is_on() {
        return super::leave_edge(g, ctx);
    }
    #[cfg(any(test, feature = "ns-check"))]
    let start = ctx.s_i;
    let rv = indexed(g, ctx);
    #[cfg(any(test, feature = "ns-check"))]
    check(g, ctx, start, rv);
    rv
}

/// `visit` for the members of `[from, to)`, in order: C's loop body for a negative cut value.
/// `Some(slot)`: the count reached `Search_size` at that slot.
fn members<G: NsGraph + ?Sized>(
    g: &G,
    ctx: &Ctx,
    from: usize,
    to: usize,
    rv: &mut Option<usize>,
    cnt: &mut i32,
) -> Option<usize> {
    let mut at = from;
    while at < to {
        let slot = ctx.neg.next_from(at)?;
        if slot >= to {
            return None;
        }
        let f = *ctx.tree_edge.get(slot)?;
        match *rv {
            Some(r) => {
                if g.cutvalue(r) > g.cutvalue(f) {
                    *rv = Some(f);
                }
            }
            None => *rv = Some(f),
        }
        *cnt = cnt.wrapping_add(1);
        if *cnt >= ctx.search_size {
            return Some(slot);
        }
        at = slot.saturating_add(1);
    }
    None
}

/// The two loops of `leave_edge` over the index.
fn indexed<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx) -> Option<usize> {
    let mut rv: Option<usize> = None;
    let mut cnt = 0;
    let j = ctx.s_i;
    let len = ctx.tree_edge.len();
    if let Some(slot) = members(g, ctx, j, len, &mut rv, &mut cnt) {
        ctx.s_i = slot;
        return rv;
    }
    // the first loop ends with S_i at the end of the array (it did not move if it was there)
    ctx.s_i = ctx.s_i.max(len);
    if j > 0 {
        if let Some(slot) = members(g, ctx, 0, j, &mut rv, &mut cnt) {
            ctx.s_i = slot;
            return rv;
        }
        ctx.s_i = j;
    }
    rv
}

/// Check mode: C's walk from the same cursor returns the same edge and leaves the same cursor,
/// and the index holds exactly the tree slots with a negative cut value.
#[cfg(any(test, feature = "ns-check"))]
fn check<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx, start: usize, got: Option<usize>) {
    let after = ctx.s_i;
    ctx.s_i = start;
    let want = super::leave_edge(g, ctx);
    assert_eq!(got, want, "leave_edge: the index chose another edge");
    assert_eq!(
        after, ctx.s_i,
        "leave_edge: the index left the cursor elsewhere"
    );
    ctx.neg.assert_agrees(g, &ctx.tree_edge);
}

#[cfg(any(test, feature = "ns-check"))]
impl NegSlots {
    /// Every slot's bit is its edge's sign, and every summary bit says whether its word is empty.
    pub(super) fn assert_agrees<G: NsGraph + ?Sized>(&self, g: &G, tree_edge: &[usize]) {
        assert_eq!(self.len, tree_edge.len(), "the index covers another array");
        for (slot, &e) in tree_edge.iter().enumerate() {
            let member = (self.word(0, slot / 64) >> (slot % 64)) & 1 == 1;
            assert_eq!(
                member,
                g.cutvalue(e) < 0,
                "slot {slot}: the index disagrees with the cut value of edge {e}"
            );
        }
        for level in 1..self.levels {
            for pos in 0..self.sizes[level - 1] {
                let summary = (self.word(level, pos / 64) >> (pos % 64)) & 1 == 1;
                assert_eq!(
                    summary,
                    self.word(level - 1, pos) != 0,
                    "level {level}, bit {pos}"
                );
            }
        }
        let mut seen = 0;
        let mut at = 0;
        while let Some(slot) = self.next_from(at) {
            assert!(slot < self.len);
            seen += 1;
            at = slot + 1;
        }
        let members = (0..self.len)
            .filter(|&slot| (self.word(0, slot / 64) >> (slot % 64)) & 1 == 1)
            .count();
        assert_eq!(seen, members, "next_from skipped or repeated a member");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ns::NsData;
    use std::collections::BTreeSet;

    /// A small deterministic generator (SplitMix64).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// The least member at or above `from`, from the model.
    fn model_next(model: &BTreeSet<usize>, from: usize) -> Option<usize> {
        model.range(from..).next().copied()
    }

    #[test]
    fn next_from_matches_an_ordered_set_at_every_level_boundary() {
        // 63..65 words per level change at 64, 4096 and 262144 slots
        for &len in &[
            1usize, 2, 63, 64, 65, 127, 128, 4095, 4096, 4097, 8200, 262_145,
        ] {
            let mut rng = Rng(len as u64);
            let ids: Vec<usize> = (0..len).collect();
            let mut model = BTreeSet::new();
            // a sparse and a dense region
            let initially = |e: usize| (e * 7919).is_multiple_of(13) || (e % 1024 > 1000);
            let mut index = NegSlots::build(&ids, initially);
            assert!(index.is_on(), "len {len}");
            model.extend((0..len).filter(|&e| initially(e)));
            let probes = 400.min(len * 2);
            for step in 0..4 * probes {
                let slot = rng.below(len);
                let on = rng.below(3) == 0;
                index.set(slot, on);
                if on {
                    model.insert(slot);
                } else {
                    model.remove(&slot);
                }
                if step % 4 == 0 {
                    for _ in 0..8 {
                        let from = rng.below(len + 3);
                        assert_eq!(
                            index.next_from(from),
                            model_next(&model, from),
                            "len {len}, from {from}"
                        );
                    }
                }
            }
            // every member in order
            let mut at = 0;
            let mut seen = Vec::new();
            while let Some(slot) = index.next_from(at) {
                seen.push(slot);
                at = slot + 1;
            }
            assert_eq!(seen, model.iter().copied().collect::<Vec<_>>(), "len {len}");
        }
    }

    /// A graph that is only a cut value per edge, and a context whose tree slots are the edges
    /// in order, with the index built from them.
    fn run_of(cuts: &[i32], s_i: usize, search_size: i32) -> (NsData, Ctx) {
        let g = NsData {
            edges: cuts
                .iter()
                .map(|&cutvalue| crate::common::ns::NsEdge {
                    cutvalue,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let tree_edge: Vec<usize> = (0..cuts.len()).collect();
        let neg = NegSlots::build(&tree_edge, |e| g.cutvalue(e) < 0);
        let ctx = Ctx {
            tree_edge,
            s_i,
            search_size,
            neg,
            subtrees: Vec::new(),
            enter_stack: Vec::new(),
            rerank_stack: Vec::new(),
            range_stack: Vec::new(),
        };
        (g, ctx)
    }

    /// The cases of `proofs/GvpProofs/Ns/LeaveEdge.lean` ("Worked cases"): the model and the port
    /// give the same slot and the same cursor, on the walk and on the index.
    #[test]
    fn the_worked_cases_of_the_proof() {
        // (cut values, cutoff, cursor on entry) and (slot chosen, cursor on exit)
        type Case = ((&'static [i32], i32, usize), (Option<usize>, usize));
        let cases: [Case; 4] = [
            // cutoff 2 from slot 1: slots 2 and 3 tie at -3, the first stays
            ((&[-1, 0, -3, -3, 5], 2, 1), (Some(2), 3)),
            // the second negative is smaller and wins
            ((&[-1, 0, -3, -4, 5], 2, 1), (Some(3), 3)),
            // one pass is not enough: the walk wraps to slot 0 and ends on its start
            ((&[-9, 0, 0, -1, 5], 30, 2), (Some(0), 2)),
            // no negatives, from slot 0: the cursor ends at the end of the array
            ((&[0, 1, 2], 30, 0), (None, 3)),
        ];
        for ((cuts, search_size, s_i), (slot, cursor)) in cases {
            let (g, mut walked) = run_of(cuts, s_i, search_size);
            let edge = crate::common::ns::leave_edge(&g, &mut walked);
            assert_eq!((edge, walked.s_i), (slot, cursor), "{cuts:?} walked");
            let (g, mut indexed) = run_of(cuts, s_i, search_size);
            assert!(indexed.neg.is_on());
            let edge = leave_edge(&g, &mut indexed);
            assert_eq!((edge, indexed.s_i), (slot, cursor), "{cuts:?} indexed");
        }
    }

    /// Many pivots on random cut values, edited as `update` edits them: the index and the walk
    /// choose the same edge and leave the same cursor every time, for any cutoff.
    #[test]
    fn the_index_follows_the_walk_through_random_pivots() {
        let mut rng = Rng(731);
        for case in 0..300 {
            let len = 1 + rng.below(if case % 10 == 0 { 700 } else { 90 });
            let sparse = rng.below(3);
            let cuts: Vec<i32> = (0..len)
                .map(|_| match sparse {
                    0 => rng.below(7) as i32 - 3,
                    1 => rng.below(40) as i32 - 1,
                    _ => rng.below(5) as i32 - 4,
                })
                .collect();
            let search_size = match case % 4 {
                0 => 30,
                1 => rng.below(4) as i32,
                _ => rng.below(60) as i32,
            };
            let s_i = rng.below(len + 1);
            let (mut g, mut indexed) = run_of(&cuts, s_i, search_size);
            let (_, mut walked) = run_of(&cuts, s_i, search_size);
            for _ in 0..60 {
                let a = leave_edge(&g, &mut indexed);
                let b = crate::common::ns::leave_edge(&g, &mut walked);
                assert_eq!(a, b, "case {case}");
                assert_eq!(indexed.s_i, walked.s_i, "case {case}");
                // a pivot: the cut values on a random cycle move, and the chosen edge is
                // replaced by one with the opposite value
                for _ in 0..rng.below(6) {
                    let slot = rng.below(len);
                    let next = g.cutvalue(slot).wrapping_add(rng.below(9) as i32 - 4);
                    g.set_cutvalue(slot, next);
                    indexed.neg.set(slot, next < 0);
                }
                if let Some(e) = a {
                    let next = g.cutvalue(e).wrapping_neg();
                    g.set_cutvalue(e, next);
                    indexed.neg.set(e, next < 0);
                }
            }
        }
    }

    #[test]
    fn the_empty_and_the_full_index() {
        let ids: Vec<usize> = (0..5000).collect();
        let mut index = NegSlots::build(&ids, |_| false);
        assert_eq!(index.next_from(0), None);
        for slot in 0..5000 {
            index.set(slot, true);
        }
        for slot in 0..5000 {
            assert_eq!(index.next_from(slot), Some(slot));
        }
        assert_eq!(index.next_from(5000), None);
        for slot in 0..5000 {
            index.set(slot, false);
        }
        assert_eq!(index.next_from(0), None);
        assert!(index.words.iter().all(|&w| w == 0), "summaries empty again");
        // a slot past the end is ignored
        index.set(9999, true);
        assert_eq!(index.next_from(0), None);
    }
}
