//! Port of `lib/dotgen/class2.c`: classify edges for mincross/position/splines
//! using the assigned ranks (virtual node chains, flat edges, merged
//! multi-edges, cluster skeleton edges).

use super::Dot;
use super::fastgr::short;
use super::types::*;

impl Dot<'_> {
    fn label_vnode(&mut self, g: G, orig: E) -> N {
        let l = self.edges[orig].label.unwrap();
        let dimen = self.labels[l].dimen;
        let v = self.virtual_node(g);
        self.nodes[v].label = Some(l);
        if std::env::var_os("GVDUMP").is_some() {
            let txt = self.edge_attr(orig, "label").unwrap_or("").to_string();
            graphviz_pure_core::diag::dump(&format!("LV [{}] {} {}", txt, dimen.x, dimen.y));
        }
        self.nodes[v].lw = self.graphs[0].nodesep as f64;
        if !self.edges[orig].label_ontop {
            if self.graphs[0].flip() {
                self.nodes[v].ht = dimen.x;
                self.nodes[v].rw = dimen.y;
            } else {
                self.nodes[v].ht = dimen.y;
                self.nodes[v].rw = dimen.x;
            }
        }
        v
    }

    fn incr_width(&mut self, g: G, v: N) {
        let width = self.graphs[g].nodesep / 2;
        self.nodes[v].lw += width as f64;
        self.nodes[v].rw += width as f64;
    }

    fn plain_vnode(&mut self, g: G) -> N {
        let v = self.virtual_node(g);
        self.incr_width(g, v);
        v
    }

    fn leader_of(&mut self, v: N) -> N {
        if self.nodes[v].ranktype != CLUSTER {
            self.uf_find(v)
        } else {
            let clust = self.nodes[v].clust.unwrap();
            self.graphs[clust].rankleader[self.nodes[v].rank as usize].unwrap()
        }
    }

    /// `make_chain`: chain of dummy nodes for edge `orig`.
    pub fn make_chain(&mut self, g: G, from: N, to: N, orig: E) -> Result<(), String> {
        let label_rank = if self.edges[orig].label.is_some() {
            (self.nodes[from].rank + self.nodes[to].rank) / 2
        } else {
            -1
        };
        let mut u = from;
        let to_rank = self.nodes[to].rank;
        let mut r = self.nodes[from].rank + 1;
        while r <= to_rank {
            // Not in C: a limit of the caller's `Limits::run` stops the chain (a large
            // `minlen` asks for many virtual nodes).
            if graphviz_pure_core::limits::tick() {
                return Err(graphviz_pure_core::limits::STOPPED.to_string());
            }
            let v = if r < to_rank {
                let v = if r == label_rank {
                    self.label_vnode(g, orig)
                } else {
                    self.plain_vnode(g)
                };
                self.nodes[v].rank = r;
                v
            } else {
                to
            };
            let e = self.virtual_edge(u, v, Some(orig));
            self.virtual_weight(e)?;
            u = v;
            r += 1;
        }
        Ok(())
    }

    fn interclrep(&mut self, g: G, e: E) -> Result<(), String> {
        let mut t = self.leader_of(self.edges[e].tail);
        let mut h = self.leader_of(self.edges[e].head);
        if self.nodes[t].rank > self.nodes[h].rank {
            std::mem::swap(&mut t, &mut h);
        }
        if self.nodes[t].clust != self.nodes[h].clust {
            if let Some(ve) = self.find_fast_edge(t, h) {
                self.merge_chain(g, e, ve, true);
                return Ok(());
            }
            if self.nodes[t].rank == self.nodes[h].rank {
                return Ok(());
            }
            self.make_chain(g, t, h, e)?;
            let hr = self.nodes[h].rank;
            let mut ve = self.edges[e].to_virt;
            while let Some(x) = ve {
                let hd = self.edges[x].head;
                if self.nodes[hd].rank > hr {
                    break;
                }
                self.edges[x].edge_type = CLUSTER_EDGE;
                ve = self.nodes[hd].out.first().copied();
            }
        }
        Ok(())
    }

    fn is_cluster_edge(&self, e: E) -> bool {
        self.nodes[self.edges[e].tail].ranktype == CLUSTER
            || self.nodes[self.edges[e].head].ranktype == CLUSTER
    }

    pub fn merge_chain(&mut self, g: G, e: E, f: E, update_count: bool) {
        let lastrank = self.nodes[self.edges[e].tail]
            .rank
            .max(self.nodes[self.edges[e].head].rank);
        self.edges[e].to_virt = Some(f);
        let mut rep = Some(f);
        while let Some(r) = rep {
            if update_count {
                self.edges[r].count = short(self.edges[r].count + self.edges[e].count);
            }
            self.edges[r].xpenalty = short(self.edges[r].xpenalty + self.edges[e].xpenalty);
            // No overflow check in C (undefined behaviour); the port wraps.
            let w = self.edges[e].weight;
            let rw = &mut self.edges[r].weight;
            *rw = rw.wrapping_add(w);
            let hd = self.edges[r].head;
            if self.nodes[hd].rank == lastrank {
                break;
            }
            self.incr_width(g, hd);
            rep = self.nodes[hd].out.first().copied();
        }
    }

    pub fn mergeable(&self, e: Option<E>, f: Option<E>) -> bool {
        match (e, f) {
            (Some(e), Some(f)) => {
                let (a, b) = (&self.edges[e], &self.edges[f]);
                a.tail == b.tail && a.head == b.head && (a.label == b.label) && self.ports_eq(e, f)
            }
            _ => false,
        }
    }

    pub fn class2(&mut self, g: G) -> Result<(), String> {
        self.graphs[g].nlist = None;
        self.mark_clusters(g);
        for c in self.graphs[g].clust.clone() {
            self.build_skeleton(g, c)?;
        }
        for n in self.nodes_of(g) {
            for e in self.out_edges(g, n) {
                let h = self.edges[e].head;
                if self.nodes[h].weight_class <= 2 {
                    self.nodes[h].weight_class += 1;
                }
                let t = self.edges[e].tail;
                if self.nodes[t].weight_class <= 2 {
                    self.nodes[t].weight_class += 1;
                }
            }
        }
        for n in self.nodes_of(g) {
            if self.nodes[n].clust.is_none() && n == self.uf_find(n) {
                self.fast_node(g, n);
            }
            let mut prev: Option<E> = None;
            for e in self.out_edges(g, n) {
                if self.edges[e].to_virt.is_some() {
                    prev = Some(e);
                    continue;
                }
                if self.is_cluster_edge(e) {
                    if self.mergeable(prev, Some(e)) {
                        let p = prev.unwrap();
                        if let Some(pv) = self.edges[p].to_virt {
                            self.merge_chain(g, e, pv, false);
                            self.other_edge(e);
                        } else if self.nodes[self.edges[e].tail].rank
                            == self.nodes[self.edges[e].head].rank
                        {
                            self.merge_oneway(e, p);
                            self.other_edge(e);
                        }
                        continue;
                    }
                    self.interclrep(g, e)?;
                    prev = Some(e);
                    continue;
                }
                let (et, eh) = (self.edges[e].tail, self.edges[e].head);
                if let Some(p) = prev
                    && et == self.edges[p].tail
                    && eh == self.edges[p].head
                {
                    if self.nodes[et].rank == self.nodes[eh].rank {
                        self.merge_oneway(e, p);
                        self.other_edge(e);
                        continue;
                    }
                    if self.edges[e].label.is_none()
                        && self.edges[p].label.is_none()
                        && self.ports_eq(e, p)
                    {
                        if self.concentrate {
                            self.edges[e].edge_type = IGNORED;
                        } else {
                            let pv = self.edges[p].to_virt.unwrap();
                            self.merge_chain(g, e, pv, true);
                            self.other_edge(e);
                        }
                        continue;
                    }
                }
                if et == eh {
                    self.other_edge(e);
                    prev = Some(e);
                    continue;
                }
                let t = self.uf_find(et);
                let h = self.uf_find(eh);
                if et != t || eh != h {
                    continue;
                }
                if self.nodes[et].rank == self.nodes[eh].rank {
                    self.flat_edge(g, e);
                    prev = Some(e);
                    continue;
                }
                if self.nodes[eh].rank > self.nodes[et].rank {
                    self.make_chain(g, et, eh, e)?;
                    prev = Some(e);
                    continue;
                }
                // backward edges
                let mut found_opp = false;
                for opp in self.out_edges(g, eh) {
                    if self.edges[opp].head != et
                        || self.edges[opp].head == eh
                        || self.edges[opp].edge_type == IGNORED
                    {
                        continue;
                    }
                    if self.edges[opp].to_virt.is_none() {
                        let (ot, oh) = (self.edges[opp].tail, self.edges[opp].head);
                        self.make_chain(g, ot, oh, opp)?;
                    }
                    if self.edges[e].label.is_none()
                        && self.edges[opp].label.is_none()
                        && self.ports_eq(e, opp)
                    {
                        if self.concentrate {
                            self.edges[e].edge_type = IGNORED;
                            self.edges[opp].conc_opp_flag = true;
                        } else {
                            self.other_edge(e);
                            let ov = self.edges[opp].to_virt.unwrap();
                            self.merge_chain(g, e, ov, true);
                        }
                        found_opp = true;
                        break;
                    }
                }
                if found_opp {
                    continue;
                }
                self.make_chain(g, eh, et, e)?;
                prev = Some(e);
            }
        }
        if g != 0 {
            let nl = self.graphs[g].nlist;
            self.graphs[g].comp = vec![nl];
        }
        Ok(())
    }

    /// `ports_eq` (position.c)
    pub fn ports_eq(&self, e: E, f: E) -> bool {
        let (a, b) = (&self.edges[e], &self.edges[f]);
        a.head_port.defined == b.head_port.defined
            && ((a.head_port.p.x == b.head_port.p.x && a.head_port.p.y == b.head_port.p.y)
                || !a.head_port.defined)
            && ((a.tail_port.p.x == b.tail_port.p.x && a.tail_port.p.y == b.tail_port.p.y)
                || !a.tail_port.defined)
    }
}
