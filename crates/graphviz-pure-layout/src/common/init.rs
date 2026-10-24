//! Graph, node and edge initialisation shared by the non-dot engines:
//! the parts of `graph_init` (`lib/common/input.c`), `common_init_node` and
//! `common_init_edge` (`lib/common/utils.c`) that layouts consult.

use super::shapes::NodeShape;
use super::types::*;
use super::utils::{late_double, map_bool, mapbool};
use graphviz_pure_core::font::{LabelSubstContext, measure_multiline_label_with_context};
use graphviz_pure_core::strtod::{atof, sscanf};
use graphviz_pure_core::{AttrMap, BoundingBox, Graph, GraphKind, NodeData, Point};

pub const DEFAULT_FONTSIZE: f64 = 14.0;
pub const MIN_FONTSIZE: f64 = 1.0;
pub const DEFAULT_FONTNAME: &str = "Times-Roman";
pub const DEFAULT_NODESEP: f64 = 0.25;
pub const MIN_NODESEP: f64 = 0.02;

/// `POINTS(a)`: inches to integral points. C (clang) fuses `a * 72 ± .5`.
pub fn points(inches: f64) -> f64 {
    f64::from(graphviz_pure_core::cnum::c_round_mul(inches, 72.0))
}

/// `getdoubles2ptf`: a `size`/`page` attribute in points and its `!` flag.
pub(crate) fn getdoubles2ptf(p: Option<&str>) -> (Point, bool) {
    let mut res = Point::default();
    let Some(p) = p else { return (res, false) };
    let r = sscanf(p, "%lf,%lf%c");
    let [x, y, ..] = r.values;
    if r.count > 1 && x > 0.0 && y > 0.0 {
        res = Point::new(points(x), points(y));
        return (res, r.c == Some(b'!'));
    }
    let r = sscanf(p, "%lf%c");
    let x = r.values[0];
    if r.count > 0 && x > 0.0 {
        res = Point::new(points(x), points(x));
        return (res, r.c == Some(b'!'));
    }
    (res, false)
}

/// Builds a [`TextLabel`] for `text` measured with the port's font metrics.
pub fn make_label(
    text: &str,
    ctx: &LabelSubstContext<'_>,
    fontname: &str,
    fontsize: f64,
) -> TextLabel {
    let m = measure_multiline_label_with_context(text, ctx, fontname, fontsize);
    TextLabel {
        text: text.to_string(),
        dimen: Point::new(m.width, m.height),
        pos: Point::default(),
        set: false,
        fontsize,
        fontname: fontname.to_string(),
    }
}

/// `do_graph_label` for a graph or cluster: the label and `GD_label_pos`.
pub fn graph_label(
    attrs: &AttrMap,
    name: &str,
    html_name: bool,
    is_root: bool,
) -> (Option<TextLabel>, u8) {
    let Some(text) = attrs.get("label").filter(|s| !s.is_empty()) else {
        return (None, 0);
    };
    let env = graphviz_pure_geom::NodeSizeEnv::new(name);
    let make = || {
        graphviz_pure_geom::graph_label_geometry_in(name, html_name, attrs, is_root, false, &env)
    };
    // layout_graph_with_options made the root label first, as graph_init does.
    let geom = if is_root {
        graphviz_pure_core::diag::quietly(make)
    } else {
        make()
    };
    let Some(geom) = geom else {
        return (None, 0);
    };
    let lbl = TextLabel {
        text: text.clone(),
        dimen: geom.label.dimen,
        pos: Point::default(),
        set: false,
        fontsize: geom.label.font_size,
        fontname: geom.label.font_name.clone(),
    };
    let pos = attrs.get("labelloc").map(String::as_str);
    let mut flag = if is_root {
        if pos.is_some_and(|p| p.starts_with('t')) {
            LABEL_AT_TOP
        } else {
            LABEL_AT_BOTTOM
        }
    } else if pos.is_some_and(|p| p.starts_with('b')) {
        LABEL_AT_BOTTOM
    } else {
        LABEL_AT_TOP
    };
    if let Some(j) = attrs.get("labeljust") {
        if j.starts_with('l') {
            flag |= LABEL_AT_LEFT;
        } else if j.starts_with('r') {
            flag |= LABEL_AT_RIGHT;
        }
    }
    (Some(lbl), flag)
}

/// `PAD(d)`: label padding of `4*GAP` by `2*GAP`.
pub fn pad(d: Point) -> Point {
    Point::new(d.x + 16.0, d.y + 8.0)
}

