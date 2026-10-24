//! [`SplineGraph`] for dot: exposes the dot arenas to the shared spline code
//! of `lib/common` (`clip_and_install`, `beginpath`/`endpath`, self loops,
//! port labels), plus the dot sides of `resolvePort` and `record_path`
//! (lib/common/shapes.c).

use super::Dot;
use super::types::*;
use crate::common::arrows::arrow_flags;
use crate::common::shapes::NodeShape;
use crate::common::splines::{LabelKind, LabelView, SplineGraph};
use graphviz_pure_core::{AttrMap, GraphKind, Point};

static EMPTY_ATTRS: std::sync::LazyLock<AttrMap> = std::sync::LazyLock::new(AttrMap::new);

impl Dot<'_> {
    fn label_index(&self, e: E, k: LabelKind) -> Option<L> {
        let ed = &self.edges[e];
        match k {
            LabelKind::Main => ed.label,
            LabelKind::Head => ed.head_label,
            LabelKind::Tail => ed.tail_label,
            LabelKind::Xlabel => ed.xlabel,
        }
    }

    /// Whether edges declare attribute `k` (`agattr(g, AGEDGE, k, NULL)`), as the global
    /// symbol dot reads it: not one setState NULLed on an aux graph.
    pub fn edge_attr_declared(&self, k: &str) -> bool {
        !self.null_syms.1.contains(&k) && self.root_input.edge_attr_declared(k)
    }

    /// [`Dot::edge_attr_declared`] for nodes (`agattr(g, AGNODE, k, NULL)`).
    pub fn node_attr_declared(&self, k: &str) -> bool {
        !self.null_syms.0.contains(&k) && self.root_input.node_attr_declared(k)
    }
}

impl SplineGraph for Dot<'_> {
    fn tail(&self, e: usize) -> usize {
        self.edges[e].tail
    }
    fn head(&self, e: usize) -> usize {
        self.edges[e].head
    }
    fn coord(&self, n: usize) -> Point {
        self.nodes[n].coord
    }
    fn lw(&self, n: usize) -> f64 {
        self.nodes[n].lw
    }
    fn rw(&self, n: usize) -> f64 {
        self.nodes[n].rw
    }
    fn ht(&self, n: usize) -> f64 {
        self.nodes[n].ht
    }
    fn shape(&self, n: usize) -> Option<&NodeShape> {
        self.nodes[n].shape.as_ref()
    }
    fn rankdir(&self) -> i32 {
        self.graphs[0].rankdir
    }
    fn flip(&self) -> bool {
        self.graphs[0].flip()
    }
    fn ranksep(&self) -> f64 {
        f64::from(self.graphs[0].ranksep)
    }
    fn rank(&self, n: usize) -> i32 {
        self.nodes[n].rank
    }
    fn order(&self, n: usize) -> i32 {
        self.nodes[n].order
    }
    fn is_normal_node(&self, n: usize) -> bool {
        self.nodes[n].node_type == NORMAL
    }
    fn to_orig(&self, e: usize) -> Option<usize> {
        self.edges[e].to_orig
    }
    fn is_normal_edge(&self, e: usize) -> bool {
        self.edges[e].edge_type == NORMAL
    }
    fn is_ignored_edge(&self, e: usize) -> bool {
        self.edges[e].edge_type == IGNORED
    }
    fn tail_port(&self, e: usize) -> &Port {
        &self.edges[e].tail_port
    }
    fn head_port(&self, e: usize) -> &Port {
        &self.edges[e].head_port
    }
    fn tail_port_mut(&mut self, e: usize) -> &mut Port {
        &mut self.edges[e].tail_port
    }
    fn head_port_mut(&mut self, e: usize) -> &mut Port {
        &mut self.edges[e].head_port
    }
    fn edge_attrs(&self, e: usize) -> &AttrMap {
        match self.edges[e].real {
            Some(r) => &self.input.edges[r].attrs,
            None => &EMPTY_ATTRS,
        }
    }
    /// `arrow_flags(e)`, including the `ED_conc_opp_flag` merge of the
    /// opposing edge's arrowheads.
    fn arrow_flags(&self, e: usize) -> (u32, u32) {
        let directed = self.input.kind == GraphKind::Directed;
        let (mut s, mut f) = arrow_flags(self.edge_attrs(e), directed);
        if self.edges[e].conc_opp_flag {
            let (t, h) = (self.edges[e].tail, self.edges[e].head);
            if let Some(o) = self.agfindedge_root(h, t) {
                let (s0, e0) = arrow_flags(self.edge_attrs(o), directed);
                f |= s0;
                s |= e0;
            }
        }
        (s, f)
    }
    fn swap_ends(&self, e: usize) -> bool {
        self.swap_ends_p(e)
    }
    fn spline_merge(&self, n: usize) -> bool {
        self.spline_merge_p(n)
    }
    fn fast_in(&self, n: usize) -> &[usize] {
        &self.nodes[n].in_
    }
    fn fast_out(&self, n: usize) -> &[usize] {
        &self.nodes[n].out
    }
    fn spl(&self, e: usize) -> Option<&Vec<Bezier>> {
        self.edges[e].spl.as_ref()
    }
    fn spl_mut(&mut self, e: usize) -> &mut Option<Vec<Bezier>> {
        &mut self.edges[e].spl
    }
    /// `GD_bb(agraphof(n))`: the root graph's box, which is not the layout
    /// graph's when this is a pack component.
    fn bb_mut(&mut self) -> &mut BoxF {
        if self.comp {
            &mut self.root_bb
        } else {
            &mut self.graphs[0].bb
        }
    }
    fn label(&self, e: usize, k: LabelKind) -> Option<LabelView> {
        let l = &self.labels[self.label_index(e, k)?];
        Some(LabelView {
            dimen: l.dimen,
            pos: l.pos,
            set: l.set,
        })
    }
    fn set_label_pos(&mut self, e: usize, k: LabelKind, pos: Point) {
        if let Some(l) = self.label_index(e, k) {
            self.labels[l].pos = pos;
            self.labels[l].set = true;
        }
    }
    fn has_port_label_attrs(&self) -> bool {
        self.edge_attr_declared("labelangle") || self.edge_attr_declared("labeldistance")
    }
    /// `resolvePort(n, other, oldport)`.
    fn resolve_port(&self, n: usize, other: usize, old: &Port) -> Port {
        crate::common::ports::resolve_port(self, n, other, old)
    }
    /// `record_path(n, prt, side, rv, &k)`; polygons (`poly_path`) have none.
    fn pboxfn(&self, n: usize, prt: &Port, side: u8) -> Option<(u8, Option<Vec<BoxF>>)> {
        let nd = &self.nodes[n];
        if !nd.is_record || nd.rec_fields.is_empty() {
            return None;
        }
        if !prt.defined {
            return Some((0, None));
        }
        let p = prt.p;
        let flip = self.graphs[0].flip();
        for f in &nd.rec_fields {
            let (ls, rs) = if flip {
                (f.ll.y, f.ur.y)
            } else {
                (f.ll.x, f.ur.x)
            };
            if ls <= p.x && p.x <= rs {
                let c = nd.coord;
                let mut b = if flip {
                    BoxF::new(f.ll.y + c.x, f.ll.x + c.y, f.ur.y + c.x, f.ur.x + c.y)
                } else {
                    BoxF::new(c.x + ls, c.y - nd.ht / 2.0, c.x + rs, 0.0)
                };
                b.ur.y = c.y + nd.ht / 2.0;
                return Some((side, Some(vec![b])));
            }
        }
        // No field contains the port: C returns `side` and writes no box (#360).
        Some((side, None))
    }
}

