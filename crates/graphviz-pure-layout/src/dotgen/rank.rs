//! Port of `lib/dotgen/rank.c`: `dot1_rank` (classic ranking with collapsed
//! clusters and rank sets) and `dot2_rank` (`newrank=true`).

use super::Dot;
use super::types::*;
use super::utils::*;
use graphviz_pure_core::strtod::atof;

const BACKWARD_PENALTY: i32 = 1000;
const STRONG_CLUSTER_WEIGHT: i32 = 1000;
const NORANK: u8 = 6;

impl Dot<'_> {
    fn renewlist(l: &mut Vec<E>) {
        l.clear();
    }

    fn cleanup1(&mut self, g: G) {
        for c in 0..self.graphs[g].comp.len() {
            self.graphs[g].nlist = self.graphs[g].comp[c];
            let mut prev: Option<N> = None;
            let mut n = self.graphs[g].nlist;
            while let Some(x) = n {
                let next = self.nodes[x].next;
                Self::renewlist(&mut self.nodes[x].in_);
                Self::renewlist(&mut self.nodes[x].out);
                self.nodes[x].mark = 0;
                if self.nodes[x].node_type == SLACKNODE {
                    match prev {
                        None => {
                            self.graphs[g].comp[c] = next;
                            self.graphs[g].nlist = next;
                        }
                        Some(p) => self.nodes[p].next = next,
                    }
                    if let Some(nx) = next {
                        self.nodes[nx].prev = prev;
                    }
                } else {
                    prev = Some(x);
                }
                n = next;
            }
        }
        for n in self.nodes_of(g) {
            for e in self.out_edges(g, n) {
                if let Some(f) = self.edges[e].to_virt
                    && self.edges[f].to_orig != Some(e)
                {
                    self.edges[e].to_virt = None;
                }
            }
        }
        for n in self.nodes_of(g) {
            for e in self.out_edges(g, n) {
                if let Some(f) = self.edges[e].to_virt
                    && self.edges[f].to_orig == Some(e)
                {
                    self.edges[e].to_virt = None;
                }
            }
        }
        self.graphs[g].comp.clear();
    }

    fn edgelabel_ranks(&mut self, g: G) {
        // `GD_has_labels(g)`: the label flags live on the cgraph root, so a pack
        // component's own record has none.
        if !self.comp && self.graphs[g].has_labels & EDGE_LABEL != 0 {
            // No overflow check in C (undefined behaviour); the port wraps.
            for n in self.nodes_of(g) {
                for e in self.out_edges(g, n) {
                    let m = &mut self.edges[e].minlen;
                    *m = m.wrapping_mul(2);
                }
            }
            self.graphs[g].ranksep = self.graphs[g].ranksep.wrapping_add(1) / 2;
        }
    }

    fn collapse_rankset(&mut self, g: G, subg: G, kind: u8) {
        let ns = self.nodes_of(subg);
        let Some(&u) = ns.first() else { return };
        self.nodes[u].ranktype = kind;
        for &v in &ns[1..] {
            self.uf_union(u, v);
            self.nodes[v].ranktype = self.nodes[u].ranktype;
        }
        match kind {
            MINRANK | SOURCERANK => {
                self.graphs[g].minset = Some(match self.graphs[g].minset {
                    None => u,
                    Some(m) => self.uf_union(m, u),
                });
            }
            MAXRANK | SINKRANK => {
                self.graphs[g].maxset = Some(match self.graphs[g].maxset {
                    None => u,
                    Some(m) => self.uf_union(m, u),
                });
            }
            _ => {}
        }
        match kind {
            SOURCERANK => {
                let m = self.graphs[g].minset.unwrap();
                self.nodes[m].ranktype = kind;
            }
            SINKRANK => {
                let m = self.graphs[g].maxset.unwrap();
                self.nodes[m].ranktype = kind;
            }
            _ => {}
        }
    }

    fn rank_set_class(&mut self, g: G) -> u8 {
        if self.is_a_cluster(g) {
            return CLUSTER;
        }
        let val = match self.agget(g, "rank") {
            Some("same") => SAMERANK,
            Some("min") => MINRANK,
            Some("source") => SOURCERANK,
            Some("max") => MAXRANK,
            Some("sink") => SINKRANK,
            _ => 0,
        };
        self.graphs[g].set_type = val;
        val
    }

    pub fn make_new_cluster(&mut self, g: G, subg: G) -> i32 {
        self.graphs[g].clust.push(subg);
        let cno = self.graphs[g].clust.len() as i32;
        self.do_graph_label(subg);
        cno
    }

    fn node_induce(&mut self, par: G, g: G) {
        for n in self.nodes_of(g) {
            if self.nodes[n].ranktype != 0 {
                self.agdelnode(g, n);
                continue;
            }
            let nc = self.graphs[par].n_cluster();
            let mut i = 1;
            while i < nc {
                if self.agcontains_node(self.graphs[par].clust[(i - 1) as usize], n) {
                    break;
                }
                i += 1;
            }
            if i < nc {
                self.agdelnode(g, n);
            }
            self.nodes[n].clust = None;
        }
        for n in self.nodes_of(g) {
            for e in self.out_edges(0, n) {
                if self.agcontains_node(g, self.edges[e].head) {
                    self.agsubedge(g, e);
                }
            }
        }
    }

    pub fn dot_scan_ranks(&mut self, g: G) {
        let mut leader: Option<N> = None;
        self.graphs[g].minrank = i32::MAX;
        self.graphs[g].maxrank = -1;
        for n in self.nodes_of(g) {
            let r = self.nodes[n].rank;
            if self.graphs[g].maxrank < r {
                self.graphs[g].maxrank = r;
            }
            if self.graphs[g].minrank > r {
                self.graphs[g].minrank = r;
            }
            match leader {
                None => leader = Some(n),
                Some(l) => {
                    if r < self.nodes[l].rank {
                        leader = Some(n);
                    }
                }
            }
        }
        self.graphs[g].leader = leader;
    }

    /// `cluster_leader(clust)`. A cluster's local ranking is normalised, so some node of it
    /// lies on rank 0, and C asserts that it finds one (`rank.c:316`). When the ranking
    /// failed (network simplex printed `trouble in init_rank` and left the ranks it could
    /// not order unnormalised), there may be none, and C aborts. The port fails the layout
    /// here, where the cluster's ranking failed, instead of panicking (issue #233).
    fn cluster_leader(&mut self, clust: G) -> Result<(), String> {
        let mut leader: Option<N> = None;
        let mut n = self.graphs[clust].nlist;
        while let Some(x) = n {
            if self.nodes[x].rank == 0 && self.nodes[x].node_type == NORMAL {
                leader = Some(x);
            }
            n = self.nodes[x].next;
        }
        let Some(leader) = leader else {
            return Err(format!(
                "cluster_leader: the ranking of cluster {} failed: none of its nodes is on rank 0",
                self.graphs[clust].name
            ));
        };
        self.graphs[clust].leader = Some(leader);
        for n in self.nodes_of(clust) {
            self.uf_union(n, leader);
            self.nodes[n].ranktype = CLUSTER;
        }
        Ok(())
    }

    fn collapse_cluster(&mut self, g: G, subg: G) -> Result<(), String> {
        if self.graphs[subg].parent.is_some() {
            return Ok(());
        }
        self.graphs[subg].parent = Some(g);
        self.node_induce(g, subg);
        if self.agfstnode(subg).is_none() {
            return Ok(());
        }
        self.make_new_cluster(g, subg);
        if self.cl_type == LOCAL {
            self.dot1_rank(subg)?;
            self.cluster_leader(subg)?;
        } else {
            self.dot_scan_ranks(subg);
        }
        Ok(())
    }

    fn collapse_sets(&mut self, rg: G, g: G) -> Result<(), String> {
        let subs = self.graphs[g].subgs.clone();
        for subg in subs {
            let c = self.rank_set_class(subg);
            if c != 0 {
                if c == CLUSTER && self.cl_type == LOCAL {
                    self.collapse_cluster(rg, subg)?;
                } else {
                    self.collapse_rankset(rg, subg, c);
                }
            } else {
                self.collapse_sets(rg, subg)?;
            }
        }
        Ok(())
    }

    fn find_clusters(&mut self, g: G) -> Result<(), String> {
        let subs = self.graphs[0].subgs.clone();
        for subg in subs {
            if self.graphs[subg].set_type == CLUSTER {
                self.collapse_cluster(g, subg)?;
            }
        }
        Ok(())
    }

    fn set_minmax(&mut self, g: G) {
        let l = self.nodes[self.graphs[g].leader.unwrap()].rank;
        self.graphs[g].minrank += l;
        self.graphs[g].maxrank += l;
        for c in self.graphs[g].clust.clone() {
            self.set_minmax(c);
        }
    }

    fn minmax_edges(&mut self, g: G) -> (i32, i32) {
        let mut slen = (0, 0);
        if self.graphs[g].maxset.is_none() && self.graphs[g].minset.is_none() {
            return slen;
        }
        if let Some(m) = self.graphs[g].minset {
            self.graphs[g].minset = Some(self.uf_find(m));
        }
        if let Some(m) = self.graphs[g].maxset {
            self.graphs[g].maxset = Some(self.uf_find(m));
        }
        if let Some(n) = self.graphs[g].maxset {
            slen.1 = (self.nodes[n].ranktype == SINKRANK) as i32;
            while let Some(&e) = self.nodes[n].out.first() {
                self.reverse_edge(e);
            }
        }
        if let Some(n) = self.graphs[g].minset {
            slen.0 = (self.nodes[n].ranktype == SOURCERANK) as i32;
            while let Some(&e) = self.nodes[n].in_.first() {
                self.reverse_edge(e);
            }
        }
        slen
    }

    fn minmax_edges2(&mut self, g: G, slen: (i32, i32)) -> bool {
        let mut made = false;
        if self.graphs[g].maxset.is_some() || self.graphs[g].minset.is_some() {
            for n in self.nodes_of(g) {
                if n != self.uf_find(n) {
                    continue;
                }
                let maxset = self.graphs[g].maxset;
                let minset = self.graphs[g].minset;
                if self.nodes[n].out.is_empty()
                    && let Some(mx) = maxset
                    && n != mx
                {
                    let e = self.virtual_edge(n, mx, None);
                    self.edges[e].minlen = slen.1;
                    self.edges[e].weight = 0;
                    made = true;
                }
                if self.nodes[n].in_.is_empty()
                    && let Some(mn) = minset
                    && n != mn
                {
                    let e = self.virtual_edge(mn, n, None);
                    self.edges[e].minlen = slen.0;
                    self.edges[e].weight = 0;
                    made = true;
                }
            }
        }
        made
    }

    pub fn rank1(&mut self, g: G) -> Result<(), String> {
        let mut maxiter = i32::MAX;
        if let Some(s) = self.agget(g, "nslimit1") {
            maxiter = scale_clamp(self.agnnodes(g) as i32, atof(s));
        }
        for c in 0..self.graphs[g].comp.len() {
            self.graphs[g].nlist = self.graphs[g].comp[c];
            let bal = if self.graphs[g].clust.is_empty() {
                1
            } else {
                0
            };
            self.ns_rank(g, bal, maxiter)?;
        }
        Ok(())
    }

    fn expand_ranksets(&mut self, g: G) -> Result<(), String> {
        let ns = self.nodes_of(g);
        if !ns.is_empty() {
            self.graphs[g].minrank = i32::MAX;
            self.graphs[g].maxrank = -1;
            for n in ns {
                let leader = self.uf_find(n);
                if leader != n {
                    self.nodes[n].rank += self.nodes[leader].rank;
                }
                let r = self.nodes[n].rank;
                if self.graphs[g].maxrank < r {
                    self.graphs[g].maxrank = r;
                }
                if self.graphs[g].minrank > r {
                    self.graphs[g].minrank = r;
                }
                if self.nodes[n].ranktype != 0 && self.nodes[n].ranktype != LEAFSET {
                    self.uf_singleton(n);
                }
            }
            if g == 0 {
                if self.cl_type == LOCAL {
                    for c in self.graphs[g].clust.clone() {
                        self.set_minmax(c);
                    }
                } else {
                    self.find_clusters(g)?;
                }
            }
        } else {
            self.graphs[g].minrank = 0;
            self.graphs[g].maxrank = 0;
        }
        Ok(())
    }

    pub fn dot1_rank(&mut self, g: G) -> Result<(), String> {
        self.edgelabel_ranks(g);
        self.collapse_sets(g, g)?;
        self.class1(g);
        let p = self.minmax_edges(g);
        self.decompose(g, 0);
        self.acyclic(g);
        if self.minmax_edges2(g, p) {
            self.decompose(g, 0);
        }
        self.rank1(g)?;
        self.expand_ranksets(g)?;
        self.cleanup1(g);
        Ok(())
    }

    /// `dot_rank(g)`. Fails where a cluster's ranking fails ([`Self::cluster_leader`]).
    pub fn dot_rank(&mut self) -> Result<(), String> {
        if mapbool(self.agget(0, "newrank")) {
            self.graphs[0].flags |= NEW_RANK;
            self.dot2_rank(0)?;
        } else {
            self.dot1_rank(0)?;
        }
        if let Some(g) = self.graphs.first() {
            graphviz_pure_core::verbose_msg!("Maxrank = {}, minrank = {}\n", g.maxrank, g.minrank);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // newrank (dot2_rank)
    // ------------------------------------------------------------------

    fn set_parent(&mut self, g: G, p: G) {
        self.graphs[g].parent = Some(p);
        self.make_new_cluster(p, g);
        self.node_induce(p, g);
    }

    fn is_a_strong_cluster(&self, g: Option<G>) -> bool {
        match g {
            Some(g) => mapbool(self.agget(g, "compact")),
            None => false,
        }
    }

    fn rankset_kind(&self, g: G) -> u8 {
        match self.agget(g, "rank") {
            Some(s) if !s.is_empty() => match s {
                "min" => MINRANK,
                "source" => SOURCERANK,
                "max" => MAXRANK,
                "sink" => SINKRANK,
                "same" => SAMERANK,
                _ => NORANK,
            },
            _ => NORANK,
        }
    }

    fn nr_find(&mut self, n: N) -> N {
        let mut path = Vec::new();
        let mut cur = n;
        loop {
            match self.nodes[cur].set {
                Some(s) if s != cur => {
                    path.push(cur);
                    cur = s;
                }
                Some(_) => break,
                None => {
                    self.nodes[cur].set = Some(cur);
                    break;
                }
            }
        }
        for p in path {
            self.nodes[p].set = Some(cur);
        }
        cur
    }

    fn union_one(&mut self, leader: N, n: Option<N>) -> N {
        match n {
            Some(n) => {
                let fl = self.nr_find(leader);
                let fnn = self.nr_find(n);
                self.nodes[fnn].set = Some(fl);
                fl
            }
            None => leader,
        }
    }

    fn union_all(&mut self, g: G) -> Option<N> {
        let ns = self.nodes_of(g);
        let first = *ns.first()?;
        let leader = self.nr_find(first);
        for &n in &ns[1..] {
            self.union_one(leader, Some(n));
        }
        Some(leader)
    }

    fn compile_samerank(&mut self, ug: G, parent_clust: Option<G>) {
        if self.agfstnode(ug).is_none() {
            return;
        }
        let clust;
        if self.is_a_cluster(ug) {
            clust = Some(ug);
            if let Some(pc) = parent_clust {
                self.graphs[ug].level = self.graphs[pc].level + 1;
                self.set_parent(ug, pc);
            } else {
                self.graphs[ug].level = 0;
            }
        } else {
            clust = parent_clust;
        }
        for s in self.graphs[ug].subgs.clone() {
            self.compile_samerank(s, clust);
        }
        if self.is_a_cluster(ug) {
            for n in self.nodes_of(ug) {
                if self.nodes[n].clust.is_none() {
                    self.nodes[n].clust = Some(ug);
                }
            }
        }
        match self.rankset_kind(ug) {
            // `set_parent`'s `node_induce` can empty a cluster after the `is_empty`
            // check (e.g. a pack `_cc_<i>` clone whose nodes the original cluster
            // already holds), so `union_all` gives NULL and C's
            // `union_one(NULL, rep)` returns NULL. The rep is NULL then too: the
            // emptied cluster's subgraphs lost the same nodes, so none set one.
            SOURCERANK | MINRANK => {
                let leader = self.union_all(ug);
                if let Some(c) = clust {
                    let mr = self.graphs[c].minrep;
                    self.graphs[c].minrep = match leader {
                        Some(l) => Some(self.union_one(l, mr)),
                        None => {
                            debug_assert!(
                                mr.is_none(),
                                "union_one(NULL, rep) dereferences NULL in C"
                            );
                            mr
                        }
                    };
                }
            }
            SINKRANK | MAXRANK => {
                let leader = self.union_all(ug);
                if let Some(c) = clust {
                    let mr = self.graphs[c].maxrep;
                    self.graphs[c].maxrep = match leader {
                        Some(l) => Some(self.union_one(l, mr)),
                        None => {
                            debug_assert!(
                                mr.is_none(),
                                "union_one(NULL, rep) dereferences NULL in C"
                            );
                            mr
                        }
                    };
                }
            }
            SAMERANK => {
                self.union_all(ug);
            }
            _ => {}
        }
        if self.is_a_cluster(ug)
            && let Some(minrep) = self.graphs[ug].minrep
            && Some(minrep) == self.graphs[ug].maxrep
        {
            let up = self.union_all(ug);
            self.graphs[ug].minrep = up;
            self.graphs[ug].maxrep = up;
        }
    }

    /// `dot_lca(c0, c1)` (`lib/dotgen/rank.c:708-717`).
    ///
    /// C reads `GD_level(c0)` and `GD_level(c1)` without checking for NULL, relying on
    /// the layout root having `is_a_cluster(root) == true` (`g == g->root`) so every node's
    /// `ND_clust` and every cluster's `GD_parent` chain ends at the layout root at
    /// `GD_level == 0`. Under `doDot`'s multi-component pack branch, a component `_cc_<i>`
    /// is a subgraph (`sg != sg->root`), so `is_a_cluster(sg)` is false in C (`compile_samerank`
    /// leaves `parent_clust == NULL`, top-level clusters get `GD_level == 0` and
    /// `GD_parent == NULL`, and nodes outside any cluster keep `ND_clust == NULL`), and C's
    /// `dot_lca` dereferences NULL (`GD_level(NULL)`) on any edge between a cluster and
    /// outside or between two top-level clusters (#427). Treating `None` as level `-1` is a
    /// sentinel below `level == 0` so `None` plays the root cluster's role: the walk climbs
    /// to `None` without crashing, and `is_internal_to_cluster` gives the same answers C
    /// gives in a normal `newrank` layout where the root is a level-0 cluster (`true` when
    /// one end is outside any cluster, `false` when the ends are in two different top-level
    /// clusters).
    fn dot_lca(&self, mut c0: Option<G>, mut c1: Option<G>) -> Option<G> {
        while c0 != c1 {
            let l0 = c0.map_or(-1, |c| self.graphs[c].level);
            let l1 = c1.map_or(-1, |c| self.graphs[c].level);
            if l0 >= l1 {
                c0 = c0.and_then(|c| self.graphs[c].parent);
            } else {
                c1 = c1.and_then(|c| self.graphs[c].parent);
            }
        }
        c0
    }

    fn is_internal_to_cluster(&self, e: E) -> bool {
        let ct = self.nodes[self.edges[e].tail].clust;
        let ch = self.nodes[self.edges[e].head].clust;
        if ct == ch {
            return true;
        }
        let par = self.dot_lca(ct, ch);
        par == ct || par == ch
    }

    fn make_xnode(&mut self, xg: G, name: &str) -> N {
        let n = self.cg_node(xg, name);
        if let Some(last) = self.xg_last {
            self.nodes[n].prev = Some(last);
            self.nodes[last].next = Some(n);
        } else {
            self.nodes[n].prev = None;
            self.graphs[xg].nlist = Some(n);
        }
        self.xg_last = Some(n);
        self.nodes[n].next = None;
        n
    }

    fn compile_nodes(&mut self, g: G, xg: G) {
        self.xg_last = None;
        for n in self.nodes_of(g) {
            if self.nr_find(n) == n {
                let name = self.nodes[n].name.clone();
                let x = self.make_xnode(xg, &name);
                self.nodes[n].rep = Some(x);
            }
        }
        for n in self.nodes_of(g) {
            if self.nodes[n].rep.is_none() {
                let f = self.nr_find(n);
                self.nodes[n].rep = self.nodes[f].rep;
            }
        }
    }

    fn nr_merge(&mut self, e: E, minlen: i32, weight: i32) {
        self.edges[e].minlen = self.edges[e].minlen.max(minlen);
        // No overflow check in C (undefined behaviour); the port wraps.
        let w = &mut self.edges[e].weight;
        *w = w.wrapping_add(weight);
    }

    fn strong(&mut self, g: G, t: N, h: N, orig: E) {
        let e = if let Some(e) = self.cg_find_edge(g, t, h) {
            e
        } else if let Some(e) = self.cg_find_edge(g, h, t) {
            e
        } else {
            self.cg_edge(g, t, h)
        };
        let (ml, w) = (self.edges[orig].minlen, self.edges[orig].weight);
        self.nr_merge(e, ml, w);
    }

    fn weak(&mut self, g: G, t: N, h: N, orig: E) {
        for e in self.in_edges(g, t) {
            let v = self.edges[e].tail;
            if let Some(f) = self.agfstout(g, v)
                && self.edges[f].head == h
            {
                return;
            }
        }
        let buf = format!("_weak_{}", self.weak_id);
        self.weak_id += 1;
        let v = self.make_xnode(g, &buf);
        let e = self.cg_edge(g, v, t);
        let f = self.cg_edge(g, v, h);
        self.edges[e].minlen = self.edges[e].minlen.max(0);
        // No overflow check in C (undefined behaviour); the port wraps.
        let w = self.edges[orig].weight;
        let ew = &mut self.edges[e].weight;
        *ew = ew.wrapping_add(w.wrapping_mul(BACKWARD_PENALTY));
        self.edges[f].minlen = self.edges[f].minlen.max(self.edges[orig].minlen);
        let fw = &mut self.edges[f].weight;
        *fw = fw.wrapping_add(w);
    }

    fn compile_edges(&mut self, ug: G, xg: G) {
        for n in self.nodes_of(ug) {
            let mut xt = self.nodes[n].rep.unwrap();
            for e in self.out_edges(ug, n) {
                if self.nonconstraint_edge(e) {
                    continue;
                }
                let fh = self.nr_find(self.edges[e].head);
                let mut xh = self.nodes[fh].rep.unwrap();
                if xt == xh {
                    continue;
                }
                let tc = self.nodes[self.edges[e].tail].clust;
                let hc = self.nodes[self.edges[e].head].clust;
                if self.is_internal_to_cluster(e) {
                    let ft = self.nr_find(self.edges[e].tail);
                    let fhd = self.nr_find(self.edges[e].head);
                    let swap = tc.is_some_and(|c| Some(ft) == self.graphs[c].maxrep)
                        || hc.is_some_and(|c| Some(fhd) == self.graphs[c].minrep);
                    if swap {
                        std::mem::swap(&mut xt, &mut xh);
                    }
                    self.strong(xg, xt, xh, e);
                } else if self.is_a_strong_cluster(tc) || self.is_a_strong_cluster(hc) {
                    self.weak(xg, xt, xh, e);
                } else {
                    self.strong(xg, xt, xh, e);
                }
            }
        }
    }

    fn compile_clusters(&mut self, g: G, xg: G, mut top: Option<N>, mut bot: Option<N>) {
        if self.is_a_cluster(g) && self.is_a_strong_cluster(Some(g)) {
            for n in self.nodes_of(g) {
                if self.agfstin(g, n).is_none() {
                    let f = self.nr_find(n);
                    let rep = self.nodes[f].rep.unwrap();
                    if top.is_none() {
                        top = Some(self.make_xnode(xg, "\x7ftop"));
                    }
                    self.cg_edge(xg, top.unwrap(), rep);
                }
                if self.agfstout(g, n).is_none() {
                    let f = self.nr_find(n);
                    let rep = self.nodes[f].rep.unwrap();
                    if bot.is_none() {
                        bot = Some(self.make_xnode(xg, "\x7fbot"));
                    }
                    self.cg_edge(xg, rep, bot.unwrap());
                }
            }
            if let (Some(t), Some(b)) = (top, bot) {
                let e = self.cg_edge(xg, t, b);
                self.nr_merge(e, 0, STRONG_CLUSTER_WEIGHT);
            }
        }
        for sub in self.graphs[g].subgs.clone() {
            self.compile_clusters(sub, xg, top, bot);
        }
    }

    fn reverse_edge2(&mut self, g: G, e: E) {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        let rev = match self.cg_find_edge(g, h, t) {
            Some(r) => r,
            None => self.cg_edge(g, h, t),
        };
        let (ml, w) = (self.edges[e].minlen, self.edges[e].weight);
        self.nr_merge(rev, ml, w);
        self.agdeledge(g, e);
    }

    /// Live `agnxtout(g, e)` for an edge of `v` (sequence order).
    fn live_next_out(&self, g: G, v: N, e: E) -> Option<E> {
        let k = self.out_key(e);
        self.nodes[v]
            .cg_out
            .iter()
            .copied()
            .find(|&x| self.out_key(x) > k && self.graphs[g].edges.contains(&x))
    }

    fn nr_dfs(&mut self, g: G, v: N) {
        if self.nodes[v].mark != 0 {
            return;
        }
        self.nodes[v].mark = 1;
        self.nodes[v].onstack = true;
        // explicit stack of (node, next out-edge to process)
        let first = self.agfstout(g, v);
        let mut stack: Vec<(N, Option<E>)> = vec![(v, first)];
        while let Some(&(cur, next)) = stack.last() {
            let Some(e) = next else {
                self.nodes[cur].onstack = false;
                stack.pop();
                continue;
            };
            let f = self.live_next_out(g, cur, e);
            stack.last_mut().unwrap().1 = f;
            let w = self.edges[e].head;
            if self.nodes[w].onstack {
                self.reverse_edge2(g, e);
            } else if self.nodes[w].mark == 0 {
                self.nodes[w].mark = 1;
                self.nodes[w].onstack = true;
                let fw = self.agfstout(g, w);
                stack.push((w, fw));
            }
        }
    }

    fn break_cycles(&mut self, g: G) {
        for n in self.nodes_of(g) {
            self.nodes[n].mark = 0;
            self.nodes[n].onstack = false;
        }
        for n in self.nodes_of(g) {
            self.nr_dfs(g, n);
        }
    }

    fn set_min_max(&mut self, g: G, do_root: bool) {
        for c in self.graphs[g].clust.clone() {
            self.set_min_max(c, false);
        }
        if self.graphs[g].parent.is_none() && !do_root {
            return;
        }
        self.graphs[g].minrank = i32::MAX;
        self.graphs[g].maxrank = -1;
        let mut leader = None;
        for n in self.nodes_of(g) {
            let v = self.nodes[n].rank;
            if self.graphs[g].maxrank < v {
                self.graphs[g].maxrank = v;
            }
            if self.graphs[g].minrank > v {
                self.graphs[g].minrank = v;
                leader = Some(n);
            }
        }
        self.graphs[g].leader = leader;
    }

    fn readout_levels(&mut self, g: G, ncc: i32) {
        self.graphs[g].minrank = i32::MAX;
        self.graphs[g].maxrank = -1;
        let mut minrk: Option<Vec<i32>> = if ncc > 1 {
            Some(vec![i32::MAX; (ncc + 1) as usize])
        } else {
            None
        };
        let mut do_root = false;
        for n in self.nodes_of(g) {
            let f = self.nr_find(n);
            let xn = self.nodes[f].rep.unwrap();
            self.nodes[n].rank = self.nodes[xn].rank;
            let r = self.nodes[n].rank;
            if self.graphs[g].maxrank < r {
                self.graphs[g].maxrank = r;
            }
            if self.graphs[g].minrank > r {
                self.graphs[g].minrank = r;
            }
            if let Some(m) = minrk.as_mut() {
                self.nodes[n].hops = self.nodes[xn].hops;
                let c = self.nodes[n].hops as usize;
                m[c] = m[c].min(r);
            }
        }
        if let Some(m) = minrk {
            for n in self.nodes_of(g) {
                let c = self.nodes[n].hops as usize;
                self.nodes[n].rank -= m[c];
            }
            do_root = true;
        } else if self.graphs[g].minrank > 0 {
            let delta = self.graphs[g].minrank;
            for n in self.nodes_of(g) {
                self.nodes[n].rank -= delta;
            }
            self.graphs[g].minrank -= delta;
            self.graphs[g].maxrank -= delta;
        }
        self.set_min_max(g, do_root);
        for n in self.nodes_of(g) {
            self.nodes[n].alg = None;
        }
    }

    fn dfscc(&mut self, g: G, n: N, cc: i32) {
        let mut stack = vec![n];
        // C recursion visits out-edges then in-edges depth-first; only the
        // component id matters, so order is irrelevant.
        while let Some(n) = stack.pop() {
            if self.nodes[n].hops != 0 {
                continue;
            }
            self.nodes[n].hops = cc;
            for e in self.out_edges(g, n) {
                stack.push(self.edges[e].head);
            }
            for e in self.in_edges(g, n) {
                stack.push(self.edges[e].tail);
            }
        }
    }

    fn connect_components(&mut self, g: G) -> i32 {
        let mut cc = 0;
        for n in self.nodes_of(g) {
            self.nodes[n].hops = 0;
        }
        for n in self.nodes_of(g) {
            if self.nodes[n].hops == 0 {
                cc += 1;
                self.dfscc(g, n, cc);
            }
        }
        if cc > 1 {
            let root = self.make_xnode(g, "\x7froot");
            let mut ncc = 1;
            for n in self.nodes_of(g) {
                if self.nodes[n].hops == ncc {
                    self.cg_edge(g, root, n);
                    ncc += 1;
                }
            }
        }
        cc
    }

    fn add_fast_edges(&mut self, g: G) {
        for n in self.nodes_of(g) {
            for e in self.out_edges(g, n) {
                self.nodes[n].out.push(e);
                let h = self.edges[e].head;
                self.nodes[h].in_.push(e);
            }
        }
    }

    pub fn dot2_rank(&mut self, g: G) -> Result<(), String> {
        self.xg_last = None;
        let xg = self.cg_open("level assignment constraints");
        self.edgelabel_ranks(g);
        let maxiter = match self.agget(g, "nslimit1") {
            Some(s) => scale_clamp(self.agnnodes(g) as i32, atof(s)),
            None => i32::MAX,
        };
        self.compile_samerank(g, None);
        self.compile_nodes(g, xg);
        self.compile_edges(g, xg);
        self.compile_clusters(g, xg, None, None);
        self.break_cycles(xg);
        let ncc = self.connect_components(xg);
        self.add_fast_edges(xg);
        let ssize = match self.agget(g, "searchsize") {
            Some(s) => atoi(s),
            None => -1,
        };
        self.rank2(xg, 1, maxiter, ssize)?;
        self.readout_levels(g, ncc);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{LayoutEngine, layout_graph};
    use graphviz_pure_core::{AttrMap, Graph, GraphKind, Point, SubgraphData, SubgraphId};

    // Issue #427: `graph G { packmode=array; newrank=true; subgraph cluster_c { {rank=same; a; b;} } f0 -- g0; z -- b; }`.
    // In a multi-component `pack` layout without `ratio`, C's `doDot` lays out each `_cc_<i>`
    // subgraph with `dotLayout(sg)`. Because `sg != sg->root`, `is_a_cluster(sg)` is false in C
    // (`utils.c:697`), so `compile_samerank` leaves `parent_clust == NULL`, top-level clusters
    // get `GD_level == 0` and `GD_parent == NULL` (never registered in `GD_clust(sg)`), and
    // nodes outside any cluster keep `ND_clust == NULL`. On `z -- b`, `dot_lca(NULL, cluster_c)`
    // (`rank.c:708-717`) then dereferences NULL (`GD_level(c0)`). In the port, `None` at level
    // `-1` in `dot_lca` plays the root cluster's role: `dot_lca(None, Some(cluster_c))` returns
    // `None == ct` (`is_internal_to_cluster` is true, as in a normal `newrank` layout where the
    // root is a level-0 cluster) and the graph is drawn without panicking.
    #[test]
    fn issue427_newrank_pack_component_edge_into_cluster_does_not_panic() {
        let mut g = Graph::new("G", GraphKind::Undirected, false);
        g.graph_attrs.insert("packmode".into(), "array".into());
        g.graph_attrs.insert("newrank".into(), "true".into());
        let [a, b, f0, g0, z] = ["a", "b", "f0", "g0", "z"].map(|n| g.ensure_node(n));
        g.add_edge(f0, g0, AttrMap::new());
        g.add_edge(z, b, AttrMap::new());
        g.subgraphs.push(SubgraphData {
            id: SubgraphId(0),
            name: "cluster_c".into(),
            is_cluster: true,
            graph_attrs: AttrMap::new(),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: vec![a, b],
            edges: Vec::new(),
            subgraphs: vec![SubgraphId(1)],
            bb: Default::default(),
            html_name: false,
            empty_name: false,
            layout: Default::default(),
        });
        g.subgraphs.push(SubgraphData {
            id: SubgraphId(1),
            name: String::new(),
            is_cluster: false,
            graph_attrs: AttrMap::from_plain([("rank".into(), "same".into())]),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: vec![a, b],
            edges: Vec::new(),
            subgraphs: Vec::new(),
            bb: Default::default(),
            html_name: false,
            empty_name: true,
            layout: Default::default(),
        });

        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        // `a` and `b` share rank 1 below `z` (rank 0), and `f0 -- g0` is packed beside them.
        assert_eq!(g.nodes[a.0 as usize].pos, Point::new(27.0, 18.0));
        assert_eq!(g.nodes[b.0 as usize].pos, Point::new(99.0, 18.0));
        assert_eq!(g.nodes[z.0 as usize].pos, Point::new(99.0, 90.0));
        assert_eq!(g.nodes[f0.0 as usize].pos, Point::new(161.0, 90.0));
        assert_eq!(g.nodes[g0.0 as usize].pos, Point::new(161.0, 18.0));
    }
}