/// `do_graph_label(subg)` for the cluster `c` of `g.clusters`: its label, `GD_label_pos`
/// and `GD_border`, with the label's diagnostics.
pub fn do_graph_label(g: &mut LGraph<'_>, c: usize) {
    let si = g.clusters[c].subgraph;
    let sg = &g.src.subgraphs[si];
    let name = g.src.subgraph_name(si);
    let (label, label_pos) = graph_label(&sg.graph_attrs, &name, sg.html_name, false);
    let cl = &mut g.clusters[c];
    if let Some(l) = &label {
        cl.border[if label_pos & LABEL_AT_TOP != 0 { 2 } else { 0 }] = pad(l.dimen);
    }
    cl.label = label;
    cl.label_pos = label_pos;
}

impl<'a> LGraph<'a> {
    /// `graph_init` + `common_init_node` + `common_init_edge` for `graph`,
    /// with `ndim` layout dimensions and `default_edge_type` for `setEdgeType`.
    pub fn new(graph: &'a Graph, ndim: usize, default_edge_type: EdgeType) -> Self {
        Self::new_with(graph, ndim, default_edge_type, |_, _| {})
    }

    /// [`Self::new`] that runs `node_init` on each node right after its `common_init_node`,
    /// before the next node's and before any edge's, as an engine's own node init does
    /// (`neato_init_node_edge`'s `user_pos`).
    pub fn new_with(
        graph: &'a Graph,
        ndim: usize,
        default_edge_type: EdgeType,
        node_init: impl FnMut(&NodeData, &mut LNode),
    ) -> Self {
        let mut lg = Self::new_unported_with(graph, ndim, default_edge_type, node_init);
        lg.common_init_edges();
        lg
    }

    /// `common_init_edge` for every edge in the engines' `agfstnode`/`agfstout` order, which
    /// is the order of its diagnostics: the edge's labels, then its ports (`chkPort`, keeping
    /// `*clip=false`).
    fn common_init_edges(&mut self) {
        let graph = self.src;
        let directed = graph.kind == GraphKind::Directed;
        let env = graphviz_pure_geom::NodeSizeEnv::from_graph(graph);
        let order: Vec<usize> = self.out.iter().flatten().copied().collect();
        for ei in order {
            let ed = &graph.edges[ei];
            let a = &ed.attrs;
            let tname = &graph.nodes[ed.tail.0 as usize].name;
            let hname = &graph.nodes[ed.head.0 as usize].name;
            let lg = graphviz_pure_geom::node_size::edge_label_geometry_of(
                u64::from(ed.id.0),
                a,
                tname,
                hname,
                directed,
                &env,
            );
            let conv = |key: &str, l: &Option<graphviz_pure_geom::node_size::LabelGeometry>| {
                l.as_ref().map(|l| TextLabel {
                    text: graphviz_pure_geom::label_text(l, a.get(key).map_or("", String::as_str)),
                    dimen: l.dimen,
                    pos: Point::default(),
                    set: false,
                    fontsize: l.font_size,
                    fontname: l.font_name.clone(),
                })
            };
            let e = &mut self.edges[ei];
            e.label = conv("label", &lg.label);
            e.xlabel = conv("xlabel", &lg.xlabel);
            e.head_label = conv("headlabel", &lg.head_label);
            e.tail_label = conv("taillabel", &lg.tail_label);
            super::ports::resolve_edge_ports(self, ei);
        }
    }

    /// [`Self::new`] without `common_init_edge`: the edges have no labels and keep centre
    /// ports, and no label or port diagnostics are printed (`patchwork_init_edge` only binds
    /// the edge record).
    pub fn new_unported(graph: &'a Graph, ndim: usize, default_edge_type: EdgeType) -> Self {
        Self::new_unported_with(graph, ndim, default_edge_type, |_, _| {})
    }

