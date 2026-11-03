//! Reach sets for `canreach` in `make_lr_constraints`, kept from one query to the next.
//!
//! C: `lib/dotgen/position.c:canreach`, a depth-first search over `ND_out` with nothing saved
//! between queries. `make_lr_constraints` asks it about pairs while it adds auxiliary edges, and
//! it asks about the same sources again. This keeps, per source, the set of nodes found so far,
//! and for each of them how many of its out-edges were followed. A query follows only the
//! out-edges appended since, and stops as soon as the target is a member.
//!
//! Why the answers are C's: a source's set is exactly what it reaches over the edges it has
//! followed; edges are only ever appended to `out` (`fast_edge` pushes, and nothing zaps an edge
//! while `make_lr_constraints` runs), so a node that was reachable stays reachable, and following
//! the new edges from the members reaches what a fresh search would. The statement and its proof
//! are in `proofs/GvpProofs/Dot/CanReach.lean` (`dfs_iff`, `resume_exact`).

use super::types::*;

/// The nodes one source reaches over the out-edges followed so far.
#[derive(Default)]
struct Reach {
    /// Membership, one bit per node index.
    bits: Vec<u64>,
    /// The members, in the order they were found. The source is first.
    seen: Vec<N>,
    /// For `seen[i]`, how many of its `out` edges were followed.
    scanned: Vec<usize>,
    /// Words of `bits` already counted against `ReachCache::words`.
    charged: usize,
}

impl Reach {
    fn contains(&self, x: N) -> bool {
        self.bits
            .get(x / 64)
            .is_some_and(|w| w.wrapping_shr((x % 64) as u32) & 1 == 1)
    }

    /// Adds `x`, which becomes the next one to follow.
    fn insert(&mut self, x: N) {
        let word = x / 64;
        if word >= self.bits.len() {
            self.bits.resize(word.saturating_add(1), 0);
        }
        if let Some(w) = self.bits.get_mut(word) {
            let bit = 1u64.wrapping_shl((x % 64) as u32);
            if *w & bit == 0 {
                *w |= bit;
                self.seen.push(x);
                self.scanned.push(0);
            }
        }
    }

    /// Empties the set, keeping its memory, in time proportional to what it holds.
    fn clear(&mut self) {
        for &x in &self.seen {
            if let Some(w) = self.bits.get_mut(x / 64) {
                *w = 0;
            }
        }
        self.seen.clear();
        self.scanned.clear();
    }

    /// Follows the out-edges not followed yet, from every member, into new members, until `v` is
    /// one or nothing is new.
    fn reaches(&mut self, nodes: &[NodeInfo], edges: &[EdgeInfo], v: N) -> bool {
        let mut i = 0;
        while i < self.seen.len() {
            if self.contains(v) {
                return true;
            }
            let Some(x) = self.seen.get(i).copied() else {
                break;
            };
            let out: &[E] = nodes.get(x).map_or(&[], |nd| nd.out.as_slice());
            while let Some(&done) = self.scanned.get(i) {
                let Some(&e) = out.get(done) else {
                    break;
                };
                if let Some(s) = self.scanned.get_mut(i) {
                    *s = s.saturating_add(1);
                }
                if let Some(ed) = edges.get(e) {
                    self.insert(ed.head);
                }
            }
            i = i.saturating_add(1);
        }
        self.contains(v)
    }
}

/// How many 64-bit words of membership bits the cache may hold. Past it, a source that is not
/// cached already is searched in a scratch set that is emptied afterwards. A set is charged its
/// full possible size when admitted, so later growth of a cached set cannot exceed the cap unless
/// the node array itself grows while the cache lives (then the growth is charged as it happens).
const MAX_WORDS: usize = 1 << 22;

/// The reach sets of one `make_lr_constraints`. While it lives, the `out` lists it was asked
/// about may only grow at their ends.
#[derive(Default)]
pub(super) struct ReachCache {
    /// Per node: 1 + the index in `sets`, or 0 when it has no set.
    slot: Vec<u32>,
    sets: Vec<Reach>,
    words: usize,
    scratch: Reach,
}

impl ReachCache {
    /// `canreach(u, v)` of `position.c`: whether `v` is reachable from `u` over `ND_out`.
    pub(super) fn canreach(&mut self, nodes: &[NodeInfo], edges: &[EdgeInfo], u: N, v: N) -> bool {
        if u >= self.slot.len() {
            self.slot.resize(nodes.len().max(u.saturating_add(1)), 0);
        }
        let have = self.slot.get(u).copied().unwrap_or(0) as usize;
        if have > 0 {
            let Some(set) = self.sets.get_mut(have.wrapping_sub(1)) else {
                return false;
            };
            let found = set.reaches(nodes, edges, v);
            // A set only grows past its reservation if `nodes` grew since it was admitted.
            if set.bits.len() > set.charged {
                self.words = self.words.saturating_add(set.bits.len() - set.charged);
                set.charged = set.bits.len();
            }
            return found;
        }
        // Reserve the most this set can hold (one bit per node index), so growth after admission
        // stays within what was charged. `insert` sizes the bit vector by the highest index seen.
        let size = (nodes.len().max(u.saturating_add(1)) / 64).saturating_add(1);
        if self.words.saturating_add(size) <= MAX_WORDS {
            let mut set = Reach::default();
            set.insert(u);
            let found = set.reaches(nodes, edges, v);
            set.charged = size.max(set.bits.len());
            self.words = self.words.saturating_add(set.charged);
            self.sets.push(set);
            if let Some(s) = self.slot.get_mut(u) {
                *s = u32::try_from(self.sets.len()).unwrap_or(0);
            }
            return found;
        }
        self.scratch.clear();
        self.scratch.insert(u);
        self.scratch.reaches(nodes, edges, v)
    }
}

