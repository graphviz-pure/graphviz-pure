//! Cut sets computed when asked for, with nothing stored but the rooted tree.
//!
//! Removing a tree edge `t` splits the tree in two. Its cut set is the non-tree edges with one
//! end on each side, and this computes it from whichever side is smaller: walk that side and
//! mark it, then read the edges at the nodes walked. An edge whose other end is unmarked has one
//! end on each side, and every tree edge but `t` has both ends on one side, so this finds exactly
//! the cut set, each edge once (`docs/perf/ns-cut-sets.md`, "Computed cut sets").
//!
//! A query costs about the smaller side of the cut, and a pivot only re-links the path from
//! `f`'s end up to `e`'s child and adjusts sizes along `f`'s tree cycle. The storage is fixed by
//! the graph when the run begins: no pivot makes it grow.

use super::{AGAINST, CutStore, ID, NsGraph, parent};

const NIL: u32 = u32::MAX;
/// [`Incident`] flag: this end of the edge is its tail.
const OUT: u32 = 0x8000_0000;

/// A node's place in the rooted tree: its parent, its children as a list, and how many nodes
/// are in its subtree. The list's order means nothing; C's lists are kept apart.
#[derive(Clone, Copy)]
struct Links {
    up: u32,
    first: u32,
    next: u32,
    prev: u32,
    size: u32,
}

const NO_LINKS: Links = Links {
    up: NIL,
    first: NIL,
    next: NIL,
    prev: NIL,
    size: 0,
};

/// An edge at one of its ends: the other end, and the edge's local index with [`OUT`].
#[derive(Clone, Copy)]
struct Incident {
    other: u32,
    k: u32,
}

/// Reusable storage of the computed path, by node id.
#[derive(Default)]
pub(super) struct Computed {
    /// The cut set last computed, as entries like the pool's.
    pub(super) cross: Vec<u32>,
    /// Whose it is: a tree slot. Nothing while the tree has changed since.
    pub(super) cut_of: Option<usize>,
    links: Vec<Links>,
    /// Where each node's edges are in `incident`.
    incident_at: Vec<[u32; 2]>,
    incident: Vec<Incident>,
    /// The number of the last walk that reached each node.
    side: Vec<u32>,
    side_gen: u32,
    /// The nodes of the last walk.
    walked: Vec<u32>,
    root: u32,
    nodes: u32,
    /// What the storage was when the run began: no pivot may change it.
    #[cfg(any(test, feature = "ns-check"))]
    words_at_build: usize,
}

impl Computed {
    /// The storage, in four-byte words.
    pub(super) fn words(&self) -> usize {
        self.cross
            .capacity()
            .saturating_add(self.links.capacity().saturating_mul(5))
            .saturating_add(self.incident_at.capacity().saturating_mul(2))
            .saturating_add(self.incident.capacity().saturating_mul(2))
            .saturating_add(self.side.capacity())
            .saturating_add(self.walked.capacity())
    }

    /// Hangs `u` under `p`, first of its children.
    fn attach(&mut self, u: u32, p: u32) {
        let first = self.links.get(p as usize).map_or(NIL, |l| l.first);
        if let Some(l) = self.links.get_mut(u as usize) {
            l.up = p;
            l.next = first;
            l.prev = NIL;
        }
        if let Some(l) = self.links.get_mut(first as usize) {
            l.prev = u;
        }
        if let Some(l) = self.links.get_mut(p as usize) {
            l.first = u;
        }
    }

    /// Takes `u` from its parent's list.
    fn detach(&mut self, u: u32) {
        let Some(&Links { up, next, prev, .. }) = self.links.get(u as usize) else {
            return;
        };
        if let Some(l) = self.links.get_mut(next as usize) {
            l.prev = prev;
        }
        if let Some(l) = self.links.get_mut(prev as usize) {
            l.next = next;
        } else if let Some(l) = self.links.get_mut(up as usize) {
            l.first = next;
        }
        if let Some(l) = self.links.get_mut(u as usize) {
            l.up = NIL;
        }
    }

