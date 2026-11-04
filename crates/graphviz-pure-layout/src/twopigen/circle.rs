//! `lib/twopigen/circle.c`: radial placement of one connected component.
//!
//! Per-node `rdata` lives in [`RData`], indexed by node like `ND_alg(n)`.

use crate::common::types::{LEdge, LGraph, Sub};
use graphviz_pure_core::cnum::is_exactly_zero;
use graphviz_pure_core::math;
use graphviz_pure_core::strtod::strtod_rest;

const DEF_RANKSEP: f64 = 1.00;
const UNSET: f64 = 10.00;
/// `MIN_RANKSEP` in `const.h`.
const MIN_RANKSEP: f64 = 0.02;

/// `rdata`.
#[derive(Debug, Clone, Default)]
pub struct RNode {
    pub n_steps_to_leaf: u64,
    pub subtree_size: u64,
    pub n_children: u64,
    pub n_steps_to_center: u64,
    pub parent: Option<usize>,
    pub span: f64,
    pub theta: f64,
}

/// `ND_alg` for every node of the root graph (`gv_calloc`'d once).
pub struct RData {
    pub d: Vec<RNode>,
    /// Scratch for the two breadth-first passes of a component: the queue of
    /// `set_n_steps_to_leaf`, then the discovery order of `set_n_steps_to_center`
    /// that `set_subtree_size` reads backwards. Kept across components.
    order: Vec<usize>,
}

impl RData {
    pub fn new(n: usize) -> Self {
        Self {
            d: vec![RNode::default(); n],
            order: Vec::new(),
        }
    }
}

/// The other end of `e` seen from `n` (`agtail(ep) == n ? aghead(ep) : agtail(ep)`).
fn other(g: &LGraph<'_>, e: usize, n: usize) -> usize {
    let t = g.edges[e].tail;
    if t == n { g.edges[e].head } else { t }
}

/// C: lib/twopigen/circle.c:setNStepsToLeaf
/// Multi-source breadth-first search from every leaf (`n_steps_to_leaf == 0`, in
/// `sg.nodes` order). C relaxes from each leaf in turn with a depth-first walk that
/// only skips the node it came from; its fixpoint is the number of edges to the
/// nearest leaf, which is what one BFS computes in O(nodes + edges).
/// Proof: `proofs/GvpProofs/Twopi/LeafDist.lean`; the C-shaped relaxation lives on
/// as the oracle in this file's tests.
fn set_n_steps_to_leaf(g: &LGraph<'_>, sg: &Sub, r: &mut RData) {
    let mut queue = std::mem::take(&mut r.order);
    leaf_bfs(
        &g.out, &g.inn, &g.edges, &sg.member, &sg.nodes, &mut r.d, &mut queue,
    );
    r.order = queue;
}

/// The body of `set_n_steps_to_leaf` over borrowed slices of `LGraph`'s own `out`, `inn` and
/// `edges` and of `Sub`'s `member` and `nodes`, with no copy, so that Aeneas can extract it
/// (`LGraph::all_edges` is an `impl Iterator` of closures, which it cannot). For each node it
/// visits the out-edges whose head is in the subgraph, then the in-edges whose tail is in it and
/// that are not loops: `all_edges`'s order. The loops sit in `relax_out` and `relax_in`, one per
/// function, and the edge lookups in plain helpers, because Aeneas rejects a loop whose function
/// takes `&[LEdge]` together with `&mut` arguments (`edge_ends` is opaque to it for that reason).
/// `#[inline(always)]` keeps the instruction count of the iterator version.
#[inline(always)]
fn leaf_bfs(
    out: &[Vec<usize>],
    inn: &[Vec<usize>],
    edges: &[LEdge],
    member: &[bool],
    nodes: &[usize],
    d: &mut [RNode],
    queue: &mut Vec<usize>,
) {
    queue.clear();
    seed_leaves(d, nodes, queue);
    let mut head = 0usize;
    while head < queue.len() {
        let Some(&n) = queue.get(head) else { break };
        head = head.saturating_add(1);
        let Some(nsteps) = next_steps(d, n) else {
            continue;
        };
        relax_out(out, edges, member, d, queue, n, nsteps);
        relax_in(inn, edges, member, d, queue, n, nsteps);
    }
}