impl crate::common::xlabels::XLabelGraph for Dot<'_> {
    fn xl_node_xlabel_text(&self, n: usize) -> String {
        self.nodes
            .get(n)
            .and_then(|nd| nd.xlabel)
            .and_then(|l| self.labels.get(l))
            .map(|l| l.text.clone())
            .unwrap_or_default()
    }
    fn xl_label_text(&self, e: usize, k: LabelKind) -> String {
        self.label_index(e, k)
            .map(|l| self.labels[l].text.clone())
            .unwrap_or_default()
    }
    fn xl_nodes(&self) -> Vec<usize> {
        self.nodes_of(0)
    }
    fn xl_out_edges(&self, n: usize) -> Vec<usize> {
        self.out_edges(0, n)
    }
    fn xl_node_size(&self, n: usize) -> Point {
        Point::new(self.nodes[n].width * 72.0, self.nodes[n].height * 72.0)
    }
    fn node_xlabel(&self, n: usize) -> Option<LabelView> {
        let l = &self.labels[self.nodes[n].xlabel?];
        Some(LabelView {
            dimen: l.dimen,
            pos: l.pos,
            set: l.set,
        })
    }
    fn set_node_xlabel_pos(&mut self, n: usize, pos: Point) {
        if let Some(l) = self.nodes[n].xlabel {
            self.labels[l].pos = pos;
            self.labels[l].set = true;
        }
    }
    fn xl_cluster_labels(&self) -> Vec<LabelView> {
        fn rec(d: &Dot<'_>, g: G, out: &mut Vec<LabelView>) {
            for &c in &d.graphs[g].clust {
                rec(d, c, out);
            }
            if g != 0
                && let Some(l) = d.graphs[g].label
                && d.labels[l].set
            {
                let l = &d.labels[l];
                out.push(LabelView {
                    dimen: l.dimen,
                    pos: l.pos,
                    set: true,
                });
            }
        }
        let mut out = Vec::new();
        rec(self, 0, &mut out);
        out
    }
    fn xl_edge_type(&self) -> crate::common::types::EdgeType {
        self.et
    }
    fn xl_forcelabels(&self) -> bool {
        super::utils::map_bool(self.agget(0, "forcelabels"), true)
    }
}

impl crate::common::ports::PortGraph for Dot<'_> {
    fn port_node(&self, n: usize) -> &graphviz_pure_core::NodeData {
        &self.input.nodes[self.nodes[n].real.expect("port on a virtual node")]
    }
    fn port_env(&self) -> graphviz_pure_geom::NodeSizeEnv<'_> {
        graphviz_pure_geom::NodeSizeEnv::from_graph(self.input)
    }
}

impl crate::ortho::OrthoGraph for Dot<'_> {
    fn ortho_nodes(&self) -> Vec<usize> {
        self.nodes_of(0)
    }
    fn ortho_out_edges(&self, n: usize) -> Vec<usize> {
        self.out_edges(0, n)
    }
    fn ortho_concentrate(&self) -> bool {
        self.concentrate
    }
    fn node_seq(&self, n: usize) -> usize {
        n
    }
    fn xl_name(&self, n: usize) -> String {
        self.nodes[n].name.clone()
    }
}