    /// Marks and lists the nodes under `start` in pre-order, leaving out the subtree of `skip`.
    fn walk(&mut self, start: u32, skip: u32) {
        if self.side_gen == u32::MAX {
            self.side.fill(0);
            self.side_gen = 0;
        }
        self.side_gen = self.side_gen.wrapping_add(1);
        let mark = self.side_gen;
        let (links, side, walked) = (&self.links, &mut self.side, &mut self.walked);
        walked.clear();
        let mut u = start;
        while let Some(l) = links.get(u as usize) {
            let mut down = NIL;
            if u != skip {
                walked.push(u);
                if let Some(m) = side.get_mut(u as usize) {
                    *m = mark;
                }
                down = l.first;
            }
            if down != NIL {
                u = down;
                continue;
            }
            // up to the nearest next sibling
            let mut at = l;
            let mut w = u;
            while w != start && at.next == NIL {
                w = at.up;
                let Some(l) = links.get(w as usize) else {
                    return;
                };
                at = l;
            }
            if w == start {
                return;
            }
            u = at.next;
        }
    }

    /// The non-tree edges with one end in the subtree under `v`, whose edge up is local edge
    /// `ke`. `tail_is_child`: `v` is that edge's tail.
    fn compute(&mut self, v: u32, ke: u32, tail_is_child: bool) {
        let size = self.links.get(v as usize).map_or(0, |l| l.size);
        let inside = size.saturating_mul(2) <= self.nodes;
        if inside {
            self.walk(v, NIL);
        } else {
            self.walk(self.root, v);
        }
        let mark = self.side_gen;
        self.cross.clear();
        for &u in &self.walked {
            let Some(&[from, to]) = self.incident_at.get(u as usize) else {
                continue;
            };
            let Some(edges) = self.incident.get(from as usize..to as usize) else {
                continue;
            };
            for x in edges {
                if self.side.get(x.other as usize).copied() == Some(mark) {
                    continue;
                }
                let k = x.k & ID;
                if k == ke {
                    continue;
                }
                // this end is the tail if OUT, and this end is under `v` if the walk was
                let tail_under = (x.k & OUT != 0) == inside;
                // against e: from head(e)'s side to tail(e)'s side
                let against = tail_under != tail_is_child;
                self.cross.push(k | if against { AGAINST } else { 0 });
            }
        }
    }
}

impl CutStore {
    /// The computed path's storage for the current tree. `false` (C's functions then run) when
    /// a node does not fit an index or the tree disagrees with `lim`/`low`.
    pub(super) fn build_computed<G: NsGraph + ?Sized>(&mut self, g: &G) -> bool {
        let Some(&root) = g.nlist().first() else {
            return false;
        };
        let most = g.nlist().iter().copied().max().unwrap_or(0);
        if most >= ID as usize || g.nlist().len() >= ID as usize {
            return false;
        }
        let ids = most.wrapping_add(1);
        let c = &mut self.computed;
        c.root = root as u32;
        c.nodes = g.nlist().len() as u32;
        c.cut_of = None;
        c.cross.clear();
        c.side.clear();
        c.side.resize(ids, 0);
        c.side_gen = 0;
        c.links.clear();
        c.links.resize(ids, NO_LINKS);
        c.incident_at.clear();
        c.incident_at.resize(ids, [0, 0]);
        c.incident.clear();
        c.incident.reserve_exact(self.edges.len().saturating_mul(2));
        // the local index of each edge, in the buffer the walks will use
        let mut local = std::mem::take(&mut c.walked);
        local.clear();
        local.reserve_exact(g.nlist().len().max(self.edges.len()));
        for (k, es) in self.edges.iter().enumerate() {
            let e = es.edge as usize;
            if local.len() <= e {
                local.resize(e.wrapping_add(1), NIL);
            }
            if let Some(x) = local.get_mut(e) {
                *x = k as u32;
            }
        }
        let c = &mut self.computed;
        for &u in g.nlist() {
            let from = c.incident.len() as u32;
            let out = g.out_edges(u).iter().map(|&e| (g.head(e), e, OUT));
            let incoming = g.in_edges(u).iter().map(|&e| (g.tail(e), e, 0));
            for (other, e, out) in out.chain(incoming) {
                // an edge with an end outside the list has no slack C would compute here
                let Some(&k) = local.get(e).filter(|&&k| k != NIL && other <= most) else {
                    return false;
                };
                c.incident.push(Incident {
                    other: other as u32,
                    k: k | out,
                });
            }
            if let Some(at) = c.incident_at.get_mut(u) {
                *at = [from, c.incident.len() as u32];
            }
            if let Some(l) = c.links.get_mut(u) {
                l.size = g.lim(u).wrapping_sub(g.low(u)).wrapping_add(1) as u32;
            }
            if let Some(p) = parent(g, u) {
                c.attach(u as u32, p as u32);
            }
        }
        c.walked = local;
        c.walked.clear();
        // a cut set has at most the non-tree edges, which are those beyond the tree's
        let nontree = self
            .edges
            .len()
            .saturating_sub(g.nlist().len().wrapping_sub(1));
        c.cross.reserve_exact(nontree);
        #[cfg(any(test, feature = "ns-check"))]
        {
            c.words_at_build = c.words();
        }
        // a tree that spans the list, as `lim`/`low` say
        c.links
            .get(root)
            .is_some_and(|l| l.size == c.nodes && l.up == NIL)
    }