#[inline(always)]
fn seed_leaves(d: &[RNode], nodes: &[usize], queue: &mut Vec<usize>) {
    for &n in nodes {
        if let Some(node) = d.get(n)
            && node.n_steps_to_leaf == 0
        {
            queue.push(n);
        }
    }
}

#[inline(always)]
fn next_steps(d: &[RNode], n: usize) -> Option<u64> {
    let node = d.get(n)?;
    node.n_steps_to_leaf.checked_add(1)
}

#[inline(always)]
fn relax_out(
    out: &[Vec<usize>],
    edges: &[LEdge],
    member: &[bool],
    d: &mut [RNode],
    queue: &mut Vec<usize>,
    n: usize,
    nsteps: u64,
) {
    let Some(outs) = out.get(n) else { return };
    for &ep in outs {
        if let Some(next) = out_neighbour(edges, member, ep, n)
            && relax_leaf(d, next, nsteps)
        {
            queue.push(next);
        }
    }
}

#[inline(always)]
fn relax_in(
    inn: &[Vec<usize>],
    edges: &[LEdge],
    member: &[bool],
    d: &mut [RNode],
    queue: &mut Vec<usize>,
    n: usize,
    nsteps: u64,
) {
    let Some(ins) = inn.get(n) else { return };
    for &ep in ins {
        if let Some(next) = in_neighbour(edges, member, ep, n)
            && relax_leaf(d, next, nsteps)
        {
            queue.push(next);
        }
    }
}

#[inline(always)]
fn other_end(tail: usize, head: usize, n: usize) -> usize {
    if tail == n { head } else { tail }
}

/// The `(tail, head)` of an edge. Kept opaque to Aeneas, which cannot take `LEdge`'s other
/// fields (`Option<TextLabel>`, `Vec<Bezier>`, ...) into a loop's context.
#[inline(always)]
fn edge_ends(e: &LEdge) -> (usize, usize) {
    (e.tail, e.head)
}

#[inline(always)]
fn out_neighbour(edges: &[LEdge], member: &[bool], ep: usize, n: usize) -> Option<usize> {
    let e = edges.get(ep)?;
    let (tail, head) = edge_ends(e);
    let inside = member.get(head)?;
    if *inside {
        Some(other_end(tail, head, n))
    } else {
        None
    }
}

#[inline(always)]
fn in_neighbour(edges: &[LEdge], member: &[bool], ep: usize, n: usize) -> Option<usize> {
    let e = edges.get(ep)?;
    let (tail, head) = edge_ends(e);
    let inside = member.get(tail)?;
    if *inside && tail != head {
        Some(other_end(tail, head, n))
    } else {
        None
    }
}

#[inline(always)]
fn relax_leaf(d: &mut [RNode], next: usize, nsteps: u64) -> bool {
    if let Some(node) = d.get_mut(next)
        && nsteps < node.n_steps_to_leaf
    {
        node.n_steps_to_leaf = nsteps;
        true
    } else {
        false
    }
}

/// `isLeaf(g, n)`.
fn is_leaf(g: &LGraph<'_>, sg: &Sub, n: usize) -> bool {
    let mut neighp: Option<usize> = None;
    for ep in g.all_edges(sg, n) {
        let np = other(g, ep, n);
        if n == np {
            continue;
        }
        match neighp {
            Some(p) if p != np => return false,
            Some(_) => {}
            None => neighp = Some(np),
        }
    }
    true
}

/// `initLayout(g)`.
fn init_layout(g: &LGraph<'_>, sg: &Sub, r: &mut RData) {
    let nnodes = sg.nodes.len() as u64;
    let inf = nnodes * nnodes;
    for &n in &sg.nodes {
        r.d[n].n_steps_to_center = inf;
        r.d[n].theta = UNSET;
        r.d[n].n_steps_to_leaf = if is_leaf(g, sg, n) { 0 } else { inf };
    }
}