/// `canreach` with nothing kept: the search `position.c` does, with a visited set. The reference
/// for `ReachCache::canreach`.
#[cfg(any(test, feature = "position-check"))]
pub(super) fn canreach_fresh(nodes: &[NodeInfo], edges: &[EdgeInfo], u: N, v: N) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![u];
    while let Some(x) = stack.pop() {
        if x == v {
            return true;
        }
        if !seen.insert(x) {
            continue;
        }
        for &e in &nodes[x].out {
            stack.push(edges[e].head);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*: a fixed sequence, so a failure names its case.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn add_edge(nodes: &mut [NodeInfo], edges: &mut Vec<EdgeInfo>, t: N, h: N) {
        edges.push(EdgeInfo {
            tail: t,
            head: h,
            ..EdgeInfo::default()
        });
        nodes[t].out.push(edges.len() - 1);
    }

    /// Queries interleaved with edge additions, from few sources so that sets are reused, answer
    /// as a fresh search does, in the order asked.
    #[test]
    fn answers_as_a_fresh_search_while_edges_are_added() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut yes, mut no) = (0, 0);
        for case in 0..600 {
            let n = 2 + rng.below(40);
            let mut nodes: Vec<NodeInfo> = (0..n).map(|_| NodeInfo::default()).collect();
            let mut edges = Vec::new();
            let mut cache = ReachCache::default();
            let sources: Vec<N> = (0..1 + rng.below(4)).map(|_| rng.below(n)).collect();
            for _ in 0..(10 + rng.below(80)) {
                if rng.below(3) == 0 {
                    let (t, h) = (rng.below(n), rng.below(n));
                    add_edge(&mut nodes, &mut edges, t, h);
                } else {
                    let u = if rng.below(4) == 0 {
                        rng.below(n)
                    } else {
                        sources[rng.below(sources.len())]
                    };
                    let v = rng.below(n);
                    let got = cache.canreach(&nodes, &edges, u, v);
                    assert_eq!(got, canreach_fresh(&nodes, &edges, u, v), "case {case}");
                    if got {
                        yes += 1;
                    } else {
                        no += 1;
                    }
                }
            }
        }
        assert!(yes > 2000 && no > 2000, "{yes} reachable, {no} not");
    }

    /// Sets grow after admission (a query from an isolated source, then an edge to a far node,
    /// then the query again). The bits actually allocated must stay within `MAX_WORDS`, and the
    /// sources past the cap must still answer correctly from the scratch set.
    #[test]
    fn membership_storage_stays_under_the_cap_as_sets_grow() {
        let n = 65_536;
        let far = n - 1;
        let sources = 8_192;
        let mut nodes: Vec<NodeInfo> = (0..n).map(|_| NodeInfo::default()).collect();
        let mut edges = Vec::new();
        let mut cache = ReachCache::default();
        for u in 0..sources {
            assert!(!cache.canreach(&nodes, &edges, u, far));
        }
        for u in 0..sources {
            add_edge(&mut nodes, &mut edges, u, far);
        }
        for u in 0..sources {
            assert!(cache.canreach(&nodes, &edges, u, far));
        }
        let allocated: usize = cache.sets.iter().map(|s| s.bits.len()).sum();
        assert!(allocated <= MAX_WORDS, "{allocated} words allocated, cap {MAX_WORDS}");
        assert!(cache.words <= MAX_WORDS && allocated <= cache.words);
        let per_set = n / 64 + 1;
        assert_eq!(cache.sets.len(), MAX_WORDS / per_set, "sources admitted up to the cap");
    }

    /// Past the size cap, a source with no set of its own is searched in the scratch set.
    #[test]
    fn scratch_set_answers_too() {
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        let n = 30;
        let mut nodes: Vec<NodeInfo> = (0..n).map(|_| NodeInfo::default()).collect();
        let mut edges = Vec::new();
        let mut cache = ReachCache {
            words: MAX_WORDS,
            ..ReachCache::default()
        };
        for _ in 0..400 {
            if rng.below(3) == 0 {
                let (t, h) = (rng.below(n), rng.below(n));
                add_edge(&mut nodes, &mut edges, t, h);
            } else {
                let (u, v) = (rng.below(n), rng.below(n));
                assert_eq!(
                    cache.canreach(&nodes, &edges, u, v),
                    canreach_fresh(&nodes, &edges, u, v)
                );
            }
        }
        assert!(cache.sets.is_empty());
    }
}
