//! dot's binding of `lib/common/ns.c` (network simplex, `rank`/`rank2`).
//!
//! The algorithm lives in [`crate::common::ns`], generic over [`NsGraph`].
//! This adapter copies the hot `NodeInfo`/`EdgeInfo` fields into flat arrays
//! and gives the mutable tree lists fixed-capacity storage in the same order
//! as `ND_in`/`ND_out`. It writes the results back when `rank2` returns.

use super::Dot;
use super::types::*;
use super::utils::atoi;
use crate::common::ns::{self, NsGraph, SEARCHSIZE};

struct Adjacency {
    offsets: Vec<usize>,
    tree: Vec<E>,
    tree_len: Vec<usize>,
}

fn adjacency_scratch(nodes: &[NodeInfo], incoming: bool) -> Adjacency {
    let mut offsets = Vec::with_capacity(nodes.len().wrapping_add(1));
    let mut tree_len = Vec::with_capacity(nodes.len());
    let mut total = 0usize;
    offsets.push(0);
    for node in nodes {
        let list = if incoming { &node.in_ } else { &node.out };
        total = total.wrapping_add(list.len());
        tree_len.push(0);
        offsets.push(total);
    }
    Adjacency {
        offsets,
        tree: vec![0; total],
        tree_len,
    }
}

/// `graph_t *g` as seen by `ns.c`: dot's arenas plus the graph being ranked.
struct DotNs<'x, 'a> {
    d: &'x mut Dot<'a>,
    g: G,
    nlist: Vec<N>,
    rank: Vec<i32>,
    mark: Vec<usize>,
    priority: Vec<i32>,
    low: Vec<i32>,
    lim: Vec<i32>,
    par: Vec<Option<E>>,
    subtree: Vec<Option<usize>>,
    incoming: Adjacency,
    outgoing: Adjacency,
    tail: Vec<N>,
    head: Vec<N>,
    minlen: Vec<i32>,
    weight: Vec<i32>,
    cutvalue: Vec<i32>,
    tree_index: Vec<i32>,
}

#[inline]
fn scratch_get<T: Copy>(values: &[T], n: usize) -> T {
    values[n]
}

#[inline]
fn scratch_set<T>(values: &mut [T], n: usize, value: T) {
    values[n] = value;
}

impl<'x, 'a> DotNs<'x, 'a> {
    fn new(d: &'x mut Dot<'a>, g: G) -> Self {
        let mut nlist = Vec::new();
        let mut n = d.graphs[g].nlist;
        while let Some(x) = n {
            nlist.push(x);
            n = d.nodes[x].next;
        }
        let incoming = adjacency_scratch(&d.nodes, true);
        let outgoing = adjacency_scratch(&d.nodes, false);
        Self {
            nlist,
            rank: d.nodes.iter().map(|n| n.rank).collect(),
            mark: d.nodes.iter().map(|n| n.mark).collect(),
            priority: d.nodes.iter().map(|n| n.priority).collect(),
            low: d.nodes.iter().map(|n| n.low).collect(),
            lim: d.nodes.iter().map(|n| n.lim).collect(),
            par: d.nodes.iter().map(|n| n.par).collect(),
            subtree: d.nodes.iter().map(|n| n.subtree).collect(),
            incoming,
            outgoing,
            tail: d.edges.iter().map(|e| e.tail).collect(),
            head: d.edges.iter().map(|e| e.head).collect(),
            minlen: d.edges.iter().map(|e| e.minlen).collect(),
            weight: d.edges.iter().map(|e| e.weight).collect(),
            cutvalue: d.edges.iter().map(|e| e.cutvalue).collect(),
            tree_index: d.edges.iter().map(|e| e.tree_index).collect(),
            d,
            g,
        }
    }