/// `findCenterNode(g)`.
fn find_center_node(g: &LGraph<'_>, sg: &Sub, r: &mut RData) -> Option<usize> {
    set_n_steps_to_leaf(g, sg, r);
    let mut center = None;
    let mut max_n_steps_to_leaf = 0;
    for &n in &sg.nodes {
        if center.is_none() || r.d[n].n_steps_to_leaf > max_n_steps_to_leaf {
            max_n_steps_to_leaf = r.d[n].n_steps_to_leaf;
            center = Some(n);
        }
    }
    center
}

/// `setNStepsToCenter(g, n)`: BFS building the tree.
fn set_n_steps_to_center(g: &LGraph<'_>, sg: &Sub, r: &mut RData, n: usize) {
    let wt = g.edge_attr_declared("weight");
    // The queue doubles as the discovery order (parents before children).
    let mut q = std::mem::take(&mut r.order);
    q.clear();
    q.push(n);
    let mut head = 0usize;
    while let Some(&n) = q.get(head) {
        head = head.saturating_add(1);
        let nsteps = r.d[n].n_steps_to_center + 1;
        for ep in g.all_edges(sg, n) {
            if wt && g.eattr(ep, "weight").unwrap_or("") == "0" {
                continue;
            }
            let next = other(g, ep, n);
            if nsteps < r.d[next].n_steps_to_center {
                r.d[next].n_steps_to_center = nsteps;
                r.d[next].parent = Some(n);
                r.d[n].n_children += 1;
                q.push(next);
            }
        }
    }
    r.order = q;
}

/// `setParentNodes(sg, center)`; `u64::MAX` if some node was not reached.
fn set_parent_nodes(g: &LGraph<'_>, sg: &Sub, r: &mut RData, center: usize) -> u64 {
    let mut maxn = 0;
    let unset = r.d[center].n_steps_to_center;
    r.d[center].n_steps_to_center = 0;
    r.d[center].parent = None;
    set_n_steps_to_center(g, sg, r, center);
    for &n in &sg.nodes {
        if r.d[n].n_steps_to_center == unset {
            return u64::MAX;
        } else if r.d[n].n_steps_to_center > maxn {
            maxn = r.d[n].n_steps_to_center;
        }
    }
    maxn
}

/// `setSubtreeSize(g)`: count leaves below each node.
/// C walks from every childless node to the root, adding 1 to each node on the way,
/// O(leaves · depth). Here the nodes are visited once in reverse discovery order of
/// `set_n_steps_to_center`, so every child is done before its parent and each node
/// adds its total to its parent: O(nodes). Needs `r.order` as that function left it,
/// every node of the component discovered, and `subtree_size` still 0 (each node
/// belongs to one component and is laid out once).
fn set_subtree_size(r: &mut RData) {
    // A countdown, `(0..len).rev()` for Aeneas.
    let mut i = r.order.len();
    while i > 0 {
        i = i.saturating_sub(1);
        let Some(&n) = r.order.get(i) else { continue };
        let Some(node) = r.d.get_mut(n) else { continue };
        node.subtree_size = node
            .subtree_size
            .saturating_add(u64::from(node.n_children == 0));
        let (size, parent) = (node.subtree_size, node.parent);
        // `parent.and_then(|p| r.d.get_mut(p))`, without the closure.
        if let Some(p) = parent
            && let Some(p) = r.d.get_mut(p)
        {
            p.subtree_size = p.subtree_size.saturating_add(size);
        }
    }
}

/// `setChildSubtreeSpans(g, n)`.
fn set_child_subtree_spans(g: &LGraph<'_>, sg: &Sub, r: &mut RData, n: usize) {
    let ratio = r.d[n].span / r.d[n].subtree_size as f64;
    let edges: Vec<usize> = g.all_edges(sg, n).collect();
    for ep in edges {
        let next = other(g, ep, n);
        if r.d[next].parent != Some(n) {
            continue;
        }
        if !is_exactly_zero(r.d[next].span) {
            continue;
        }
        r.d[next].span = ratio * r.d[next].subtree_size as f64;
        if r.d[next].n_children > 0 {
            set_child_subtree_spans(g, sg, r, next);
        }
    }
}

