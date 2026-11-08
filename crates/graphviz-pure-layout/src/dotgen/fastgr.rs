//! Port of `lib/dotgen/fastgr.c`: operations on the fast internal graph.

use super::Dot;
use super::types::*;

/// `zapinlist`: remove `e` from `l`, filling the hole with the last member.
pub fn zapinlist(l: &mut Vec<E>, e: E) {
    if let Some(i) = l.iter().position(|&x| x == e) {
        l.swap_remove(i);
    }
}

/// C `short` assignment semantics for `ED_count` / `ED_xpenalty`.
#[inline]
pub fn short(v: i32) -> i32 {
    v as i16 as i32
}

/// Not in C: the bytes the last reallocation takes when `extra` elements of `size` bytes are
/// pushed one at a time onto an arena of `len` in `capacity`. Each push into a full `Vec`
/// grows it to `max(2 · capacity, len + 1)`, and an empty one to its smallest capacity (8
/// for 1-byte elements, 4 up to 1 KiB, else 1), as `RawVec::grow_amortized` does. 0 when
/// they fit.
pub(super) fn arena_growth_bytes(len: usize, capacity: usize, extra: usize, size: usize) -> u64 {
    let need = len.saturating_add(extra);
    if need <= capacity {
        return 0;
    }
    let smallest = match size {
        1 => 8,
        2..=1024 => 4,
        _ => 1,
    };
    let mut cap = capacity;
    while cap < need {
        cap = cap
            .saturating_mul(2)
            .max(cap.saturating_add(1))
            .max(smallest);
    }
    (cap as u64).saturating_mul(size as u64)
}

