//! Construction of the dot state from the input graph, plus ports of
//! `graph_init`/`do_graph_label` (lib/common/input.c), `dot_init_subg` and
//! `dot_init_node_edge` (lib/dotgen/dotinit.c), `common_init_node`,
//! `common_init_edge` (lib/common/utils.c) and the port resolution of
//! `poly_port`/`record_port`/`compassPort` (lib/common/shapes.c).

use super::Dot;
use super::types::*;
use super::utils::*;
use graphviz_pure_core::strtod::{atof, sscanf};
use graphviz_pure_core::{Graph, GraphKind, GraphvizError, Point};

const GAP: f64 = 4.0;

/// `PAD(d)`: `d.x += 4*GAP; d.y += 2*GAP`.
pub fn pad(d: Point) -> Point {
    Point::new(d.x + 4.0 * GAP, d.y + 2.0 * GAP)
}

pub use crate::common::ports::{ccwrotatepf, cwrotatepf};

/// What `common_init_node` measured of each node of a graph that dot's own initialisation
/// reads, by input node: C keeps it in `ND_shape_info`, `ND_label` and `ND_xlabel`.
///
/// C measures each node once, in `dot_init_node`. The port measures the input's nodes
/// before it builds dot's records ([`MeasuredNodes::measure`], quietly), for their sizes.
/// `dot_init_node` and the port lookups of `common_init_edge` then take the measurement
/// from here instead of measuring again, wherever a repeat would give the same result and
/// do nothing more ([`graphviz_pure_core::diag::observe`]). It is dropped once the nodes
/// are initialised, before the layout's own peak: the renderer measures the nodes again
/// (#833).
pub struct MeasuredNodes(Vec<MeasuredNode>);

/// [`MeasuredNodes`] of one node.
struct MeasuredNode {
    /// `ND_width`, `ND_height` in inches.
    width: f64,
    height: f64,
    /// `ND_outline_width`, `ND_outline_height` in inches.
    outline: (f64, f64),
    /// `ND_shape_info` of a polygon. `dot_init_node` moves the vertices into the shape.
    polygon: Option<graphviz_pure_geom::PolygonGeometry>,
    /// `ND_shape_info` of a record.
    record: Option<Box<graphviz_pure_geom::RecordField>>,
    /// `ND_xlabel`.
    xlabel: Option<Box<graphviz_pure_geom::LabelGeometry>>,
    /// `ND_label(n)->u.html`, whose cells port lookups read.
    html_label: Option<Box<graphviz_pure_geom::html_label::HtmlLabel>>,
    /// The measurement wrote no diagnostic, so a quiet repeat (a port lookup) would give
    /// it and write nothing either.
    quiet: bool,
    /// The measurement neither consulted nor wrote diagnostics, so `dot_init_node`'s
    /// reported one would give it and do nothing more.
    reported: bool,
}

impl MeasuredNode {
    fn new(
        geom: graphviz_pure_geom::NodeGeometry,
        seen: graphviz_pure_core::diag::Observed,
    ) -> Self {
        Self {
            width: geom.width,
            height: geom.height,
            outline: (geom.outline_width, geom.outline_height),
            polygon: geom.polygon,
            record: geom.record.map(Box::new),
            xlabel: geom.xlabel.map(Box::new),
            html_label: geom.label.html_label,
            quiet: !seen.wrote,
            reported: seen.untouched(),
        }
    }

    /// Whether `fresh`, a new measurement of the node, gives this one bit for bit: the
    /// `Debug` forms show every float's shortest round-trip digits and sign. Debug builds
    /// check each node `dot_init_node` takes from here.
    #[cfg(any(debug_assertions, test))]
    fn is_measurement(&self, fresh: &graphviz_pure_geom::NodeGeometry) -> bool {
        let bits = |a: f64, b: f64| a.to_bits() == b.to_bits();
        bits(self.width, fresh.width)
            && bits(self.height, fresh.height)
            && bits(self.outline.0, fresh.outline_width)
            && bits(self.outline.1, fresh.outline_height)
            && format!("{:?}", self.polygon) == format!("{:?}", fresh.polygon)
            && format!("{:?}", self.record.as_deref()) == format!("{:?}", fresh.record.as_ref())
            && format!("{:?}", self.xlabel.as_deref()) == format!("{:?}", fresh.xlabel.as_ref())
            && format!("{:?}", self.html_label.as_deref())
                == format!("{:?}", fresh.label.html_label.as_deref())
    }
}