    fn finish(self) {
        for &n in &self.nlist {
            let in_start = scratch_get(&self.incoming.offsets, n);
            let in_len = scratch_get(&self.incoming.tree_len, n);
            let out_start = scratch_get(&self.outgoing.offsets, n);
            let out_len = scratch_get(&self.outgoing.tree_len, n);
            let tree_in = self
                .incoming
                .tree
                .get(in_start..in_start.wrapping_add(in_len))
                .unwrap_or_default();
            let tree_out = self
                .outgoing
                .tree
                .get(out_start..out_start.wrapping_add(out_len))
                .unwrap_or_default();
            let Some(node) = self.d.nodes.get_mut(n) else {
                continue;
            };
            node.rank = scratch_get(&self.rank, n);
            node.mark = scratch_get(&self.mark, n);
            node.priority = scratch_get(&self.priority, n);
            node.low = scratch_get(&self.low, n);
            node.lim = scratch_get(&self.lim, n);
            node.par = scratch_get(&self.par, n);
            node.subtree = scratch_get(&self.subtree, n);
            node.tree_in.clear();
            node.tree_in.extend_from_slice(tree_in);
            node.tree_out.clear();
            node.tree_out.extend_from_slice(tree_out);
        }
        for (i, edge) in self.d.edges.iter_mut().enumerate() {
            edge.cutvalue = scratch_get(&self.cutvalue, i);
            edge.tree_index = scratch_get(&self.tree_index, i);
        }
    }

    fn tree_remove(tree: &mut [E], offsets: &[usize], lens: &mut [usize], n: usize, e: E) {
        let start = scratch_get(offsets, n);
        let len = scratch_get(lens, n);
        let end = start.wrapping_add(len);
        if let Some(i) = tree
            .get(start..end)
            .and_then(|list| list.iter().position(|&x| x == e))
        {
            tree.swap(
                start.wrapping_add(i),
                start.wrapping_add(len.wrapping_sub(1)),
            );
            scratch_set(lens, n, len.wrapping_sub(1));
        }
    }

    fn tree_clear(lens: &mut [usize], n: usize) {
        scratch_set(lens, n, 0);
    }
}

