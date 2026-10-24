//! Port of `lib/common/ns.c`: network simplex (`rank`, `rank2`), generic over
//! the graph representation.
//!
//! C's `ns.c` works on any `graph_t` whose nodes carry `ND_in`/`ND_out` fast
//! edge lists, `ND_rank` and the network-simplex scratch fields, and whose
//! edges carry `ED_minlen`/`ED_weight`. Both dot (ranking and x coordinates)
//! and neato's orthogonal overlap removal (`constraint.c`) call it. The
//! [`NsGraph`] trait describes exactly that interface; dot implements it over
//! its fast graph (`dotgen::ns`), neato over a standalone [`NsData`] graph.
//!
//! Search order, tie-breaking and balancing modes follow C exactly. The
//! recursive C helpers (`rerank`, `tree_adjust`) are iterative here; they only
//! add a constant to every node of a subtree, so visiting order is irrelevant.
//!
//! C's `int` arithmetic on ranks, minimum lengths, weights and cut values has
//! no overflow check except `x_cutval`'s sum, which the port reports as C does
//! ([`OVERFLOW_EDGE_WEIGHT_SUM`]). Everywhere else overflow is undefined
//! behaviour in C and the port wraps (`wrapping_*`), as the reference build
//! does, so extreme `weight`, `minlen` or `nodesep` values reach C's checks
//! instead of panicking on the way.

use graphviz_pure_core::diag::{elapsed_sec, fmt_f, fprintf_stderr, start_timer, verbose_on};

pub const SEARCHSIZE: i32 = 30;

/// The node and edge fields `ns.c` reads and writes. Nodes and edges are
/// indices; `nlist` is `GD_nlist(g)` in order.
pub trait NsGraph {
    /// `GD_nlist(g)` walked via `ND_next`.
    fn nlist(&self) -> Vec<usize>;
    /// `agget(g, name)` on the graph being ranked.
    fn attr(&self, name: &str) -> Option<String>;
    /// `ND_node_type(n) == NORMAL`.
    fn is_normal(&self, n: usize) -> bool;
    /// Node name for the `GVDUMP` traces (dot only).
    fn dump_name(&self, _n: usize) -> String {
        String::new()
    }

    /// `agnameof(n)` in error messages: anonymous and virtual nodes print as `%0`.
    fn err_name(&self, _n: usize) -> String {
        "%0".to_string()
    }

    fn rank(&self, n: usize) -> i32;
    fn set_rank(&mut self, n: usize, r: i32);
    fn set_mark(&mut self, n: usize, m: bool);
    fn priority(&self, n: usize) -> i32;
    fn set_priority(&mut self, n: usize, p: i32);
    fn low(&self, n: usize) -> i32;
    fn set_low(&mut self, n: usize, v: i32);
    fn lim(&self, n: usize) -> i32;
    fn set_lim(&mut self, n: usize, v: i32);
    fn par(&self, n: usize) -> Option<usize>;
    fn set_par(&mut self, n: usize, e: Option<usize>);
    fn subtree(&self, n: usize) -> Option<usize>;
    fn set_subtree(&mut self, n: usize, s: Option<usize>);
    fn in_edges(&self, n: usize) -> &[usize];
    fn out_edges(&self, n: usize) -> &[usize];
    fn tree_in(&self, n: usize) -> &[usize];
    fn tree_out(&self, n: usize) -> &[usize];
    fn tree_in_mut(&mut self, n: usize) -> &mut Vec<usize>;
    fn tree_out_mut(&mut self, n: usize) -> &mut Vec<usize>;

    fn tail(&self, e: usize) -> usize;
    fn head(&self, e: usize) -> usize;
    fn minlen(&self, e: usize) -> i32;
    fn weight(&self, e: usize) -> i32;
    fn cutvalue(&self, e: usize) -> i32;
    fn set_cutvalue(&mut self, e: usize, v: i32);
    fn tree_index(&self, e: usize) -> i32;
    fn set_tree_index(&mut self, e: usize, v: i32);
}

/// A standalone graph for `ns.c` (the constraint graphs of `constraint.c`).
#[derive(Debug, Default, Clone)]
pub struct NsData {
    pub nodes: Vec<NsNode>,
    pub edges: Vec<NsEdge>,
    /// `GD_nlist` order.
    pub nlist: Vec<usize>,
}

#[derive(Debug, Default, Clone)]
pub struct NsNode {
    pub rank: i32,
    pub mark: bool,
    pub priority: i32,
    pub low: i32,
    pub lim: i32,
    pub par: Option<usize>,
    pub subtree: Option<usize>,
    pub in_: Vec<usize>,
    pub out: Vec<usize>,
    pub tree_in: Vec<usize>,
    pub tree_out: Vec<usize>,
}

#[derive(Debug, Default, Clone)]
pub struct NsEdge {
    pub tail: usize,
    pub head: usize,
    pub minlen: i32,
    pub weight: i32,
    pub cutvalue: i32,
    pub tree_index: i32,
}

