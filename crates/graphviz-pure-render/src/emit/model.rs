//! The laid-out graph as the emitter sees it: what C keeps in `Agraphinfo_t`, `Agnodeinfo_t`
//! and `Agedgeinfo_t` after layout (`GD_bb`, `GD_clust`, `GD_label`, `ND_coord`, `ND_lw/rw/ht`,
//! `ND_shape_info`, `ND_label`, `ED_spl`, `ED_label`, ...).
//!
//! Node shapes and labels come from `graphviz_pure_geom::node_size` (`common_init_node` and the shape
//! init functions); positions come from the layout (`NodeData::pos`, `EdgeData::spline`,
//! `EdgeData::label_pos`, `SubgraphData::bb`, `Graph::bb`, and the `layout` fields of each
//! object). Attributes such as `xlp` or `lp` in the input are not layout results: only
//! `neato -n` reads them, as its layout.

use super::attrs::Attrs;
use graphviz_pure_core::font::{PostscriptFontAlias, TextJustify, translate_postscript_fontname};
use graphviz_pure_core::strtod::sscanf;
use graphviz_pure_core::{
    AttrMap, BoundingBox, ClusterReg, Graph, GraphKind, GraphLayout, LabelPlace, Point, PortNames,
};
use graphviz_pure_geom::html_label::HtmlLabel;
use graphviz_pure_geom::node_size::{
    LABEL_AT_LEFT, LABEL_AT_RIGHT, LABEL_AT_TOP, LabelGeometry, NodeGeometry, NodeSizeEnv,
    graph_label_geometry_in, node_geometry,
};
use graphviz_pure_geom::shapes::{RecordField, ShapeKind};

/// `textfont_t`.
#[derive(Debug, Clone)]
pub(crate) struct TextFont {
    pub name: String,
    pub size: f64,
    pub flags: u8,
    pub ps: Option<&'static PostscriptFontAlias>,
}

impl TextFont {
    pub fn new(name: &str, size: f64, flags: u8) -> Self {
        TextFont {
            name: name.to_string(),
            size,
            flags,
            ps: translate_postscript_fontname(name),
        }
    }
}

/// `textspan_t`.
#[derive(Debug, Clone)]
pub(crate) struct TextSpan {
    pub str: String,
    /// `'l'`, `'r'` or `'n'`.
    pub just: char,
    pub width: f64,
    pub height: f64,
    pub yoffset_centerline: f64,
    pub font: TextFont,
}

/// `textlabel_t`.
#[derive(Debug, Clone)]
pub(crate) struct TextLabel {
    pub text: String,
    pub fontname: String,
    pub fontcolor: String,
    pub fontsize: f64,
    pub dimen: Point,
    pub space: Point,
    pub pos: Point,
    pub set: bool,
    pub valign: char,
    pub html: Option<Box<HtmlLabel>>,
    pub is_html: bool,
    pub spans: Vec<TextSpan>,
}