impl NsGraph for DotNs<'_, '_> {
    fn nlist(&self) -> Vec<usize> {
        self.nlist.clone()
    }
    fn attr(&self, name: &str) -> Option<String> {
        self.d.agget(self.g, name).map(str::to_string)
    }
    fn is_normal(&self, n: usize) -> bool {
        self.d.nodes[n].node_type == NORMAL
    }
    fn dump_name(&self, n: usize) -> String {
        self.d.nodes[n].name.clone()
    }
    fn err_name(&self, n: usize) -> String {
        if self.d.nodes[n].node_type == NORMAL {
            self.d.nodes[n].name.clone()
        } else {
            "%0".to_string()
        }
    }
    fn rank(&self, n: usize) -> i32 {
        scratch_get(&self.rank, n)
    }
    fn set_rank(&mut self, n: usize, r: i32) {
        scratch_set(&mut self.rank, n, r);
    }
    fn set_mark(&mut self, n: usize, m: bool) {
        scratch_set(&mut self.mark, n, m as usize);
    }
    fn priority(&self, n: usize) -> i32 {
        scratch_get(&self.priority, n)
    }
    fn set_priority(&mut self, n: usize, p: i32) {
        scratch_set(&mut self.priority, n, p);
    }
    fn low(&self, n: usize) -> i32 {
        scratch_get(&self.low, n)
    }
    fn set_low(&mut self, n: usize, v: i32) {
        scratch_set(&mut self.low, n, v);
    }
    fn lim(&self, n: usize) -> i32 {
        scratch_get(&self.lim, n)
    }
    fn set_lim(&mut self, n: usize, v: i32) {
        scratch_set(&mut self.lim, n, v);
    }
    fn par(&self, n: usize) -> Option<usize> {
        scratch_get(&self.par, n)
    }
    fn set_par(&mut self, n: usize, e: Option<usize>) {
        scratch_set(&mut self.par, n, e);
    }
    fn subtree(&self, n: usize) -> Option<usize> {
        scratch_get(&self.subtree, n)
    }
    fn set_subtree(&mut self, n: usize, s: Option<usize>) {
        scratch_set(&mut self.subtree, n, s);
    }
    fn in_edges(&self, n: usize) -> &[usize] {
        &self.d.nodes[n].in_
    }
    fn out_edges(&self, n: usize) -> &[usize] {
        &self.d.nodes[n].out
    }
    fn tree_in(&self, n: usize) -> &[usize] {
        let start = scratch_get(&self.incoming.offsets, n);
        let len = scratch_get(&self.incoming.tree_len, n);
        &self.incoming.tree[start..start.wrapping_add(len)]
    }
    fn tree_out(&self, n: usize) -> &[usize] {
        let start = scratch_get(&self.outgoing.offsets, n);
        let len = scratch_get(&self.outgoing.tree_len, n);
        &self.outgoing.tree[start..start.wrapping_add(len)]
    }
    fn tree_in_push(&mut self, n: usize, e: usize) {
        let len = scratch_get(&self.incoming.tree_len, n);
        let i = scratch_get(&self.incoming.offsets, n).wrapping_add(len);
        scratch_set(&mut self.incoming.tree, i, e);
        scratch_set(&mut self.incoming.tree_len, n, len.wrapping_add(1));
    }
    fn tree_out_push(&mut self, n: usize, e: usize) {
        let len = scratch_get(&self.outgoing.tree_len, n);
        let i = scratch_get(&self.outgoing.offsets, n).wrapping_add(len);
        scratch_set(&mut self.outgoing.tree, i, e);
        scratch_set(&mut self.outgoing.tree_len, n, len.wrapping_add(1));
    }
    fn tree_in_remove(&mut self, n: usize, e: usize) {
        Self::tree_remove(
            &mut self.incoming.tree,
            &self.incoming.offsets,
            &mut self.incoming.tree_len,
            n,
            e,
        );
    }
    fn tree_out_remove(&mut self, n: usize, e: usize) {
        Self::tree_remove(
            &mut self.outgoing.tree,
            &self.outgoing.offsets,
            &mut self.outgoing.tree_len,
            n,
            e,
        );
    }
    fn tree_in_clear(&mut self, n: usize) {
        Self::tree_clear(&mut self.incoming.tree_len, n);
    }
    fn tree_out_clear(&mut self, n: usize) {
        Self::tree_clear(&mut self.outgoing.tree_len, n);
    }
    fn tail(&self, e: usize) -> usize {
        scratch_get(&self.tail, e)
    }
    fn head(&self, e: usize) -> usize {
        scratch_get(&self.head, e)
    }
    fn minlen(&self, e: usize) -> i32 {
        scratch_get(&self.minlen, e)
    }
    fn weight(&self, e: usize) -> i32 {
        scratch_get(&self.weight, e)
    }
    fn cutvalue(&self, e: usize) -> i32 {
        scratch_get(&self.cutvalue, e)
    }
    fn set_cutvalue(&mut self, e: usize, v: i32) {
        scratch_set(&mut self.cutvalue, e, v);
    }
    fn tree_index(&self, e: usize) -> i32 {
        scratch_get(&self.tree_index, e)
    }
    fn set_tree_index(&mut self, e: usize, v: i32) {
        scratch_set(&mut self.tree_index, e, v);
    }
}

impl Dot<'_> {
    /// `rank2(g, balance, maxiter, search_size)`
    pub fn rank2(
        &mut self,
        g: G,
        balance: i32,
        maxiter: i32,
        search_size: i32,
    ) -> Result<i32, String> {
        let mut ns = DotNs::new(self, g);
        let result = ns::rank2(&mut ns, balance, maxiter, search_size);
        ns.finish();
        result
    }

    /// `rank(g, balance, maxiter)`
    pub fn ns_rank(&mut self, g: G, balance: i32, maxiter: i32) -> Result<i32, String> {
        let search_size = match self.agget(g, "searchsize") {
            Some(s) => atoi(s),
            None => SEARCHSIZE,
        };
        self.rank2(g, balance, maxiter, search_size)
    }
}