impl NsGraph for NsData {
    fn nlist(&self) -> Vec<usize> {
        self.nlist.clone()
    }
    fn attr(&self, _name: &str) -> Option<String> {
        None
    }
    fn is_normal(&self, _n: usize) -> bool {
        true
    }
    fn rank(&self, n: usize) -> i32 {
        self.nodes[n].rank
    }
    fn set_rank(&mut self, n: usize, r: i32) {
        self.nodes[n].rank = r;
    }
    fn set_mark(&mut self, n: usize, m: bool) {
        self.nodes[n].mark = m;
    }
    fn priority(&self, n: usize) -> i32 {
        self.nodes[n].priority
    }
    fn set_priority(&mut self, n: usize, p: i32) {
        self.nodes[n].priority = p;
    }
    fn low(&self, n: usize) -> i32 {
        self.nodes[n].low
    }
    fn set_low(&mut self, n: usize, v: i32) {
        self.nodes[n].low = v;
    }
    fn lim(&self, n: usize) -> i32 {
        self.nodes[n].lim
    }
    fn set_lim(&mut self, n: usize, v: i32) {
        self.nodes[n].lim = v;
    }
    fn par(&self, n: usize) -> Option<usize> {
        self.nodes[n].par
    }
    fn set_par(&mut self, n: usize, e: Option<usize>) {
        self.nodes[n].par = e;
    }
    fn subtree(&self, n: usize) -> Option<usize> {
        self.nodes[n].subtree
    }
    fn set_subtree(&mut self, n: usize, s: Option<usize>) {
        self.nodes[n].subtree = s;
    }
    fn in_edges(&self, n: usize) -> &[usize] {
        &self.nodes[n].in_
    }
    fn out_edges(&self, n: usize) -> &[usize] {
        &self.nodes[n].out
    }
    fn tree_in(&self, n: usize) -> &[usize] {
        &self.nodes[n].tree_in
    }
    fn tree_out(&self, n: usize) -> &[usize] {
        &self.nodes[n].tree_out
    }
    fn tree_in_mut(&mut self, n: usize) -> &mut Vec<usize> {
        &mut self.nodes[n].tree_in
    }
    fn tree_out_mut(&mut self, n: usize) -> &mut Vec<usize> {
        &mut self.nodes[n].tree_out
    }
    fn tail(&self, e: usize) -> usize {
        self.edges[e].tail
    }
    fn head(&self, e: usize) -> usize {
        self.edges[e].head
    }
    fn minlen(&self, e: usize) -> i32 {
        self.edges[e].minlen
    }
    fn weight(&self, e: usize) -> i32 {
        self.edges[e].weight
    }
    fn cutvalue(&self, e: usize) -> i32 {
        self.edges[e].cutvalue
    }
    fn set_cutvalue(&mut self, e: usize, v: i32) {
        self.edges[e].cutvalue = v;
    }
    fn tree_index(&self, e: usize) -> i32 {
        self.edges[e].tree_index
    }
    fn set_tree_index(&mut self, e: usize, v: i32) {
        self.edges[e].tree_index = v;
    }
}

struct Subtree {
    rep: usize,
    size: i32,
    heap_index: usize,
    par: usize,
}

const NOT_ON_HEAP: usize = usize::MAX;

/// `network_simplex_ctx_t`.
struct Ctx {
    tree_edge: Vec<usize>,
    s_i: usize,
    search_size: i32,
    subtrees: Vec<Subtree>,
}

/// `zapinlist`: remove `e` from `l`, moving the last element into its slot.
fn zapinlist(l: &mut Vec<usize>, e: usize) {
    if let Some(i) = l.iter().position(|&x| x == e) {
        l.swap_remove(i);
    }
}

#[inline]
fn slack<G: NsGraph + ?Sized>(g: &G, e: usize) -> i32 {
    g.rank(g.head(e))
        .wrapping_sub(g.rank(g.tail(e)))
        .wrapping_sub(g.minlen(e))
}

#[inline]
fn tree_edge<G: NsGraph + ?Sized>(g: &G, e: usize) -> bool {
    g.tree_index(e) >= 0
}

#[inline]
fn seq<G: NsGraph + ?Sized>(g: &G, v: usize, lo: i32, li: i32) -> bool {
    let l = g.lim(v);
    lo <= l && l <= li
}

fn add_tree_edge<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, e: usize) -> i32 {
    if tree_edge(g, e) {
        graphviz_pure_core::diag::agerrorf("add_tree_edge: missing tree edge\n");
        return -1;
    }
    g.set_tree_index(e, ctx.tree_edge.len() as i32);
    ctx.tree_edge.push(e);
    let t = g.tail(e);
    g.set_mark(t, true);
    g.tree_out_mut(t).push(e);
    if g.out_edges(t).len() < g.tree_out(t).len() {
        graphviz_pure_core::diag::agerrorf("add_tree_edge: empty outedge list\n");
        return -1;
    }
    let h = g.head(e);
    g.set_mark(h, true);
    g.tree_in_mut(h).push(e);
    if g.in_edges(h).len() < g.tree_in(h).len() {
        graphviz_pure_core::diag::agerrorf("add_tree_edge: empty inedge list\n");
        return -1;
    }
    0
}

fn invalidate_path<G: NsGraph + ?Sized>(g: &mut G, lca: usize, mut to_node: usize) {
    loop {
        if g.low(to_node) == -1 {
            break;
        }
        g.set_low(to_node, -1);
        let Some(e) = g.par(to_node) else {
            break;
        };
        if g.lim(to_node) >= g.lim(lca) {
            if to_node != lca {
                graphviz_pure_core::diag::agerrorf("invalidate_path: skipped over LCA\n");
            }
            break;
        }
        let (t, h) = (g.tail(e), g.head(e));
        to_node = if g.lim(t) > g.lim(h) { t } else { h };
    }
}

fn exchange_tree_edges<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, e: usize, f: usize) {
    let ti = g.tree_index(e);
    g.set_tree_index(f, ti);
    ctx.tree_edge[ti as usize] = f;
    g.set_tree_index(e, -1);
    let t = g.tail(e);
    zapinlist(g.tree_out_mut(t), e);
    let h = g.head(e);
    zapinlist(g.tree_in_mut(h), e);
    let t = g.tail(f);
    g.tree_out_mut(t).push(f);
    let h = g.head(f);
    g.tree_in_mut(h).push(f);
}

