//! dot's binding of `lib/common/ns.c` (network simplex, `rank`/`rank2`).
//!
//! The algorithm lives in [`crate::common::ns`], generic over [`NsGraph`].
//! This adapter copies each ranked graph's hot `NodeInfo`/`EdgeInfo` fields
//! into reusable flat arrays and gives the mutable tree lists fixed-capacity
//! storage. It writes the results back when `rank2` returns.

use super::Dot;
use super::types::*;
use super::utils::atoi;
use crate::common::ns::{self, NsGraph, SEARCHSIZE};

#[derive(Default)]
struct Adjacency {
    offsets: Vec<u32>,
    capacities: Vec<u32>,
    tree: Vec<E>,
    tree_len: Vec<u32>,
}

impl Adjacency {
    fn resize_nodes(&mut self, n: usize) {
        self.offsets.resize(n, 0);
        self.capacities.resize(n, 0);
        self.tree_len.resize(n, 0);
    }

    fn prepare(&mut self, n: usize, capacity: usize) {
        if (self.capacities[n] as usize) < capacity {
            let start = self.tree.len();
            let required = start.wrapping_add(capacity);
            if self.tree.capacity() < required {
                let current = self.tree.capacity();
                let target = required.max(current.wrapping_add(current / 8));
                self.tree.reserve_exact(target.wrapping_sub(start));
            }
            self.offsets[n] = start as u32;
            self.tree.resize(required, 0);
            self.capacities[n] = capacity as u32;
        }
        self.tree_len[n] = 0;
    }

    fn edges(&self, n: usize) -> &[E] {
        let start = self.offsets[n] as usize;
        &self.tree[start..start.wrapping_add(self.tree_len[n] as usize)]
    }

    fn push(&mut self, n: usize, e: E) -> bool {
        let len = self.tree_len[n] as usize;
        if len >= self.capacities[n] as usize {
            return false;
        }
        let i = (self.offsets[n] as usize).wrapping_add(len);
        self.tree[i] = e;
        self.tree_len[n] = len.wrapping_add(1) as u32;
        true
    }

    fn remove(&mut self, n: usize, e: E) {
        let start = self.offsets[n] as usize;
        let len = self.tree_len[n] as usize;
        let end = start.wrapping_add(len);
        if let Some(i) = self.tree[start..end].iter().position(|&x| x == e) {
            self.tree.swap(
                start.wrapping_add(i),
                start.wrapping_add(len.wrapping_sub(1)),
            );
            self.tree_len[n] = len.wrapping_sub(1) as u32;
        }
    }

    fn clear(&mut self, n: usize) {
        self.tree_len[n] = 0;
    }
}