impl MeasuredNodes {
    /// Measures every node of `graph` and sets each one's `ND_width` and `ND_height`
    /// (`size_graph_nodes`). The caller makes it quiet (`diag::quietly`): C's one
    /// measurement is `dot_init_node`'s. A node that fails stops the measurement, with
    /// the nodes before it sized.
    pub fn measure(graph: &mut Graph) -> graphviz_pure_core::Result<Self> {
        let mut nodes = Vec::with_capacity(graph.nodes.len());
        let mut failed = None;
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(graph);
        for nd in &graph.nodes {
            let (geom, seen) =
                graphviz_pure_core::diag::observe(|| graphviz_pure_geom::node_geometry(nd, &env));
            match geom {
                Ok(geom) => nodes.push(MeasuredNode::new(geom, seen)),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        for (nd, m) in graph.nodes.iter_mut().zip(&nodes) {
            nd.width = m.width;
            nd.height = m.height;
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(Self(nodes)),
        }
    }

    /// The measurement of input node `i` for `dot_init_node`, where its own reported
    /// measurement would give the same and do nothing more. It is taken once: the shape
    /// takes its vertices, and a node that two pack components share (a node of two
    /// non-nested clusters) is measured again by the second.
    fn for_init(&mut self, i: usize) -> Option<&mut MeasuredNode> {
        let m = self.0.get_mut(i).filter(|m| m.reported)?;
        m.reported = false;
        Some(m)
    }

    /// What a port lookup reads of input node `i`'s measurement, where a quiet repeat
    /// would give the same and write nothing.
    pub fn for_port(&self, i: usize) -> Option<crate::common::ports::PortShape<'_>> {
        let m = self.0.get(i).filter(|m| m.quiet)?;
        Some(crate::common::ports::PortShape {
            record: m.record.as_deref(),
            html_label: m.html_label.as_deref(),
        })
    }
}

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
            next_anon_id: input.next_anon_id,
            attrs: input.graph_attrs.clone(),
            bound: true,
            ..GraphInfo::default()
        });
        d.reserve_nodes_exact(nn);
        d.cold.reserve_exact(nn);
        for (i, n) in input.nodes.iter().enumerate() {
            d.push_node(NodeInfo {
                name: n.name.clone().into(),
                ..NodeInfo::default()
            });
            d.cold.push(NodeCold {
                real: Some(i),
                ..NodeCold::default()
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
            d.cold_mut(t).cg_out.push(i);
            d.cold_mut(h).cg_in.push(i);
            d.graphs[0].edges.insert(i);
        }
        d.edge_seq = ne as u64;
        // cgraph orders a node's out-edges by (head seq, edge seq) and its in-edges by
        // (tail seq, edge seq) (`agedgeseqcmpf`).
        for n in 0..nn {
            let mut o = std::mem::take(&mut d.cold_mut(n).cg_out);
            o.sort_by_key(|&e| (d.edges[e].head, d.edges[e].seq));
            d.cold_mut(n).cg_out = o;
            let mut i = std::mem::take(&mut d.cold_mut(n).cg_in);
            i.sort_by_key(|&e| (d.edges[e].tail, d.edges[e].seq));
            d.cold_mut(n).cg_in = i;
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
        // GD_label stays on an input cluster for the renderer, which draws it unflipped.
        if sg != 0
            && let Some(si) = self.graphs.get(sg).and_then(|g| g.sub_index)
            && let Some(kept) = &mut self.kept_labels
        {
            kept.clusters.push((si, Some(gl.unflipped(flip))));
        }
    }

    fn push_label(&mut self, lg: &graphviz_pure_geom::LabelGeometry) -> L {
        self.push_label_text(lg, String::new())
    }

    /// [`Self::push_label`] keeping `lp->text` (`text`), which `addXLabels` reports.
    fn push_label_text(&mut self, lg: &graphviz_pure_geom::LabelGeometry, text: String) -> L {
        self.labels.push(TextLabel {
            text,
            fontname: lg.font_name.clone(),
            fontsize: lg.font_size,
            dimen: lg.dimen,
            space: lg.space,
            html: lg.html,
            ..TextLabel::default()
        })
    }

    /// `dot_init_subg(g, g)`: every subgraph gets an Agraphinfo_t record.
    pub fn dot_init_subg(&mut self) {
        for g in 0..self.graphs.len() {
            self.graphs[g].bound = true;
        }
    }

    /// `dot_init_node_edge`
    pub fn dot_init_node_edge(&mut self) -> Result<(), String> {
        let input = self.input;
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(input);
        for n in self.nodes_of(0) {
            self.dot_init_node(n, &env)?;
        }
        for n in self.nodes_of(0) {
            for e in self.out_edges(0, n) {
                self.dot_init_edge(e)?;
            }
        }
        Ok(())
    }

    /// `dot_init_node`, whose `common_init_node` measures the node in `env` (the graph's
    /// [`graphviz_pure_geom::NodeSizeEnv`]), or takes what [`MeasuredNodes::measure`]
    /// measured where that is the same.
    fn dot_init_node(
        &mut self,
        n: N,
        env: &graphviz_pure_geom::NodeSizeEnv<'_>,
    ) -> Result<(), String> {
        let r = self.cold(n).real.unwrap();
        let input = self.input;
        let nd = &input.nodes[r];
        let i = self.measured_index(r);
        if let Some(mut measured) = self.measured.take() {
            let taken = measured
                .for_init(i)
                .map(|m| self.init_node_measured(n, nd, env, m));
            self.measured = Some(measured);
            if let Some(result) = taken {
                return result;
            }
        }
        let mut geom = graphviz_pure_geom::node_geometry(nd, env).map_err(super::fatal)?;
        let shape =
            crate::common::shapes::NodeShape::from_geometry(nd, &mut geom).map_err(super::fatal)?;
        self.init_node_from(
            n,
            nd,
            shape,
            (geom.width, geom.height),
            geom.record.as_ref(),
            geom.xlabel.as_ref(),
        );
        Ok(())
    }

    /// [`Self::dot_init_node`] from the node's measurement `m`, whose vertices the shape
    /// takes.
    fn init_node_measured(
        &mut self,
        n: N,
        nd: &graphviz_pure_core::NodeData,
        env: &graphviz_pure_geom::NodeSizeEnv<'_>,
        m: &mut MeasuredNode,
    ) -> Result<(), String> {
        #[cfg(debug_assertions)]
        debug_assert!(
            graphviz_pure_core::diag::quietly(|| graphviz_pure_geom::node_geometry(nd, env))
                .is_ok_and(|fresh| m.is_measurement(&fresh)),
            "node {} was measured differently before dot_init_node",
            nd.name
        );
        #[cfg(not(debug_assertions))]
        let _ = env;
        let vertices = m.polygon.as_mut().map(|p| std::mem::take(&mut p.vertices));
        let shape = crate::common::shapes::NodeShape::from_parts(
            nd,
            m.width,
            m.height,
            m.outline,
            m.polygon.as_ref().zip(vertices),
            m.record.as_ref().map(|r| r.bbox),
        )
        .map_err(super::fatal)?;
        self.init_node_from(
            n,
            nd,
            shape,
            (m.width, m.height),
            m.record.as_deref(),
            m.xlabel.as_deref(),
        );
        Ok(())
    }

    /// The rest of `dot_init_node` for a measured node: `ND_shape`, its record fields,
    /// `ND_width`/`ND_height` (`(width, height)`), `ND_lw`/`ND_rw`/`ND_ht` (`gv_nodesize`)
    /// and `ND_xlabel`.
    fn init_node_from(
        &mut self,
        n: N,
        nd: &graphviz_pure_core::NodeData,
        shape: crate::common::shapes::NodeShape,
        (width, height): (f64, f64),
        record: Option<&graphviz_pure_geom::RecordField>,
        xlabel: Option<&graphviz_pure_geom::LabelGeometry>,
    ) {
        let (lw, rw, ht) =
            graphviz_pure_geom::node_size::nodesize(width, height, self.graphs[0].flip());
        // lp->text, which addXLabels' -v trace prints
        let xlabel_text = xlabel.map(|xl| {
            graphviz_pure_geom::label_text(xl, nd.attrs.get("xlabel").map_or("", String::as_str))
        });
        let rec_fields = record
            .map(|r| r.children.iter().map(|f| f.bbox).collect())
            .unwrap_or_default();
        let cold = self.cold_mut(n);
        cold.shape = Some(shape);
        cold.rec_fields = rec_fields;
        cold.is_record = record.is_some();
        let info = &mut self.nodes[n];
        info.width = width;
        info.height = height;
        info.lw = lw;
        info.rw = rw;
        info.ht = ht;
        info.uf_size = 1;
        if let Some(xl) = xlabel {
            let l = self.push_label_text(xl, xlabel_text.unwrap_or_default());
            self.nodes[n].xlabel = Some(l);
            self.graphs[0].has_labels |= NODE_XLABEL;
        }
    }

    /// The index in [`Dot::measured`] of input node `r`: the root graph's node, when the
    /// input is a pack component.
    pub(crate) fn measured_index(&self, r: usize) -> usize {
        self.cg_root_node
            .as_ref()
            .and_then(|m| m.get(r).copied())
            .unwrap_or(r)
    }

    fn dot_init_edge(&mut self, e: E) -> Result<(), String> {
        self.common_init_edge(e)?;
        let mut weight = late_int(self.edge_attr(e, "weight"), 1, 0);
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        let tg = self.node_attr(t, "group").unwrap_or("");
        let hg = self.node_attr(h, "group").unwrap_or("");
        // `tailgroup == headgroup` compares interned strings, so the HTML flag counts.
        let html = |n: N| {
            self.cold(n)
                .real
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
        Ok(())
    }

    fn common_init_edge(&mut self, e: E) -> Result<(), String> {
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
        // ED_label and the edge's other labels stay on the input edge for the renderer.
        if let Some(kept) = &mut self.kept_labels
            && let Some(edge) = graphviz_pure_geom::LaidOutEdge::of(geo)
        {
            kept.edges.push((r, edge));
        }
        let ie = &self.input.edges[r];
        let tp = ie.tail_port.clone();
        let hp = ie.head_port.clone();
        let tail_str = port_string(&tp);
        if !tail_str.is_empty() {
            self.nodes[t].has_port = true;
        }
        let mut port = self.chk_port(t, &tp).map_err(super::fatal)?;
        if no_clip(self.edge_attr(e, "tailclip")) {
            port.clip = false;
        }
        self.edges[e].tail_port = port;
        let head_str = port_string(&hp);
        if !head_str.is_empty() {
            self.nodes[h].has_port = true;
        }
        let mut port = self.chk_port(h, &hp).map_err(super::fatal)?;
        if no_clip(self.edge_attr(e, "headclip")) {
            port.clip = false;
        }
        self.edges[e].head_port = port;
        Ok(())
    }

    /// `chkPort(ND_shape(n)->fns->portfn, n, s)`.
    fn chk_port(&self, n: N, p: &graphviz_pure_core::Port) -> Result<Port, GraphvizError> {
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

#[cfg(test)]
mod tests {
    use super::MeasuredNodes;
    use graphviz_pure_core::{Graph, GraphKind};

    /// The measurement keeps every node, and marks which repeats it can stand in for: an
    /// ordinary node for all of them, a node whose measurement asks whether to report (an
    /// unknown shape, an HTML label) only for the quiet ones.
    #[test]
    fn measured_nodes_stand_in_where_a_repeat_does_nothing_more() {
        let mut graph = Graph::new("G", GraphKind::Directed, false);
        graph.graph_attrs.set("rankdir", "LR".to_owned(), false);
        for (name, attrs, html) in [
            ("a", &[][..], false),
            ("b", &[("label", "two\\nlines"), ("xlabel", "x")][..], false),
            (
                "c",
                &[("shape", "record"), ("label", "<p>l|{m|r}")][..],
                false,
            ),
            ("d", &[("shape", "nosuchshape")][..], false),
            (
                "e",
                &[("label", "<TABLE><TR><TD PORT=\"p\">t</TD></TR></TABLE>")][..],
                true,
            ),
        ] {
            let id = graph.ensure_node(name);
            let a = &mut graph.nodes[id.0 as usize].attrs;
            for &(key, value) in attrs {
                a.set(key, value.to_owned(), html && key == "label");
            }
        }
        graphviz_pure_core::charset::begin_layout(graph.root_name(), None);
        let mut measured =
            graphviz_pure_core::diag::quietly(|| MeasuredNodes::measure(&mut graph)).unwrap();
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(&graph);
        for (i, nd) in graph.nodes.iter().enumerate() {
            let fresh = graphviz_pure_geom::node_geometry(nd, &env).unwrap();
            assert!(measured.0[i].is_measurement(&fresh), "{}", nd.name);
            assert_eq!((nd.width, nd.height), (fresh.width, fresh.height));
            assert!(measured.for_port(i).is_some(), "{}", nd.name);
        }
        let port = |i: usize| measured.for_port(i).unwrap();
        assert!(port(2).record.is_some() && port(4).html_label.is_some());
        let reported: Vec<_> = (0..graph.nodes.len())
            .map(|i| measured.for_init(i).is_some())
            .collect();
        assert_eq!(reported, [true, true, true, false, false]);
        // Taken once.
        assert!(measured.for_init(0).is_none());
        assert!(measured.for_port(0).is_some());
    }
}