fn init_rank<G: NsGraph + ?Sized>(g: &mut G) {
    let mut q = std::collections::VecDeque::new();
    for v in g.nlist() {
        if g.priority(v) == 0 {
            q.push_back(v);
        }
    }
    let mut ctr = 0usize;
    while let Some(v) = q.pop_front() {
        g.set_rank(v, 0);
        ctr += 1;
        for i in 0..g.in_edges(v).len() {
            let e = g.in_edges(v)[i];
            let r = g.rank(g.tail(e)).wrapping_add(g.minlen(e));
            if r > g.rank(v) {
                g.set_rank(v, r);
            }
        }
        for i in 0..g.out_edges(v).len() {
            let e = g.out_edges(v)[i];
            let h = g.head(e);
            let p = g.priority(h) - 1;
            g.set_priority(h, p);
            if p <= 0 {
                q.push_back(h);
            }
        }
    }
    let nlist = g.nlist();
    if ctr != nlist.len() {
        graphviz_pure_core::diag::agerrorf("trouble in init_rank\n");
        for v in nlist {
            if g.priority(v) != 0 {
                graphviz_pure_core::diag::agerr_prev(&format!(
                    "\t{} {}\n",
                    g.err_name(v),
                    g.priority(v)
                ));
            }
        }
    }
}

fn leave_edge<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx) -> Option<usize> {
    let mut rv: Option<usize> = None;
    let mut cnt = 0;
    let j = ctx.s_i;
    while ctx.s_i < ctx.tree_edge.len() {
        let f = ctx.tree_edge[ctx.s_i];
        if g.cutvalue(f) < 0 {
            match rv {
                Some(r) => {
                    if g.cutvalue(r) > g.cutvalue(f) {
                        rv = Some(f);
                    }
                }
                None => rv = Some(f),
            }
            cnt += 1;
            if cnt >= ctx.search_size {
                return rv;
            }
        }
        ctx.s_i += 1;
    }
    if j > 0 {
        ctx.s_i = 0;
        while ctx.s_i < j {
            let f = ctx.tree_edge[ctx.s_i];
            if g.cutvalue(f) < 0 {
                match rv {
                    Some(r) => {
                        if g.cutvalue(r) > g.cutvalue(f) {
                            rv = Some(f);
                        }
                    }
                    None => rv = Some(f),
                }
                cnt += 1;
                if cnt >= ctx.search_size {
                    return rv;
                }
            }
            ctx.s_i += 1;
        }
    }
    rv
}

fn dfs_enter_outedge<G: NsGraph + ?Sized>(g: &G, v: usize, low: i32, lim: i32) -> Option<usize> {
    let mut enter: Option<usize> = None;
    let mut slk = i32::MAX;
    let mut todo = vec![v];
    while let Some(v) = todo.pop() {
        for &e in g.out_edges(v) {
            let h = g.head(e);
            if !tree_edge(g, e) {
                if !seq(g, h, low, lim) {
                    let s = slack(g, e);
                    if s < slk || enter.is_none() {
                        enter = Some(e);
                        slk = s;
                    }
                }
            } else if g.lim(h) < g.lim(v) {
                todo.push(h);
            }
        }
        for &e in g.tree_in(v) {
            if slk <= 0 {
                break;
            }
            let t = g.tail(e);
            if g.lim(t) < g.lim(v) {
                todo.push(t);
            }
        }
    }
    enter
}

fn dfs_enter_inedge<G: NsGraph + ?Sized>(g: &G, v: usize, low: i32, lim: i32) -> Option<usize> {
    let mut enter: Option<usize> = None;
    let mut slk = i32::MAX;
    let mut todo = vec![v];
    while let Some(v) = todo.pop() {
        for &e in g.in_edges(v) {
            let t = g.tail(e);
            if !tree_edge(g, e) {
                if !seq(g, t, low, lim) {
                    let s = slack(g, e);
                    if s < slk || enter.is_none() {
                        enter = Some(e);
                        slk = s;
                    }
                }
            } else if g.lim(t) < g.lim(v) {
                todo.push(t);
            }
        }
        for &e in g.tree_out(v) {
            if slk <= 0 {
                break;
            }
            let h = g.head(e);
            if g.lim(h) < g.lim(v) {
                todo.push(h);
            }
        }
    }
    enter
}

fn enter_edge<G: NsGraph + ?Sized>(g: &G, e: usize) -> Option<usize> {
    let (t, h) = (g.tail(e), g.head(e));
    if g.lim(t) < g.lim(h) {
        dfs_enter_inedge(g, t, g.low(t), g.lim(t))
    } else {
        dfs_enter_outedge(g, h, g.low(h), g.lim(h))
    }
}

fn init_cutvalues<G: NsGraph + ?Sized>(g: &mut G) -> Result<(), String> {
    let first = g.nlist()[0];
    dfs_range_impl(g, first, None, 1, false);
    dfs_cutval(g, first, None)
}

