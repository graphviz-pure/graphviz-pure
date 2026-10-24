//! Construction of the dot state from the input graph, plus ports of
//! `graph_init`/`do_graph_label` (lib/common/input.c), `dot_init_subg` and
//! `dot_init_node_edge` (lib/dotgen/dotinit.c), `common_init_node`,
//! `common_init_edge` (lib/common/utils.c) and the port resolution of
//! `poly_port`/`record_port`/`compassPort` (lib/common/shapes.c).

use super::Dot;
use super::types::*;
use super::utils::*;
use graphviz_pure_core::strtod::{atof, sscanf};
use graphviz_pure_core::{Graph, GraphKind, Point};

const GAP: f64 = 4.0;

/// `PAD(d)`: `d.x += 4*GAP; d.y += 2*GAP`.
pub fn pad(d: Point) -> Point {
    Point::new(d.x + 4.0 * GAP, d.y + 2.0 * GAP)
}

pub use crate::common::ports::{ccwrotatepf, cwrotatepf};

impl<'a> Dot<'a> {
    /// Builds the arenas from the (already node-sized) input graph.
    pub fn new(input: &'a Graph) -> Self {
        let mut d = Dot::empty(input);
        let nn = input.nodes.len();
        let ne = input.edges.len();
        // Root graph.
        d.graphs.push(GraphInfo {
            name: input.name.clone(),
            root: 0,
            attrs: input.graph_attrs.clone(),
            bound: true,
            ..GraphInfo::default()
        });
        for (i, n) in input.nodes.iter().enumerate() {
            d.nodes.push(NodeInfo {
                real: Some(i),
                name: n.name.clone(),
                cg_root: Some(0),
                ..NodeInfo::default()
            });
            d.graphs[0].nodes.insert(i);
        }
        for (i, e) in input.edges.iter().enumerate() {
            let (t, h) = (e.tail.0 as usize, e.head.0 as usize);
            d.edges.push(EdgeInfo {
                tail: t,
                head: h,
                real: Some(i),
                seq: (i + 1) as u64,
                ..EdgeInfo::default()
            });
            d.nodes[t].cg_out.push(i);
            d.nodes[h].cg_in.push(i);
            d.graphs[0].edges.insert(i);
        }
        d.edge_seq = ne as u64;
        // cgraph orders a node's out-edges by (head seq, edge seq) and its in-edges by
        // (tail seq, edge seq) (`agedgeseqcmpf`).
        for n in 0..nn {
            let mut o = std::mem::take(&mut d.nodes[n].cg_out);
            o.sort_by_key(|&e| (d.edges[e].head, d.edges[e].seq));
            d.nodes[n].cg_out = o;
            let mut i = std::mem::take(&mut d.nodes[n].cg_in);
            i.sort_by_key(|&e| (d.edges[e].tail, d.edges[e].seq));
            d.nodes[n].cg_in = i;
        }
        d.n_real_nodes = nn;
        // Subgraphs: create in input order, then wire the hierarchy.
        let base = d.graphs.len();
        let mut is_child = vec![false; input.subgraphs.len()];
        for s in &input.subgraphs {
            for c in &s.subgraphs {
                is_child[c.0 as usize] = true;
            }
        }
        for (i, s) in input.subgraphs.iter().enumerate() {
            let mut gi = GraphInfo {
                name: s.name.clone(),
                root: 0,
                attrs: s.graph_attrs.clone(),
                sub_index: Some(i),
                ..GraphInfo::default()
            };
            for nid in &s.nodes {
                gi.nodes.insert(nid.0 as usize);
            }
            for eid in &s.edges {
                gi.edges.insert(eid.0 as usize);
            }
            d.graphs.push(gi);
        }
        for (i, s) in input.subgraphs.iter().enumerate() {
            for c in &s.subgraphs {
                let ci = base + c.0 as usize;
                if d.graphs[ci].cg_parent.is_none() {
                    d.graphs[ci].cg_parent = Some(base + i);
                    d.graphs[base + i].subgs.push(ci);
                }
            }
        }
        for (i, child) in is_child.iter().enumerate() {
            if !child {
                d.graphs[base + i].cg_parent = Some(0);
                d.graphs[0].subgs.push(base + i);
            }
        }
        // Each subgraph's lists already hold its nested subgraphs' nodes and edges.
        d
    }

