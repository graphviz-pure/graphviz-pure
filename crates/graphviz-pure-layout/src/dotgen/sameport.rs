//! Port of `lib/dotgen/sameport.c`: merge edge ports for `samehead`/`sametail`.

use super::Dot;
use super::types::*;
use graphviz_pure_core::Point;
use graphviz_pure_core::math;

impl Dot<'_> {
    fn attr_declared(&self, name: &str) -> bool {
        self.edge_attr_declared(name)
    }

    pub fn dot_sameports(&mut self) {
        let g = 0;
        let has_head = self.attr_declared("samehead");
        let has_tail = self.attr_declared("sametail");
        if !(has_head || has_tail) {
            return;
        }
        for n in self.nodes_of(g) {
            let mut samehead: Vec<(String, Vec<E>)> = Vec::new();
            let mut sametail: Vec<(String, Vec<E>)> = Vec::new();
            for e in self.all_edges(g, n) {
                let (t, h) = (self.edges[e].tail, self.edges[e].head);
                if t == h {
                    continue;
                }
                let hid = self.edge_attr(e, "samehead").unwrap_or("").to_string();
                let tid = self.edge_attr(e, "sametail").unwrap_or("").to_string();
                if h == n && has_head && !hid.is_empty() {
                    sameedge(&mut samehead, e, hid);
                } else if t == n && has_tail && !tid.is_empty() {
                    sameedge(&mut sametail, e, tid);
                }
            }
            for (_, l) in &samehead {
                if l.len() > 1 {
                    self.sameport(n, l);
                }
            }
            for (_, l) in &sametail {
                if l.len() > 1 {
                    self.sameport(n, l);
                }
            }
        }
    }

    fn sameport(&mut self, u: N, l: &[E]) {
        let (mut x, mut y) = (0.0f64, 0.0f64);
        for &e in l {
            let v = if self.edges[e].head == u {
                self.edges[e].tail
            } else {
                self.edges[e].head
            };
            let x1 = self.nodes[v].coord.x - self.nodes[u].coord.x;
            let y1 = self.nodes[v].coord.y - self.nodes[u].coord.y;
            let r = math::hypot(x1, y1);
            x += x1 / r;
            y += y1 / r;
        }
        // sameport.c:122: 0/0 where the ends cancel out, so the port is NaN, as in C
        // (`docs/porting/crashes.md`, "sameport's 0/0 direction").
        let r = math::hypot(x, y);
        x /= r;
        y /= r;
        let nu = &self.nodes[u];
        let (x1, y1) = (nu.coord.x, nu.coord.y);
        let r = (nu.lw + nu.rw).max(nu.ht + self.graphs[0].ranksep as f64);
        // C (clang) fuses these multiply-adds.
        let (x2, y2) = (math::fma(x, r, x1), math::fma(y, r, y1));
        // now move (x1,y1) to the node boundary
        let mut curve = [
            Point::new(x1, y1),
            Point::new((2.0 * x1 + x2) / 3.0, (2.0 * y1 + y2) / 3.0),
            Point::new((2.0 * x2 + x1) / 3.0, (2.0 * y2 + y1) / 3.0),
            Point::new(x2, y2),
        ];
        crate::common::splines::shape_clip(self, u, &mut curve);
        let p = Point::new(curve[0].x - x1, curve[0].y - y1);
        let pr = Point::new(p.x.round(), p.y.round());
        let nu = &self.nodes[u];
        let order = ((MC_SCALE as f64 * (nu.lw + pr.x)) / (nu.lw + nu.rw)) as i32 as u8;
        let prt = Port {
            p: pr,
            order,
            defined: true,
            ..Port::default()
        };
        for &e0 in l {
            let mut e = Some(e0);
            while let Some(ee) = e {
                let mut f = Some(ee);
                while let Some(ff) = f {
                    if self.edges[ff].head == u {
                        self.edges[ff].head_port = prt.clone();
                    }
                    if self.edges[ff].tail == u {
                        self.edges[ff].tail_port = prt.clone();
                    }
                    let hd = self.edges[ff].head;
                    f = if self.edges[ff].edge_type == VIRTUAL
                        && self.nodes[hd].node_type == VIRTUAL
                        && self.nodes[hd].out.len() == 1
                    {
                        Some(self.nodes[hd].out[0])
                    } else {
                        None
                    };
                }
                let mut f = Some(ee);
                while let Some(ff) = f {
                    if self.edges[ff].head == u {
                        self.edges[ff].head_port = prt.clone();
                    }
                    if self.edges[ff].tail == u {
                        self.edges[ff].tail_port = prt.clone();
                    }
                    let tl = self.edges[ff].tail;
                    f = if self.edges[ff].edge_type == VIRTUAL
                        && self.nodes[tl].node_type == VIRTUAL
                        && self.nodes[tl].in_.len() == 1
                    {
                        Some(self.nodes[tl].in_[0])
                    } else {
                        None
                    };
                }
                e = self.edges[ee].to_virt;
            }
        }
        self.nodes[u].has_port = true;
    }
}

fn sameedge(same: &mut Vec<(String, Vec<E>)>, e: E, id: String) {
    for s in same.iter_mut() {
        if s.0 == id {
            s.1.push(e);
            return;
        }
    }
    same.push((id, vec![e]));
}