fn tight_subtree_search<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, v: usize, st: usize) -> i32 {
    struct Tst {
        v: usize,
        in_i: usize,
        out_i: usize,
        rv: i32,
    }
    let mut rv = 1;
    g.set_subtree(v, Some(st));
    let mut todo = vec![Tst {
        v,
        in_i: 0,
        out_i: 0,
        rv: 1,
    }];
    while !todo.is_empty() {
        let mut updated = false;
        let top = todo.len() - 1;
        let tv = todo[top].v;
        while todo[top].in_i < g.in_edges(tv).len() {
            let e = g.in_edges(tv)[todo[top].in_i];
            if tree_edge(g, e) {
                todo[top].in_i += 1;
                continue;
            }
            let t = g.tail(e);
            if g.subtree(t).is_none() && slack(g, e) == 0 {
                if add_tree_edge(g, ctx, e) != 0 {
                    todo.pop();
                    if todo.is_empty() {
                        rv = -1;
                    } else {
                        todo.last_mut().unwrap().rv -= 1;
                    }
                } else {
                    todo[top].in_i += 1;
                    g.set_subtree(t, Some(st));
                    todo.push(Tst {
                        v: t,
                        in_i: 0,
                        out_i: 0,
                        rv: 1,
                    });
                }
                updated = true;
                break;
            }
            todo[top].in_i += 1;
        }
        if updated {
            continue;
        }
        while todo[top].out_i < g.out_edges(tv).len() {
            let e = g.out_edges(tv)[todo[top].out_i];
            if tree_edge(g, e) {
                todo[top].out_i += 1;
                continue;
            }
            let h = g.head(e);
            if g.subtree(h).is_none() && slack(g, e) == 0 {
                if add_tree_edge(g, ctx, e) != 0 {
                    todo.pop();
                    if todo.is_empty() {
                        rv = -1;
                    } else {
                        todo.last_mut().unwrap().rv -= 1;
                    }
                } else {
                    todo[top].out_i += 1;
                    g.set_subtree(h, Some(st));
                    todo.push(Tst {
                        v: h,
                        in_i: 0,
                        out_i: 0,
                        rv: 1,
                    });
                }
                updated = true;
                break;
            }
            todo[top].out_i += 1;
        }
        if updated {
            continue;
        }
        let last = todo.pop().unwrap();
        if todo.is_empty() {
            rv = last.rv;
        } else {
            todo.last_mut().unwrap().rv += last.rv;
        }
    }
    rv
}

fn find_tight_subtree<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, v: usize) -> Option<usize> {
    let st = ctx.subtrees.len();
    ctx.subtrees.push(Subtree {
        rep: v,
        size: 0,
        heap_index: NOT_ON_HEAP,
        par: st,
    });
    let size = tight_subtree_search(g, ctx, v, st);
    if size < 0 {
        return None;
    }
    ctx.subtrees[st].size = size;
    ctx.subtrees[st].par = st;
    Some(st)
}

fn st_set_find<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx, n0: usize) -> usize {
    let mut s0 = g.subtree(n0).unwrap();
    while ctx.subtrees[s0].par != s0 {
        let p = ctx.subtrees[s0].par;
        let pp = ctx.subtrees[p].par;
        ctx.subtrees[s0].par = pp;
        s0 = ctx.subtrees[s0].par;
    }
    s0
}

fn st_set_union(ctx: &mut Ctx, s0: usize, s1: usize) -> usize {
    let mut r0 = s0;
    while ctx.subtrees[r0].par != r0 {
        r0 = ctx.subtrees[r0].par;
    }
    let mut r1 = s1;
    while ctx.subtrees[r1].par != r1 {
        r1 = ctx.subtrees[r1].par;
    }
    if r0 == r1 {
        return r0;
    }
    let on0 = ctx.subtrees[r0].heap_index != NOT_ON_HEAP;
    let on1 = ctx.subtrees[r1].heap_index != NOT_ON_HEAP;
    let r = if !on1 {
        r0
    } else if !on0 {
        r1
    } else if ctx.subtrees[r1].size < ctx.subtrees[r0].size {
        r0
    } else {
        r1
    };
    ctx.subtrees[r0].par = r;
    ctx.subtrees[r1].par = r;
    ctx.subtrees[r].size = ctx.subtrees[r0].size + ctx.subtrees[r1].size;
    r
}

fn inter_tree_edge_search<G: NsGraph + ?Sized>(g: &G, ctx: &mut Ctx, v: usize) -> Option<usize> {
    struct St {
        v: usize,
        ts: usize,
        from: Option<usize>,
        out_i: usize,
        in_i: usize,
    }
    let ts0 = st_set_find(g, ctx, v);
    let mut todo = vec![St {
        v,
        ts: ts0,
        from: None,
        out_i: 0,
        in_i: 0,
    }];
    let mut best: Option<usize> = None;
    while !todo.is_empty() {
        let top = todo.len() - 1;
        if todo[top].out_i == 0 && todo[top].in_i == 0 && best.is_some_and(|b| slack(g, b) == 0) {
            todo.pop();
            continue;
        }
        let sv = todo[top].v;
        let mut updated = false;
        while todo[top].out_i < g.out_edges(sv).len() {
            let e = g.out_edges(sv)[todo[top].out_i];
            let h = g.head(e);
            if tree_edge(g, e) {
                if Some(h) == todo[top].from {
                    todo[top].out_i += 1;
                    continue;
                }
                todo[top].out_i += 1;
                let ts = st_set_find(g, ctx, h);
                todo.push(St {
                    v: h,
                    ts,
                    from: Some(sv),
                    out_i: 0,
                    in_i: 0,
                });
                updated = true;
                break;
            } else {
                let hs = st_set_find(g, ctx, h);
                if hs != todo[top].ts && (best.is_none() || slack(g, e) < slack(g, best.unwrap())) {
                    best = Some(e);
                }
            }
            todo[top].out_i += 1;
        }
        if updated {
            continue;
        }
        while todo[top].in_i < g.in_edges(sv).len() {
            let e = g.in_edges(sv)[todo[top].in_i];
            let t = g.tail(e);
            if tree_edge(g, e) {
                if Some(t) == todo[top].from {
                    todo[top].in_i += 1;
                    continue;
                }
                todo[top].in_i += 1;
                let ts = st_set_find(g, ctx, t);
                todo.push(St {
                    v: t,
                    ts,
                    from: Some(sv),
                    out_i: 0,
                    in_i: 0,
                });
                updated = true;
                break;
            } else {
                let tsf = st_set_find(g, ctx, t);
                if tsf != todo[top].ts && (best.is_none() || slack(g, e) < slack(g, best.unwrap()))
                {
                    best = Some(e);
                }
            }
            todo[top].in_i += 1;
        }
        if updated {
            continue;
        }
        todo.pop();
    }
    best
}

