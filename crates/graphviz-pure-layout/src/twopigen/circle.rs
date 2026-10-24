//! `lib/twopigen/circle.c`: radial placement of one connected component.
//!
//! Per-node `rdata` lives in [`RData`], indexed by node like `ND_alg(n)`.

use crate::common::types::{LGraph, Sub};
use graphviz_pure_core::cnum::is_exactly_zero;
use graphviz_pure_core::math;
use graphviz_pure_core::strtod::strtod_rest;
use std::collections::VecDeque;

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
}

impl RData {
    pub fn new(n: usize) -> Self {
        Self {
            d: vec![RNode::default(); n],
        }
    }
}

/// The other end of `e` seen from `n` (`agtail(ep) == n ? aghead(ep) : agtail(ep)`).
fn other(g: &LGraph<'_>, e: usize, n: usize) -> usize {
    let t = g.edges[e].tail;
    if t == n { g.edges[e].head } else { t }
}

/// `setNStepsToLeaf(g, n, prev)`.
fn set_n_steps_to_leaf(g: &LGraph<'_>, sg: &Sub, r: &mut RData, n: usize, prev: Option<usize>) {
    let nsteps = r.d[n].n_steps_to_leaf + 1;
    let edges: Vec<usize> = g.all_edges(sg, n).collect();
    for ep in edges {
        let next = other(g, ep, n);
        if prev == Some(next) {
            continue;
        }
        if nsteps < r.d[next].n_steps_to_leaf {
            r.d[next].n_steps_to_leaf = nsteps;
            set_n_steps_to_leaf(g, sg, r, next, Some(n));
        }
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
    for &n in &sg.nodes {
        if r.d[n].n_steps_to_leaf == 0 {
            set_n_steps_to_leaf(g, sg, r, n, None);
        }
    }
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
    let mut q = VecDeque::new();
    q.push_back(n);
    while let Some(n) = q.pop_front() {
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
                q.push_back(next);
            }
        }
    }
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
fn set_subtree_size(sg: &Sub, r: &mut RData) {
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
    set_subtree_size(sg, r);
    set_subtree_spans(g, sg, r, center);
    set_positions(g, sg, r, center);
    set_absolute_pos(g, sg, r, max_n_steps_to_center);
    Some(center)
}
