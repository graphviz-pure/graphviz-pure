//! Port of `lib/dotgen/mincross.c`: crossing minimisation.
//!
//! The rank structure is global (`Root` is always the dot root, graph 0):
//! cluster rank arrays are windows of the root arrays, see [`super::types::Rank`].

use super::Dot;
use super::qsort::qsort;
use super::types::*;
use super::utils::*;
use graphviz_pure_core::strtod::atof;
use std::collections::VecDeque;

const C_EE: i32 = 1;
const C_VS: i32 = 2;
const C_SS: i32 = 2;
const C_VV: i32 = 4;
const TABLE: [[i32; 3]; 3] = [[C_EE, C_EE, C_EE], [C_EE, C_SS, C_VS], [C_EE, C_VS, C_VV]];
const CONVERGENCE: f64 = 0.995;

/// Error of a dot phase after which C abandons the layout (dot_layout returns before
/// dotneato_postprocess); the message was already reported.
pub const ABORT_LAYOUT: &str = "\u{1}abort";

/// C's `agerrorf` before `graphviz_exit(EXIT_FAILURE)` in `virtual_weight`.
pub const OVERFLOW_VIRTUAL_WEIGHT: &str = "overflow when calculating virtual weight of edge";

impl Dot<'_> {
    // ---------------- checkLabelOrder / fixLabelOrder ----------------

    fn lg_idx(&self, n: N) -> i32 {
        self.nodes[self.nodes[n].rep.unwrap()].order
    }

    fn is_backedge(&self, e: E) -> bool {
        self.lg_idx(self.edges[e].head) > self.lg_idx(self.edges[e].tail)
    }

    fn find_source(&self, g: G, sg: G) -> Option<N> {
        self.nodes_of(sg)
            .into_iter()
            .find(|&n| self.agdegree(g, n, true, false) == 0)
    }

    fn topsort(&mut self, g: G, sg: G) -> Vec<N> {
        let mut arr = Vec::new();
        while let Some(n) = self.find_source(g, sg) {
            arr.push(self.nodes[n].rep.unwrap());
            self.agdelnode(sg, n);
            for e in self.out_edges(g, n) {
                self.agdeledge(g, e);
            }
        }
        arr
    }

    fn get_comp(&mut self, g: G, n: N, comp: G, indices: &mut Vec<i32>) -> i32 {
        let mut backedge = 0;
        self.nodes[n].state = 1;
        let k = self.agnnodes(comp);
        if indices.len() <= k {
            indices.resize(k + 1, 0);
        }
        indices[k] = self.lg_idx(n);
        self.agsubnode(comp, n);
        for e in self.out_edges(g, n) {
            if self.is_backedge(e) {
                backedge += 1;
            }
            let h = self.edges[e].head;
            if self.nodes[h].state == 0 {
                backedge += self.get_comp(g, h, comp, indices);
            }
        }
        for e in self.in_edges(g, n) {
            if self.is_backedge(e) {
                backedge += 1;
            }
            let t = self.edges[e].tail;
            if self.nodes[t].state == 0 {
                backedge += self.get_comp(g, t, comp, indices);
            }
        }
        backedge
    }

    fn fix_label_order(&mut self, lg: G, r: i32) {
        let mut have_backedge = false;
        let ns = self.nodes_of(lg);
        for (i, &n) in ns.iter().enumerate() {
            for &v in &ns[i + 1..] {
                if self.nodes[v].lim <= self.nodes[n].low {
                    have_backedge = true;
                    self.cg_edge(lg, v, n);
                } else if self.nodes[n].lim <= self.nodes[v].low {
                    self.cg_edge(lg, n, v);
                }
            }
        }
        if !have_backedge {
            return;
        }
        let sg = self.cg_subg(lg, "comp");
        let mut indices: Vec<i32> = vec![0; self.agnnodes(lg)];
        for n in self.nodes_of(lg) {
            if self.nodes[n].state != 0 || self.agdegree(lg, n, true, true) == 0 {
                continue;
            }
            if self.get_comp(lg, n, sg, &mut indices) != 0 {
                let arr = self.topsort(lg, sg);
                let cnt = arr.len();
                qsort(&mut indices[..cnt], &mut |a: &i32, b: &i32| a.cmp(b));
                for (i, &u) in arr.iter().enumerate() {
                    self.nodes[u].order = indices[i];
                    self.set_rv(0, r, indices[i], Some(u));
                }
            }
            for x in self.nodes_of(sg) {
                self.agdelnode(sg, x);
            }
        }
    }

    pub fn check_label_order(&mut self, g: G) {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let mut lg: Option<G> = None;
            for j in 0..self.rank_n(g, r) {
                let u = self.rv(g, r, j).unwrap();
                if self.nodes[u].alg.is_some() {
                    let l = match lg {
                        Some(l) => l,
                        None => {
                            let ng = self.cg_open("lg");
                            lg = Some(ng);
                            ng
                        }
                    };
                    let n = self.cg_node(l, &j.to_string());
                    let mut lo = self.nodes[self.edges[self.nodes[u].out[0]].head].order;
                    let mut hi = self.nodes[self.edges[self.nodes[u].out[1]].head].order;
                    if lo > hi {
                        std::mem::swap(&mut lo, &mut hi);
                    }
                    self.nodes[n].low = lo;
                    self.nodes[n].lim = hi;
                    self.nodes[n].rep = Some(u);
                    self.nodes[n].state = 0;
                }
            }
            if let Some(l) = lg
                && self.agnnodes(l) > 1
            {
                self.fix_label_order(l, r);
            }
        }
    }

    // ---------------- dot_mincross ----------------

    pub fn dot_mincross(&mut self) -> Result<(), String> {
        let g = 0;
        let mut i = 0;
        while i < self.graphs[g].clust.len() {
            let c = self.graphs[g].clust[i];
            if self.agfstnode(c).is_none() {
                graphviz_pure_core::diag::agwarningf("removing empty cluster\n");
                self.graphs[g].clust.remove(i);
            } else {
                i += 1;
            }
        }
        self.init_mincross(g)?;
        let mut nc: i64 = 0;
        // C's `goto done` on a failed pass runs cleanup2, which reports the crossings so far;
        // the port abandons the layout, so it reports them alone
        macro_rules! pass {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.report_crossings(g, nc);
                        return Err(e);
                    }
                }
            };
        }
        for comp in 0..self.graphs[g].comp.len() {
            self.init_mccomp(g, comp);
            nc += pass!(self.mincross(g, 0));
        }
        self.merge2(g);
        for c in self.graphs[g].clust.clone() {
            nc += pass!(self.mincross_clust(c));
        }
        if !self.graphs[g].clust.is_empty() {
            let rem = self.agget(g, "remincross");
            if rem.is_none() || mapbool(rem) {
                self.mark_lowclusters(g);
                self.remincross = true;
                nc = pass!(self.mincross(g, 2));
            }
        }
        self.cleanup2(g, nc, true)
    }

    /// cleanup2's `-v` line: the crossings and the time since init_mincross.
    fn report_crossings(&self, g: G, nc: i64) {
        graphviz_pure_core::verbose_msg!(
            "mincross {}: {nc} crossings, {} secs.\n",
            self.graph_name(g),
            graphviz_pure_core::diag::fmt_f(graphviz_pure_core::diag::elapsed_sec(), 2)
        );
    }

    fn init_mccomp(&mut self, g: G, c: usize) {
        self.graphs[g].nlist = self.graphs[g].comp[c];
        if c > 0 {
            for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
                let rk = self.rk_mut(g, r);
                rk.voff += rk.n as usize;
                rk.n = 0;
            }
        }
    }

    fn betweenclust(&self, mut e: E) -> bool {
        while let Some(o) = self.edges[e].to_orig {
            e = o;
        }
        self.nodes[self.edges[e].tail].clust != self.nodes[self.edges[e].head].clust
    }

    fn do_ordering_node(&mut self, g: G, n: N, outflag: bool) {
        if self.nodes[n].clust.is_some() {
            return;
        }
        let list = if outflag {
            self.nodes[n].out.clone()
        } else {
            self.nodes[n].in_.clone()
        };
        let mut sortlist: Vec<E> = list
            .into_iter()
            .filter(|&e| !self.betweenclust(e))
            .collect();
        if sortlist.len() <= 1 {
            return;
        }
        {
            let edges = &self.edges;
            qsort(&mut sortlist, &mut |a: &E, b: &E| {
                edges[*a].seq.cmp(&edges[*b].seq)
            });
        }
        for k in 1..sortlist.len() {
            let (e, f) = (sortlist[k - 1], sortlist[k]);
            let (u, v) = if outflag {
                (self.edges[e].head, self.edges[f].head)
            } else {
                (self.edges[e].tail, self.edges[f].tail)
            };
            if self.find_flat_edge(u, v).is_some() {
                return;
            }
            let fe = self.new_virtual_edge(u, v, None);
            self.edges[fe].edge_type = FLATORDER;
            self.flat_edge(g, fe);
        }
    }

    fn do_ordering(&mut self, g: G, outflag: bool) {
        for n in self.nodes_of(g) {
            self.do_ordering_node(g, n, outflag);
        }
    }

    fn do_ordering_for_nodes(&mut self, g: G) {
        for n in self.nodes_of(g) {
            let ord = self.node_attr(n, "ordering").map(str::to_string);
            if let Some(o) = ord {
                if o == "out" {
                    self.do_ordering_node(g, n, true);
                } else if o == "in" {
                    self.do_ordering_node(g, n, false);
                } else if !o.is_empty() {
                    graphviz_pure_core::diag::agerrorf(&format!(
                        "ordering '{o}' not recognized for node '{}'.\n",
                        self.nodes.get(n).map_or("", |d| d.name.as_str())
                    ));
                }
            }
        }
    }

    /// `G_ordering` exists when any graph sets `ordering`.
    fn g_ordering_declared(&self) -> bool {
        self.root_input.graph_attr_declared("ordering")
    }

    fn n_ordering_declared(&self) -> bool {
        self.node_attr_declared("ordering")
    }

    fn ordered_edges(&mut self, g: G) {
        let gdecl = self.g_ordering_declared();
        let ndecl = self.n_ordering_declared();
        if !gdecl && !ndecl {
            return;
        }
        if gdecl {
            let ordering = self.agget(g, "ordering").unwrap_or("").to_string();
            if ordering == "out" {
                self.do_ordering(g, true);
            } else if ordering == "in" {
                self.do_ordering(g, false);
            } else if !ordering.is_empty() {
                graphviz_pure_core::diag::agerrorf(&format!(
                    "ordering '{ordering}' not recognized.\n"
                ));
            }
        } else {
            for subg in self.graphs[g].subgs.clone() {
                if !self.is_a_cluster(subg) {
                    self.ordered_edges(subg);
                }
            }
            if ndecl {
                self.do_ordering_for_nodes(g);
            }
        }
    }

    fn mincross_clust(&mut self, g: G) -> Result<i64, String> {
        self.expand_cluster(g)?;
        self.ordered_edges(g);
        self.flat_breakcycles(g);
        self.flat_reorder(g)?;
        let mut nc = self.mincross(g, 2)?;
        for c in self.graphs[g].clust.clone() {
            nc += self.mincross_clust(c)?;
        }
        self.save_vlist(g);
        Ok(nc)
    }

    fn left2right(&self, g: G, v: N, w: N) -> bool {
        if !self.remincross {
            let (cv, cw) = (self.nodes[v].clust, self.nodes[w].clust);
            if cv != cw && cv.is_some() && cw.is_some() {
                if self.nodes[v].ranktype == CLUSTER && self.nodes[v].node_type == VIRTUAL {
                    return false;
                }
                if self.nodes[w].ranktype == CLUSTER && self.nodes[w].node_type == VIRTUAL {
                    return false;
                }
                return true;
            }
        } else if self.nodes[v].clust != self.nodes[w].clust {
            return true;
        }
        let Some(m) = &self.rk(g, self.nodes[v].rank).flat else {
            return false;
        };
        let (a, b) = if self.graphs[g].flip() {
            (w, v)
        } else {
            (v, w)
        };
        m.get(self.nodes[a].low as usize, self.nodes[b].low as usize)
    }

    fn in_cross(&self, v: N, w: N) -> i64 {
        let mut cross: i64 = 0;
        for &e2 in &self.nodes[w].in_ {
            let cnt = self.edges[e2].xpenalty as i64;
            let inv = self.nodes[self.edges[e2].tail].order;
            for &e1 in &self.nodes[v].in_ {
                let t = self.nodes[self.edges[e1].tail].order - inv;
                if t > 0 || (t == 0 && self.edges[e1].tail_port.p.x > self.edges[e2].tail_port.p.x)
                {
                    cross += self.edges[e1].xpenalty as i64 * cnt;
                }
            }
        }
        cross
    }

    fn out_cross(&self, v: N, w: N) -> i64 {
        let mut cross: i32 = 0;
        for &e2 in &self.nodes[w].out {
            let cnt = self.edges[e2].xpenalty;
            let inv = self.nodes[self.edges[e2].head].order;
            for &e1 in &self.nodes[v].out {
                let t = self.nodes[self.edges[e1].head].order - inv;
                if t > 0 || (t == 0 && self.edges[e1].head_port.p.x > self.edges[e2].head_port.p.x)
                {
                    cross = cross.wrapping_add(self.edges[e1].xpenalty.wrapping_mul(cnt));
                }
            }
        }
        cross as i64
    }

    fn exchange(&mut self, v: N, w: N) {
        let r = self.nodes[v].rank;
        let vi = self.nodes[v].order;
        let wi = self.nodes[w].order;
        self.nodes[v].order = wi;
        self.set_rv(0, r, wi, Some(v));
        self.nodes[w].order = vi;
        self.set_rv(0, r, vi, Some(w));
    }

    fn transpose_step(&mut self, g: G, r: i32, reverse: bool) -> i64 {
        let mut rv: i64 = 0;
        self.rk_mut(g, r).candidate = false;
        let mut i = 0;
        while i < self.rank_n(g, r) - 1 {
            let v = self.rv(g, r, i).unwrap();
            let w = self.rv(g, r, i + 1).unwrap();
            if self.left2right(g, v, w) {
                i += 1;
                continue;
            }
            let mut c0: i64 = 0;
            let mut c1: i64 = 0;
            if r > 0 {
                c0 += self.in_cross(v, w);
                c1 += self.in_cross(w, v);
            }
            if self.rank_n(g, r + 1) > 0 {
                c0 += self.out_cross(v, w);
                c1 += self.out_cross(w, v);
            }
            if c1 < c0 || (c0 > 0 && reverse && c1 == c0) {
                self.exchange(v, w);
                rv += c0 - c1;
                self.rk_mut(0, r).valid = false;
                self.rk_mut(g, r).candidate = true;
                if r > self.graphs[g].minrank {
                    self.rk_mut(0, r - 1).valid = false;
                    self.rk_mut(g, r - 1).candidate = true;
                }
                if r < self.graphs[g].maxrank {
                    self.rk_mut(0, r + 1).valid = false;
                    self.rk_mut(g, r + 1).candidate = true;
                }
            }
            i += 1;
        }
        rv
    }

    fn transpose(&mut self, g: G, reverse: bool) {
        let (minr, maxr) = (self.graphs[g].minrank, self.graphs[g].maxrank);
        for r in minr..=maxr {
            self.rk_mut(g, r).candidate = true;
        }
        loop {
            let mut delta: i64 = 0;
            for r in minr..=maxr {
                if self.rk(g, r).candidate {
                    delta += self.transpose_step(g, r, reverse);
                }
            }
            if delta < 1 {
                break;
            }
        }
    }

    fn mincross(&mut self, g: G, startpass: i32) -> Result<i64, String> {
        let endpass = 2;
        let mut maxthispass;
        let mut cur_cross;
        let mut best_cross;
        if startpass > 1 {
            cur_cross = self.ncross();
            best_cross = cur_cross;
            self.save_best(g);
        } else {
            cur_cross = i64::MAX;
            best_cross = i64::MAX;
        }
        let mut pass = startpass;
        while pass <= endpass {
            if pass <= 1 {
                maxthispass = 4.min(self.max_iter);
                if g == 0 {
                    self.build_ranks(g, pass)?;
                }
                if pass == 0 {
                    self.flat_breakcycles(g);
                }
                self.flat_reorder(g)?;
                cur_cross = self.ncross();
                if cur_cross <= best_cross {
                    self.save_best(g);
                    best_cross = cur_cross;
                }
            } else {
                maxthispass = self.max_iter;
                if cur_cross > best_cross {
                    self.restore_best(g);
                }
                cur_cross = best_cross;
            }
            let mut trying = 0;
            for iter in 0..maxthispass {
                graphviz_pure_core::verbose_msg!(
                    "mincross: pass {pass} iter {iter} trying {trying} cur_cross {cur_cross} \
                     best_cross {best_cross}\n"
                );
                let t = trying;
                trying += 1;
                if t >= self.min_quit {
                    break;
                }
                if cur_cross == 0 {
                    break;
                }
                self.mincross_step(g, iter);
                cur_cross = self.ncross();
                if cur_cross <= best_cross {
                    self.save_best(g);
                    if (cur_cross as f64) < CONVERGENCE * best_cross as f64 {
                        trying = 0;
                    }
                    best_cross = cur_cross;
                }
            }
            if cur_cross == 0 {
                break;
            }
            pass += 1;
        }
        if cur_cross > best_cross {
            self.restore_best(g);
        }
        if best_cross > 0 {
            self.transpose(g, false);
            best_cross = self.ncross();
        }
        Ok(best_cross)
    }

    fn restore_best(&mut self, g: G) {
        let (minr, maxr) = (self.graphs[g].minrank, self.graphs[g].maxrank);
        for r in minr..=maxr {
            for i in 0..self.rank_n(g, r) {
                let n = self.rv(g, r, i).unwrap();
                self.nodes[n].order = self.nodes[n].coord.x as i32;
            }
        }
        for r in minr..=maxr {
            self.rk_mut(0, r).valid = false;
            let cnt = self.rank_n(g, r) as usize;
            let (b, o) = (self.rk(g, r).vbuf, self.rk(g, r).voff);
            let mut slice: Vec<N> = self.bufs[b][o..o + cnt]
                .iter()
                .map(|x| x.unwrap())
                .collect();
            {
                let nodes = &self.nodes;
                qsort(&mut slice, &mut |a: &N, b: &N| {
                    nodes[*a].order.cmp(&nodes[*b].order)
                });
            }
            for (i, n) in slice.into_iter().enumerate() {
                self.bufs[b][o + i] = Some(n);
            }
        }
    }

    fn save_best(&mut self, g: G) {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            for i in 0..self.rank_n(g, r) {
                let n = self.rv(g, r, i).unwrap();
                self.nodes[n].coord.x = self.nodes[n].order as f64;
            }
        }
    }

    fn merge_components(&mut self, g: G) {
        if self.graphs[g].comp.len() <= 1 {
            return;
        }
        let mut u: Option<N> = None;
        for c in 0..self.graphs[g].comp.len() {
            let mut v = self.graphs[g].comp[c].unwrap();
            if let Some(uu) = u {
                self.nodes[uu].next = Some(v);
            }
            self.nodes[v].prev = u;
            while let Some(nx) = self.nodes[v].next {
                v = nx;
            }
            u = Some(v);
        }
        let first = self.graphs[g].comp[0];
        self.graphs[g].comp = vec![first];
        self.graphs[g].nlist = first;
        self.graphs[g].minrank = self.global_min_rank;
        self.graphs[g].maxrank = self.global_max_rank;
    }

    fn merge2(&mut self, g: G) {
        self.merge_components(g);
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let rk = self.rk_mut(g, r);
            rk.n = rk.an;
            rk.vbuf = rk.avbuf;
            rk.voff = 0;
            for i in 0..self.rank_n(g, r) {
                match self.rv(g, r, i) {
                    None => {
                        graphviz_pure_core::verbose_msg!(
                            "merge2: graph {}, rank {r} has only {i} < {} nodes\n",
                            self.graph_name(g),
                            self.rank_n(g, r)
                        );
                        self.rk_mut(g, r).n = i;
                        break;
                    }
                    Some(v) => self.nodes[v].order = i,
                }
            }
        }
    }

    fn cleanup2(&mut self, g: G, nc: i64, has_vlists: bool) -> Result<(), String> {
        if has_vlists {
            for c in self.graphs[g].clust.clone() {
                self.rec_reset_vlists(c)?;
            }
        }
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            for i in 0..self.rank_n(g, r) {
                let v = self.rv(g, r, i).unwrap();
                self.nodes[v].order = i;
                let mut j = 0;
                while j < self.nodes[v].flat_out.len() {
                    let e = self.nodes[v].flat_out[j];
                    if self.edges[e].edge_type == FLATORDER {
                        self.delete_flat_edge(e);
                    } else {
                        j += 1;
                    }
                }
            }
            // free_matrix leaves a dangling (non-NULL) pointer in C; flat_edges()
            // only tests it for NULL, so the matrix is kept.
        }
        self.report_crossings(g, nc);
        Ok(())
    }

    /// `neighbor(v, dir)`, with C's `assert(rv == 0 || (ND_order(rv) - ND_order(v)) * dir > 0)`
    /// as a layout error. It fails on a rank leader that `remove_rankleaders` freed, whose
    /// stale order no longer matches the rank it is read from (#358): without it the walk in
    /// `furthestnode` can come back to a node and never end.
    fn neighbor(&self, v: N, dir: i32) -> Result<Option<N>, String> {
        let nd = &self.nodes[v];
        let (r, o) = (nd.rank, nd.order);
        let rv = if dir < 0 {
            if o > 0 { self.rv(0, r, o - 1) } else { None }
        } else {
            self.rv(0, r, o + 1)
        };
        if let Some(un) = rv.and_then(|u| self.nodes.get(u)) {
            let uo = un.order;
            if (dir < 0 && uo >= o) || (dir > 0 && uo <= o) {
                return Err(format!(
                    "neighbor: on rank {r}, the node next to order {o} has order {uo} \
                     (C asserts or crashes here)"
                ));
            }
        }
        Ok(rv)
    }

    fn is_a_normal_node_of(&self, g: G, v: N) -> bool {
        self.nodes[v].node_type == NORMAL && self.agcontains_node(g, v)
    }

    fn is_a_vnode_of_an_edge_of(&self, g: G, v: N) -> bool {
        let nd = &self.nodes[v];
        if nd.node_type == VIRTUAL && nd.in_.len() == 1 && nd.out.len() == 1 {
            let mut e = nd.out[0];
            while self.edges[e].edge_type != NORMAL {
                match self.edges[e].to_orig {
                    Some(o) => e = o,
                    None => return false,
                }
            }
            if self.agcontains_edge(g, e) {
                self.agcontains_edge_hit(g, e);
                return true;
            }
        }
        false
    }

    fn inside_cluster(&self, g: G, v: N) -> bool {
        self.is_a_normal_node_of(g, v) || self.is_a_vnode_of_an_edge_of(g, v)
    }

    fn furthestnode(&self, g: G, v: N, dir: i32) -> Result<N, String> {
        let mut rv = v;
        let mut u = v;
        while let Some(x) = self.neighbor(u, dir)? {
            u = x;
            if self.is_a_normal_node_of(g, u) || self.is_a_vnode_of_an_edge_of(g, u) {
                rv = u;
            }
        }
        Ok(rv)
    }

    pub fn save_vlist(&mut self, g: G) {
        if !self.graphs[g].rankleader.is_empty() {
            for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
                let v0 = self.rv(g, r, 0);
                self.graphs[g].rankleader[r as usize] = v0;
            }
        }
    }

    pub fn rec_save_vlists(&mut self, g: G) {
        self.save_vlist(g);
        for c in self.graphs[g].clust.clone() {
            self.rec_save_vlists(c);
        }
    }

    pub fn rec_reset_vlists(&mut self, g: G) -> Result<(), String> {
        for c in self.graphs[g].clust.clone() {
            self.rec_reset_vlists(c)?;
        }
        if !self.graphs[g].rankleader.is_empty() {
            for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
                let Some(v) = self.graphs[g].rankleader[r as usize] else {
                    continue;
                };
                let u = self.furthestnode(g, v, -1)?;
                let w = self.furthestnode(g, v, 1)?;
                self.graphs[g].rankleader[r as usize] = Some(u);
                let (rb, ro) = (self.rk(0, r).vbuf, self.rk(0, r).voff);
                let (ou, ow) = (self.nodes[u].order, self.nodes[w].order);
                let rk = self.rk_mut(g, r);
                rk.vbuf = rb;
                rk.voff = ro + ou as usize;
                rk.n = ow - ou + 1;
            }
        }
        Ok(())
    }

    fn real_fill_ranks(&mut self, g: G, ranks: &mut Vec<bool>, mut sg: Option<G>) -> Option<G> {
        for c in self.graphs[g].clust.clone() {
            sg = self.real_fill_ranks(c, ranks, sg);
        }
        if g == 0 {
            return sg;
        }
        ranks.iter_mut().for_each(|b| *b = false);
        for n in self.nodes_of(g) {
            let r = self.nodes[n].rank as usize;
            if r < ranks.len() {
                ranks[r] = true;
            }
            for e in self.out_edges(g, n) {
                let mut i = self.nodes[n].rank + 1;
                while i <= self.nodes[self.edges[e].head].rank {
                    if (i as usize) < ranks.len() {
                        ranks[i as usize] = true;
                    }
                    i += 1;
                }
            }
        }
        for i in self.graphs[g].minrank..=self.graphs[g].maxrank {
            if !ranks.get(i as usize).copied().unwrap_or(false) {
                let s = match sg {
                    Some(s) => s,
                    None => {
                        let s = self.cg_subg(0, "_new_rank");
                        sg = Some(s);
                        s
                    }
                };
                let name = format!("%fill{}", self.nodes.len());
                let n = self.cg_new_node(s, &name);
                self.nodes[n].rank = i;
                self.nodes[n].lw = 0.5;
                self.nodes[n].rw = 0.5;
                self.nodes[n].ht = 1.0;
                self.nodes[n].uf_size = 1;
                self.agsubnode(g, n);
            }
        }
        sg
    }

    fn fill_ranks(&mut self, g: G) {
        let sz = (self.graphs[g].maxrank + 2).max(0) as usize;
        let mut rnks = vec![false; sz];
        self.real_fill_ranks(g, &mut rnks, None);
    }

    fn init_mincross(&mut self, g: G) -> Result<(), String> {
        if graphviz_pure_core::diag::verbose_on() {
            graphviz_pure_core::diag::start_timer();
        }
        // Network simplex normalises each component's ranks to start at 0, but returns
        // before that when `nslimit1` allows no iteration (`maxiter <= 0`, ns.c:984), and
        // newrank's `readout_levels` only shifts a single component's ranks down from above
        // 0. The root's ranks can then start below 0. C indexes its arrays with them from
        // here on: `fill_ranks`' bitarray asserts on the bound, `make_chain` takes rank -1 for
        // the label rank of an edge with no label and `label_vnode` reads its NULL label,
        // `allocate_ranks` writes before its arrays, and `build_skeleton` reads before a
        // cluster's rank leaders. C crashes there, or exits 1 where what it reads out of
        // bounds trips one of its own checks (#357). Ranks that start above 0 are kept: C
        // draws most of those.
        let minrank = self.graphs[g].minrank;
        if minrank < 0 {
            return Err(format!(
                "dot_mincross: the ranking of {} stopped before normalising its ranks, \
                 which start at {minrank}, below 0",
                self.graph_name(g)
            ));
        }
        self.remincross = false;
        self.mincross_options(g);
        if self.graphs[g].flags & NEW_RANK != 0 {
            self.fill_ranks(g);
        }
        self.class2(g)?;
        self.decompose(g, 1);
        self.allocate_ranks(g);
        self.ordered_edges(g);
        self.global_min_rank = self.graphs[g].minrank;
        self.global_max_rank = self.graphs[g].maxrank;
        Ok(())
    }

    pub fn flat_rev(&mut self, g: G, e: E) {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        let rev = self.nodes[h]
            .flat_out
            .iter()
            .copied()
            .find(|&r| self.edges[r].head == t);
        if let Some(rev) = rev {
            self.merge_oneway(e, rev);
            if self.edges[rev].edge_type == FLATORDER && self.edges[rev].to_orig.is_none() {
                self.edges[rev].to_orig = Some(e);
            }
            self.nodes[t].other.push(e);
        } else {
            let rev = self.new_virtual_edge(h, t, Some(e));
            self.edges[rev].edge_type = if self.edges[e].edge_type == FLATORDER {
                FLATORDER
            } else {
                REVERSED
            };
            self.edges[rev].label = self.edges[e].label;
            self.flat_edge(g, rev);
        }
    }

    fn flat_search(&mut self, g: G, v: N) {
        let hascl = !self.graphs[0].clust.is_empty();
        self.nodes[v].mark = 1;
        self.nodes[v].onstack = true;
        let mut stack: Vec<(N, usize)> = vec![(v, 0)];
        while let Some(&(cur, i)) = stack.last() {
            if i >= self.nodes[cur].flat_out.len() {
                self.nodes[cur].onstack = false;
                stack.pop();
                continue;
            }
            let e = self.nodes[cur].flat_out[i];
            let (et, eh) = (self.edges[e].tail, self.edges[e].head);
            if hascl && !(self.agcontains_node(g, et) && self.agcontains_node(g, eh)) {
                stack.last_mut().unwrap().1 += 1;
                continue;
            }
            if self.edges[e].weight == 0 {
                stack.last_mut().unwrap().1 += 1;
                continue;
            }
            let r = self.nodes[cur].rank;
            if self.nodes[eh].onstack {
                let (a, b) = (self.nodes[eh].low as usize, self.nodes[et].low as usize);
                if let Some(m) = self.rk_mut(g, r).flat.as_mut() {
                    m.set(a, b);
                }
                self.delete_flat_edge(e);
                // i-- ; i++ => same index
                if self.edges[e].edge_type == FLATORDER {
                    continue;
                }
                self.flat_rev(g, e);
            } else {
                let (a, b) = (self.nodes[et].low as usize, self.nodes[eh].low as usize);
                if let Some(m) = self.rk_mut(g, r).flat.as_mut() {
                    m.set(a, b);
                }
                stack.last_mut().unwrap().1 += 1;
                if self.nodes[eh].mark == 0 {
                    self.nodes[eh].mark = 1;
                    self.nodes[eh].onstack = true;
                    stack.push((eh, 0));
                }
            }
        }
    }

    pub fn flat_breakcycles(&mut self, g: G) {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let mut flat = false;
            let n = self.rank_n(g, r);
            for i in 0..n {
                let v = self.rv(g, r, i).unwrap();
                self.nodes[v].mark = 0;
                self.nodes[v].onstack = false;
                self.nodes[v].low = i;
                if !self.nodes[v].flat_out.is_empty() && !flat {
                    self.rk_mut(g, r).flat = Some(AdjMatrix::new(n as usize, n as usize));
                    flat = true;
                }
            }
            if flat {
                for i in 0..n {
                    let v = self.rv(g, r, i).unwrap();
                    if self.nodes[v].mark == 0 {
                        self.flat_search(g, v);
                    }
                }
            }
        }
    }

    pub fn allocate_ranks(&mut self, g: G) {
        let maxr = self.graphs[g].maxrank;
        let mut cn = vec![0i32; (maxr + 2).max(1) as usize];
        for n in self.nodes_of(g) {
            let r = self.nodes[n].rank;
            if r >= 0 && (r as usize) < cn.len() {
                cn[r as usize] += 1;
            }
            for e in self.out_edges(g, n) {
                let mut low = self.nodes[self.edges[e].tail].rank;
                let mut high = self.nodes[self.edges[e].head].rank;
                if low > high {
                    std::mem::swap(&mut low, &mut high);
                }
                for r in (low + 1)..high {
                    if r >= 0 && (r as usize) < cn.len() {
                        cn[r as usize] += 1;
                    }
                }
            }
        }
        self.graphs[g].rank = vec![Rank::default(); (maxr + 3).max(1) as usize];
        for r in self.graphs[g].minrank..=maxr {
            let sz = cn[r as usize] + 1;
            let b = self.new_buf(sz as usize);
            let rk = self.rk_mut(g, r);
            rk.an = sz;
            rk.n = sz;
            rk.avbuf = b;
            rk.vbuf = b;
            rk.voff = 0;
        }
    }

    /// `install_in_rank(g, n)`. Its failures make dot_mincross, and so the layout, give up
    /// ([`ABORT_LAYOUT`]).
    ///
    /// The checks are C's, in C's order. They are what stops dot when the ranks it was given
    /// do not fit the rank arrays built from them: a node of a cluster that is also in a
    /// rank set, or a cluster ranking that printed `trouble in init_rank` (issue #232). C
    /// fails the `n <= an` assertion after writing past the array; the port fails the
    /// layout before writing. Past the first component the store can still land one slot
    /// past the array before C's `>` check at line 1190 notices; C writes it too, and the
    /// port's `set_rv` grows the buffer, so both go on (issue #232's input A reports 1190
    /// on the next node).
    pub fn install_in_rank(&mut self, g: G, n: N) -> Result<(), String> {
        let r = self.nodes[n].rank;
        // C reads GD_rank(g)[r] before checking the range; the entries past the ranks
        // are zeroed.
        let (i, an) = self.graphs[g]
            .rank
            .get((r + 1).max(0) as usize)
            .map_or((0, 0), |rk| (rk.n, rk.an));
        let name = |d: &Self, n: N| d.node_agnameof(n).name.into_owned();
        if an <= 0 {
            // mincross.c:1157-1158: both names share agnameof's static buffer when
            // anonymous, so C prints the node's name twice.
            let [graph, node] = graphviz_pure_core::agnameof_buffer([
                self.graph_agnameof(g),
                self.node_agnameof(n),
            ]);
            graphviz_pure_core::diag::agerrorf(&format!(
                "install_in_rank, line 1158: {graph} {node} rank {r} i = {i} an = 0\n"
            ));
            return Err(ABORT_LAYOUT.to_string());
        }
        // assert(GD_rank(g)[r].n <= GD_rank(g)[r].an), after the store to v[i].
        if i >= an {
            return Err(format!(
                "install_in_rank: rank {r} of {} is full ({an} allocated), no slot for {}",
                self.graph_name(g),
                name(self, n)
            ));
        }
        self.set_rv(g, r, i, Some(n));
        self.nodes[n].order = i;
        self.rk_mut(g, r).n += 1;
        let root_an = self.graphs[0]
            .rank
            .get((r + 1) as usize)
            .map_or(0, |rk| rk.an);
        if i > root_an {
            graphviz_pure_core::diag::agerrorf(&format!(
                "install_in_rank, line 1179: ND_order({}) [{i}] > GD_rank(Root)[{r}].an \
                 [{root_an}]\n",
                name(self, n)
            ));
            return Err(ABORT_LAYOUT.to_string());
        }
        if r < self.graphs[g].minrank || r > self.graphs[g].maxrank {
            graphviz_pure_core::diag::agerrorf(&format!(
                "install_in_rank, line 1184: rank {r} not in rank range [{},{}]\n",
                self.graphs[g].minrank, self.graphs[g].maxrank
            ));
            return Err(ABORT_LAYOUT.to_string());
        }
        // GD_rank(g)[r].v + ND_order(n) > GD_rank(g)[r].av + GD_rank(Root)[r].an: v is av
        // advanced past the earlier components (init_mccomp).
        let rk = self.rk(g, r);
        if rk.vbuf == rk.avbuf && rk.voff as i64 + i64::from(i) > i64::from(root_an) {
            graphviz_pure_core::diag::agerrorf(&format!(
                "install_in_rank, line 1190: GD_rank(g)[{r}].v + ND_order({}) [{i}] > \
                 GD_rank(g)[{r}].av + GD_rank(Root)[{r}].an [{root_an}]\n",
                name(self, n)
            ));
            return Err(ABORT_LAYOUT.to_string());
        }
        Ok(())
    }

    pub fn build_ranks(&mut self, g: G, pass: i32) -> Result<(), String> {
        let mut nl = Vec::new();
        let mut n = self.graphs[g].nlist;
        while let Some(x) = n {
            nl.push(x);
            self.nodes[x].mark = 0;
            n = self.nodes[x].next;
        }
        for i in self.graphs[g].minrank..=self.graphs[g].maxrank {
            self.rk_mut(g, i).n = 0;
        }
        // `g != agroot(g)`: also true for a pack component's root.
        let walkbackwards = g != 0 || self.comp;
        if walkbackwards {
            nl.reverse();
        }
        let mut q: VecDeque<N> = VecDeque::new();
        for n in nl {
            let other = if pass == 0 {
                &self.nodes[n].in_
            } else {
                &self.nodes[n].out
            };
            if !other.is_empty() {
                continue;
            }
            if self.nodes[n].mark == 0 {
                self.nodes[n].mark = 1;
                q.push_back(n);
                while let Some(n0) = q.pop_front() {
                    if self.nodes[n0].ranktype != CLUSTER {
                        self.install_in_rank(g, n0)?;
                        self.enqueue_neighbors(&mut q, n0, pass);
                    } else {
                        self.install_cluster(g, n0, pass, &mut q)?;
                    }
                }
            }
        }
        for i in self.graphs[g].minrank..=self.graphs[g].maxrank {
            self.rk_mut(0, i).valid = false;
            let cnt = self.rank_n(g, i);
            if self.graphs[g].flip() && cnt > 0 {
                let num_nodes_1 = cnt - 1;
                let half = num_nodes_1 / 2;
                for j in 0..=half {
                    let a = self.rv(g, i, j).unwrap();
                    let b = self.rv(g, i, num_nodes_1 - j).unwrap();
                    self.exchange(a, b);
                }
            }
        }
        if g == 0 && self.ncross() > 0 {
            self.transpose(g, false);
        }
        Ok(())
    }

    pub fn enqueue_neighbors(&mut self, q: &mut VecDeque<N>, n0: N, pass: i32) {
        if pass == 0 {
            for i in 0..self.nodes[n0].out.len() {
                let e = self.nodes[n0].out[i];
                let h = self.edges[e].head;
                if self.nodes[h].mark == 0 {
                    self.nodes[h].mark = 1;
                    q.push_back(h);
                }
            }
        } else {
            for i in 0..self.nodes[n0].in_.len() {
                let e = self.nodes[n0].in_[i];
                let t = self.edges[e].tail;
                if self.nodes[t].mark == 0 {
                    self.nodes[t].mark = 1;
                    q.push_back(t);
                }
            }
        }
    }

    fn constraining_flat_edge(&self, g: G, e: E) -> bool {
        if self.edges[e].weight == 0 {
            return false;
        }
        if !self.inside_cluster(g, self.edges[e].tail) {
            return false;
        }
        if !self.inside_cluster(g, self.edges[e].head) {
            return false;
        }
        true
    }

    /// `postorder(g, v, list, r)`, with C's `assert(ND_rank(v) == r)` as a layout error.
    fn postorder(&mut self, g: G, v: N, list: &mut Vec<N>, r: i32) -> Result<(), String> {
        self.nodes[v].mark = 1;
        let mut stack: Vec<(N, usize)> = vec![(v, 0)];
        while let Some(&(cur, i)) = stack.last() {
            let nd = &self.nodes[cur];
            let (nout, cr) = (nd.flat_out.len(), nd.rank);
            if i < nout {
                stack.last_mut().unwrap().1 += 1;
                let e = self.nodes[cur].flat_out[i];
                if !self.constraining_flat_edge(g, e) {
                    continue;
                }
                let h = self.edges[e].head;
                if self.nodes[h].mark == 0 {
                    self.nodes[h].mark = 1;
                    stack.push((h, 0));
                }
            } else {
                if cr != r {
                    return Err(format!(
                        "flat_reorder: a flat edge on rank {r} leads to rank {cr} \
                         (C's assertion fails)"
                    ));
                }
                list.push(cur);
                stack.pop();
            }
        }
        Ok(())
    }

    /// `flat_reorder(g)`, with C's assertions as layout errors: `LIST_GET`'s bound when the
    /// topological sort holds fewer nodes than the rank, `postorder`'s rank, and that an edge
    /// it reverses is not constraining. After a cluster lost nodes to a rank set (#358), a
    /// cycle of constraining flat edges can be left on a rank: no node of it is without a
    /// flat in-edge, so the sort never reaches it. Without the bound, the rank was left half
    /// rewritten and `transpose` never ended. The last assertion fails on other routes too
    /// (newrank with `ordering=in`).
    pub fn flat_reorder(&mut self, g: G) -> Result<(), String> {
        if !self.graphs[g].has_flat_edges {
            return Ok(());
        }
        let flip = self.graphs[g].flip();
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let n = self.rank_n(g, r);
            if n == 0 {
                continue;
            }
            let base_order = self.nodes[self.rv(g, r, 0).unwrap()].order;
            for i in 0..n {
                let v = self.rv(g, r, i).unwrap();
                self.nodes[v].mark = 0;
            }
            let mut temprank: Vec<N> = Vec::new();
            for i in 0..n {
                let v = if flip {
                    self.rv(g, r, i).unwrap()
                } else {
                    self.rv(g, r, n - i - 1).unwrap()
                };
                let mut local_in_cnt = 0;
                let mut local_out_cnt = 0;
                for &fe in &self.nodes[v].flat_in {
                    if self.constraining_flat_edge(g, fe) {
                        local_in_cnt += 1;
                    }
                }
                for &fe in &self.nodes[v].flat_out {
                    if self.constraining_flat_edge(g, fe) {
                        local_out_cnt += 1;
                    }
                }
                if local_in_cnt == 0 && local_out_cnt == 0 {
                    temprank.push(v);
                } else if self.nodes[v].mark == 0 && local_in_cnt == 0 {
                    self.postorder(g, v, &mut temprank, r)?;
                }
            }
            if !temprank.is_empty() {
                if !flip {
                    temprank.reverse();
                }
                for i in 0..n {
                    let Some(&v) = temprank.get(i as usize) else {
                        return Err(format!(
                            "flat_reorder: rank {r} of {} has {n} nodes, but its flat-edge \
                             order holds {} (C's assertion fails)",
                            self.graph_name(g),
                            temprank.len()
                        ));
                    };
                    self.set_rv(g, r, i, Some(v));
                    self.nodes[v].order = i + base_order;
                }
                for i in 0..n {
                    let v = self.rv(g, r, i).unwrap();
                    let mut j = 0;
                    while j < self.nodes[v].flat_out.len() {
                        let e = self.nodes[v].flat_out[j];
                        let (ho, to) = (
                            self.nodes[self.edges[e].head].order,
                            self.nodes[self.edges[e].tail].order,
                        );
                        if (!flip && ho < to) || (flip && ho > to) {
                            if self.constraining_flat_edge(g, e) {
                                return Err(format!(
                                    "flat_reorder: a constraining flat edge on rank {r} \
                                     points against the order (C's assertion fails)"
                                ));
                            }
                            self.delete_flat_edge(e);
                            self.flat_rev(g, e);
                        } else {
                            j += 1;
                        }
                    }
                }
            }
            self.rk_mut(0, r).valid = false;
        }
        Ok(())
    }

    fn reorder(&mut self, g: G, r: i32, reverse: bool, hasfixed: bool) {
        let mut changed = 0;
        let n = self.rank_n(g, r);
        let mut ep = n;
        let mut nelt = n - 1;
        while nelt >= 0 {
            let mut lp = 0;
            while lp < ep {
                while lp < ep && self.nodes[self.rv(g, r, lp).unwrap()].mval < 0.0 {
                    lp += 1;
                }
                if lp >= ep {
                    break;
                }
                let mut sawclust = false;
                let mut muststay = false;
                let lpn = self.rv(g, r, lp).unwrap();
                let mut rp = lp + 1;
                while rp < ep {
                    let rpn = self.rv(g, r, rp).unwrap();
                    if sawclust && self.nodes[rpn].clust.is_some() {
                        rp += 1;
                        continue;
                    }
                    if self.left2right(g, lpn, rpn) {
                        muststay = true;
                        break;
                    }
                    if self.nodes[rpn].mval >= 0.0 {
                        break;
                    }
                    if self.nodes[rpn].clust.is_some() {
                        sawclust = true;
                    }
                    rp += 1;
                }
                if rp >= ep {
                    break;
                }
                if !muststay {
                    let rpn = self.rv(g, r, rp).unwrap();
                    let p1 = self.nodes[lpn].mval;
                    let p2 = self.nodes[rpn].mval;
                    if p1 > p2 || (p1 >= p2 && reverse) {
                        self.exchange(lpn, rpn);
                        changed += 1;
                    }
                }
                lp = rp;
            }
            if !hasfixed && !reverse {
                ep -= 1;
            }
            nelt -= 1;
        }
        if changed != 0 {
            self.rk_mut(0, r).valid = false;
            if r > 0 {
                self.rk_mut(0, r - 1).valid = false;
            }
        }
    }

    fn mincross_step(&mut self, g: G, pass: i32) {
        let reverse = pass % 4 < 2;
        let (first, last, dir);
        if pass % 2 == 0 {
            let mut f = self.graphs[g].minrank + 1;
            if self.graphs[g].minrank > self.graphs[0].minrank {
                f -= 1;
            }
            first = f;
            last = self.graphs[g].maxrank;
            dir = 1;
        } else {
            let mut f = self.graphs[g].maxrank - 1;
            last = self.graphs[g].minrank;
            if self.graphs[g].maxrank < self.graphs[0].maxrank {
                f += 1;
            }
            first = f;
            dir = -1;
        }
        let mut r = first;
        while r != last + dir {
            let other = r - dir;
            let hasfixed = self.medians(g, r, other);
            self.reorder(g, r, reverse, hasfixed);
            r += dir;
        }
        self.transpose(g, !reverse);
    }

    fn local_cross(&self, l: &[E], dir: i32) -> i64 {
        let mut cross: i32 = 0;
        let is_out = dir > 0;
        for i in 0..l.len() {
            let e = l[i];
            for &f in &l[i + 1..] {
                let (ef, ee) = (&self.edges[f], &self.edges[e]);
                if is_out {
                    let a = (self.nodes[ef.head].order - self.nodes[ee.head].order) as f64;
                    if a * (ef.tail_port.p.x - ee.tail_port.p.x) < 0.0 {
                        cross += ee.xpenalty * ef.xpenalty;
                    }
                } else {
                    let a = (self.nodes[ef.tail].order - self.nodes[ee.tail].order) as f64;
                    if a * (ef.head_port.p.x - ee.head_port.p.x) < 0.0 {
                        cross += ee.xpenalty * ef.xpenalty;
                    }
                }
            }
        }
        cross as i64
    }

    fn rcross(&self, g: G, r: i32) -> i64 {
        let mut max = 0;
        let mut cross: i64 = 0;
        let cnt_len = (self.rank_n(0, r + 1) + 1).max(1) as usize;
        let mut count = vec![0i64; cnt_len];
        let n = self.rank_n(g, r);
        for top in 0..n {
            let v = self.rv(g, r, top).unwrap();
            if max > 0 {
                for &e in &self.nodes[v].out {
                    let o = self.nodes[self.edges[e].head].order;
                    for k in (o + 1)..=max {
                        cross += count.get(k as usize).copied().unwrap_or(0)
                            * self.edges[e].xpenalty as i64;
                    }
                }
            }
            for &e in &self.nodes[v].out {
                let inv = self.nodes[self.edges[e].head].order;
                if inv > max {
                    max = inv;
                }
                if inv >= 0 {
                    let iu = inv as usize;
                    if iu >= count.len() {
                        count.resize(iu + 1, 0);
                    }
                    count[iu] += self.edges[e].xpenalty as i64;
                }
            }
        }
        for top in 0..n {
            let v = self.rv(g, r, top).unwrap();
            if self.nodes[v].has_port {
                cross += self.local_cross(&self.nodes[v].out, 1);
            }
        }
        for bot in 0..self.rank_n(g, r + 1) {
            let v = self.rv(g, r + 1, bot).unwrap();
            if self.nodes[v].has_port {
                cross += self.local_cross(&self.nodes[v].in_, -1);
            }
        }
        cross
    }

    fn ncross(&mut self) -> i64 {
        let g = 0;
        let mut count: i64 = 0;
        let (minr, maxr) = (self.graphs[g].minrank, self.graphs[g].maxrank);
        for r in minr..maxr {
            if self.rk(g, r).valid {
                count += self.rk(g, r).cache_nc;
            } else {
                let nc = self.rcross(g, r);
                let rk = self.rk_mut(g, r);
                rk.cache_nc = nc;
                rk.valid = true;
                count += nc;
            }
        }
        count
    }

    fn flat_mval(&mut self, n: N) -> bool {
        if !self.nodes[n].flat_in.is_empty() {
            let fl = &self.nodes[n].flat_in;
            let mut nn = self.edges[fl[0]].tail;
            for &e in &fl[1..] {
                let t = self.edges[e].tail;
                if self.nodes[t].order > self.nodes[nn].order {
                    nn = t;
                }
            }
            if self.nodes[nn].mval >= 0.0 {
                self.nodes[n].mval = self.nodes[nn].mval + 1.0;
                return false;
            }
        } else if !self.nodes[n].flat_out.is_empty() {
            let fl = &self.nodes[n].flat_out;
            let mut nn = self.edges[fl[0]].head;
            for &e in &fl[1..] {
                let h = self.edges[e].head;
                if self.nodes[h].order < self.nodes[nn].order {
                    nn = h;
                }
            }
            if self.nodes[nn].mval > 0.0 {
                self.nodes[n].mval = self.nodes[nn].mval - 1.0;
                return false;
            }
        }
        true
    }

    fn medians(&mut self, g: G, r0: i32, r1: i32) -> bool {
        let mut hasfixed = false;
        let n = self.rank_n(g, r0);
        let mut list: Vec<i32> = Vec::new();
        for i in 0..n {
            let v = self.rv(g, r0, i).unwrap();
            list.clear();
            if r1 > r0 {
                for &e in &self.nodes[v].out {
                    if self.edges[e].xpenalty > 0 {
                        let h = self.edges[e].head;
                        list.push(
                            MC_SCALE * self.nodes[h].order + self.edges[e].head_port.order as i32,
                        );
                    }
                }
            } else {
                for &e in &self.nodes[v].in_ {
                    if self.edges[e].xpenalty > 0 {
                        let t = self.edges[e].tail;
                        list.push(
                            MC_SCALE * self.nodes[t].order + self.edges[e].tail_port.order as i32,
                        );
                    }
                }
            }
            let j = list.len();
            let mval = match j {
                0 => -1.0,
                1 => list[0] as f64,
                2 => ((list[0] + list[1]) / 2) as f64,
                _ => {
                    qsort(&mut list, &mut |a: &i32, b: &i32| a.cmp(b));
                    if j % 2 == 1 {
                        list[j / 2] as f64
                    } else {
                        let rm = j / 2;
                        let lm = rm - 1;
                        let rspan = list[j - 1] - list[rm];
                        let lspan = list[lm] - list[0];
                        if lspan == rspan {
                            ((list[lm] + list[rm]) / 2) as f64
                        } else {
                            // C (clang) fuses this multiply-add.
                            let w = (list[lm] as f64)
                                .mul_add(rspan as f64, list[rm] as f64 * lspan as f64);
                            w / (lspan + rspan) as f64
                        }
                    }
                }
            };
            self.nodes[v].mval = mval;
        }
        for i in 0..n {
            let v = self.rv(g, r0, i).unwrap();
            if self.nodes[v].out.is_empty() && self.nodes[v].in_.is_empty() {
                hasfixed |= self.flat_mval(v);
            }
        }
        hasfixed
    }

    fn endpoint_class(&self, n: N) -> usize {
        if self.nodes[n].node_type == VIRTUAL {
            return 2;
        }
        if self.nodes[n].weight_class <= 1 {
            return 1;
        }
        0
    }

    /// `virtual_weight(e)`. C checks the product for overflow (mincross.c:1725) and exits
    /// with its error; the port fails the layout with C's message.
    pub fn virtual_weight(&mut self, e: E) -> Result<(), String> {
        let t =
            TABLE[self.endpoint_class(self.edges[e].tail)][self.endpoint_class(self.edges[e].head)];
        let w = &mut self.edges[e].weight;
        if i32::MAX / t < *w {
            return Err(OVERFLOW_VIRTUAL_WEIGHT.to_string());
        }
        // A weight that already wrapped negative passes C's check and can overflow here,
        // unchecked (undefined behaviour); the port wraps.
        *w = w.wrapping_mul(t);
        Ok(())
    }

    fn mincross_options(&mut self, g: G) {
        self.min_quit = 8;
        self.max_iter = 24;
        if let Some(p) = self.agget(g, "mclimit") {
            let f = atof(p);
            if f > 0.0 {
                self.min_quit = 1.max(scale_clamp(self.min_quit, f));
                self.max_iter = 1.max(scale_clamp(self.max_iter, f));
            }
        }
    }
}