fn st_heapify(ctx: &mut Ctx, heap: &mut [usize], size: usize, mut i: usize) {
    loop {
        let left = 2 * (i + 1) - 1;
        let right = 2 * (i + 1);
        let mut smallest = i;
        if left < size && ctx.subtrees[heap[left]].size < ctx.subtrees[heap[smallest]].size {
            smallest = left;
        }
        if right < size && ctx.subtrees[heap[right]].size < ctx.subtrees[heap[smallest]].size {
            smallest = right;
        }
        if smallest != i {
            heap.swap(i, smallest);
            ctx.subtrees[heap[i]].heap_index = i;
            ctx.subtrees[heap[smallest]].heap_index = smallest;
            i = smallest;
        } else {
            break;
        }
        if i >= size {
            break;
        }
    }
}

fn tree_adjust<G: NsGraph + ?Sized>(g: &mut G, v: usize, delta: i32) {
    let mut stack: Vec<(usize, Option<usize>)> = vec![(v, None)];
    while let Some((v, from)) = stack.pop() {
        let r = g.rank(v).wrapping_add(delta);
        g.set_rank(v, r);
        for i in 0..g.tree_in(v).len() {
            let w = g.tail(g.tree_in(v)[i]);
            if Some(w) != from {
                stack.push((w, Some(v)));
            }
        }
        for i in 0..g.tree_out(v).len() {
            let w = g.head(g.tree_out(v)[i]);
            if Some(w) != from {
                stack.push((w, Some(v)));
            }
        }
    }
}

fn merge_trees<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, e: usize) -> Option<usize> {
    let t0 = st_set_find(g, ctx, g.tail(e));
    let t1 = st_set_find(g, ctx, g.head(e));
    if ctx.subtrees[t0].heap_index == NOT_ON_HEAP {
        let delta = slack(g, e);
        if delta != 0 {
            tree_adjust(g, ctx.subtrees[t0].rep, delta);
        }
    } else {
        let delta = slack(g, e).wrapping_neg();
        if delta != 0 {
            tree_adjust(g, ctx.subtrees[t1].rep, delta);
        }
    }
    if add_tree_edge(g, ctx, e) != 0 {
        return None;
    }
    Some(st_set_union(ctx, t0, t1))
}

fn feasible_tree<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx) -> Result<i32, String> {
    let nl = g.nlist();
    for &n in &nl {
        g.set_subtree(n, None);
    }
    let mut tree: Vec<usize> = Vec::new();
    for &n in &nl {
        if g.subtree(n).is_none() {
            match find_tight_subtree(g, ctx, n) {
                Some(t) => tree.push(t),
                None => return Ok(2),
            }
        }
    }
    // STbuildheap
    let mut size = tree.len();
    for (i, &t) in tree.iter().enumerate() {
        ctx.subtrees[t].heap_index = i;
    }
    let mut i = size / 2;
    loop {
        st_heapify(ctx, &mut tree, size, i);
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let mut error = 0;
    while size > 1 {
        // STextractmin
        let rv = tree[0];
        ctx.subtrees[rv].heap_index = NOT_ON_HEAP;
        tree[0] = tree[size - 1];
        ctx.subtrees[tree[0]].heap_index = 0;
        tree[size - 1] = rv;
        size -= 1;
        st_heapify(ctx, &mut tree, size, 0);
        let rep = ctx.subtrees[rv].rep;
        let Some(ee) = inter_tree_edge_search(g, ctx, rep) else {
            error = 1;
            break;
        };
        let Some(t1) = merge_trees(g, ctx, ee) else {
            error = 2;
            break;
        };
        let hi = ctx.subtrees[t1].heap_index;
        st_heapify(ctx, &mut tree, size, hi);
    }
    for &n in &nl {
        g.set_subtree(n, None);
    }
    if error != 0 {
        return Ok(error);
    }
    init_cutvalues(g)?;
    Ok(0)
}

fn treeupdate<G: NsGraph + ?Sized>(
    g: &mut G,
    mut v: usize,
    w: usize,
    cutvalue: i32,
    dir: bool,
) -> usize {
    loop {
        if seq(g, w, g.low(v), g.lim(v)) {
            break;
        }
        let e = g.par(v).unwrap();
        let d = if v == g.tail(e) { dir } else { !dir };
        let cv = g.cutvalue(e);
        if d {
            g.set_cutvalue(e, cv.wrapping_add(cutvalue));
        } else {
            g.set_cutvalue(e, cv.wrapping_sub(cutvalue));
        }
        let (t, h) = (g.tail(e), g.head(e));
        v = if g.lim(t) > g.lim(h) { t } else { h };
    }
    v
}

fn rerank<G: NsGraph + ?Sized>(g: &mut G, v: usize, delta: i32) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        let r = g.rank(v).wrapping_sub(delta);
        g.set_rank(v, r);
        let par = g.par(v);
        for i in 0..g.tree_out(v).len() {
            let e = g.tree_out(v)[i];
            if Some(e) != par {
                stack.push(g.head(e));
            }
        }
        for i in 0..g.tree_in(v).len() {
            let e = g.tree_in(v)[i];
            if Some(e) != par {
                stack.push(g.tail(e));
            }
        }
    }
}