#[derive(Default)]
pub(super) struct NsScratch {
    nlist: Vec<N>,
    edges: Vec<E>,
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

impl NsScratch {
    fn resize(&mut self, nodes: usize, edges: usize) {
        if self.rank.len() != nodes || self.tail.len() != edges {
            self.incoming.tree = Vec::new();
            self.incoming.offsets.fill(0);
            self.incoming.capacities.fill(0);
            self.incoming.tree_len.fill(0);
            self.outgoing.tree = Vec::new();
            self.outgoing.offsets.fill(0);
            self.outgoing.capacities.fill(0);
            self.outgoing.tree_len.fill(0);
        }
        self.rank.resize(nodes, 0);
        self.mark.resize(nodes, 0);
        self.priority.resize(nodes, 0);
        self.low.resize(nodes, 0);
        self.lim.resize(nodes, 0);
        self.par.resize(nodes, None);
        self.subtree.resize(nodes, None);
        self.incoming.resize_nodes(nodes);
        self.outgoing.resize_nodes(nodes);
        self.tail.resize(edges, 0);
        self.head.resize(edges, 0);
        self.minlen.resize(edges, 0);
        self.weight.resize(edges, 0);
        self.cutvalue.resize(edges, 0);
        self.tree_index.resize(edges, 0);
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// `graph_t *g` as seen by `ns.c`: dot's arenas plus the graph being ranked.
struct DotNs<'x, 'a> {
    d: &'x mut Dot<'a>,
    g: G,
    scratch: &'x mut NsScratch,
}

impl<'x, 'a> DotNs<'x, 'a> {
    fn new(d: &'x mut Dot<'a>, g: G, scratch: &'x mut NsScratch) -> Self {
        scratch.nlist.clear();
        scratch.edges.clear();
        let mut n = d.graphs[g].nlist;
        while let Some(x) = n {
            scratch.nlist.push(x);
            n = d.nodes[x].next;
        }
        scratch.resize(d.nodes.len(), d.edges.len());
        for &n in &scratch.nlist {
            let node = &d.nodes[n];
            scratch.rank[n] = node.rank;
            scratch.mark[n] = node.mark;
            scratch.priority[n] = node.priority;
            scratch.low[n] = node.low;
            scratch.lim[n] = node.lim;
            scratch.par[n] = node.par;
            scratch.subtree[n] = node.subtree;
            scratch.incoming.prepare(n, node.in_.len());
            scratch.outgoing.prepare(n, node.out.len());
            scratch.edges.extend_from_slice(&node.in_);
        }
        for &e in &scratch.edges {
            let edge = &d.edges[e];
            scratch.tail[e] = edge.tail;
            scratch.head[e] = edge.head;
            scratch.minlen[e] = edge.minlen;
            scratch.weight[e] = edge.weight;
            scratch.cutvalue[e] = edge.cutvalue;
            scratch.tree_index[e] = edge.tree_index;
        }
        Self { d, g, scratch }
    }

    fn finish(self) {
        for &n in &self.scratch.nlist {
            let tree_in = self.scratch.incoming.edges(n);
            let tree_out = self.scratch.outgoing.edges(n);
            let node = &mut self.d.nodes[n];
            node.rank = self.scratch.rank[n];
            node.mark = self.scratch.mark[n];
            node.priority = self.scratch.priority[n];
            node.low = self.scratch.low[n];
            node.lim = self.scratch.lim[n];
            node.par = self.scratch.par[n];
            node.subtree = self.scratch.subtree[n];
            node.tree_in.clear();
            node.tree_in.extend_from_slice(tree_in);
            node.tree_out.clear();
            node.tree_out.extend_from_slice(tree_out);
        }
        for &e in &self.scratch.edges {
            let edge = &mut self.d.edges[e];
            edge.cutvalue = self.scratch.cutvalue[e];
            edge.tree_index = self.scratch.tree_index[e];
        }
    }
}

impl NsGraph for DotNs<'_, '_> {
    fn nlist(&self) -> &[usize] {
        &self.scratch.nlist
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
        self.scratch.rank[n]
    }
    fn set_rank(&mut self, n: usize, r: i32) {
        self.scratch.rank[n] = r;
    }
    fn set_mark(&mut self, n: usize, m: bool) {
        self.scratch.mark[n] = m as usize;
    }
    fn priority(&self, n: usize) -> i32 {
        self.scratch.priority[n]
    }
    fn set_priority(&mut self, n: usize, p: i32) {
        self.scratch.priority[n] = p;
    }
    fn low(&self, n: usize) -> i32 {
        self.scratch.low[n]
    }
    fn set_low(&mut self, n: usize, v: i32) {
        self.scratch.low[n] = v;
    }
    fn lim(&self, n: usize) -> i32 {
        self.scratch.lim[n]
    }
    fn set_lim(&mut self, n: usize, v: i32) {
        self.scratch.lim[n] = v;
    }
    fn par(&self, n: usize) -> Option<usize> {
        self.scratch.par[n]
    }
    fn set_par(&mut self, n: usize, e: Option<usize>) {
        self.scratch.par[n] = e;
    }
    fn subtree(&self, n: usize) -> Option<usize> {
        self.scratch.subtree[n]
    }
    fn set_subtree(&mut self, n: usize, s: Option<usize>) {
        self.scratch.subtree[n] = s;
    }
    fn in_edges(&self, n: usize) -> &[usize] {
        &self.d.nodes[n].in_
    }
    fn out_edges(&self, n: usize) -> &[usize] {
        &self.d.nodes[n].out
    }
    fn tree_in(&self, n: usize) -> &[usize] {
        self.scratch.incoming.edges(n)
    }
    fn tree_out(&self, n: usize) -> &[usize] {
        self.scratch.outgoing.edges(n)
    }
    fn tree_in_push(&mut self, n: usize, e: usize) -> bool {
        self.scratch.incoming.push(n, e)
    }
    fn tree_out_push(&mut self, n: usize, e: usize) -> bool {
        self.scratch.outgoing.push(n, e)
    }
    fn tree_in_remove(&mut self, n: usize, e: usize) {
        self.scratch.incoming.remove(n, e);
    }
    fn tree_out_remove(&mut self, n: usize, e: usize) {
        self.scratch.outgoing.remove(n, e);
    }
    fn tree_in_clear(&mut self, n: usize) {
        self.scratch.incoming.clear(n);
    }
    fn tree_out_clear(&mut self, n: usize) {
        self.scratch.outgoing.clear(n);
    }
    fn tail(&self, e: usize) -> usize {
        self.scratch.tail[e]
    }
    fn head(&self, e: usize) -> usize {
        self.scratch.head[e]
    }
    fn minlen(&self, e: usize) -> i32 {
        self.scratch.minlen[e]
    }
    fn weight(&self, e: usize) -> i32 {
        self.scratch.weight[e]
    }
    fn cutvalue(&self, e: usize) -> i32 {
        self.scratch.cutvalue[e]
    }
    fn set_cutvalue(&mut self, e: usize, v: i32) {
        self.scratch.cutvalue[e] = v;
    }
    fn tree_index(&self, e: usize) -> i32 {
        self.scratch.tree_index[e]
    }
    fn set_tree_index(&mut self, e: usize, v: i32) {
        self.scratch.tree_index[e] = v;
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
        let mut scratch = std::mem::take(&mut self.ns_scratch);
        let result = {
            let mut ns = DotNs::new(self, g, &mut scratch);
            let result = ns::rank2(&mut ns, balance, maxiter, search_size);
            ns.finish();
            result
        };
        self.ns_scratch = scratch;
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

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{Graph, GraphKind};

    #[test]
    fn tree_list_preserves_swap_remove_order_and_capacity() {
        let mut adjacency = Adjacency::default();
        adjacency.resize_nodes(1);
        adjacency.prepare(0, 3);
        for e in [10, 20, 30] {
            assert!(adjacency.push(0, e));
        }
        assert!(!adjacency.push(0, 40));
        adjacency.remove(0, 20);
        assert_eq!(adjacency.edges(0), &[10, 30]);
    }

    #[test]
    fn rank_error_still_writes_back_scratch() {
        let input = Graph::new("G", GraphKind::Directed, false);
        let mut d = Dot::empty(&input);
        d.graphs.push(GraphInfo {
            nlist: Some(0),
            ..GraphInfo::default()
        });
        d.nodes = vec![NodeInfo::default(); 4];
        for (i, node) in d.nodes.iter_mut().enumerate() {
            node.node_type = NORMAL;
            node.next = (i < 3).then_some(i + 1);
        }
        d.nodes[0].out = vec![0, 1];
        d.nodes[1].in_ = vec![0];
        d.nodes[1].out = vec![2];
        d.nodes[2].in_ = vec![1];
        d.nodes[2].out = vec![3];
        d.nodes[3].in_ = vec![2, 3];
        for (tail, head) in [(0, 1), (0, 2), (1, 3), (2, 3)] {
            d.edges.push(EdgeInfo {
                tail,
                head,
                minlen: 1,
                weight: 2_000_000_000,
                ..EdgeInfo::default()
            });
        }

        let result = d.rank2(0, 0, i32::MAX, SEARCHSIZE);
        assert_eq!(result, Err(ns::OVERFLOW_EDGE_WEIGHT_SUM.to_string()));
        for (i, node) in d.nodes.iter().enumerate() {
            assert_eq!(node.rank, d.ns_scratch.rank[i]);
            assert_eq!(node.low, d.ns_scratch.low[i]);
            assert_eq!(node.lim, d.ns_scratch.lim[i]);
            assert_eq!(node.par, d.ns_scratch.par[i]);
            assert_eq!(node.tree_in.as_slice(), d.ns_scratch.incoming.edges(i));
            assert_eq!(node.tree_out.as_slice(), d.ns_scratch.outgoing.edges(i));
        }
        for (i, edge) in d.edges.iter().enumerate() {
            assert_eq!(edge.cutvalue, d.ns_scratch.cutvalue[i]);
            assert_eq!(edge.tree_index, d.ns_scratch.tree_index[i]);
        }
    }
}