impl TextLabel {
    /// Builds a label from its geometry. `raw` is the attribute value.
    pub fn from_geometry(g: &LabelGeometry, raw: &str, fontcolor: &str) -> Self {
        let mut spans = Vec::new();
        let mut text = raw.to_string();
        if g.html {
            text = match g.html_label.as_deref() {
                Some(HtmlLabel::Table(_)) => "<TABLE>".to_string(),
                // `make_html_label` after a parse error: `lp->html` is cleared and
                // `lp->text` stays the unconverted source.
                None if g.html_failed => raw.to_string(),
                // HTML text: `lp->text` is the source string, entities not decoded.
                _ => raw.to_string(),
            };
        }
        if let Some(t) = &g.text {
            text = t.source.clone();
            for line in &t.lines {
                spans.push(TextSpan {
                    str: line.text.clone(),
                    just: match line.justify {
                        TextJustify::Left => 'l',
                        TextJustify::Right => 'r',
                        TextJustify::Center => 'n',
                    },
                    width: line.width,
                    height: line.height,
                    yoffset_centerline: 0.1 * line.font_size,
                    font: TextFont::new(&g.font_name, line.font_size, 0),
                });
            }
        }
        TextLabel {
            text,
            fontname: g.font_name.clone(),
            fontcolor: fontcolor.to_string(),
            fontsize: g.font_size,
            dimen: g.dimen,
            space: g.space,
            pos: Point::default(),
            set: false,
            valign: if g.valign == '\0' { 'c' } else { g.valign },
            html: g.html_label.clone(),
            is_html: g.html && !g.html_failed,
            spans,
        }
    }
    /// C: lib/common/labels.c:make_label, make_simple_label
    /// Transfers measured label buffers into the emitter instead of copying them.
    pub fn from_owned_geometry(g: LabelGeometry, raw: &str, fontcolor: &str) -> Self {
        let mut spans = Vec::new();
        let text = if let Some(t) = g.text {
            spans.reserve_exact(t.lines.len());
            for line in t.lines {
                spans.push(TextSpan {
                    str: line.text,
                    just: match line.justify {
                        TextJustify::Left => 'l',
                        TextJustify::Right => 'r',
                        TextJustify::Center => 'n',
                    },
                    width: line.width,
                    height: line.height,
                    yoffset_centerline: 0.1 * line.font_size,
                    font: TextFont::new(&g.font_name, line.font_size, 0),
                });
            }
            t.source
        } else if g.html && matches!(g.html_label.as_deref(), Some(HtmlLabel::Table(_))) {
            "<TABLE>".to_string()
        } else {
            raw.to_string()
        };
        TextLabel {
            text,
            fontname: g.font_name,
            fontcolor: fontcolor.to_string(),
            fontsize: g.font_size,
            dimen: g.dimen,
            space: g.space,
            pos: Point::default(),
            set: false,
            valign: if g.valign == '\0' { 'c' } else { g.valign },
            html: g.html_label,
            is_html: g.html && !g.html_failed,
            spans,
        }
    }
}

/// One Bézier of an edge (`bezier`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Bz<'g> {
    pub list: &'g [Point],
    pub sflag: u32,
    pub eflag: u32,
    pub sp: Point,
    pub ep: Point,
}

/// C: lib/common/types.h:Agnodeinfo_t
/// Shape data retained by the emitter after its labels have been transferred.
pub(crate) struct ShapeGeometry {
    pub shape: ShapeKind,
    pub width: f64,
    pub height: f64,
    pub polygon: Option<graphviz_pure_geom::node_size::PolygonGeometry>,
    pub record: Option<RecordField>,
    pub epsf: Option<(Point, i32)>,
    pub usershape: bool,
}

pub(crate) struct RNode<'g> {
    pub name: &'g str,
    pub attrs: Attrs<'g>,
    pub geom: ShapeGeometry,
    pub label: TextLabel,
    pub xlabel: Option<TextLabel>,
    pub coord: Point,
    pub lw: f64,
    pub rw: f64,
    pub ht: f64,
}

pub(crate) struct REdge<'g> {
    pub tail: usize,
    pub head: usize,
    pub attrs: Attrs<'g>,
    pub bz: Vec<Bz<'g>>,
    pub label: Option<TextLabel>,
    pub xlabel: Option<TextLabel>,
    pub head_label: Option<TextLabel>,
    pub tail_label: Option<TextLabel>,
    pub tail_port: Option<&'g str>,
    pub head_port: Option<&'g str>,
}

pub(crate) struct RCluster<'g> {
    /// `agnameof` of the cluster.
    pub name: String,
    pub attrs: Attrs<'g>,
    pub sub: usize,
    /// `AGSEQ` of the cluster's subgraph.
    pub seq: usize,
    pub bb: BoundingBox,
    pub label: Option<TextLabel>,
    pub children: Vec<usize>,
    /// Nodes (indices) directly or indirectly in the cluster.
    #[allow(dead_code)]
    pub nodes: Vec<usize>,
}