fn update<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx, e: usize, f: usize) -> i32 {
    let delta = slack(g, f);
    let (et, eh) = (g.tail(e), g.head(e));
    if delta > 0 {
        let s = g.tree_in(et).len() + g.tree_out(et).len();
        if s == 1 {
            rerank(g, et, delta);
        } else {
            let s = g.tree_in(eh).len() + g.tree_out(eh).len();
            if s == 1 {
                rerank(g, eh, delta.wrapping_neg());
            } else if g.lim(et) < g.lim(eh) {
                rerank(g, et, delta);
            } else {
                rerank(g, eh, delta.wrapping_neg());
            }
        }
    }
    let cutvalue = g.cutvalue(e);
    let (ft, fh) = (g.tail(f), g.head(f));
    let lca = treeupdate(g, ft, fh, cutvalue, true);
    if treeupdate(g, fh, ft, cutvalue, false) != lca {
        graphviz_pure_core::diag::agerrorf("update: mismatched lca in treeupdates\n");
        return 2;
    }
    let lca_low = g.low(lca);
    invalidate_path(g, lca, fh);
    invalidate_path(g, lca, ft);
    g.set_cutvalue(f, cutvalue.wrapping_neg());
    g.set_cutvalue(e, 0);
    exchange_tree_edges(g, ctx, e, f);
    let par = g.par(lca);
    dfs_range(g, lca, par, lca_low);
    0
}

fn scan_and_normalize<G: NsGraph + ?Sized>(g: &mut G) -> i32 {
    let mut minrank = i32::MAX;
    let mut maxrank = i32::MIN;
    let nl = g.nlist();
    for &n in &nl {
        if g.is_normal(n) {
            minrank = minrank.min(g.rank(n));
            maxrank = maxrank.max(g.rank(n));
        }
    }
    for &n in &nl {
        let r = g.rank(n).wrapping_sub(minrank);
        g.set_rank(n, r);
    }
    maxrank.wrapping_sub(minrank)
}

fn free_tree_list<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx) {
    for n in g.nlist() {
        g.tree_in_mut(n).clear();
        g.tree_out_mut(n).clear();
        g.set_mark(n, false);
    }
    ctx.tree_edge.clear();
}

fn lr_balance<G: NsGraph + ?Sized>(g: &mut G, ctx: &mut Ctx) {
    for i in 0..ctx.tree_edge.len() {
        let e = ctx.tree_edge[i];
        if g.cutvalue(e) == 0 {
            let Some(f) = enter_edge(g, e) else {
                continue;
            };
            let delta = slack(g, f);
            if delta <= 1 {
                continue;
            }
            let (t, h) = (g.tail(e), g.head(e));
            if g.lim(t) < g.lim(h) {
                rerank(g, t, delta / 2);
            } else {
                rerank(g, h, delta.wrapping_neg() / 2);
            }
        }
    }
    free_tree_list(g, ctx);
}

fn tb_balance<G: NsGraph + ?Sized>(g: &mut G) -> Result<(), String> {
    let maxrank = scan_and_normalize(g);
    // C asserts `Maxrank >= 0` (ns.c:823); ranks that wrapped past INT_MAX fail it.
    if maxrank < 0 {
        return Err(RANKS_OVERFLOWED.to_string());
    }
    let mut adj = 0;
    if let Some(s) = g.attr("TBbalance") {
        if s == "min" {
            adj = 1;
        } else if s == "max" {
            adj = 2;
        }
        if adj != 0 {
            for n in g.nlist() {
                if g.is_normal(n) {
                    if g.in_edges(n).is_empty() && adj == 1 {
                        g.set_rank(n, 0);
                    }
                    if g.out_edges(n).is_empty() && adj == 2 {
                        g.set_rank(n, maxrank);
                    }
                }
            }
        }
    }
    let nsz = (maxrank.max(0) as usize) + 1;
    let mut nrank = vec![0i32; nsz];
    let mut tree_node = g.nlist();
    let dump = std::env::var_os("GVDUMP").is_some();
    let dump_line = |g: &G, tag: &str, nodes: &[usize]| {
        let mut l = String::from(tag);
        for &n in nodes {
            let nm = if g.is_normal(n) {
                g.dump_name(n)
            } else {
                "_".to_string()
            };
            l.push_str(&format!(" {}:{}", nm, g.rank(n)));
        }
        graphviz_pure_core::diag::dump(&l);
    };
    if dump {
        dump_line(g, "TBPRE", &tree_node);
    }
    if adj > 1 {
        crate::cutil::qsort_by(&mut tree_node, |a, b| g.rank(*b).cmp(&g.rank(*a)) as i32);
    } else {
        crate::cutil::qsort_by(&mut tree_node, |a, b| g.rank(*a).cmp(&g.rank(*b)) as i32);
    }
    for &n in &tree_node {
        if g.is_normal(n) {
            let r = g.rank(n);
            if r >= 0 && (r as usize) < nsz {
                nrank[r as usize] += 1;
            }
        }
    }
    if dump {
        dump_line(g, "TBORDER", &tree_node);
    }
    for &n in &tree_node {
        if !g.is_normal(n) {
            continue;
        }
        let mut inweight: i32 = 0;
        let mut outweight: i32 = 0;
        let mut low = 0;
        let mut high = maxrank;
        for &e in g.in_edges(n) {
            inweight = inweight.wrapping_add(g.weight(e));
            low = low.max(g.rank(g.tail(e)).wrapping_add(g.minlen(e)));
        }
        for &e in g.out_edges(n) {
            outweight = outweight.wrapping_add(g.weight(e));
            high = high.min(g.rank(g.head(e)).wrapping_sub(g.minlen(e)));
        }
        if low < 0 {
            low = 0;
        }
        if adj != 0 {
            if inweight == outweight {
                g.set_rank(n, if adj == 1 { low } else { high });
            }
        } else if inweight == outweight {
            let get = |nr: &Vec<i32>, i: i32| -> i32 {
                if i >= 0 && (i as usize) < nr.len() {
                    nr[i as usize]
                } else {
                    0
                }
            };
            let mut choice = low;
            // C's `int i = low + 1` wraps past INT_MAX and reads `nrank[INT_MIN]` out of
            // bounds (it crashes); the port has no rank above `low` to choose.
            for i in low.saturating_add(1)..=high {
                if get(&nrank, i) < get(&nrank, choice) {
                    choice = i;
                }
            }
            let r = g.rank(n);
            if r >= 0 && (r as usize) < nsz {
                nrank[r as usize] -= 1;
            }
            if choice >= 0 && (choice as usize) < nsz {
                nrank[choice as usize] += 1;
            }
            if dump {
                graphviz_pure_core::diag::dump(&format!(
                    "TB {} {} {} {} -> {}",
                    g.dump_name(n),
                    r,
                    low,
                    high,
                    choice
                ));
            }
            g.set_rank(n, choice);
        }
        g.tree_in_mut(n).clear();
        g.tree_out_mut(n).clear();
        g.set_mark(n, false);
    }
    Ok(())
}