    /// `graph_init` (the parts dot uses).
    pub fn graph_init(&mut self) {
        let g = 0;
        let mut rankdir = RANKDIR_TB;
        if let Some(p) = self.agget(g, "rankdir") {
            rankdir = match p {
                "LR" => RANKDIR_LR,
                "BT" => RANKDIR_BT,
                "RL" => RANKDIR_RL,
                _ => RANKDIR_TB,
            };
        }
        self.graphs[g].rankdir = rankdir;
        let mut xf = late_double(self.agget(g, "nodesep"), 0.25, 0.02);
        self.graphs[g].nodesep = points(xf);
        if let Some(p) = self.agget(g, "ranksep") {
            let r = sscanf(p, "%lf");
            match r.count {
                0 => xf = 0.5,
                // EOF (an empty or blank value) assigns nothing: `xf` keeps the nodesep.
                -1 => xf = if xf < 0.02 { 0.02 } else { xf },
                _ => {
                    xf = if r.values[0] < 0.02 {
                        0.02
                    } else {
                        r.values[0]
                    }
                }
            }
            if p.contains("equally") {
                self.graphs[g].exact_ranksep = true;
            }
        } else {
            xf = 0.5;
        }
        self.graphs[g].ranksep = points(xf);
        // setRatio
        if let Some(p) = self.agget(g, "ratio").map(str::to_string) {
            let dr = &mut self.graphs[g].drawing;
            match p.as_str() {
                "auto" => dr.ratio_kind = RatioKind::Auto,
                "compress" => dr.ratio_kind = RatioKind::Compress,
                "expand" => dr.ratio_kind = RatioKind::Expand,
                "fill" => dr.ratio_kind = RatioKind::Fill,
                _ => {
                    let r = atof(&p);
                    if r > 0.0 {
                        dr.ratio_kind = RatioKind::Value;
                        dr.ratio = r;
                    }
                }
            }
        }
        let (size, filled) = getdoubles2ptf(self.agget(g, "size"));
        if let Some(s) = size {
            self.graphs[g].drawing.size = s;
        }
        self.graphs[g].drawing.filled = filled;
        if let (Some(p), _) = getdoubles2ptf(self.agget(g, "page")) {
            self.graphs[g].drawing.page = p;
        }
        let p = self.agget(g, "clusterrank");
        self.cl_type = match p {
            Some("local") => LOCAL,
            Some("global") => GLOBAL,
            Some("none") => NOCLUST,
            _ => LOCAL,
        };
        self.concentrate = mapbool(self.agget(g, "concentrate"));
        self.do_graph_label(g);
    }

    /// `do_graph_label`
    pub fn do_graph_label(&mut self, sg: G) {
        let flip = self.graphs[0].flip();
        // `agnameof(sg)`, which `\G` substitutes; the root label is the input root's.
        let name = if sg == 0 {
            self.input.root_name().to_string()
        } else {
            self.graph_name(sg)
        };
        let html_name = self.graphs[sg]
            .sub_index
            .is_some_and(|i| self.input.subgraphs[i].html_name);
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(self.input);
        let make = || {
            graphviz_pure_geom::graph_label_geometry_in(
                &name,
                html_name,
                &self.graphs[sg].attrs,
                sg == 0,
                flip,
                &env,
            )
        };
        // layout_graph_with_options made the root label first, as graph_init does.
        let gl = if sg == 0 {
            graphviz_pure_core::diag::quietly(make)
        } else {
            make()
        };
        let Some(gl) = gl else {
            return;
        };
        self.graphs[0].has_labels |= GRAPH_LABEL;
        let l = self.push_label(&gl.label);
        self.graphs[sg].label = Some(l);
        self.graphs[sg].label_pos = gl.pos_flag;
        if let Some((side, d)) = gl.border {
            self.graphs[sg].border[side as usize] = d;
        }
    }