    /// Tree slot `s`'s cut set, in [`Computed::cross`]: `v` is the child end of its edge.
    pub(super) fn compute_cut(&mut self, s: usize, v: usize, tail_is_child: bool) {
        let ke = self.tree(s).edge;
        self.computed.compute(v as u32, ke, tail_is_child);
        self.computed.cut_of = Some(s);
    }

    /// The rooted tree once the subtree under `v` hangs from `y` by its node `x`, below `lca`.
    pub(super) fn move_links(&mut self, v: usize, x: usize, y: usize, lca: usize) {
        let (v, x, y, lca) = (v as u32, x as u32, y as u32, lca as u32);
        let c = &mut self.computed;
        let moved = c.links.get(v as usize).map_or(0, |l| l.size);
        // the old ancestors lose the subtree
        let mut u = c.links.get(v as usize).map_or(NIL, |l| l.up);
        while u != lca {
            let Some(l) = c.links.get_mut(u as usize) else {
                break;
            };
            l.size = l.size.wrapping_sub(moved);
            u = l.up;
        }
        // x .. v turn over: each now holds all of the subtree but what was below it
        c.detach(v);
        let (mut u, mut above, mut below) = (x, y, 0u32);
        loop {
            let (up, size) = c.links.get(u as usize).map_or((NIL, 0), |l| (l.up, l.size));
            if u != v {
                c.detach(u);
            }
            c.attach(u, above);
            if let Some(l) = c.links.get_mut(u as usize) {
                l.size = moved.wrapping_sub(below);
            }
            if u == v {
                break;
            }
            below = size;
            above = u;
            u = up;
        }
        // the new ancestors gain it
        let mut u = y;
        while u != lca {
            let Some(l) = c.links.get_mut(u as usize) else {
                break;
            };
            l.size = l.size.wrapping_add(moved);
            u = l.up;
        }
    }
}

/// Check mode: the links are the tree of the parent pointers, with C's subtree sizes.
#[cfg(any(test, feature = "ns-check"))]
impl Computed {
    pub(super) fn assert_links<G: NsGraph + ?Sized>(&self, g: &G) {
        assert_eq!(
            self.words(),
            self.words_at_build,
            "the computed path's storage grew"
        );
        let mut children = 0usize;
        for &u in g.nlist() {
            let l = self.links[u];
            assert_eq!(
                (l.up != NIL).then_some(l.up as usize),
                parent(g, u),
                "parent link of node {u}"
            );
            assert_eq!(
                l.size as i32,
                g.lim(u) - g.low(u) + 1,
                "subtree size of node {u}"
            );
            let mut c = l.first;
            let mut prev = NIL;
            while c != NIL {
                assert_eq!(self.links[c as usize].up as usize, u, "child of node {u}");
                assert_eq!(
                    self.links[c as usize].prev, prev,
                    "sibling link at node {c}"
                );
                children += 1;
                prev = c;
                c = self.links[c as usize].next;
            }
        }
        assert_eq!(
            children + 1,
            g.nlist().len(),
            "every node but the root is a child"
        );
    }