/// `setSubtreeSpans(sg, center)`.
fn set_subtree_spans(g: &LGraph<'_>, sg: &Sub, r: &mut RData, center: usize) {
    r.d[center].span = 2.0 * std::f64::consts::PI;
    set_child_subtree_spans(g, sg, r, center);
}

/// `is_set(a)`.
fn is_set(a: f64) -> bool {
    a != UNSET
}

/// `setChildPositions(sg, n)`.
fn set_child_positions(g: &LGraph<'_>, sg: &Sub, r: &mut RData, n: usize) {
    let mut theta = if r.d[n].parent.is_none() {
        0.0
    } else {
        r.d[n].theta - r.d[n].span / 2.0
    };
    let edges: Vec<usize> = g.all_edges(sg, n).collect();
    for ep in edges {
        let next = other(g, ep, n);
        if r.d[next].parent != Some(n) {
            continue;
        }
        if is_set(r.d[next].theta) {
            continue;
        }
        r.d[next].theta = theta + r.d[next].span / 2.0;
        theta += r.d[next].span;
        if r.d[next].n_children > 0 {
            set_child_positions(g, sg, r, next);
        }
    }
}

/// `setPositions(sg, center)`.
fn set_positions(g: &LGraph<'_>, sg: &Sub, r: &mut RData, center: usize) {
    r.d[center].theta = 0.0;
    set_child_positions(g, sg, r, center);
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// `getRankseps(g, maxrank)`: radius of each rank from `ranksep`.
fn get_rankseps(g: &LGraph<'_>, maxrank: u64) -> Vec<f64> {
    let maxrank = maxrank as usize;
    let mut rk = 1usize;
    let mut ranks = vec![0.0f64; maxrank + 1];
    let mut xf = 0.0f64;
    let mut delx = 0.0f64;
    if let Some(mut p) = g.gattr("ranksep") {
        while rk <= maxrank {
            let d = match strtod_rest(p) {
                Some((d, rest)) if d > 0.0 => {
                    p = rest;
                    d
                }
                _ => break,
            };
            delx = d.max(MIN_RANKSEP);
            xf += delx;
            ranks[rk] = xf;
            rk += 1;
            let skip = p.bytes().take_while(|&c| is_space(c) || c == b':').count();
            p = &p[skip..];
        }
    } else {
        delx = DEF_RANKSEP;
    }
    for r in ranks.iter_mut().take(maxrank + 1).skip(rk) {
        xf += delx;
        *r = xf;
    }
    ranks
}

/// `setAbsolutePos(g, maxrank)`: polar to Cartesian.
fn set_absolute_pos(g: &mut LGraph<'_>, sg: &Sub, r: &RData, maxrank: u64) {
    let ranksep = get_rankseps(g, maxrank);
    if graphviz_pure_core::diag::verbose_on() {
        let mut m = String::from("Rank separation = ");
        for v in ranksep.iter().take((maxrank as usize).wrapping_add(1)) {
            m.push_str(&format!("{} ", graphviz_pure_core::diag::fmt_f(*v, 3)));
        }
        m.push('\n');
        graphviz_pure_core::diag::fprintf_stderr(&m);
    }
    for &n in &sg.nodes {
        let hyp = ranksep[r.d[n].n_steps_to_center as usize];
        g.nodes[n].pos[0] = hyp * math::cos(r.d[n].theta);
        g.nodes[n].pos[1] = hyp * math::sin(r.d[n].theta);
    }
}

/// `circleLayout(sg, center)`: lay out connected, non-empty `sg` around
/// `center` (chosen when `None`); returns the centre used.
pub fn circle_layout(
    g: &mut LGraph<'_>,
    sg: &Sub,
    r: &mut RData,
    center: Option<usize>,
) -> Option<usize> {
    if sg.nodes.len() == 1 {
        let n = sg.nodes[0];
        g.nodes[n].pos[0] = 0.0;
        g.nodes[n].pos[1] = 0.0;
        return center;
    }
    init_layout(g, sg, r);
    let center = match center {
        Some(c) => c,
        None => find_center_node(g, sg, r)?,
    };
    let max_n_steps_to_center = set_parent_nodes(g, sg, r, center);
    graphviz_pure_core::verbose_msg!(
        "root = {} max steps to root = {max_n_steps_to_center}\n",
        g.node_agnameof(center).name
    );
    if max_n_steps_to_center == u64::MAX {
        // C leaves the positions untouched.
        graphviz_pure_core::diag::agerrorf(
            "twopi: use of weight=0 creates disconnected component.\n",
        );
        return Some(center);
    }
    set_subtree_size(r);
    set_subtree_spans(g, sg, r, center);
    set_positions(g, sg, r, center);
    set_absolute_pos(g, sg, r, max_n_steps_to_center);
    Some(center)
}

#[cfg(test)]
mod leaf_tests {
    use super::*;
    use crate::common::types::EdgeType;
    use graphviz_pure_core::{AttrMap, Graph, GraphKind};

    // ---- Oracles: the shipped code before #729, kept as the runtime check. ----

    /// C: lib/twopigen/circle.c:setNStepsToLeaf
    /// A suspended recursive call, including the distance captured on entry.
    struct LeafFrame {
        n: usize,
        prev: Option<usize>,
        edges: std::ops::Range<usize>,
        nsteps: u64,
    }

    /// C: lib/twopigen/circle.c:setNStepsToLeaf
    fn relax_from_leaf(
        r: &mut RData,
        n: usize,
        adjacency: &[usize],
        offsets: &[std::ops::Range<usize>],
        stack: &mut Vec<LeafFrame>,
    ) {
        stack.clear();
        let (Some(node), Some(edges)) = (r.d.get(n), offsets.get(n)) else {
            return;
        };
        let Some(nsteps) = node.n_steps_to_leaf.checked_add(1) else {
            return;
        };
        stack.push(LeafFrame {
            n,
            prev: None,
            edges: edges.clone(),
            nsteps,
        });
        while let Some(frame) = stack.last_mut() {
            let Some(ep) = frame.edges.next() else {
                stack.pop();
                continue;
            };
            let Some(&next) = adjacency.get(ep) else {
                continue;
            };
            if frame.prev == Some(next) {
                continue;
            }
            if let Some(node) = r.d.get_mut(next)
                && frame.nsteps < node.n_steps_to_leaf
            {
                node.n_steps_to_leaf = frame.nsteps;
                let (Some(edges), Some(nsteps)) = (offsets.get(next), frame.nsteps.checked_add(1))
                else {
                    continue;
                };
                let child = LeafFrame {
                    n: next,
                    prev: Some(frame.n),
                    edges: edges.clone(),
                    nsteps,
                };
                stack.push(child);
            }
        }
    }

    /// C: lib/twopigen/circle.c:setNStepsToLeaf
    /// Snapshot the immutable component in cgraph's edge order.
    fn leaf_adjacency(
        g: &LGraph<'_>,
        sg: &Sub,
        offsets: &mut [std::ops::Range<usize>],
    ) -> Vec<usize> {
        // Preserve all_edges' out-then-in order, filtering membership just once.
        let mut adjacency = Vec::new();
        for &n in &sg.nodes {
            let start = adjacency.len();
            adjacency.extend(g.all_edges(sg, n).map(|e| other(g, e, n)));
            if let Some(offset) = offsets.get_mut(n) {
                *offset = start..adjacency.len();
            }
        }
        adjacency
    }

    /// C: lib/twopigen/circle.c:setSubtreeSize
    /// The per-leaf walk to the root that `set_subtree_size` replaced.
    fn subtree_size_by_walks(sg: &Sub, r: &mut RData) {
        for &n in &sg.nodes {
            if r.d[n].n_children > 0 {
                continue;
            }
            r.d[n].subtree_size += 1;
            let mut parent = r.d[n].parent;
            while let Some(p) = parent {
                r.d[p].subtree_size += 1;
                parent = r.d[p].parent;
            }
        }
    }

    /// The relaxation over every leaf in `sg.nodes` order, as `find_center_node` ran it.
    fn relax_all(g: &LGraph<'_>, sg: &Sub, r: &mut RData) {
        let mut offsets = vec![0..0; r.d.len()];
        let adjacency = leaf_adjacency(g, sg, &mut offsets);
        let mut stack = Vec::with_capacity(sg.nodes.len());
        for &n in &sg.nodes {
            if r.d[n].n_steps_to_leaf == 0 {
                relax_from_leaf(r, n, &adjacency, &offsets, &mut stack);
            }
        }
    }

    fn recursive_reference(g: &LGraph<'_>, sg: &Sub, r: &mut RData, n: usize, prev: Option<usize>) {
        let nsteps = r.d[n].n_steps_to_leaf + 1;
        let edges: Vec<_> = g.all_edges(sg, n).collect();
        for ep in edges {
            let next = other(g, ep, n);
            if prev != Some(next) && nsteps < r.d[next].n_steps_to_leaf {
                r.d[next].n_steps_to_leaf = nsteps;
                recursive_reference(g, sg, r, next, Some(n));
            }
        }
    }

    fn build(count: usize, edges: &[(usize, usize)]) -> Graph {
        let mut source = Graph::new("G", GraphKind::Directed, false);
        let nodes: Vec<_> = (0..count)
            .map(|i| source.ensure_node(&format!("n{i}")))
            .collect();
        for &(t, h) in edges {
            source.add_edge(nodes[t], nodes[h], AttrMap::new());
        }
        source
    }

    fn leaf_steps(r: &RData) -> Vec<u64> {
        r.d.iter().map(|n| n.n_steps_to_leaf).collect()
    }

    /// Run both leaf-distance passes on `sg` and assert the same integers per node.
    fn assert_same_leaf_steps(g: &LGraph<'_>, sg: &Sub, total: usize) {
        let mut by_relaxation = RData::new(total);
        init_layout(g, sg, &mut by_relaxation);
        let mut by_bfs = RData::new(total);
        by_bfs.d.clone_from(&by_relaxation.d);
        relax_all(g, sg, &mut by_relaxation);
        set_n_steps_to_leaf(g, sg, &mut by_bfs);
        assert_eq!(leaf_steps(&by_bfs), leaf_steps(&by_relaxation));
    }

    /// Both subtree-size passes after `set_parent_nodes`, same integers per node.
    fn assert_same_subtree_sizes(g: &LGraph<'_>, sg: &Sub, total: usize, center: usize) {
        let mut r = RData::new(total);
        init_layout(g, sg, &mut r);
        assert_ne!(set_parent_nodes(g, sg, &mut r, center), u64::MAX);
        // What the Lean theorem assumes of `order`: every node once, parents before children.
        let position: std::collections::HashMap<usize, usize> =
            r.order.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        assert_eq!(r.order.len(), sg.nodes.len());
        assert_eq!(position.len(), sg.nodes.len());
        for &n in &sg.nodes {
            if let Some(p) = r.d[n].parent {
                assert!(position[&p] < position[&n]);
            }
        }
        let mut walked = RData::new(total);
        walked.d.clone_from(&r.d);
        subtree_size_by_walks(sg, &mut walked);
        set_subtree_size(&mut r);
        let sizes = |r: &RData| r.d.iter().map(|n| n.subtree_size).collect::<Vec<_>>();
        assert_eq!(sizes(&r), sizes(&walked));
    }

    #[test]
    fn iterative_relaxation_matches_recursive_calls() {
        let source = build(
            7,
            &[
                (0, 1),
                (1, 2),
                (1, 2),
                (1, 3),
                (2, 3),
                (3, 3),
                (3, 4),
                (0, 0),
                (0, 5),
                (6, 4),
            ],
        );
        // Cycles, parallel edges, loops, mixed directions and edges outside sg.
        let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
        let sg = Sub::new(7, vec![0, 1, 2, 3, 4]);
        let mut actual = RData::new(7);
        init_layout(&g, &sg, &mut actual);
        let mut expected = RData::new(7);
        expected.d.clone_from(&actual.d);
        let mut offsets = vec![0..0; 7];
        let adjacency = leaf_adjacency(&g, &sg, &mut offsets);
        // Check the cache independently: ordering/filtering changes must fail even
        // when the final shortest-path distances would agree.
        for &n in &sg.nodes {
            let old_neighbors: Vec<_> = g.all_edges(&sg, n).map(|ep| other(&g, ep, n)).collect();
            assert_eq!(&adjacency[offsets[n].clone()], old_neighbors);
        }
        let mut stack = Vec::new();
        for &leaf in &sg.nodes {
            if actual.d[leaf].n_steps_to_leaf == 0 {
                recursive_reference(&g, &sg, &mut expected, leaf, None);
                relax_from_leaf(&mut actual, leaf, &adjacency, &offsets, &mut stack);
                assert_eq!(leaf_steps(&actual), leaf_steps(&expected));
            }
        }
    }

    #[test]
    fn bfs_matches_relaxation_on_the_hand_built_graph() {
        let source = build(
            7,
            &[
                (0, 1),
                (1, 2),
                (1, 2),
                (1, 3),
                (2, 3),
                (3, 3),
                (3, 4),
                (0, 0),
                (0, 5),
                (6, 4),
            ],
        );
        let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
        assert_same_leaf_steps(&g, &Sub::new(7, vec![0, 1, 2, 3, 4]), 7);
        assert_same_leaf_steps(&g, &Sub::new(7, (0..7).collect()), 7);
    }

    /// A small deterministic generator (SplitMix64), so the cases are the same on every run.
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

    #[test]
    fn bfs_and_postorder_match_the_oracles_on_random_graphs() {
        let mut rng = Rng(729);
        for case in 0..400 {
            let count = 2 + rng.below(30);
            // A random spanning tree keeps the component connected; extra edges add
            // cycles, parallel edges and loops (every kind of non-tree graph twopi lays out).
            let mut edges: Vec<(usize, usize)> = (1..count)
                .map(|v| {
                    let u = rng.below(v);
                    if rng.below(2) == 0 { (u, v) } else { (v, u) }
                })
                .collect();
            let extra = if case % 3 == 0 { 0 } else { rng.below(count) };
            for _ in 0..extra {
                edges.push((rng.below(count), rng.below(count)));
            }
            let source = build(count, &edges);
            let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
            let sg = Sub::new(count, (0..count).collect());
            assert_same_leaf_steps(&g, &sg, count);
            // The centre `find_center_node` picks, then the subtree sizes around it.
            let mut r = RData::new(count);
            init_layout(&g, &sg, &mut r);
            let center = find_center_node(&g, &sg, &mut r).expect("non-empty");
            let mut expected = RData::new(count);
            init_layout(&g, &sg, &mut expected);
            relax_all(&g, &sg, &mut expected);
            let mut best = None;
            let mut best_steps = 0;
            for &n in &sg.nodes {
                if best.is_none() || expected.d[n].n_steps_to_leaf > best_steps {
                    best_steps = expected.d[n].n_steps_to_leaf;
                    best = Some(n);
                }
            }
            assert_eq!(Some(center), best, "case {case}");
            assert_same_subtree_sizes(&g, &sg, count, center);
            // And from an arbitrary root, as `root=` does.
            assert_same_subtree_sizes(&g, &sg, count, rng.below(count));
        }
    }

    #[test]
    fn nodes_with_no_leaf_keep_their_infinity() {
        // A cycle has no leaf: neither pass changes anything.
        let source = build(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]);
        let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
        let sg = Sub::new(5, (0..5).collect());
        assert_same_leaf_steps(&g, &sg, 5);
        let mut r = RData::new(5);
        init_layout(&g, &sg, &mut r);
        set_n_steps_to_leaf(&g, &sg, &mut r);
        assert_eq!(leaf_steps(&r), vec![25; 5]);
    }

    #[test]
    fn deep_paths_and_stars_match_the_oracles() {
        // Path: one relaxation per end walks the whole depth. Star: many leaves, depth 1.
        // Broom: a long handle and many bristles, the O(leaves * depth) shape of the old walk.
        let count = 4_000;
        let path: Vec<_> = (1..count).map(|v| (v - 1, v)).collect();
        let star: Vec<_> = (1..count).map(|v| (0, v)).collect();
        let handle = count / 2;
        let mut broom: Vec<_> = (1..handle).map(|v| (v - 1, v)).collect();
        broom.extend((handle..count).map(|v| (handle - 1, v)));
        for (name, edges) in [("path", path), ("star", star), ("broom", broom)] {
            let source = build(count, &edges);
            let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
            let sg = Sub::new(count, (0..count).collect());
            assert_same_leaf_steps(&g, &sg, count);
            assert_same_subtree_sizes(&g, &sg, count, 0);
            assert_same_subtree_sizes(&g, &sg, count, count / 2);
            let mut r = RData::new(count);
            init_layout(&g, &sg, &mut r);
            set_n_steps_to_leaf(&g, &sg, &mut r);
            let deepest = r.d.iter().map(|n| n.n_steps_to_leaf).max();
            assert!(deepest.is_some_and(|d| d < count as u64), "{name}");
        }
    }

    #[test]
    fn bfs_is_iterative_on_a_20k_path() {
        // No recursion anywhere: a 20,000-deep path must not touch the thread stack.
        let count = 20_000;
        let path: Vec<_> = (1..count).map(|v| (v - 1, v)).collect();
        let source = build(count, &path);
        let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
        let sg = Sub::new(count, (0..count).collect());
        let mut r = RData::new(count);
        init_layout(&g, &sg, &mut r);
        let center = find_center_node(&g, &sg, &mut r).expect("non-empty");
        assert_eq!(r.d[center].n_steps_to_leaf, (count as u64 - 1) / 2);
        assert_ne!(set_parent_nodes(&g, &sg, &mut r, center), u64::MAX);
        set_subtree_size(&mut r);
        assert_eq!(r.d[center].subtree_size, 2);
    }

    #[test]
    fn many_components_reuse_the_same_scratch_allocation() {
        let count = 200;
        let pairs: Vec<_> = (0..count).step_by(2).map(|n| (n, n + 1)).collect();
        let source = build(count, &pairs);
        let g = LGraph::new(&source, 2, EdgeType::Line).expect("layout graph");
        let mut r = RData::new(count);
        let mut allocation = None;
        for n in (0..count).step_by(2) {
            let sg = Sub::new(count, vec![n, n + 1]);
            init_layout(&g, &sg, &mut r);
            assert_eq!(find_center_node(&g, &sg, &mut r), Some(n));
            let current = (r.order.as_ptr(), r.order.capacity());
            if let Some(previous) = allocation {
                assert_eq!(current, previous);
            } else {
                allocation = Some(current);
            }
        }
    }

    #[test]
    fn deep_relaxation_uses_heap_stack() {
        let count = 20_000;
        let adjacency: Vec<_> = (1..count).collect();
        let offsets: Vec<_> = (0..count)
            .map(|n| n.min(count - 1)..(n + 1).min(count - 1))
            .collect();
        let mut r = RData::new(count);
        for n in &mut r.d {
            n.n_steps_to_leaf = u64::MAX;
        }
        r.d[0].n_steps_to_leaf = 0;
        relax_from_leaf(&mut r, 0, &adjacency, &offsets, &mut Vec::new());
        assert_eq!(r.d[count - 1].n_steps_to_leaf, (count - 1) as u64);
    }
}