/// `GD_has_labels` bits.
pub(crate) const EDGE_LABEL: u32 = 1 << 0;
pub(crate) const HEAD_LABEL: u32 = 1 << 1;
pub(crate) const TAIL_LABEL: u32 = 1 << 2;
pub(crate) const GRAPH_LABEL: u32 = 1 << 3;
pub(crate) const NODE_XLABEL: u32 = 1 << 4;
pub(crate) const EDGE_XLABEL: u32 = 1 << 5;

pub(crate) struct Model<'g> {
    pub g: &'g Graph,
    /// `N_penwidth != NULL`: nodes declare `penwidth`.
    pub node_penwidth_declared: bool,
    pub directed: bool,
    pub bb: BoundingBox,
    pub label: Option<TextLabel>,
    pub nodes: Vec<RNode<'g>>,
    pub edges: Vec<REdge<'g>>,
    /// Out-edges of each node in edge sequence order (`agfstout`/`agnxtout`).
    pub out_edges: Vec<Vec<usize>>,
    pub clusters: Vec<RCluster<'g>>,
    pub root_clusters: Vec<usize>,
    #[allow(dead_code)]
    pub has_labels: u32,
}

impl<'g> Model<'g> {
    /// `agnameof(n)` of node `n`.
    pub fn node_agnameof(&self, n: usize) -> graphviz_pure_core::AgName<'g> {
        self.g
            .agnameof(graphviz_pure_core::Obj::Node(graphviz_pure_core::NodeId(
                n as u32,
            )))
    }

    /// `AGSEQ(n)`.
    pub fn node_seq(&self, n: usize) -> usize {
        self.g.nodes[n].layout.seq.unwrap_or(n + 1)
    }
    /// `AGSEQ(e)`.
    pub fn edge_seq(&self, e: usize) -> usize {
        self.g.edges[e].layout.seq.unwrap_or(e + 1)
    }
}

/// `is_a_cluster(g)` for a subgraph (`lib/common/utils.c`).
pub(crate) fn is_a_cluster(name: &str, attrs: &AttrMap) -> bool {
    graphviz_pure_core::has_cluster_prefix(name) || Attrs(attrs).mapbool("cluster")
}