fn init_graph<G: NsGraph + ?Sized>(g: &mut G) -> bool {
    let nl = g.nlist();
    for &n in &nl {
        g.set_mark(n, false);
    }
    let mut feasible = true;
    for &n in &nl {
        g.set_priority(n, 0);
        for i in 0..g.in_edges(n).len() {
            let e = g.in_edges(n)[i];
            let p = g.priority(n) + 1;
            g.set_priority(n, p);
            g.set_cutvalue(e, 0);
            g.set_tree_index(e, -1);
            if g.rank(g.head(e)).wrapping_sub(g.rank(g.tail(e))) < g.minlen(e) {
                feasible = false;
            }
        }
        g.tree_in_mut(n).clear();
        g.tree_out_mut(n).clear();
    }
    feasible
}

/// `rank2(g, balance, maxiter, search_size)`. `Ok` is C's return value; `Err` is where C
/// exits (`x_cutval`'s overflow check, with C's message), crashes ([`NO_ENTERING_EDGE`]) or
/// asserts ([`RANKS_OVERFLOWED`]).
pub fn rank2<G: NsGraph + ?Sized>(
    g: &mut G,
    balance: i32,
    maxiter: i32,
    search_size: i32,
) -> Result<i32, String> {
    let mut ctx = Ctx {
        tree_edge: Vec::new(),
        s_i: 0,
        search_size: SEARCHSIZE,
        subtrees: Vec::new(),
    };
    const NS: &str = "network simplex: ";
    // graphSize, and ctx.N_nodes and ctx.N_edges, which init_graph counts the same way
    let size = verbose_on().then(|| {
        let nlist = g.nlist();
        let ne: usize = nlist.iter().map(|&n| g.out_edges(n).len()).sum();
        fprintf_stderr(&format!(
            "{NS} {} nodes {ne} edges maxiter={maxiter} balance={balance}\n",
            nlist.len()
        ));
        start_timer();
        (nlist.len(), ne)
    });
    let done = |iter: i32| {
        if let Some((nn, ne)) = size {
            let nl = if iter >= 100 { "\n" } else { "" };
            fprintf_stderr(&format!(
                "{nl}{NS}{nn} nodes {ne} edges {iter} iter {} sec\n",
                fmt_f(elapsed_sec(), 2)
            ));
        }
    };
    if g.nlist().is_empty() {
        done(0);
        return Ok(0);
    }
    let feasible = init_graph(g);
    if !feasible {
        init_rank(g);
    }
    ctx.search_size = if search_size >= 0 {
        search_size
    } else {
        SEARCHSIZE
    };
    let err = feasible_tree(g, &mut ctx);
    if err != Ok(0) {
        free_tree_list(g, &mut ctx);
        return err;
    }
    if maxiter <= 0 {
        free_tree_list(g, &mut ctx);
        return Ok(0);
    }
    let mut iter = 0;
    while let Some(e) = leave_edge(g, &mut ctx) {
        // C passes a NULL entering edge to `update`, which dereferences it (`SLACK(f)`,
        // ns.c:710): C crashes. Weights that wrapped negative get here (#252). The port
        // fails the layout.
        let Some(f) = enter_edge(g, e) else {
            free_tree_list(g, &mut ctx);
            return Err(NO_ENTERING_EDGE.to_string());
        };
        let err = update(g, &mut ctx, e, f);
        if err != 0 {
            free_tree_list(g, &mut ctx);
            return Ok(err);
        }
        iter += 1;
        if size.is_some() && iter % 100 == 0 {
            let lead = if iter % 1000 == 100 { NS } else { "" };
            let nl = if iter % 1000 == 0 { "\n" } else { "" };
            fprintf_stderr(&format!("{lead}{iter} {nl}"));
        }
        if iter >= maxiter {
            break;
        }
    }
    match balance {
        1 => {
            let balanced = tb_balance(g);
            ctx.tree_edge.clear();
            balanced?;
        }
        2 => lr_balance(g, &mut ctx),
        _ => {
            scan_and_normalize(g);
            free_tree_list(g, &mut ctx);
        }
    }
    done(iter);
    Ok(0)
}

/// `rank(g, balance, maxiter)`: `rank2` with `searchsize` from the graph.
pub fn rank<G: NsGraph + ?Sized>(g: &mut G, balance: i32, maxiter: i32) -> Result<i32, String> {
    let search_size = match g.attr("searchsize") {
        Some(s) => crate::common::utils::atoi(&s) as i32,
        None => SEARCHSIZE,
    };
    rank2(g, balance, maxiter, search_size)
}

fn x_val<G: NsGraph + ?Sized>(g: &G, e: usize, v: usize, dir: i32) -> i32 {
    let other = if g.tail(e) == v { g.head(e) } else { g.tail(e) };
    let f;
    let mut rv;
    if !seq(g, other, g.low(v), g.lim(v)) {
        f = true;
        rv = g.weight(e);
    } else {
        f = false;
        rv = if tree_edge(g, e) { g.cutvalue(e) } else { 0 };
        // C has no check here (undefined behaviour on overflow); the port wraps, and
        // `x_cutval`'s checked sum is where C reports the overflow.
        rv = rv.wrapping_sub(g.weight(e));
    }
    let mut d = if dir > 0 {
        if g.head(e) == v { 1 } else { -1 }
    } else if g.tail(e) == v {
        1
    } else {
        -1
    };
    if f {
        d = -d;
    }
    if d < 0 {
        rv = rv.wrapping_neg();
    }
    rv
}