    /// The tests' corruption: a walk from `u` or from the root no longer reaches the children.
    pub(super) fn cut_children(&mut self, u: usize) {
        self.links[u].first = NIL;
        self.links[self.root as usize].first = NIL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree given by its parents, with no edges but what a test adds.
    fn tree(parents: &[Option<u32>]) -> Computed {
        let n = parents.len();
        let mut c = Computed {
            links: vec![NO_LINKS; n],
            side: vec![0; n],
            incident_at: vec![[0, 0]; n],
            root: 0,
            nodes: n as u32,
            ..Computed::default()
        };
        for (u, p) in parents.iter().enumerate() {
            c.links[u].size = 1;
            if let Some(p) = p {
                c.attach(u as u32, *p);
            }
        }
        // parents come first in these lists: sizes from the leaves up
        for u in (0..n).rev() {
            if let Some(p) = parents[u] {
                c.links[p as usize].size += c.links[u].size;
            }
        }
        c
    }

    fn walked(c: &Computed) -> Vec<u32> {
        let mut w = c.walked.clone();
        w.sort_unstable();
        w
    }

    //     0
    //    / \
    //   1   2
    //  /|   |
    // 3 4   5
    const PARENTS: [Option<u32>; 6] = [None, Some(0), Some(0), Some(1), Some(1), Some(2)];

    #[test]
    fn a_walk_covers_the_subtree_or_all_but_it() {
        let mut c = tree(&PARENTS);
        c.walk(1, NIL);
        assert_eq!(walked(&c), [1, 3, 4]);
        c.walk(0, 1);
        assert_eq!(walked(&c), [0, 2, 5]);
        c.walk(3, NIL);
        assert_eq!(walked(&c), [3]);
    }

    #[test]
    fn marks_survive_the_walk_count_wrapping() {
        let mut c = tree(&PARENTS);
        c.walk(1, NIL);
        c.side_gen = u32::MAX;
        c.side[0] = 1;
        c.walk(2, NIL);
        let mark = c.side_gen;
        let marked: Vec<usize> = (0..6).filter(|&u| c.side[u] == mark).collect();
        assert_eq!(marked, [2, 5]);
    }

    #[test]
    fn detaching_keeps_the_rest_of_the_list() {
        let mut c = tree(&[None, Some(0), Some(0), Some(0)]);
        // the list is 3, 2, 1: take the middle, then the first, then the last
        c.detach(2);
        assert_eq!(c.links[3].next, 1);
        assert_eq!(c.links[1].prev, 3);
        c.detach(3);
        assert_eq!(c.links[0].first, 1);
        assert_eq!(c.links[1].prev, NIL);
        c.detach(1);
        assert_eq!(c.links[0].first, NIL);
        assert_eq!(c.links[1].up, NIL);
    }

    #[test]
    fn computing_reads_both_ends_and_both_directions() {
        // tree edge 1 -> 0 (local 0, 0 the parent); non-tree 3 -> 2 (local 1) and 5 -> 4 (local 2)
        let mut c = tree(&PARENTS);
        let edges: [(usize, u32, u32); 4] =
            [(3, 2, 1 | OUT), (2, 3, 1), (5, 4, 2 | OUT), (4, 5, 2)];
        let mut at = [[0u32; 2]; 6];
        for (u, slot) in at.iter_mut().enumerate() {
            slot[0] = c.incident.len() as u32;
            for &(end, other, k) in &edges {
                if end == u {
                    c.incident.push(Incident { other, k });
                }
            }
            slot[1] = c.incident.len() as u32;
        }
        c.incident_at = at.to_vec();
        // the subtree under 1 is {1, 3, 4}: it is the smaller side only if we say so
        c.compute(1, 0, true);
        let mut got = c.cross.clone();
        got.sort_unstable();
        // 3 -> 2 leaves the subtree, 5 -> 4 enters it: against e when e's tail is the child
        assert_eq!(got, [1, 2 | AGAINST]);
        c.compute(1, 0, false);
        got = c.cross.clone();
        got.sort_unstable();
        assert_eq!(got, [2, 1 | AGAINST]);
        // walking the other side finds the same: 1 is more than half of a smaller count
        c.nodes = 4;
        c.compute(1, 0, true);
        assert_eq!(c.walked.len(), 3, "the walk took all but the subtree");
        got = c.cross.clone();
        got.sort_unstable();
        assert_eq!(got, [1, 2 | AGAINST]);
    }

    #[test]
    fn moving_a_subtree_turns_its_path_over() {
        //     0
        //    / \
        //   1   2
        //   |   |
        //   3   5
        //   |
        //   4
        let mut c = tree(&[None, Some(0), Some(0), Some(1), Some(3), Some(2)]);
        let mut store = CutStore {
            computed: std::mem::take(&mut c),
            ..CutStore::default()
        };
        // the subtree under 1 hangs from 5 by its node 4
        store.move_links(1, 4, 5, 0);
        let c = &store.computed;
        let up = |u: usize| c.links[u].up;
        assert_eq!((up(4), up(3), up(1)), (5, 4, 3));
        assert_eq!(up(2), 0);
        let size = |u: usize| c.links[u].size;
        assert_eq!((size(4), size(3), size(1)), (3, 2, 1));
        assert_eq!((size(5), size(2), size(0)), (4, 5, 6));
    }
}