impl<'g> Model<'g> {
    pub fn root_attrs(&self) -> Attrs<'g> {
        Attrs(&self.g.graph_attrs)
    }

    /// `GD_drawing(g)->size` in points and `filled` (a `!` right after the numbers), as
    /// `getdoubles2ptf` in `lib/common/input.c` reads them.
    pub fn drawing_size(&self) -> Option<(f64, f64, bool)> {
        let p = self.root_attrs().get("size")?;
        // POINTS(a): ROUND(a * 72), with `a * 72 ± .5` fused by C (clang).
        let points = |a: f64| f64::from(graphviz_pure_core::cnum::c_round_mul(a, 72.0));
        let r = sscanf(p, "%lf,%lf%c");
        let [x, y, ..] = r.values;
        if r.count > 1 && x > 0.0 && y > 0.0 {
            return Some((points(x), points(y), r.c == Some(b'!')));
        }
        let r = sscanf(p, "%lf%c");
        let x = r.values[0];
        (r.count > 0 && x > 0.0).then(|| (points(x), points(x), r.c == Some(b'!')))
    }

    pub fn landscape(&self) -> bool {
        let a = self.root_attrs();
        if a.get("rotate").is_some_and(|r| super::attrs::atoi(r) == 90) {
            return true;
        }
        if a.get("orientation")
            .is_some_and(|o| o.starts_with('l') || o.starts_with('L'))
        {
            return true;
        }
        a.mapbool("landscape")
    }

    pub fn centered(&self) -> bool {
        self.root_attrs().mapbool("center")
    }

    /// The render model of `g`. Its node, edge and cluster labels are sized again here
    /// (`diag::quietly`): C made them in layout, and reported their diagnostics there.
    pub fn build(g: &'g Graph) -> graphviz_pure_core::Result<Self> {
        // The labels were measured by layout; measure them again as C measured them then.
        graphviz_pure_core::font::textfont_remeasuring(|| Self::build_measured(g))
    }

    fn build_measured(g: &'g Graph) -> graphviz_pure_core::Result<Self> {
        graphviz_pure_core::diag::quietly(|| Self::build_quiet(g))
    }

    fn build_quiet(g: &'g Graph) -> graphviz_pure_core::Result<Self> {
        let directed = g.kind == GraphKind::Directed;
        let env = NodeSizeEnv::from_graph(g);
        let ga = Attrs(&g.graph_attrs);

        let mut has_labels = 0u32;
        let mut nodes = Vec::with_capacity(g.nodes.len());
        for n in &g.nodes {
            let NodeGeometry {
                shape,
                width,
                height,
                label: label_geometry,
                xlabel: xlabel_geometry,
                polygon,
                record,
                epsf,
                usershape,
                ..
            } = node_geometry(n, &env)?;
            let geom = ShapeGeometry {
                shape,
                width,
                height,
                polygon,
                record,
                epsf,
                usershape,
            };
            let a = Attrs(&n.attrs);
            let fontcolor = a.late_nnstring("fontcolor", "black");
            let raw = a.get("label").unwrap_or("\\N");
            let mut label = TextLabel::from_owned_geometry(label_geometry, raw, fontcolor);
            label.pos = n.pos;
            label.set = true;
            let xlabel = xlabel_geometry.map(|xg| {
                has_labels |= NODE_XLABEL;
                let mut l =
                    TextLabel::from_owned_geometry(xg, a.get("xlabel").unwrap_or(""), fontcolor);
                if let Some(p) = n.layout.xlabel {
                    l.pos = p;
                    l.set = true;
                }
                l
            });
            let w = n.width * 72.0;
            // The layout's ND_lw/ND_rw when they differ (dot's self-loop space in ND_rw).
            let (lw, rw) = n.layout.lw_rw.unwrap_or((w / 2.0, w / 2.0));
            nodes.push(RNode {
                name: &n.name,
                attrs: a,
                geom,
                label,
                xlabel,
                coord: n.pos,
                lw,
                rw,
                ht: n.height * 72.0,
            });
        }

        let no_edge_init = g.layout.no_edge_init;
        let mut edges = Vec::with_capacity(g.edges.len());
        let mut out_edges = vec![Vec::new(); g.nodes.len()];
        for (ei, e) in g.edges.iter().enumerate() {
            out_edges[e.tail.0 as usize].push(ei);
            let a = Attrs(&e.attrs);
            // The layout's `bz.sflag`/`bz.eflag`, which `emit_edge_graphics` draws.
            let bz = e
                .spline
                .segments
                .iter()
                .map(|s| Bz {
                    list: &s.points,
                    sflag: s.sflag,
                    eflag: s.eflag,
                    sp: s.start_arrow.unwrap_or_default(),
                    ep: s.end_arrow.unwrap_or_default(),
                })
                .collect();
            let tname = &g.nodes[e.tail.0 as usize].name;
            let hname = &g.nodes[e.head.0 as usize].name;
            let lg = if no_edge_init {
                graphviz_pure_geom::node_size::EdgeLabelGeometry::default()
            } else {
                graphviz_pure_geom::node_size::edge_label_geometry_of(
                    u64::from(e.id.0),
                    &e.attrs,
                    tname,
                    hname,
                    directed,
                    &env,
                )
            };
            let fontcolor = a.late_nnstring("fontcolor", "black");
            let lfontcolor = a.late_nnstring("labelfontcolor", fontcolor);
            // The layout's `pos` and `set` of each label.
            let mk = |geo: Option<LabelGeometry>,
                      key: &str,
                      place: Option<LabelPlace>,
                      color: &str|
             -> Option<TextLabel> {
                geo.map(|lgeo| {
                    let mut l =
                        TextLabel::from_owned_geometry(lgeo, a.get(key).unwrap_or(""), color);
                    if let Some(p) = place {
                        l.pos = p.pos;
                        l.set = p.set;
                    }
                    l
                })
            };
            let main = e.label_pos.map(|pos| LabelPlace {
                pos,
                set: !e.label_unplaced,
            });
            let label = mk(lg.label, "label", main, fontcolor);
            let placed = |pos| LabelPlace { pos, set: true };
            let xlabel = mk(lg.xlabel, "xlabel", e.layout.xlabel.map(placed), fontcolor);
            let head_label = mk(lg.head_label, "headlabel", e.layout.head_label, lfontcolor);
            let tail_label = mk(lg.tail_label, "taillabel", e.layout.tail_label, lfontcolor);
            if label.is_some() {
                has_labels |= EDGE_LABEL;
            }
            if xlabel.is_some() {
                has_labels |= EDGE_XLABEL;
            }
            if head_label.is_some() {
                has_labels |= HEAD_LABEL;
            }
            if tail_label.is_some() {
                has_labels |= TAIL_LABEL;
            }
            edges.push(REdge {
                tail: e.tail.0 as usize,
                head: e.head.0 as usize,
                attrs: a,
                bz,
                label,
                xlabel,
                head_label,
                tail_label,
                tail_port: match &e.layout.port_names {
                    PortNames::Layout { tail, .. } => Some(tail.as_str()).filter(|s| !s.is_empty()),
                    PortNames::Input => e
                        .tail_port
                        .compass
                        .as_deref()
                        .or(e.tail_port.name.as_deref()),
                },
                head_port: match &e.layout.port_names {
                    PortNames::Layout { head, .. } => Some(head.as_str()).filter(|s| !s.is_empty()),
                    PortNames::Input => e
                        .head_port
                        .compass
                        .as_deref()
                        .or(e.head_port.name.as_deref()),
                },
            });
        }

        // `agfstout`/`agnxtout` order: by head node id (cgraph `agedgeseqcmpf`), then by edge
        // sequence. Node ids follow the interning of their names, approximated by creation order.
        for list in &mut out_edges {
            list.sort_by_key(|&ei| (g.edges[ei].head.0, ei));
        }

        // Cluster tree (`GD_clust`): clusters with a box, found through plain subgraphs.
        let mut is_child = vec![false; g.subgraphs.len()];
        for s in &g.subgraphs {
            for c in &s.subgraphs {
                is_child[c.0 as usize] = true;
            }
        }
        let mut clusters: Vec<RCluster<'g>> = Vec::new();
        let mut root_clusters = Vec::new();
        let flip = false;
        fn dfs<'g>(
            g: &'g Graph,
            sid: usize,
            out: &mut Vec<usize>,
            clusters: &mut Vec<RCluster<'g>>,
            has_labels: &mut u32,
            env: &NodeSizeEnv<'_>,
            flip: bool,
        ) {
            let s = &g.subgraphs[sid];
            let drawn = match s.layout.reg {
                ClusterReg::Registered => true,
                ClusterReg::Unregistered => false,
                ClusterReg::Unknown => s.bb != BoundingBox::default(),
            };
            if is_a_cluster(&s.name, &s.graph_attrs) && drawn {
                let a = Attrs(&s.graph_attrs);
                let name = g.subgraph_name(sid);
                let label = if s.layout.no_label {
                    None
                } else {
                    graph_label_geometry_in(&name, s.html_name, &s.graph_attrs, false, flip, env)
                };
                let label = label.map(|gl| {
                    *has_labels |= GRAPH_LABEL;
                    let mut l = TextLabel::from_owned_geometry(
                        gl.label,
                        a.get("label").unwrap_or(""),
                        a.late_nnstring("fontcolor", "black"),
                    );
                    l.pos = s.layout.label_pos.unwrap_or_else(|| {
                        let d = gl.border.map_or(Point::default(), |b| b.1);
                        let bb = s.bb;
                        let y = if gl.pos_flag & LABEL_AT_TOP != 0 {
                            bb.ur.y - d.y / 2.0
                        } else {
                            bb.ll.y + d.y / 2.0
                        };
                        let x = if gl.pos_flag & LABEL_AT_RIGHT != 0 {
                            bb.ur.x - d.x / 2.0
                        } else if gl.pos_flag & LABEL_AT_LEFT != 0 {
                            bb.ll.x + d.x / 2.0
                        } else {
                            (bb.ll.x + bb.ur.x) / 2.0
                        };
                        Point::new(x, y)
                    });
                    l.set = true;
                    l
                });
                let idx = clusters.len();
                clusters.push(RCluster {
                    name,
                    attrs: a,
                    sub: sid,
                    seq: s.layout.seq.unwrap_or(sid + 1),
                    bb: s.bb,
                    label,
                    children: Vec::new(),
                    nodes: s.nodes.iter().map(|n| n.0 as usize).collect(),
                });
                out.push(idx);
                let mut kids = Vec::new();
                for c in &s.subgraphs {
                    dfs(g, c.0 as usize, &mut kids, clusters, has_labels, env, flip);
                }
                clusters[idx].children = kids;
            } else {
                for c in &s.subgraphs {
                    dfs(g, c.0 as usize, out, clusters, has_labels, env, flip);
                }
            }
        }
        for (i, child) in is_child.iter().enumerate() {
            if !child {
                dfs(
                    g,
                    i,
                    &mut root_clusters,
                    &mut clusters,
                    &mut has_labels,
                    &env,
                    flip,
                );
            }
        }
        // A layout that exported its `GD_clust` lists (dot): walk them as they are,
        // in their order and with their repeats (`copyClusterInfo` in the pack branch).
        // A graph's `GD_clust` only holds clusters below it, so entries that are not
        // descendants (the key written in the input, inherited by subgraphs) are dropped:
        // they could make the cluster tree cyclic.
        let mut of_sub: Vec<Option<usize>> = vec![None; g.subgraphs.len()];
        for (ci, c) in clusters.iter().enumerate() {
            of_sub[c.sub] = Some(ci);
        }
        let mut parent: Vec<Option<usize>> = vec![None; g.subgraphs.len()];
        for (i, s) in g.subgraphs.iter().enumerate() {
            for c in &s.subgraphs {
                parent[c.0 as usize] = Some(i);
            }
        }
        let below = |si: usize, owner: Option<usize>| {
            let Some(owner) = owner else { return true };
            let mut cur = parent[si];
            while let Some(p) = cur {
                if p == owner {
                    return true;
                }
                cur = parent[p];
            }
            false
        };
        let listed = |layout: &GraphLayout, owner: Option<usize>| -> Option<Vec<usize>> {
            Some(
                layout
                    .clusters
                    .as_ref()?
                    .iter()
                    .copied()
                    .filter(|&si| si < parent.len() && below(si, owner))
                    .filter_map(|si| of_sub[si])
                    .collect(),
            )
        };
        if let Some(list) = listed(&g.layout, None) {
            root_clusters = list;
        }
        // Only the root's list repeats clusters (`copyClusterInfo`); repeats below it
        // would multiply at every level.
        for c in &mut clusters {
            if let Some(mut list) = listed(&g.subgraphs[c.sub].layout, Some(c.sub)) {
                let mut seen = std::collections::HashSet::new();
                list.retain(|&k| seen.insert(k));
                c.children = list;
            }
        }

        let bb = g.bb;
        let root_label =
            graph_label_geometry_in(g.root_name(), false, &g.graph_attrs, true, flip, &env);
        let label = root_label.map(|gl| {
            has_labels |= GRAPH_LABEL;
            let label_dimen = gl.label.dimen;
            let mut l = TextLabel::from_owned_geometry(
                gl.label,
                ga.get("label").unwrap_or(""),
                ga.late_nnstring("fontcolor", "black"),
            );
            l.pos = g.layout.label_pos.unwrap_or_else(|| {
                // place_root_label
                let d = Point::new(label_dimen.x + 16.0, label_dimen.y + 8.0);
                let x = if gl.pos_flag & LABEL_AT_RIGHT != 0 {
                    bb.ur.x - d.x / 2.0
                } else if gl.pos_flag & LABEL_AT_LEFT != 0 {
                    bb.ll.x + d.x / 2.0
                } else {
                    (bb.ll.x + bb.ur.x) / 2.0
                };
                let y = if gl.pos_flag & LABEL_AT_TOP != 0 {
                    bb.ur.y - d.y / 2.0
                } else {
                    bb.ll.y + d.y / 2.0
                };
                Point::new(x, y)
            });
            l.set = true;
            l
        });

        Ok(Model {
            g,
            node_penwidth_declared: g.node_attr_declared("penwidth"),
            directed,
            bb,
            label,
            nodes,
            edges,
            out_edges,
            clusters,
            root_clusters,
            has_labels,
        })
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn owned_labels_match_borrowed_conversion_and_move_buffers() {
        for src in [
            r#"digraph { n [label="alpha\nbeta"] }"#,
            r#"digraph { n [label=<<B>alpha</B>>] }"#,
            r#"digraph { n [label=<<TABLE><TR><TD>alpha</TD></TR></TABLE>>] }"#,
        ] {
            let graph = graphviz_pure_parser::parse_dot(src).unwrap();
            let node = &graph.nodes[0];
            let geom = node_geometry(node, &NodeSizeEnv::from_graph(&graph)).unwrap();
            let raw = node.attrs.get("label").unwrap();
            let font = geom.label.font_name.as_ptr();
            let source = geom.label.text.as_ref().map(|t| t.source.as_ptr());
            let lines: Vec<_> = geom
                .label
                .text
                .as_ref()
                .map(|t| t.lines.iter().map(|line| line.text.as_ptr()).collect())
                .unwrap_or_default();
            let html = geom.label.html_label.as_deref().map(std::ptr::from_ref);
            let expected = TextLabel::from_geometry(&geom.label, raw, "black");
            let actual = TextLabel::from_owned_geometry(geom.label, raw, "black");
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "{src}");
            assert_eq!(actual.fontname.as_ptr(), font);
            if let Some(source) = source {
                assert_eq!(actual.text.as_ptr(), source);
            }
            for (span, line) in actual.spans.iter().zip(lines) {
                assert_eq!(span.str.as_ptr(), line);
            }
            assert_eq!(actual.html.as_deref().map(std::ptr::from_ref), html);
        }
    }

    #[test]
    fn render_beziers_borrow_source_points_in_segment_order() {
        let mut graph = graphviz_pure_parser::parse_dot("digraph { a -> b }").unwrap();
        graph.edges[0].spline.segments = [1.0, 3.0]
            .map(|x| graphviz_pure_core::BezierSegment {
                points: vec![Point::new(x, 2.0), Point::new(x + 1.0, 4.0)],
                start_arrow: Some(Point::new(x, 0.0)),
                end_arrow: None,
                sflag: 1,
                eflag: 0,
            })
            .into();
        let model = Model::build(&graph).unwrap();
        assert_eq!(model.edges[0].bz.len(), 2);
        for (rendered, source) in model.edges[0]
            .bz
            .iter()
            .zip(&graph.edges[0].spline.segments)
        {
            assert_eq!(rendered.list.as_ptr(), source.points.as_ptr());
            assert_eq!(rendered.list, source.points);
            assert_eq!(rendered.sflag, source.sflag);
            assert_eq!(rendered.sp, source.start_arrow.unwrap());
        }
    }
}