/// The port's error where C's `rank2` passes a NULL entering edge to `update` and crashes.
pub const NO_ENTERING_EDGE: &str =
    "network simplex: no edge can enter the tree for a tree edge with a negative cut value";

/// The port's error where C's `TB_balance` fails `assert(Maxrank >= 0)`.
pub const RANKS_OVERFLOWED: &str =
    "network simplex: the ranks overflowed, the highest rank is below the lowest";

/// C's `agerrorf` before `graphviz_exit(EXIT_FAILURE)` in `x_cutval`.
pub const OVERFLOW_EDGE_WEIGHT_SUM: &str = "overflow when computing edge weight sum";

/// `x_cutval(f)`. C checks the sum for overflow (`sadd_overflow`, ns.c:1060) and exits
/// with its error; the port fails with C's message.
fn x_cutval<G: NsGraph + ?Sized>(g: &mut G, f: usize) -> Result<(), String> {
    let ft = g.tail(f);
    let (v, dir) = if g.par(ft) == Some(f) {
        (ft, 1)
    } else {
        (g.head(f), -1)
    };
    let mut sum: i32 = 0;
    for &e in g.out_edges(v).iter().chain(g.in_edges(v)) {
        sum = sum
            .checked_add(x_val(g, e, v, dir))
            .ok_or_else(|| OVERFLOW_EDGE_WEIGHT_SUM.to_string())?;
    }
    g.set_cutvalue(f, sum);
    Ok(())
}

fn dfs_cutval<G: NsGraph + ?Sized>(g: &mut G, v: usize, par: Option<usize>) -> Result<(), String> {
    struct St {
        v: usize,
        par: Option<usize>,
        out_i: usize,
        in_i: usize,
    }
    let mut todo = vec![St {
        v,
        par,
        out_i: 0,
        in_i: 0,
    }];
    while !todo.is_empty() {
        let top = todo.len() - 1;
        let tv = todo[top].v;
        let mut updated = false;
        while todo[top].out_i < g.tree_out(tv).len() {
            let e = g.tree_out(tv)[todo[top].out_i];
            todo[top].out_i += 1;
            if Some(e) != todo[top].par {
                todo.push(St {
                    v: g.head(e),
                    par: Some(e),
                    out_i: 0,
                    in_i: 0,
                });
                updated = true;
                break;
            }
        }
        if updated {
            continue;
        }
        while todo[top].in_i < g.tree_in(tv).len() {
            let e = g.tree_in(tv)[todo[top].in_i];
            todo[top].in_i += 1;
            if Some(e) != todo[top].par {
                todo.push(St {
                    v: g.tail(e),
                    par: Some(e),
                    out_i: 0,
                    in_i: 0,
                });
                updated = true;
                break;
            }
        }
        if updated {
            continue;
        }
        if let Some(p) = todo[top].par {
            x_cutval(g, p)?;
        }
        todo.pop();
    }
    Ok(())
}

fn dfs_range<G: NsGraph + ?Sized>(g: &mut G, v: usize, par: Option<usize>, low: i32) -> i32 {
    if g.par(v) == par && g.low(v) == low {
        return g.lim(v) + 1;
    }
    dfs_range_impl(g, v, par, low, true)
}

fn dfs_range_impl<G: NsGraph + ?Sized>(
    g: &mut G,
    v: usize,
    par: Option<usize>,
    low: i32,
    incremental: bool,
) -> i32 {
    struct St {
        v: usize,
        par: Option<usize>,
        lim: i32,
        out_i: usize,
        in_i: usize,
    }
    let mut lim = 0;
    g.set_par(v, par);
    g.set_low(v, low);
    let mut todo = vec![St {
        v,
        par,
        lim: low,
        out_i: 0,
        in_i: 0,
    }];
    while !todo.is_empty() {
        let top = todo.len() - 1;
        let sv = todo[top].v;
        let mut pushed = false;
        while todo[top].out_i < g.tree_out(sv).len() {
            let e = g.tree_out(sv)[todo[top].out_i];
            todo[top].out_i += 1;
            if Some(e) != todo[top].par {
                let n = g.head(e);
                let slim = todo[top].lim;
                if incremental && g.par(n) == Some(e) && g.low(n) == slim {
                    todo[top].lim = g.lim(n) + 1;
                } else {
                    g.set_par(n, Some(e));
                    g.set_low(n, slim);
                    todo.push(St {
                        v: n,
                        par: Some(e),
                        lim: slim,
                        out_i: 0,
                        in_i: 0,
                    });
                }
                pushed = true;
                break;
            }
        }
        if pushed {
            continue;
        }
        while todo[top].in_i < g.tree_in(sv).len() {
            let e = g.tree_in(sv)[todo[top].in_i];
            todo[top].in_i += 1;
            if Some(e) != todo[top].par {
                let n = g.tail(e);
                let slim = todo[top].lim;
                if incremental && g.par(n) == Some(e) && g.low(n) == slim {
                    todo[top].lim = g.lim(n) + 1;
                } else {
                    g.set_par(n, Some(e));
                    g.set_low(n, slim);
                    todo.push(St {
                        v: n,
                        par: Some(e),
                        lim: slim,
                        out_i: 0,
                        in_i: 0,
                    });
                }
                pushed = true;
                break;
            }
        }
        if pushed {
            continue;
        }
        g.set_lim(sv, todo[top].lim);
        lim = todo[top].lim;
        todo.pop();
        if let Some(b) = todo.last_mut() {
            b.lim = lim + 1;
        }
    }
    lim + 1
}