    fn new_unported_with(
        graph: &'a Graph,
        ndim: usize,
        default_edge_type: EdgeType,
        mut node_init: impl FnMut(&NodeData, &mut LNode),
    ) -> Self {
        let gattrs = &graph.graph_attrs;
        // `GD_nodesep = POINTS(…)`, an `int`: 1e300 inches saturates at INT_MAX points.
        let nodesep = i64::from(graphviz_pure_core::cnum::c_round_mul(
            late_double(gattrs, "nodesep", DEFAULT_NODESEP, MIN_NODESEP),
            72.0,
        ));
        let mut drawing = Drawing::default();
        if let Some(p) = gattrs.get("ratio") {
            match p.as_str() {
                "auto" => drawing.ratio_kind = RatioKind::Auto,
                "compress" => drawing.ratio_kind = RatioKind::Compress,
                "expand" => drawing.ratio_kind = RatioKind::Expand,
                "fill" => drawing.ratio_kind = RatioKind::Fill,
                _ => {
                    let r = atof(p);
                    if r > 0.0 {
                        drawing.ratio_kind = RatioKind::Value;
                        drawing.ratio = r;
                    }
                }
            }
        }
        let (size, filled) = getdoubles2ptf(gattrs.get("size").map(String::as_str));
        drawing.size = size;
        drawing.filled = filled;
        drawing.landscape = if let Some(p) = gattrs.get("rotate") {
            super::utils::atoi(p) == 90
        } else if let Some(p) = gattrs.get("orientation") {
            p.starts_with('l') || p.starts_with('L')
        } else {
            mapbool(gattrs.get("landscape").map(String::as_str))
        };
        let concentrate = mapbool(gattrs.get("concentrate").map(String::as_str));
        let (label, label_pos) = graph_label(gattrs, graph.root_name(), false, true);

        let edge_type = match gattrs.get("splines") {
            None => default_edge_type,
            Some(s) if s.is_empty() => EdgeType::None,
            Some(s) => edge_type_of(s, default_edge_type),
        };

        let env = graphviz_pure_geom::NodeSizeEnv::from_graph_attrs(graph, gattrs);
        let mut nodes = Vec::with_capacity(graph.nodes.len());
        for nd in &graph.nodes {
            let geom = graphviz_pure_geom::node_geometry(nd, &env);
            let mut n = nd.clone();
            n.width = geom.width;
            n.height = geom.height;
            let mut shape = NodeShape::new(&n);
            shape.apply_geometry(&geom);
            let (lw, rw, ht) = geom.nodesize(false);
            let xlabel = geom.xlabel.as_ref().map(|l| TextLabel {
                // lp->text: what addXLabels' `-v2` trace prints
                text: graphviz_pure_geom::label_text(
                    l,
                    nd.attrs.get("xlabel").map_or("", String::as_str),
                ),
                dimen: l.dimen,
                pos: Point::default(),
                set: false,
                fontsize: l.font_size,
                fontname: l.font_name.clone(),
            });
            let mut node = LNode {
                pos: vec![0.0; ndim],
                coord: Point::default(),
                width: n.width,
                height: n.height,
                lw,
                rw,
                ht,
                pinned: 0,
                id: 0,
                heapindex: -1,
                hops: 0,
                dist: 0.0,
                mark: false,
                shape,
                xlabel,
                clust: None,
                lim: -1,
            };
            node_init(nd, &mut node);
            nodes.push(node);
        }

        let mut out = vec![Vec::new(); graph.nodes.len()];
        let mut inn = vec![Vec::new(); graph.nodes.len()];
        let mut edges = Vec::with_capacity(graph.edges.len());
        for (ei, ed) in graph.edges.iter().enumerate() {
            let t = ed.tail.0 as usize;
            let h = ed.head.0 as usize;
            out[t].push(ei);
            inn[h].push(ei);
            let a = &ed.attrs;
            let clip = |key: &str| map_bool(a.get(key).map(String::as_str), true);
            edges.push(LEdge {
                tail: t,
                head: h,
                dist: 1.0,
                factor: late_double(a, "weight", 1.0, 1.0),
                count: 0,
                to_virt: None,
                spl: None,
                label: None,
                xlabel: None,
                head_label: None,
                tail_label: None,
                tail_port: PortInfo {
                    clip: clip("tailclip"),
                    ..PortInfo::default()
                },
                head_port: PortInfo {
                    clip: clip("headclip"),
                    ..PortInfo::default()
                },
                path: Vec::new(),
            });
        }
        let has_labelangle = graph.edge_attr_declared("labelangle");
        let has_labeldistance = graph.edge_attr_declared("labeldistance");

        // cgraph orders a node's out-edges by head sequence, then edge
        // sequence (`agedgeseqcmpf`); in-edges by tail, then edge.
        for list in out.iter_mut() {
            list.sort_by_key(|&e| (graph.edges[e].head.0, e));
        }
        for list in inn.iter_mut() {
            list.sort_by_key(|&e| (graph.edges[e].tail.0, e));
        }
        LGraph {
            src: graph,
            nodes,
            edges,
            out,
            inn,
            ndim,
            odim: 0,
            nodesep,
            bb: BoundingBox::default(),
            label,
            label_pos,
            drawing,
            clusters: Vec::new(),
            root_clusters: Vec::new(),
            edge_type,
            concentrate,
            has_labelangle,
            has_labeldistance,
        }
    }