    fn push_label(&mut self, lg: &graphviz_pure_geom::LabelGeometry) -> L {
        self.push_label_text(lg, String::new())
    }

    /// [`Self::push_label`] keeping `lp->text` (`text`), which `addXLabels` reports.
    fn push_label_text(&mut self, lg: &graphviz_pure_geom::LabelGeometry, text: String) -> L {
        let l = self.labels.len();
        self.labels.push(TextLabel {
            text,
            fontname: lg.font_name.clone(),
            fontsize: lg.font_size,
            dimen: lg.dimen,
            space: lg.space,
            html: lg.html,
            ..TextLabel::default()
        });
        l
    }

    /// `dot_init_subg(g, g)`: every subgraph gets an Agraphinfo_t record.
    pub fn dot_init_subg(&mut self) {
        for g in 0..self.graphs.len() {
            self.graphs[g].bound = true;
        }
    }

    /// `dot_init_node_edge`
    pub fn dot_init_node_edge(&mut self) {
        let flip = self.graphs[0].flip();
        for n in self.nodes_of(0) {
            self.dot_init_node(n, flip);
        }
        for n in self.nodes_of(0) {
            for e in self.out_edges(0, n) {
                self.dot_init_edge(e);
            }
        }
    }

    fn dot_init_node(&mut self, n: N, flip: bool) {
        let r = self.nodes[n].real.unwrap();
        let nd = &self.input.nodes[r];
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(self.input);
        let geom = graphviz_pure_geom::node_geometry(nd, &env);
        // lp->text, which addXLabels' -v trace prints
        let xlabel_text = geom.xlabel.as_ref().map(|xl| {
            graphviz_pure_geom::label_text(xl, nd.attrs.get("xlabel").map_or("", String::as_str))
        });
        let (lw, rw, ht) = geom.nodesize(flip);
        let mut sized = nd.clone();
        sized.width = geom.width;
        sized.height = geom.height;
        let mut shape = crate::common::shapes::NodeShape::new(&sized);
        shape.apply_geometry(&geom);
        let rec_fields = geom
            .record
            .as_ref()
            .map(|r| r.children.iter().map(|f| f.bbox).collect())
            .unwrap_or_default();
        let info = &mut self.nodes[n];
        info.shape = Some(shape);
        info.rec_fields = rec_fields;
        info.width = geom.width;
        info.height = geom.height;
        info.is_record = geom.record.is_some();
        info.lw = lw;
        info.rw = rw;
        info.ht = ht;
        info.uf_size = 1;
        if let Some(xl) = &geom.xlabel {
            let l = self.push_label_text(xl, xlabel_text.unwrap_or_default());
            self.nodes[n].xlabel = Some(l);
            self.graphs[0].has_labels |= NODE_XLABEL;
        }
    }

    fn dot_init_edge(&mut self, e: E) {
        self.common_init_edge(e);
        let mut weight = late_int(self.edge_attr(e, "weight"), 1, 0);
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        let tg = self.node_attr(t, "group").unwrap_or("");
        let hg = self.node_attr(h, "group").unwrap_or("");
        // `tailgroup == headgroup` compares interned strings, so the HTML flag counts.
        let html = |n: N| {
            self.nodes
                .get(n)
                .and_then(|nd| nd.real)
                .and_then(|r| self.input.nodes.get(r))
                .is_some_and(|nd| nd.attrs.is_html("group"))
        };
        let mut xpenalty = 1;
        if !tg.is_empty() && tg == hg && html(t) == html(h) {
            xpenalty = CL_CROSS;
            // No overflow check in C (undefined behaviour); the port wraps.
            weight = weight.wrapping_mul(100);
        }
        if self.nonconstraint_edge(e) {
            xpenalty = 0;
            weight = 0;
        }
        let minlen = late_int(self.edge_attr(e, "minlen"), 1, 0);
        let ed = &mut self.edges[e];
        ed.count = 1;
        ed.xpenalty = xpenalty;
        ed.weight = weight;
        ed.minlen = minlen;
    }

