//! Port of `lib/dotgen/position.c`: node coordinates. Y coordinates come from
//! rank heights; X coordinates from network simplex on an auxiliary graph of
//! separation constraints (`create_aux_edges`), whose ranks are the X values.

use super::Dot;
use super::types::*;
use super::utils::*;
use graphviz_pure_core::Point;
use graphviz_pure_core::cnum::{c_max, c_min};
use graphviz_pure_core::strtod::atof;

impl Dot<'_> {
    /// `make_aux_edge`
    pub fn make_aux_edge(&mut self, u: N, v: N, len: f64, wt: i32) -> E {
        let e = self.alloc_edge(u, v);
        // `largeMinlen`: C reports the clamp as an error and goes on.
        let len = if len > i32::MAX as f64 {
            graphviz_pure_core::diag::agerrorf(&format!(
                "Edge length {len:.6} larger than maximum {} allowed.\nCheck for overwide \
                 node(s).\n",
                i32::MAX
            ));
            i32::MAX as f64
        } else {
            len
        };
        self.edges[e].minlen = round_c(len);
        self.edges[e].weight = wt;
        self.fast_edge(e);
        e
    }

    fn margin_of(&self, g: G) -> i32 {
        late_int(self.agget(g, "margin"), CL_OFFSET, 0)
    }

    fn connect_graph(&mut self, g: G) {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let n = self.rank_n(g, r);
            let mut found = false;
            let mut tp: Option<N> = None;
            for i in 0..n {
                let t = self.rv(g, r, i).unwrap();
                tp = Some(t);
                if let Some(so) = &self.nodes[t].save_out {
                    for &e in so {
                        if self.nodes[self.edges[e].head].rank > r
                            || self.nodes[self.edges[e].tail].rank > r
                        {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        break;
                    }
                }
                if let Some(si) = &self.nodes[t].save_in {
                    for &e in si {
                        if self.nodes[self.edges[e].tail].rank > r
                            || self.nodes[self.edges[e].head].rank > r
                        {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        break;
                    }
                }
            }
            if found || tp.is_none() {
                continue;
            }
            let tp = self.rv(g, r, 0).unwrap();
            let hp = if r < self.graphs[g].maxrank {
                self.rv(g, r + 1, 0)
            } else {
                self.rv(g, r - 1, 0)
            };
            let Some(hp) = hp else { continue };
            let sn = self.virtual_node(g);
            self.nodes[sn].node_type = SLACKNODE;
            self.make_aux_edge(sn, tp, 0.0, 0);
            self.make_aux_edge(sn, hp, 0.0, 0);
            self.nodes[sn].rank = self.nodes[tp].rank.min(self.nodes[hp].rank);
        }
    }

    pub fn dot_position(&mut self) -> Result<(), String> {
        let g = 0;
        if self.graphs[g].nlist.is_none() {
            return Ok(());
        }
        self.mark_lowclusters(g);
        self.set_ycoords(g)?;
        if self.concentrate {
            // dot_position returns the failure: doDot gives up on the layout and
            // dot_layout skips dotneato_postprocess, leaving the drawing half made.
            if let Err(msg) = self.dot_concentrate() {
                graphviz_pure_core::diag::agerrorf(&format!("{msg}\n"));
                graphviz_pure_core::diag::agerr_prev("concentrate=true may not work correctly.\n");
                self.aborted = true;
                return Ok(());
            }
        }
        self.expand_leaves(g);
        if self.flat_edges(g)? {
            self.set_ycoords(g)?;
        }
        self.create_aux_edges(g);
        if std::env::var_os("GVDUMP").is_some() {
            let nl = self.nlist_nodes(g);
            for (i, &n) in nl.iter().enumerate() {
                self.nodes[n].low = i as i32;
            }
            for &n in &nl {
                let nm = if self.nodes[n].node_type == NORMAL {
                    self.nodes[n].name.as_str()
                } else {
                    "_"
                };
                let nd = &self.nodes[n];
                let mut l = format!(
                    "A {} {} r={} o={} t={} rw={} lw={}:",
                    nd.low, nm, nd.rank, nd.order, nd.node_type, nd.rw, nd.lw
                );
                for &e in &self.nodes[n].out {
                    let ed = &self.edges[e];
                    l.push_str(&format!(
                        " {}/{}/{}",
                        self.nodes[ed.head].low, ed.minlen, ed.weight
                    ));
                }
                graphviz_pure_core::diag::dump(&l);
            }
        }
        let it = self.nsiter2(g);
        if self.ns_rank(g, 2, it)? != 0 {
            self.connect_graph(g);
            let it = self.nsiter2(g);
            self.ns_rank(g, 2, it)?;
        }
        self.ns_scratch.clear();
        self.set_xcoords(g);
        self.set_aspect(g);
        self.remove_aux_edges(g);
        Ok(())
    }

    fn nsiter2(&self, g: G) -> i32 {
        match self.agget(g, "nslimit") {
            Some(s) => scale_clamp(self.agnnodes(g) as i32, atof(s)),
            None => i32::MAX,
        }
    }

    /// `canreach(u, v)`: reachability over `ND_out` (C `go` without memo).
    fn canreach(&self, u: N, v: N) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![u];
        while let Some(x) = stack.pop() {
            if x == v {
                return true;
            }
            if !seen.insert(x) {
                continue;
            }
            for &e in &self.nodes[x].out {
                stack.push(self.edges[e].head);
            }
        }
        false
    }

    fn nlist_nodes(&self, g: G) -> Vec<N> {
        let mut v = Vec::new();
        let mut n = self.graphs[g].nlist;
        while let Some(x) = n {
            v.push(x);
            n = self.nodes[x].next;
        }
        v
    }

    fn allocate_aux_edges(&mut self, g: G) {
        for n in self.nlist_nodes(g) {
            let nd = &mut self.nodes[n];
            nd.save_in = Some(std::mem::take(&mut nd.in_));
            nd.save_out = Some(std::mem::take(&mut nd.out));
        }
    }

    fn make_lr_constraints(&mut self, g: G) {
        let nodesep = self.graphs[g].nodesep;
        let sep = if self.graphs[0].has_labels & EDGE_LABEL != 0 {
            [nodesep, 5]
        } else {
            [nodesep, nodesep]
        };
        let flip = self.graphs[0].flip();
        for i in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let n = self.rank_n(g, i);
            if n == 0 {
                continue;
            }
            let v0 = self.rv(g, i, 0).unwrap();
            self.nodes[v0].rank = 0;
            let mut last: f64 = 0.0;
            let ns = sep[(i & 1) as usize];
            for j in 0..n {
                let u = self.rv(g, i, j).unwrap();
                self.nodes[u].mval = self.nodes[u].rw;
                if !self.nodes[u].other.is_empty() {
                    let mut sw = 0.0;
                    for k in 0..self.nodes[u].other.len() {
                        let e = self.nodes[u].other[k];
                        if self.edges[e].tail == self.edges[e].head {
                            sw += self.self_right_space(e, flip);
                        }
                    }
                    self.nodes[u].rw += sw;
                }
                if j + 1 < n {
                    let v = self.rv(g, i, j + 1).unwrap();
                    let width = self.nodes[u].rw + self.nodes[v].lw + ns as f64;
                    self.make_aux_edge(u, v, width, 0);
                    self.nodes[v].rank = (last + width) as i32;
                    last = self.nodes[v].rank as f64;
                }
                if let Some(e) = self.nodes[u].alg {
                    let so = self.nodes[u].save_out.clone().unwrap_or_default();
                    if so.len() >= 2 {
                        let (mut e0, mut e1) = (so[0], so[1]);
                        if self.nodes[self.edges[e0].head].order
                            > self.nodes[self.edges[e1].head].order
                        {
                            std::mem::swap(&mut e0, &mut e1);
                        }
                        // No overflow check in C (undefined behaviour); the port wraps.
                        let m0 = self.edges[e].minlen.wrapping_mul(nodesep) / 2;
                        let (t0, h0) = (self.edges[e0].tail, self.edges[e0].head);
                        let m1 = m0 as f64 + self.nodes[h0].rw + self.nodes[t0].lw;
                        if !self.canreach(t0, h0) {
                            let w = self.edges[e].weight;
                            self.make_aux_edge(h0, t0, m1, w);
                        }
                        let (t1, h1) = (self.edges[e1].tail, self.edges[e1].head);
                        let m1 = m0 as f64 + self.nodes[t1].rw + self.nodes[h1].lw;
                        if !self.canreach(h1, t1) {
                            let w = self.edges[e].weight;
                            self.make_aux_edge(t1, h1, m1, w);
                        }
                    }
                }
                for k in 0..self.nodes[u].flat_out.len() {
                    let e = self.nodes[u].flat_out[k];
                    let (et, eh) = (self.edges[e].tail, self.edges[e].head);
                    let (t0, h0) = if self.nodes[et].order < self.nodes[eh].order {
                        (et, eh)
                    } else {
                        (eh, et)
                    };
                    let width = self.nodes[t0].rw + self.nodes[h0].lw;
                    // No overflow check in C (undefined behaviour); the port wraps.
                    let mut m0 = (self.edges[e].minlen.wrapping_mul(nodesep) as f64 + width) as i32;
                    if let Some(e0) = self.find_fast_edge(t0, h0) {
                        let alt = width + nodesep as f64 + round_c(self.edges[e].dist) as f64;
                        m0 = c_max(m0 as f64, alt) as i32;
                        self.edges[e0].minlen = self.edges[e0].minlen.max(m0);
                        self.edges[e0].weight = self.edges[e0].weight.max(self.edges[e].weight);
                    } else if self.edges[e].label.is_none() {
                        let w = self.edges[e].weight;
                        self.make_aux_edge(t0, h0, m0 as f64, w);
                    }
                }
            }
        }
    }

    /// `selfRightSpace` (common/splines.c)
    pub fn self_right_space(&self, e: E, flip: bool) -> f64 {
        let ed = &self.edges[e];
        let (tp, hp) = (&ed.tail_port, &ed.head_port);
        if (!tp.defined && !hp.defined)
            || (tp.side & LEFT == 0
                && hp.side & LEFT == 0
                && (tp.side != hp.side || tp.side & (TOP | BOTTOM) == 0))
        {
            let mut sw = SELF_EDGE_SIZE;
            if let Some(l) = ed.label {
                let d = self.labels[l].dimen;
                sw += if flip { d.y } else { d.x };
            }
            sw
        } else {
            0.0
        }
    }

    fn make_edge_pairs(&mut self, g: G) {
        for n in self.nlist_nodes(g) {
            let Some(so) = self.nodes[n].save_out.clone() else {
                continue;
            };
            for e in so {
                let sn = self.virtual_node(g);
                self.nodes[sn].node_type = SLACKNODE;
                let mut m0 = (self.edges[e].head_port.p.x - self.edges[e].tail_port.p.x) as i32;
                let m1;
                if m0 > 0 {
                    m1 = 0;
                } else {
                    m1 = m0.wrapping_neg();
                    m0 = 0;
                }
                // No overflow check in C (undefined behaviour). The reference build converts
                // `m + 1` to `double` as `(double)m + 1`, which does not wrap: with
                // `node [margin="-2147483648"]` (attr-extremes' dot-margin-N-v2-small),
                // `largeMinlen` receives 2147483648. The rank below wraps.
                let (t, h, w) = (self.edges[e].tail, self.edges[e].head, self.edges[e].weight);
                self.make_aux_edge(sn, t, f64::from(m0) + 1.0, w);
                self.make_aux_edge(sn, h, f64::from(m1) + 1.0, w);
                let (rt, rh) = (self.nodes[t].rank, self.nodes[h].rank);
                self.nodes[sn].rank = rt
                    .wrapping_sub(m0)
                    .wrapping_sub(1)
                    .min(rh.wrapping_sub(m1).wrapping_sub(1));
            }
        }
    }

    fn contain_clustnodes(&mut self, g: G) {
        if g != 0 {
            self.contain_nodes(g);
            let (ln, rn) = (self.graphs[g].ln.unwrap(), self.graphs[g].rn.unwrap());
            if let Some(e) = self.find_fast_edge(ln, rn) {
                // No overflow check in C (undefined behaviour); the port wraps.
                let w = &mut self.edges[e].weight;
                *w = w.wrapping_add(128);
            } else {
                self.make_aux_edge(ln, rn, 1.0, 128);
            }
        }
        for c in self.graphs[g].clust.clone() {
            self.contain_clustnodes(c);
        }
    }

    fn vnode_not_related_to(&self, g: G, v: N) -> bool {
        if self.nodes[v].node_type != VIRTUAL {
            return false;
        }
        let Some(&first) = self.nodes[v].save_out.as_ref().and_then(|s| s.first()) else {
            return true;
        };
        let mut e = first;
        while let Some(o) = self.edges[e].to_orig {
            e = o;
        }
        if self.agcontains_node(g, self.edges[e].tail) {
            return false;
        }
        if self.agcontains_node(g, self.edges[e].head) {
            return false;
        }
        true
    }

    fn keepout_othernodes(&mut self, g: G) {
        let margin = self.margin_of(g);
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            if self.rank_n(g, r) == 0 {
                continue;
            }
            let Some(v) = self.rv(g, r, 0) else { continue };
            let mut i = self.nodes[v].order - 1;
            while i >= 0 {
                let u = self.rv(0, r, i).unwrap();
                if self.nodes[u].node_type == NORMAL || self.vnode_not_related_to(g, u) {
                    let ln = self.graphs[g].ln.unwrap();
                    let w = margin as f64 + self.nodes[u].rw;
                    self.make_aux_edge(u, ln, w, 0);
                    break;
                }
                i -= 1;
            }
            let mut i = self.nodes[v].order + self.rank_n(g, r);
            while i < self.rank_n(0, r) {
                let u = self.rv(0, r, i).unwrap();
                if self.nodes[u].node_type == NORMAL || self.vnode_not_related_to(g, u) {
                    let rn = self.graphs[g].rn.unwrap();
                    let w = margin as f64 + self.nodes[u].lw;
                    self.make_aux_edge(rn, u, w, 0);
                    break;
                }
                i += 1;
            }
        }
        for c in self.graphs[g].clust.clone() {
            self.keepout_othernodes(c);
        }
    }

    fn contain_subclust(&mut self, g: G) {
        let margin = self.margin_of(g);
        self.make_lrvn(g);
        for subg in self.graphs[g].clust.clone() {
            self.make_lrvn(subg);
            let (gl, gr) = (self.graphs[g].ln.unwrap(), self.graphs[g].rn.unwrap());
            let (sl, sr) = (self.graphs[subg].ln.unwrap(), self.graphs[subg].rn.unwrap());
            let bl = self.graphs[g].border[LEFT_IX].x;
            let br = self.graphs[g].border[RIGHT_IX].x;
            self.make_aux_edge(gl, sl, margin as f64 + bl, 0);
            self.make_aux_edge(sr, gr, margin as f64 + br, 0);
            self.contain_subclust(subg);
        }
    }

    fn separate_subclust(&mut self, g: G) {
        let margin = self.margin_of(g);
        let cl = self.graphs[g].clust.clone();
        for &c in &cl {
            self.make_lrvn(c);
        }
        for i in 0..cl.len() {
            for j in (i + 1)..cl.len() {
                let (mut low, mut high) = (cl[i], cl[j]);
                if self.graphs[low].minrank > self.graphs[high].minrank {
                    std::mem::swap(&mut low, &mut high);
                }
                if self.graphs[low].maxrank < self.graphs[high].minrank {
                    continue;
                }
                let hm = self.graphs[high].minrank;
                let lo = self.rv(low, hm, 0).map_or(0, |v| self.nodes[v].order);
                let ho = self.rv(high, hm, 0).map_or(0, |v| self.nodes[v].order);
                let (left, right) = if lo < ho { (low, high) } else { (high, low) };
                let (lr, rl) = (
                    self.graphs[left].rn.unwrap(),
                    self.graphs[right].ln.unwrap(),
                );
                self.make_aux_edge(lr, rl, margin as f64, 0);
            }
            self.separate_subclust(cl[i]);
        }
    }

    fn pos_clusters(&mut self, g: G) {
        if !self.graphs[g].clust.is_empty() {
            self.contain_clustnodes(g);
            self.keepout_othernodes(g);
            self.contain_subclust(g);
            self.separate_subclust(g);
        }
    }

    fn compress_graph(&mut self, g: G) {
        if self.graphs[g].drawing.ratio_kind != RatioKind::Compress {
            return;
        }
        let p = self.graphs[g].drawing.size;
        if p.x * p.y <= 1.0 {
            return;
        }
        self.contain_nodes(g);
        let x = if !self.graphs[g].flip() { p.x } else { p.y };
        let x = c_min(x, u16::MAX as f64);
        let (ln, rn) = (self.graphs[g].ln.unwrap(), self.graphs[g].rn.unwrap());
        self.make_aux_edge(ln, rn, x, 1000);
    }

    fn create_aux_edges(&mut self, g: G) {
        self.allocate_aux_edges(g);
        self.make_lr_constraints(g);
        self.make_edge_pairs(g);
        self.pos_clusters(g);
        self.compress_graph(g);
    }

    fn remove_aux_edges(&mut self, g: G) {
        for n in self.nlist_nodes(g) {
            let nd = &mut self.nodes[n];
            nd.out = nd.save_out.clone().unwrap_or_default();
            nd.in_ = nd.save_in.clone().unwrap_or_default();
        }
        let mut nprev: Option<N> = None;
        let mut n = self.graphs[g].nlist;
        while let Some(x) = n {
            let nnext = self.nodes[x].next;
            if self.nodes[x].node_type == SLACKNODE {
                match nprev {
                    Some(p) => self.nodes[p].next = nnext,
                    None => self.graphs[g].nlist = nnext,
                }
                if let Some(nx) = nnext {
                    self.nodes[nx].prev = nprev;
                }
            } else {
                nprev = Some(x);
            }
            n = nnext;
        }
    }

    fn set_xcoords(&mut self, g: G) {
        for i in self.graphs[g].minrank..=self.graphs[g].maxrank {
            for j in 0..self.rank_n(g, i) {
                let v = self.rv(g, i, j).unwrap();
                self.nodes[v].coord.x = self.nodes[v].rank as f64;
                self.nodes[v].rank = i;
            }
        }
    }

    /// `ND_coord(rank[r].v[0]).y`. A root rank with no node has a NULL `v[0]`, which C
    /// dereferences and crashes on (#358): the layout fails there.
    fn rank0_y(&self, r: i32) -> Result<f64, String> {
        self.rank0(r).map(|v| self.nodes[v].coord.y)
    }

    /// `rank[r].v[0]` of the root, where C dereferences it without checking.
    fn rank0(&self, r: i32) -> Result<N, String> {
        self.rv(0, r, 0).ok_or_else(|| {
            format!(
                "set_ycoords: rank {r} of {} has no node (C dereferences its NULL first node)",
                self.graph_name(0)
            )
        })
    }

    fn adjust_simple(&mut self, g: G, delta: f64, margin_total: i32) {
        let root = 0;
        let maxr = self.graphs[g].maxrank;
        let minr = self.graphs[g].minrank;
        let bottom = (delta + 1.0) / 2.0;
        let delbottom =
            self.graphs[g].ht1 + bottom - (self.rk(root, maxr).ht1 - margin_total as f64);
        let deltop;
        if delbottom > 0.0 {
            let mut r = maxr;
            while r >= minr {
                if self.rank_n(root, r) > 0 {
                    let v = self.rv(root, r, 0).unwrap();
                    self.nodes[v].coord.y += delbottom;
                }
                r -= 1;
            }
            deltop = self.graphs[g].ht2 + (delta - bottom) + delbottom
                - (self.rk(root, minr).ht2 - margin_total as f64);
        } else {
            deltop = self.graphs[g].ht2 + (delta - bottom)
                - (self.rk(root, minr).ht2 - margin_total as f64);
        }
        if deltop > 0.0 {
            let mut r = minr - 1;
            while r >= self.graphs[root].minrank {
                if self.rank_n(root, r) > 0 {
                    let v = self.rv(root, r, 0).unwrap();
                    self.nodes[v].coord.y += deltop;
                }
                r -= 1;
            }
        }
        self.graphs[g].ht2 += delta - bottom;
        self.graphs[g].ht1 += bottom;
    }

    fn adjust_ranks(&mut self, g: G, margin_total: i32) -> Result<(), String> {
        let margin = if g == 0 { 0 } else { self.margin_of(g) };
        let mut ht1 = self.graphs[g].ht1;
        let mut ht2 = self.graphs[g].ht2;
        for subg in self.graphs[g].clust.clone() {
            self.adjust_ranks(subg, margin + margin_total)?;
            if self.graphs[subg].maxrank == self.graphs[g].maxrank {
                ht1 = ht1.max(self.graphs[subg].ht1 + margin as f64);
            }
            if self.graphs[subg].minrank == self.graphs[g].minrank {
                ht2 = ht2.max(self.graphs[subg].ht2 + margin as f64);
            }
        }
        self.graphs[g].ht1 = ht1;
        self.graphs[g].ht2 = ht2;
        if g != 0 && self.graphs[g].label.is_some() {
            let lht = c_max(
                self.graphs[g].border[LEFT_IX].y,
                self.graphs[g].border[RIGHT_IX].y,
            );
            let maxr = self.graphs[g].maxrank;
            let minr = self.graphs[g].minrank;
            let rht = self.rank0_y(minr)? - self.rank0_y(maxr)?;
            let delta = lht - (rht + ht1 + ht2);
            if delta > 0.0 {
                self.adjust_simple(g, delta, margin_total);
            }
        }
        if g != 0 {
            let (minr, maxr) = (self.graphs[g].minrank, self.graphs[g].maxrank);
            let (h1, h2) = (self.graphs[g].ht1, self.graphs[g].ht2);
            let rk = self.rk_mut(0, minr);
            rk.ht2 = rk.ht2.max(h2);
            let rk = self.rk_mut(0, maxr);
            rk.ht1 = rk.ht1.max(h1);
        }
        Ok(())
    }

    fn clust_ht(&mut self, g: G) -> bool {
        let margin = if g == 0 { CL_OFFSET } else { self.margin_of(g) };
        let mut have_label = false;
        let mut ht1 = self.graphs[g].ht1;
        let mut ht2 = self.graphs[g].ht2;
        for subg in self.graphs[g].clust.clone() {
            have_label |= self.clust_ht(subg);
            if self.graphs[subg].maxrank == self.graphs[g].maxrank {
                ht1 = c_max(ht1, self.graphs[subg].ht1 + margin as f64);
            }
            if self.graphs[subg].minrank == self.graphs[g].minrank {
                ht2 = c_max(ht2, self.graphs[subg].ht2 + margin as f64);
            }
        }
        if g != 0 && self.graphs[g].label.is_some() {
            have_label = true;
            if !self.graphs[0].flip() {
                ht1 += self.graphs[g].border[BOTTOM_IX].y;
                ht2 += self.graphs[g].border[TOP_IX].y;
            }
        }
        self.graphs[g].ht1 = ht1;
        self.graphs[g].ht2 = ht2;
        if g != 0 {
            let (minr, maxr) = (self.graphs[g].minrank, self.graphs[g].maxrank);
            let rk = self.rk_mut(0, minr);
            rk.ht2 = c_max(rk.ht2, ht2);
            let rk = self.rk_mut(0, maxr);
            rk.ht1 = c_max(rk.ht1, ht1);
        }
        have_label
    }

    /// `set_ycoords(g)`. Fails where C dereferences the NULL first node of a root rank
    /// with no node (#358): a rank set or an earlier cluster took a cluster's nodes after
    /// it was ranked, or the ranking stopped before normalising the ranks (`nslimit1`,
    /// #357). C reads it on every such input that gets here and crashes.
    pub fn set_ycoords(&mut self, g: G) -> Result<(), String> {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            for i in 0..self.rank_n(g, r) {
                let n = self.rv(g, r, i).unwrap();
                let mut ht2 = self.nodes[n].ht / 2.0;
                for &e in &self.nodes[n].other {
                    if self.edges[e].tail == self.edges[e].head
                        && let Some(l) = self.edges[e].label
                    {
                        ht2 = ht2.max(self.labels[l].dimen.y / 2.0);
                    }
                }
                let rk = self.rk_mut(g, r);
                if rk.pht2 < ht2 {
                    rk.pht2 = ht2;
                    rk.ht2 = ht2;
                }
                if rk.pht1 < ht2 {
                    rk.pht1 = ht2;
                    rk.ht1 = ht2;
                }
                if let Some(clust) = self.nodes[n].clust {
                    let yoff = if clust == g { 0 } else { self.margin_of(clust) };
                    let nr = self.nodes[n].rank;
                    if nr == self.graphs[clust].minrank {
                        self.graphs[clust].ht2 = self.graphs[clust].ht2.max(ht2 + yoff as f64);
                    }
                    if nr == self.graphs[clust].maxrank {
                        self.graphs[clust].ht1 = self.graphs[clust].ht1.max(ht2 + yoff as f64);
                    }
                }
            }
        }
        let lbl = self.clust_ht(g);
        let mut maxht: f64 = 0.0;
        let mut r = self.graphs[g].maxrank;
        let v = self.rank0(r)?;
        self.nodes[v].coord.y = self.rk(g, r).ht1;
        let ranksep = self.graphs[g].ranksep as f64;
        r -= 1;
        while r >= self.graphs[g].minrank {
            let d0 = self.rk(g, r + 1).pht2 + self.rk(g, r).pht1 + ranksep;
            let d1 = self.rk(g, r + 1).ht2 + self.rk(g, r).ht1 + CL_OFFSET as f64;
            let delta = d0.max(d1);
            if self.rank_n(g, r) > 0 {
                let v = self.rank0(r)?;
                let y = self.rank0_y(r + 1)?;
                self.nodes[v].coord.y = y + delta;
            }
            maxht = maxht.max(delta);
            r -= 1;
        }
        if lbl && self.graphs[g].flip() {
            self.adjust_ranks(g, 0)?;
            if self.graphs[g].exact_ranksep {
                maxht = 0.0;
                let mut r = self.graphs[g].maxrank;
                let mut d0 = self.rank0_y(r)?;
                r -= 1;
                while r >= self.graphs[g].minrank {
                    let d1 = self.rank0_y(r)?;
                    maxht = maxht.max(d1 - d0);
                    d0 = d1;
                    r -= 1;
                }
            }
        }
        if self.graphs[g].exact_ranksep {
            let mut r = self.graphs[g].maxrank - 1;
            while r >= self.graphs[g].minrank {
                if self.rank_n(g, r) > 0 {
                    let v = self.rank0(r)?;
                    let y = self.rank0_y(r + 1)?;
                    self.nodes[v].coord.y = y + maxht;
                }
                r -= 1;
            }
        }
        for n in self.nlist_nodes(g) {
            let r = self.nodes[n].rank;
            let y = self.rank0_y(r)?;
            self.nodes[n].coord.y = y;
        }
        Ok(())
    }

    fn dot_compute_bb(&mut self, g: G, root: G) {
        let mut ll = Point::new(0.0, 0.0);
        let mut ur = Point::new(0.0, 0.0);
        if g == 0 {
            ll.x = i32::MAX as f64;
            ur.x = -(i32::MAX as f64);
            for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
                let rnkn = self.rank_n(g, r);
                if rnkn == 0 {
                    continue;
                }
                let Some(mut v) = self.rv(g, r, 0) else {
                    continue;
                };
                let mut c = 1;
                while self.nodes[v].node_type != NORMAL && c < rnkn {
                    v = self.rv(g, r, c).unwrap();
                    c += 1;
                }
                if self.nodes[v].node_type == NORMAL {
                    let x = self.nodes[v].coord.x - self.nodes[v].lw;
                    ll.x = c_min(ll.x, x);
                } else {
                    continue;
                }
                let mut v = self.rv(g, r, rnkn - 1).unwrap();
                let mut c = rnkn - 2;
                while self.nodes[v].node_type != NORMAL {
                    v = self.rv(g, r, c).unwrap();
                    c -= 1;
                }
                let x = self.nodes[v].coord.x + self.nodes[v].rw;
                ur.x = c_max(ur.x, x);
            }
            let offset = CL_OFFSET as f64;
            for c in self.graphs[g].clust.clone() {
                let x = self.graphs[c].bb.ll.x - offset;
                ll.x = c_min(ll.x, x);
                let x = self.graphs[c].bb.ur.x + offset;
                ur.x = c_max(ur.x, x);
            }
        } else {
            ll.x = self.nodes[self.graphs[g].ln.unwrap()].rank as f64;
            ur.x = self.nodes[self.graphs[g].rn.unwrap()].rank as f64;
        }
        let maxr = self.graphs[g].maxrank;
        let minr = self.graphs[g].minrank;
        ll.y = self
            .rv(root, maxr, 0)
            .map_or(0.0, |v| self.nodes[v].coord.y)
            - self.graphs[g].ht1;
        ur.y = self
            .rv(root, minr, 0)
            .map_or(0.0, |v| self.nodes[v].coord.y)
            + self.graphs[g].ht2;
        self.graphs[g].bb = BoxF { ll, ur };
    }

    fn rec_bb(&mut self, g: G, root: G) {
        for c in self.graphs[g].clust.clone() {
            self.rec_bb(c, root);
        }
        self.dot_compute_bb(g, root);
    }

    fn scale_bb(&mut self, g: G, xf: f64, yf: f64) {
        for c in self.graphs[g].clust.clone() {
            self.scale_bb(c, xf, yf);
        }
        let bb = &mut self.graphs[g].bb;
        bb.ll.x *= xf;
        bb.ll.y *= yf;
        bb.ur.x *= xf;
        bb.ur.y *= yf;
    }

    fn set_aspect(&mut self, g: G) {
        self.rec_bb(g, g);
        let kind = self.graphs[g].drawing.ratio_kind;
        if self.graphs[g].maxrank > 0 && kind != RatioKind::None {
            let bb = self.graphs[g].bb;
            let mut sz = Point::new(bb.ur.x - bb.ll.x, bb.ur.y - bb.ll.y);
            if self.graphs[g].flip() {
                sz = Point::new(sz.y, sz.x);
            }
            let mut scale_it = true;
            let filled = if kind == RatioKind::Auto {
                self.idealsize(g, 0.5)
            } else {
                kind == RatioKind::Fill
            };
            let size = self.graphs[g].drawing.size;
            let (mut xf, mut yf) = (0.0, 0.0);
            if filled {
                if size.x <= 0.0 {
                    scale_it = false;
                } else {
                    xf = size.x / sz.x;
                    yf = size.y / sz.y;
                    if xf < 1.0 || yf < 1.0 {
                        if xf < yf {
                            yf /= xf;
                            xf = 1.0;
                        } else {
                            xf /= yf;
                            yf = 1.0;
                        }
                    }
                }
            } else if kind == RatioKind::Expand {
                if size.x <= 0.0 {
                    scale_it = false;
                } else {
                    xf = size.x / bb.ur.x;
                    yf = size.y / bb.ur.y;
                    if xf > 1.0 && yf > 1.0 {
                        let s = xf.min(yf);
                        xf = s;
                        yf = s;
                    } else {
                        scale_it = false;
                    }
                }
            } else if kind == RatioKind::Value {
                let desired = self.graphs[g].drawing.ratio;
                let actual = sz.y / sz.x;
                if actual < desired {
                    yf = desired / actual;
                    xf = 1.0;
                } else {
                    xf = actual / desired;
                    yf = 1.0;
                }
            } else {
                scale_it = false;
            }
            if scale_it {
                if self.graphs[g].flip() {
                    std::mem::swap(&mut xf, &mut yf);
                }
                for n in self.nlist_nodes(g) {
                    let c = &mut self.nodes[n].coord;
                    c.x = (c.x * xf).round();
                    c.y = (c.y * yf).round();
                }
                self.scale_bb(g, xf, yf);
            }
        }
    }

    fn make_leafslots(&mut self, g: G) {
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let mut j = 0;
            for i in 0..self.rank_n(g, r) {
                let v = self.rv(g, r, i).unwrap();
                self.nodes[v].order = j;
                if self.nodes[v].ranktype == LEAFSET {
                    j += self.nodes[v].uf_size;
                } else {
                    j += 1;
                }
            }
            if j <= self.rank_n(g, r) {
                continue;
            }
            let b = self.new_buf((j + 1) as usize);
            for i in (0..self.rank_n(g, r)).rev() {
                let v = self.rv(g, r, i).unwrap();
                let o = self.nodes[v].order as usize;
                self.bufs[b][o] = Some(v);
            }
            let rk = self.rk_mut(g, r);
            rk.n = j;
            rk.vbuf = b;
            rk.voff = 0;
        }
    }

    fn expand_leaves(&mut self, g: G) {
        self.make_leafslots(g);
        // The remainder of expand_leaves computes `d = rank(head) - rank(head)`,
        // which is always 0, so it never changes anything.
    }

    pub fn make_lrvn(&mut self, g: G) {
        if self.graphs[g].ln.is_some() {
            return;
        }
        let ln = self.virtual_node(0);
        self.nodes[ln].node_type = SLACKNODE;
        let rn = self.virtual_node(0);
        self.nodes[rn].node_type = SLACKNODE;
        if self.graphs[g].label.is_some() && g != 0 && !self.graphs[0].flip() {
            let w = self.graphs[g].border[BOTTOM_IX]
                .x
                .max(self.graphs[g].border[TOP_IX].x);
            self.make_aux_edge(ln, rn, w, 0);
        }
        self.graphs[g].ln = Some(ln);
        self.graphs[g].rn = Some(rn);
    }

    fn contain_nodes(&mut self, g: G) {
        let margin = self.margin_of(g);
        self.make_lrvn(g);
        let (ln, rn) = (self.graphs[g].ln.unwrap(), self.graphs[g].rn.unwrap());
        for r in self.graphs[g].minrank..=self.graphs[g].maxrank {
            let n = self.rank_n(g, r);
            if n == 0 {
                continue;
            }
            let Some(v) = self.rv(g, r, 0) else {
                graphviz_pure_core::diag::agerrorf(&format!(
                    "contain_nodes clust {} rank {r} missing node\n",
                    self.graph_name(g)
                ));
                continue;
            };
            let w = self.nodes[v].lw + margin as f64 + self.graphs[g].border[LEFT_IX].x;
            self.make_aux_edge(ln, v, w, 0);
            let v = self.rv(g, r, n - 1).unwrap();
            let w = self.nodes[v].rw + margin as f64 + self.graphs[g].border[RIGHT_IX].x;
            self.make_aux_edge(v, rn, w, 0);
        }
    }

    fn idealsize(&mut self, g: G, minallowed: f64) -> bool {
        let dr = &self.graphs[g].drawing;
        let mut relpage = dr.page;
        if relpage.x < 0.001 || relpage.y < 0.001 {
            return false;
        }
        let margin = dr.margin;
        relpage.x -= 2.0 * margin.x;
        relpage.y -= 2.0 * margin.y;
        let b = self.graphs[g].bb.ur;
        let xf = relpage.x / b.x;
        let yf = relpage.y / b.y;
        if xf >= 1.0 && yf >= 1.0 {
            return false;
        }
        let f = c_min(xf, yf);
        let s = c_max(f, minallowed);
        let r = (s * b.x / relpage.x).ceil();
        let xf = r * relpage.x / b.x;
        let r = (s * b.y / relpage.y).ceil();
        let yf = r * relpage.y / b.y;
        self.graphs[g].drawing.size = Point::new(b.x * xf, b.y * yf);
        true
    }
}