impl Dot<'_> {
    fn ffe(&self, u: N, ul: &[E], v: N, vl: &[E]) -> Option<E> {
        if !ul.is_empty() && !vl.is_empty() {
            if ul.len() < vl.len() {
                ul.iter().copied().find(|&e| self.edges[e].head == v)
            } else {
                vl.iter().copied().find(|&e| self.edges[e].tail == u)
            }
        } else {
            None
        }
    }

    pub fn find_fast_edge(&self, u: N, v: N) -> Option<E> {
        self.ffe(u, &self.nodes[u].out, v, &self.nodes[v].in_)
    }

    pub fn find_flat_edge(&self, u: N, v: N) -> Option<E> {
        self.ffe(u, &self.nodes[u].flat_out, v, &self.nodes[v].flat_in)
    }

    pub fn fast_edge(&mut self, e: E) -> E {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        self.nodes[t].out.push(e);
        self.nodes[h].in_.push(e);
        e
    }

    pub fn delete_fast_edge(&mut self, e: E) {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        zapinlist(&mut self.nodes[t].out, e);
        zapinlist(&mut self.nodes[h].in_, e);
    }

    pub fn other_edge(&mut self, e: E) {
        let t = self.edges[e].tail;
        self.nodes[t].other.push(e);
    }

    pub fn safe_other_edge(&mut self, e: E) {
        let t = self.edges[e].tail;
        if !self.nodes[t].other.contains(&e) {
            self.nodes[t].other.push(e);
        }
    }

    /// Allocates a bare edge record (`make_aux_edge`/`new_virtual_edge` storage).
    pub fn alloc_edge(&mut self, u: N, v: N) -> E {
        let e = self.edges.len();
        self.note_edge_push();
        self.edges.push(EdgeInfo {
            tail: u,
            head: v,
            ..EdgeInfo::default()
        });
        e
    }

    pub fn new_virtual_edge(&mut self, u: N, v: N, orig: Option<E>) -> E {
        let e = self.alloc_edge(u, v);
        self.edges[e].edge_type = VIRTUAL;
        if let Some(o) = orig {
            let oe = self.edges[o].clone();
            let ed = &mut self.edges[e];
            ed.seq = oe.seq;
            ed.count = oe.count;
            ed.xpenalty = oe.xpenalty;
            ed.weight = oe.weight;
            ed.minlen = oe.minlen;
            if u == oe.tail {
                ed.tail_port = oe.tail_port.clone();
            } else if u == oe.head {
                ed.tail_port = oe.head_port.clone();
            }
            if v == oe.head {
                ed.head_port = oe.head_port.clone();
            } else if v == oe.tail {
                ed.head_port = oe.tail_port.clone();
            }
            if self.edges[o].to_virt.is_none() {
                self.edges[o].to_virt = Some(e);
            }
            self.edges[e].to_orig = Some(o);
        } else {
            let ed = &mut self.edges[e];
            ed.weight = 1;
            ed.xpenalty = 1;
            ed.count = 1;
            ed.minlen = 1;
        }
        e
    }

    pub fn virtual_edge(&mut self, u: N, v: N, orig: Option<E>) -> E {
        let e = self.new_virtual_edge(u, v, orig);
        self.fast_edge(e)
    }

    /// `fast_node`: prepends `n` to `GD_nlist(g)`.
    pub fn fast_node(&mut self, g: G, n: N) {
        let head = self.graphs[g].nlist;
        self.nodes[n].next = head;
        if let Some(h) = head {
            self.nodes[h].prev = Some(n);
        }
        self.graphs[g].nlist = Some(n);
        self.nodes[n].prev = None;
    }

    pub fn delete_fast_node(&mut self, g: G, n: N) {
        let (next, prev) = (self.nodes[n].next, self.nodes[n].prev);
        if let Some(nx) = next {
            self.nodes[nx].prev = prev;
        }
        if let Some(pv) = prev {
            self.nodes[pv].next = next;
        } else {
            self.graphs[g].nlist = next;
        }
    }

    pub fn virtual_node(&mut self, g: G) -> N {
        self.note_node_push();
        let n = self.push_node(NodeInfo {
            name: std::borrow::Cow::Borrowed("virtual"),
            node_type: VIRTUAL,
            lw: 1.0,
            rw: 1.0,
            ht: 1.0,
            uf_size: 1,
            ..NodeInfo::default()
        });
        self.fast_node(g, n);
        n
    }

    /// Not in C: before a push onto the node arena, checks the reallocation it causes against
    /// the caller's `Limits::max_allocation`. Past the limit the call stops after this push
    /// ([`Self::check_arena_growth`] checks the bulk growers before they start).
    pub(super) fn note_node_push(&self) {
        let n = self.nodes.len();
        if n == self.nodes.capacity() {
            let bytes = arena_growth_bytes(n, n, 1, Self::NODE_BYTES);
            let _ = graphviz_pure_core::limits::check_allocation(bytes);
        }
    }

    /// Not in C: [`Self::note_node_push`] for the edge arena.
    pub(super) fn note_edge_push(&self) {
        let e = self.edges.len();
        if e == self.edges.capacity() {
            let bytes = arena_growth_bytes(e, e, 1, size_of::<EdgeInfo>());
            let _ = graphviz_pure_core::limits::check_allocation(bytes);
        }
    }

    /// Not in C: checks the reallocations of the node and edge arenas that `nodes` and `edges`
    /// more elements cause against the caller's `Limits::max_allocation`, before a phase that
    /// adds them in bulk starts. Past it the layout stops.
    pub(super) fn check_arena_growth(
        &self,
        nodes: usize,
        edges: usize,
    ) -> std::result::Result<(), String> {
        let n = arena_growth_bytes(
            self.nodes.len(),
            self.nodes.capacity(),
            nodes,
            Self::NODE_BYTES,
        );
        let e = arena_growth_bytes(
            self.edges.len(),
            self.edges.capacity(),
            edges,
            size_of::<EdgeInfo>(),
        );
        if graphviz_pure_core::limits::check_allocation(n).is_err()
            || graphviz_pure_core::limits::check_allocation(e).is_err()
        {
            return Err(graphviz_pure_core::limits::STOPPED.to_string());
        }
        Ok(())
    }

    pub fn flat_edge(&mut self, g: G, e: E) {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        self.nodes[t].flat_out.push(e);
        self.nodes[h].flat_in.push(e);
        self.graphs[0].has_flat_edges = true;
        self.graphs[g].has_flat_edges = true;
    }

    pub fn delete_flat_edge(&mut self, e: E) {
        if let Some(o) = self.edges[e].to_orig
            && self.edges[o].to_virt == Some(e)
        {
            self.edges[o].to_virt = None;
        }
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        zapinlist(&mut self.nodes[t].flat_out, e);
        zapinlist(&mut self.nodes[h].flat_in, e);
    }

    fn basic_merge(&mut self, e: E, rep: E) {
        if self.edges[rep].minlen < self.edges[e].minlen {
            self.edges[rep].minlen = self.edges[e].minlen;
        }
        let (c, x, w) = (
            self.edges[e].count,
            self.edges[e].xpenalty,
            self.edges[e].weight,
        );
        let mut r = Some(rep);
        while let Some(rr) = r {
            let ed = &mut self.edges[rr];
            ed.count = short(ed.count + c);
            ed.xpenalty = short(ed.xpenalty + x);
            // No overflow check in C (undefined behaviour); the port wraps.
            ed.weight = ed.weight.wrapping_add(w);
            r = ed.to_virt;
        }
    }

    pub fn merge_oneway(&mut self, e: E, rep: E) {
        if Some(rep) == self.edges[e].to_virt || Some(e) == self.edges[rep].to_virt {
            graphviz_pure_core::diag::agwarningf("merge_oneway glitch\n");
            return;
        }
        self.edges[e].to_virt = Some(rep);
        self.basic_merge(e, rep);
    }
}

#[cfg(test)]
mod arena_growth_tests {
    use super::arena_growth_bytes;

    /// The capacity a `Vec` of `T` reaches when `extra` elements are pushed one at a time
    /// onto `len` elements in `capacity`.
    fn real<T: Clone>(x: T, len: usize, capacity: usize, extra: usize) -> usize {
        let mut v: Vec<T> = Vec::with_capacity(capacity);
        v.extend((0..len).map(|_| x.clone()));
        let before = v.capacity();
        for _ in 0..extra {
            v.push(x.clone());
        }
        if v.capacity() == before {
            0
        } else {
            v.capacity()
        }
    }

    #[test]
    fn the_estimate_is_vec_s_growth_for_any_starting_capacity() {
        for capacity in 0..40 {
            for len in 0..=capacity {
                for extra in [0, 1, 2, 3, 7, 40, 100] {
                    let bytes = |size: usize| arena_growth_bytes(len, capacity, extra, size);
                    assert_eq!(bytes(1), real(0u8, len, capacity, extra) as u64);
                    assert_eq!(bytes(8), 8 * real(0u64, len, capacity, extra) as u64);
                    assert_eq!(
                        bytes(2048),
                        2048 * real([0u64; 256], len, capacity, extra) as u64,
                        "len {len} capacity {capacity} extra {extra}"
                    );
                }
            }
        }
    }
}