    fn common_init_edge(&mut self, e: E) {
        let label_text =
            |lg: &graphviz_pure_geom::LabelGeometry, a: &graphviz_pure_core::AttrMap, key: &str| {
                graphviz_pure_geom::label_text(lg, a.get(key).map_or("", String::as_str))
            };
        let r = self.edges[e].real.unwrap();
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(self.input);
        let directed = self.input.kind == GraphKind::Directed;
        let input = self.input;
        let ie = &input.edges[r];
        let geo = graphviz_pure_geom::node_size::edge_label_geometry_of(
            u64::from(ie.id.0),
            &ie.attrs,
            &self.nodes[t].name,
            &self.nodes[h].name,
            directed,
            &env,
        );
        if let Some(lg) = &geo.label {
            let text = label_text(lg, &ie.attrs, "label");
            let l = self.push_label_text(lg, text);
            self.edges[e].label = Some(l);
            self.graphs[0].has_labels |= EDGE_LABEL;
            self.edges[e].label_ontop = geo.label_float;
        }
        if let Some(lg) = &geo.xlabel {
            let text = label_text(lg, &ie.attrs, "xlabel");
            let l = self.push_label_text(lg, text);
            self.edges[e].xlabel = Some(l);
            self.graphs[0].has_labels |= EDGE_XLABEL;
        }
        if let Some(lg) = &geo.head_label {
            let text = label_text(lg, &ie.attrs, "headlabel");
            let l = self.push_label_text(lg, text);
            self.edges[e].head_label = Some(l);
            self.graphs[0].has_labels |= HEAD_LABEL;
        }
        if let Some(lg) = &geo.tail_label {
            let text = label_text(lg, &ie.attrs, "taillabel");
            let l = self.push_label_text(lg, text);
            self.edges[e].tail_label = Some(l);
            self.graphs[0].has_labels |= TAIL_LABEL;
        }
        let ie = &self.input.edges[r];
        let tp = ie.tail_port.clone();
        let hp = ie.head_port.clone();
        let tail_str = port_string(&tp);
        if !tail_str.is_empty() {
            self.nodes[t].has_port = true;
        }
        let mut port = self.chk_port(t, &tp);
        if no_clip(self.edge_attr(e, "tailclip")) {
            port.clip = false;
        }
        self.edges[e].tail_port = port;
        let head_str = port_string(&hp);
        if !head_str.is_empty() {
            self.nodes[h].has_port = true;
        }
        let mut port = self.chk_port(h, &hp);
        if no_clip(self.edge_attr(e, "headclip")) {
            port.clip = false;
        }
        self.edges[e].head_port = port;
    }

    /// `chkPort(ND_shape(n)->fns->portfn, n, s)`.
    fn chk_port(&self, n: N, p: &graphviz_pure_core::Port) -> Port {
        crate::common::ports::chk_port(self, n, p)
    }
}

fn port_string(p: &graphviz_pure_core::Port) -> String {
    match (&p.name, &p.compass) {
        (Some(n), Some(c)) => format!("{n}:{c}"),
        (Some(n), None) => n.clone(),
        (None, Some(c)) => c.clone(),
        (None, None) => String::new(),
    }
}

/// `noClip`: "" means clip.
fn no_clip(v: Option<&str>) -> bool {
    match v {
        Some(s) if !s.is_empty() => !mapbool(Some(s)),
        _ => false,
    }
}

/// `getdoubles2ptf`
fn getdoubles2ptf(p: Option<&str>) -> (Option<Point>, bool) {
    let Some(p) = p else { return (None, false) };
    let r = sscanf(p, "%lf,%lf%c");
    let [xf, yf, ..] = r.values;
    if r.count > 1 && xf > 0.0 && yf > 0.0 {
        return (
            Some(Point::new(points(xf) as f64, points(yf) as f64)),
            r.c == Some(b'!'),
        );
    }
    let r = sscanf(p, "%lf%c");
    let xf = r.values[0];
    if r.count > 0 && xf > 0.0 {
        return (
            Some(Point::new(points(xf) as f64, points(xf) as f64)),
            r.c == Some(b'!'),
        );
    }
    (None, false)
}