    /// Writes positions, splines, label positions and bounding boxes back
    /// into `graph` (points, with the final translation already applied).
    pub fn write_back(&self, graph: &mut Graph) {
        graph.layout.odim = self.odim;
        for (i, n) in self.nodes.iter().enumerate() {
            let nd = &mut graph.nodes[i];
            nd.pos = n.coord;
            nd.width = n.width;
            nd.height = n.height;
            nd.layout.xlabel = n.xlabel.as_ref().filter(|l| l.set).map(|l| l.pos);
            nd.layout.higher_dims = higher_dims(&n.pos, self.odim);
        }
        let place = |l: &Option<TextLabel>| {
            l.as_ref().map(|l| graphviz_pure_core::LabelPlace {
                pos: l.pos,
                set: l.set,
            })
        };
        for (i, e) in self.edges.iter().enumerate() {
            let ed = &mut graph.edges[i];
            ed.spline = graphviz_pure_core::Spline {
                segments: e
                    .spl
                    .as_ref()
                    .map(|v| {
                        v.iter()
                            .map(|b| graphviz_pure_core::BezierSegment {
                                points: b.list.clone(),
                                start_arrow: (b.sflag != 0).then_some(b.sp),
                                end_arrow: (b.eflag != 0).then_some(b.ep),
                                sflag: b.sflag,
                                eflag: b.eflag,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            // C prints label positions even when placement failed (the
            // unplaced position is the origin, moved by the final translation).
            ed.label_pos = e.label.as_ref().map(|l| l.pos);
            ed.label_unplaced = e.label.as_ref().is_some_and(|l| !l.set);
            ed.layout.xlabel = e.xlabel.as_ref().filter(|l| l.set).map(|l| l.pos);
            ed.layout.head_label = place(&e.head_label);
            ed.layout.tail_label = place(&e.tail_label);
        }
        graph.bb = self.bb;
        // Clusters the layout did not register keep the `bb` they were given,
        // which C prints unchanged.
        for sg in &mut graph.subgraphs {
            if let Some(b) = sg.graph_attrs.get("bb") {
                let v: Vec<f64> = b.split(',').filter_map(|t| t.trim().parse().ok()).collect();
                if v.len() == 4 {
                    sg.bb = BoundingBox::new(v[0], v[1], v[2], v[3]);
                }
            }
        }
        let mut registered = vec![false; graph.subgraphs.len()];
        for c in &self.clusters {
            let sg = &mut graph.subgraphs[c.subgraph];
            sg.bb = c.bb;
            // GD_clust holds it even with an empty box, which is drawn.
            sg.layout.reg = graphviz_pure_core::ClusterReg::Registered;
            registered[c.subgraph] = true;
        }
        for (i, sg) in graph.subgraphs.iter_mut().enumerate() {
            if !registered[i] && sg.bb != graphviz_pure_core::BoundingBox::default() {
                sg.layout.reg = graphviz_pure_core::ClusterReg::Unregistered;
            }
        }
        // Placed graph and cluster labels; the renderer would otherwise re-place them with
        // dot's rules (which flip for rankdir).
        graph.layout.label_pos = self.label.as_ref().filter(|l| l.set).map(|l| l.pos);
        for c in &self.clusters {
            if let Some(l) = c.label.as_ref().filter(|l| l.set) {
                graph.subgraphs[c.subgraph].layout.label_pos = Some(l.pos);
            }
        }
    }
}

/// `ND_pos(n)[2..odim]` in points, which `attach_attrs_and_arrows` writes after `x` and `y`
/// when `GD_odim(g) >= 3`.
pub fn higher_dims(pos: &[f64], odim: usize) -> Vec<f64> {
    if odim < 3 {
        return Vec::new();
    }
    (2..odim)
        .map(|k| 72.0 * pos.get(k).copied().unwrap_or(0.0))
        .collect()
}

/// `setEdgeType(gc, EDGETYPE_LINE)` on a component of neato or sfdp: the component's type
/// is the root's, already set, but C reads `splines` again and warns again.
pub fn component_set_edge_type(g: &LGraph<'_>) {
    if let Some(s) = g.gattr("splines").filter(|s| !s.is_empty()) {
        edge_type_of(s, EdgeType::Line);
    }
}

/// `edgeType` in `lib/common/utils.c`. Its warning is not repeated inside
/// [`graphviz_pure_core::diag::silently`], where the port builds a graph C builds once.
pub fn edge_type_of(s: &str, dflt: EdgeType) -> EdgeType {
    let b = s.as_bytes();
    if b.is_empty() {
        return dflt;
    }
    if b[0] == b'0' {
        return EdgeType::Line;
    }
    if (b'1'..=b'9').contains(&b[0]) {
        return EdgeType::Spline;
    }
    let l = s.to_ascii_lowercase();
    match l.as_str() {
        "curved" => EdgeType::Curved,
        "compound" => EdgeType::CompoundEdge,
        "false" | "line" | "no" => EdgeType::Line,
        "none" => EdgeType::None,
        "ortho" => EdgeType::Ortho,
        "polyline" => EdgeType::Pline,
        "spline" | "true" | "yes" => EdgeType::Spline,
        _ => {
            if graphviz_pure_core::diag::reporting_ports() {
                graphviz_pure_core::diag::agwarningf(&format!(
                    "Unknown \"splines\" value: \"{s}\" - ignored\n"
                ));
            }
            dflt
        }
    }
}
