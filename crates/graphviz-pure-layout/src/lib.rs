//! Implementation crate of graphviz-pure, with no semver promise. The stable crates are
//! `graphviz-pure` and `graphviz-pure-raster`.
//!
//! Graphviz layout engines in pure Rust (`dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage`, `patchwork`).

pub use graphviz_pure_core::cdt;
pub mod circogen;
pub mod common;
pub mod cutil;
pub mod dotgen;
pub mod fdpgen;
pub mod neatogen;
pub mod ortho;
pub mod osage;
pub mod pack;
pub mod patchwork;
pub mod sfdpgen;
pub mod sparse;
pub mod twopigen;
pub mod vpsc;

use graphviz_pure_core::{Graph, GraphvizError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutEngine {
    Dot,
    Neato,
    Fdp,
    Sfdp,
    Twopi,
    Circo,
    Osage,
    Patchwork,
    /// `nop`/`nop1`: neato with `-n1`.
    Nop1,
    /// `nop2`: neato with `-n2`.
    Nop2,
}

/// Engine options set on the command line.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LayoutOptions {
    /// `-n[N]`: neato keeps the input positions (`Nop` in C). 0 runs the full layout.
    pub nop: u8,
    /// `-s[v]`: `PSinputscale`, which overrides the `inputscale` attribute of neato and fdp
    /// when positive.
    pub inputscale: f64,
    /// `-x`: `Reduce`, neato removes singletons and trees.
    pub reduce: bool,
    /// `-L`: fdp's parameters (`fdp_parms`).
    pub fdp: fdpgen::tlayout::FdpParms,
}

impl LayoutEngine {
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "dot" => Ok(Self::Dot),
            "neato" => Ok(Self::Neato),
            "fdp" => Ok(Self::Fdp),
            "sfdp" => Ok(Self::Sfdp),
            "twopi" => Ok(Self::Twopi),
            "circo" => Ok(Self::Circo),
            "osage" => Ok(Self::Osage),
            "patchwork" => Ok(Self::Patchwork),
            "nop" | "nop1" => Ok(Self::Nop1),
            "nop2" => Ok(Self::Nop2),
            other => Err(GraphvizError::UnsupportedEngine(other.to_string())),
        }
    }
}

pub fn layout_graph(graph: &mut Graph, engine: LayoutEngine) -> Result<()> {
    layout_graph_with_options(graph, engine, LayoutOptions::default())
}

/// Lays `graph` out with `engine`. Outside a [`cutil::with_rand_state`], the C library's
/// generators start fresh, as in a new process ([`cutil::in_rand_run`]).
pub fn layout_graph_with_options(
    graph: &mut Graph,
    engine: LayoutEngine,
    options: LayoutOptions,
) -> Result<()> {
    let mut options = options;
    layout_graph_in_run(graph, engine, &mut options)
}

/// [`layout_graph_with_options`] as one of a run's layouts: `options.nop` is the run's `Nop`,
/// C's global, and comes back as the layout left it. The `nop` engines and fdp's splines
/// set it to 0, so `-n` no longer applies to the graphs after them.
pub fn layout_graph_in_run(
    graph: &mut Graph,
    engine: LayoutEngine,
    options: &mut LayoutOptions,
) -> Result<()> {
    let opts = *options;
    neatogen::neatosplines::with_nop(&mut options.nop, || {
        cutil::in_rand_run(|| layout_in_run(graph, engine, opts))
    })
}

fn layout_in_run(graph: &mut Graph, engine: LayoutEngine, options: LayoutOptions) -> Result<()> {
    debug_assert!(
        graph.subgraph_lists_are_closed(),
        "a subgraph lists a member twice or lacks one of its nested subgraphs'"
    );
    common::splines::reset_bz_split_budget();
    graphviz_pure_core::font::textfont_layout_begin();
    common::utils::set_ps_inputscale(options.inputscale);
    fdpgen::tlayout::set_fdp_parms(options.fdp);
    neatogen::stuff::set_reduce(options.reduce);
    graphviz_pure_core::charset::begin_layout(
        graph.root_name(),
        graph.graph_attrs.get("charset").map(String::as_str),
    );
    graphviz_pure_core::usershape::begin_layout(
        graph.graph_attrs.get("imagepath").map(String::as_str),
        graph.node_attr_declared("shapefile"),
        graphviz_pure_core::cnum::drawing_dpi(&graph.graph_attrs),
    );
    // A layout of the same graph starts from no result (the sequence numbers of nodes an
    // earlier `Reduce` deleted stay: the objects are gone).
    clear_layout_results(graph);
    // graph_init's do_graph_label(g) (lib/common/input.c:711) makes the root label before
    // any engine's node and edge init, so its diagnostics come first; the engines' own
    // calls do not repeat them.
    let _ = graphviz_pure_geom::graph_label_geometry_in(
        graph.root_name(),
        false,
        &graph.graph_attrs,
        true,
        false,
        &graphviz_pure_geom::NodeSizeEnv::from_graph(graph),
    );
    // graph_init's `GD_drawing(g)->xdots = init_xdot(g)` (lib/common/input.c:784).
    graph.layout.background = graphviz_pure_core::xdot::init_xdot(graph);
    let effective_engine = if engine == LayoutEngine::Dot
        && let Some(attr_layout) = graph.graph_attrs.get("layout")
        && let Ok(parsed) = LayoutEngine::parse(attr_layout)
    {
        parsed
    } else {
        engine
    };
    let result = match effective_engine {
        LayoutEngine::Dot => dotgen::layout_dot(graph),
        LayoutEngine::Neato => neatogen::neatoinit::neato_layout(graph),
        LayoutEngine::Nop1 => neatogen::neatoinit::nop_layout(graph, 1),
        LayoutEngine::Nop2 => neatogen::neatoinit::nop_layout(graph, 2),
        LayoutEngine::Fdp => fdpgen::layout::fdp_layout(graph),
        LayoutEngine::Sfdp => sfdpgen::sfdpinit::sfdp_layout(graph),
        LayoutEngine::Twopi => twopigen::twopiinit::twopi_layout(graph),
        LayoutEngine::Circo => circogen::circularinit::circo_layout(graph),
        LayoutEngine::Osage => osage::osage_layout(graph),
        LayoutEngine::Patchwork => patchwork::patchwork_layout(graph),
    };
    // what the renderers draw the images and epsf shapes from
    graph.layout.usershapes = graphviz_pure_core::usershape::layout_result();
    result
}

/// Resets every layout result of `graph` but the sequence numbers.
fn clear_layout_results(graph: &mut Graph) {
    use graphviz_pure_core::{EdgeLayout, GraphLayout, NodeLayout};
    graph.layout = GraphLayout::default();
    graph.render_view = None;
    for s in &mut graph.subgraphs {
        s.layout = GraphLayout::default();
    }
    for n in &mut graph.nodes {
        n.layout = NodeLayout {
            seq: n.layout.seq,
            ..NodeLayout::default()
        };
    }
    for e in &mut graph.edges {
        e.spline = graphviz_pure_core::Spline::default();
        e.label_pos = None;
        e.label_unplaced = false;
        e.layout = EdgeLayout {
            seq: e.layout.seq,
            ..EdgeLayout::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{AttrMap, Graph, GraphKind, Point, SubgraphData, SubgraphId};

    fn make_chain_graph() -> Graph {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let a = g.ensure_node("a");
        let b = g.ensure_node("b");
        let c = g.ensure_node("c");
        g.add_edge(a, b, AttrMap::new());
        g.add_edge(b, c, AttrMap::new());
        g
    }

    #[test]
    fn relayout_drops_the_previous_lw_rw_split() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let a = g.ensure_node("a");
        g.add_edge(a, a, AttrMap::new());
        g.graph_attrs.insert("splines".into(), String::new());
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert!(g.nodes[a.0 as usize].layout.lw_rw.is_some());
        g.graph_attrs.insert("splines".into(), "true".into());
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert!(g.nodes[a.0 as usize].layout.lw_rw.is_none());
    }

    // A layout of a graph starts from no result: fdp's render view and the splines of an
    // earlier layout do not survive a dot layout that routes no edge.
    #[test]
    fn relayout_drops_the_previous_view_and_splines() {
        let mut g = make_chain_graph();
        g.edges[0].attrs.insert("label".into(), "l".into());
        layout_graph(&mut g, LayoutEngine::Fdp).unwrap();
        assert!(g.edges.iter().all(|e| !e.spline.segments.is_empty()));
        g.render_view = Some(Box::new(g.clone()));
        g.graph_attrs.insert("splines".into(), "none".into());
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert!(g.render_view.is_none());
        assert!(g.edges.iter().all(|e| e.spline.segments.is_empty()));
    }

    /// An edge as (tail, head, attributes).
    type TestEdge<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

    /// A directed graph with `edges`.
    fn weighted_graph(edges: &[TestEdge]) -> Graph {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        for &(t, h, attrs) in edges {
            let (t, h) = (g.ensure_node(t), g.ensure_node(h));
            let attrs =
                AttrMap::from_plain(attrs.iter().map(|&(k, v)| (k.to_string(), v.to_string())));
            g.add_edge(t, h, attrs);
        }
        g
    }

    fn dot_error(g: &mut Graph) -> String {
        match layout_graph(g, LayoutEngine::Dot) {
            Err(graphviz_pure_core::GraphvizError::LayoutError { message, .. }) => message,
            other => panic!("expected a layout error, got {other:?}"),
        }
    }

    // Issue #252: C's `virtual_weight` (mincross.c:1725) exits with this error where the
    // weight times the endpoint factor (up to 4, between two virtual nodes) overflows.
    #[test]
    fn virtual_weight_overflow_fails_the_layout_like_c() {
        let w = [("weight", "2000000000"), ("minlen", "3")];
        let mut g = weighted_graph(&[("a", "c", &w)]);
        assert_eq!(
            dot_error(&mut g),
            "overflow when calculating virtual weight of edge"
        );
    }

    // Issue #252: C's `x_cutval` (ns.c:1060) exits with this error where a cut value's
    // weight sum overflows.
    #[test]
    fn cut_value_overflow_fails_the_layout_like_c() {
        let w: &[(&str, &str)] = &[("weight", "2000000000")];
        let mut g = weighted_graph(&[("a", "b", w), ("a", "c", w), ("b", "d", w), ("c", "d", w)]);
        assert_eq!(dot_error(&mut g), "overflow when computing edge weight sum");
    }

    // Weight sums C leaves unchecked (`group`'s x100, merged parallel edges, newrank's
    // merged constraint edges) wrap as in C instead of panicking in a build with overflow
    // checks. Three parallel edges wrap back to a positive weight, and C draws the graph;
    // two wrap negative, and C crashes in `update` (ns.c:710) for want of an entering edge,
    // where the port fails the layout.
    #[test]
    fn unchecked_weight_sums_wrap_like_c() {
        let w: &[(&str, &str)] = &[("weight", "2000000000")];
        let mut three = weighted_graph(&[("a", "b", w), ("a", "b", w), ("a", "b", w)]);
        layout_graph(&mut three, LayoutEngine::Dot).unwrap();

        let mut grouped = weighted_graph(&[("a", "b", &[("weight", "30000000")])]);
        for n in &mut grouped.nodes {
            n.attrs.insert("group".into(), "g".into());
        }
        let mut parallel = weighted_graph(&[("a", "b", w), ("a", "b", w)]);
        let mut newrank = weighted_graph(&[("a", "b", w), ("a", "b", w)]);
        newrank.graph_attrs.insert("newrank".into(), "true".into());
        for g in [&mut grouped, &mut parallel, &mut newrank] {
            assert_eq!(dot_error(g), common::ns::NO_ENTERING_EDGE);
        }
    }

    #[test]
    fn test_engine_parse() {
        assert_eq!(LayoutEngine::parse("dot").unwrap(), LayoutEngine::Dot);
        assert_eq!(LayoutEngine::parse("NEATO").unwrap(), LayoutEngine::Neato);
        assert_eq!(LayoutEngine::parse("fdp").unwrap(), LayoutEngine::Fdp);
        assert_eq!(LayoutEngine::parse("sfdp").unwrap(), LayoutEngine::Sfdp);
        assert_eq!(LayoutEngine::parse("twopi").unwrap(), LayoutEngine::Twopi);
        assert_eq!(LayoutEngine::parse("circo").unwrap(), LayoutEngine::Circo);
        assert_eq!(LayoutEngine::parse("osage").unwrap(), LayoutEngine::Osage);
        assert_eq!(
            LayoutEngine::parse("patchwork").unwrap(),
            LayoutEngine::Patchwork
        );
        assert!(LayoutEngine::parse("unknown").is_err());
    }

    #[test]
    fn test_dot_rank_assignment_and_tb_coordinates() {
        let mut g = make_chain_graph();
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();

        assert_eq!(g.nodes[0].rank, 0);
        assert_eq!(g.nodes[1].rank, 1);
        assert_eq!(g.nodes[2].rank, 2);

        // In default TB rankdir, rank 0 is at top (highest Y) and rank 2 is at bottom (lowest Y).
        assert!(g.nodes[0].pos.y > g.nodes[1].pos.y);
        assert!(g.nodes[1].pos.y > g.nodes[2].pos.y);
        // Single-chain nodes should be vertically aligned on X.
        assert!((g.nodes[0].pos.x - g.nodes[1].pos.x).abs() < 1e-4);
        assert!((g.nodes[1].pos.x - g.nodes[2].pos.x).abs() < 1e-4);

        // Graph bounding box lower-left must be normalized to (0, 0).
        assert_eq!(g.bb.ll, Point::new(0.0, 0.0));
        assert!(g.bb.width() > 0.0 && g.bb.height() > 0.0);
    }

    #[test]
    fn test_dot_rankdir_variants() {
        // BT: rank 0 at bottom (smaller Y), rank 2 at top (larger Y)
        let mut g_bt = make_chain_graph();
        g_bt.graph_attrs
            .insert("rankdir".to_string(), "BT".to_string());
        layout_graph(&mut g_bt, LayoutEngine::Dot).unwrap();
        assert!(g_bt.nodes[0].pos.y < g_bt.nodes[1].pos.y);
        assert!(g_bt.nodes[1].pos.y < g_bt.nodes[2].pos.y);

        // LR: rank 0 on left (smaller X), rank 2 on right (larger X)
        let mut g_lr = make_chain_graph();
        g_lr.graph_attrs
            .insert("rankdir".to_string(), "LR".to_string());
        layout_graph(&mut g_lr, LayoutEngine::Dot).unwrap();
        assert!(g_lr.nodes[0].pos.x < g_lr.nodes[1].pos.x);
        assert!(g_lr.nodes[1].pos.x < g_lr.nodes[2].pos.x);

        // RL: rank 0 on right (larger X), rank 2 on left (smaller X)
        let mut g_rl = make_chain_graph();
        g_rl.graph_attrs
            .insert("rankdir".to_string(), "RL".to_string());
        layout_graph(&mut g_rl, LayoutEngine::Dot).unwrap();
        assert!(g_rl.nodes[0].pos.x > g_rl.nodes[1].pos.x);
        assert!(g_rl.nodes[1].pos.x > g_rl.nodes[2].pos.x);
    }

    #[test]
    fn test_dot_cycle_breaking_and_rank_same() {
        let mut g = Graph::new("Cycle", GraphKind::Directed, false);
        let a = g.ensure_node("a");
        let b = g.ensure_node("b");
        let c = g.ensure_node("c");
        let d = g.ensure_node("d");

        let mut heavy = AttrMap::new();
        heavy.insert("weight".to_string(), "10".to_string());
        g.add_edge(a, b, heavy.clone());
        g.add_edge(b, c, heavy);
        // Light back-edge c -> a should be reversed during cycle breaking.
        g.add_edge(c, a, AttrMap::new());
        g.add_edge(b, d, AttrMap::new());

        // Constrain c and d to rank=same.
        let mut sub_attrs = AttrMap::new();
        sub_attrs.insert("rank".to_string(), "same".to_string());
        g.subgraphs.push(SubgraphData {
            id: SubgraphId(0),
            name: "same_cd".to_string(),
            is_cluster: false,
            graph_attrs: sub_attrs,
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: vec![c, d],
            edges: Vec::new(),
            subgraphs: Vec::new(),
            bb: Default::default(),
            html_name: false,
            empty_name: false,
            layout: Default::default(),
        });

        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert!(g.nodes[a.0 as usize].rank < g.nodes[b.0 as usize].rank);
        assert!(g.nodes[b.0 as usize].rank < g.nodes[c.0 as usize].rank);
        assert_eq!(g.nodes[c.0 as usize].rank, g.nodes[d.0 as usize].rank);
    }

    /// A subgraph of `g` with `nodes` and `edges` and no attributes but `attrs`.
    fn add_subgraph(
        g: &mut Graph,
        name: &str,
        attrs: &[(&str, &str)],
        nodes: &[graphviz_pure_core::NodeId],
        edges: &[graphviz_pure_core::EdgeId],
    ) -> usize {
        let i = g.subgraphs.len();
        g.subgraphs.push(SubgraphData {
            id: SubgraphId(i as u32),
            name: name.to_string(),
            is_cluster: name.starts_with("cluster"),
            graph_attrs: AttrMap::from_plain(
                attrs.iter().map(|&(k, v)| (k.to_string(), v.to_string())),
            ),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes: nodes.to_vec(),
            edges: edges.to_vec(),
            subgraphs: Vec::new(),
            bb: Default::default(),
            html_name: false,
            empty_name: false,
            layout: Default::default(),
        });
        i
    }

    fn members(g: &Graph, s: usize) -> (Vec<u32>, Vec<u32>) {
        let mut n: Vec<u32> = g.subgraphs[s].nodes.iter().map(|x| x.0).collect();
        let mut e: Vec<u32> = g.subgraphs[s].edges.iter().map(|x| x.0).collect();
        n.sort_unstable();
        e.sort_unstable();
        (n, e)
    }

    // Issue #106: dot's node_induce adds a root edge between two nodes of a cluster to
    // the cluster (`agsubedge`), and the writers read that membership.
    #[test]
    fn dot_exports_the_edges_node_induce_adds_to_a_cluster() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let (a, b) = (g.ensure_node("a"), g.ensure_node("b"));
        let ab = g.add_edge(a, b, AttrMap::new());
        let c = add_subgraph(&mut g, "cluster_1", &[], &[a, b], &[]);
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert_eq!(members(&g, c), (vec![a.0, b.0], vec![ab.0]));
    }

    // Issue #102: node_induce deletes a node from a cluster when an earlier sibling
    // cluster holds it (`agdelete`), with its edges.
    #[test]
    fn dot_exports_a_node_node_induce_deletes_from_a_later_cluster() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let [v6, v0, v1, v5] = ["v6", "v0", "v1", "v5"].map(|n| g.ensure_node(n));
        let e60 = g.add_edge(v6, v0, AttrMap::new());
        let e15 = g.add_edge(v1, v5, AttrMap::new());
        let e10 = g.add_edge(v1, v0, AttrMap::new());
        let c0 = add_subgraph(&mut g, "cluster_0", &[], &[v6], &[]);
        let c2 = add_subgraph(&mut g, "cluster_2", &[], &[v6, v0, v1, v5], &[e60, e15]);
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert_eq!(members(&g, c0), (vec![v6.0], vec![]));
        assert_eq!(
            members(&g, c2),
            (vec![v0.0, v1.0, v5.0], vec![e15.0, e10.0])
        );
    }

    // Issues #102 and #108: mark_clusters deletes a cluster node that is in a rank set
    // (`agdelete`, "already in a rankset").
    #[test]
    fn dot_exports_a_node_mark_clusters_deletes_from_a_cluster() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        let [x, y, z] = ["x", "y", "z"].map(|n| g.ensure_node(n));
        let xy = g.add_edge(x, y, AttrMap::new());
        let c = add_subgraph(&mut g, "cluster_a", &[], &[x, y], &[xy]);
        let r = add_subgraph(&mut g, "", &[("rank", "same")], &[x, z], &[]);
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert_eq!(members(&g, c), (vec![y.0], vec![]));
        assert_eq!(members(&g, r), (vec![x.0, z.0], vec![]));
    }

    // Issue #102: with pack and several components, dot lays out the components'
    // projections (`projectG`): the original clusters keep their declared members.
    #[test]
    fn dot_pack_keeps_the_declared_members_of_the_original_clusters() {
        let mut g = Graph::new("G", GraphKind::Directed, false);
        g.graph_attrs.insert("pack".into(), "true".into());
        let [a, b, c] = ["a", "b", "c"].map(|n| g.ensure_node(n));
        let ab = g.add_edge(a, b, AttrMap::new());
        g.add_edge(c, b, AttrMap::new());
        g.ensure_node("x");
        let s = add_subgraph(&mut g, "cluster_2", &[], &[a, b, c], &[ab]);
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        assert_eq!(members(&g, s), (vec![a.0, b.0, c.0], vec![ab.0]));
    }

    #[test]
    fn test_dot_crossing_minimization() {
        // Construct a 2-layer bipartite graph where initial order (a1, a2) -> (b1, b2)
        // has edges a1->b2 and a2->b1 that would cross unless mincross reorders one rank.
        let mut g = Graph::new("Cross", GraphKind::Directed, false);
        let a1 = g.ensure_node("a1");
        let a2 = g.ensure_node("a2");
        let b1 = g.ensure_node("b1");
        let b2 = g.ensure_node("b2");
        g.add_edge(a1, b2, AttrMap::new());
        g.add_edge(a2, b1, AttrMap::new());

        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        // Either a1 is to the left of a2 AND b2 is to the left of b1, or vice versa (0 crossings).
        let a_diff = g.nodes[a1.0 as usize].pos.x - g.nodes[a2.0 as usize].pos.x;
        let b_diff = g.nodes[b2.0 as usize].pos.x - g.nodes[b1.0 as usize].pos.x;
        assert!(a_diff * b_diff > 0.0);
    }

    #[test]
    fn test_all_eight_layout_engines_succeed() {
        let engines = [
            LayoutEngine::Dot,
            LayoutEngine::Neato,
            LayoutEngine::Fdp,
            LayoutEngine::Sfdp,
            LayoutEngine::Twopi,
            LayoutEngine::Circo,
            LayoutEngine::Osage,
            LayoutEngine::Patchwork,
        ];

        for engine in engines {
            let mut g = Graph::new("TestAll", GraphKind::Directed, false);
            let a = g.ensure_node("a");
            let b = g.ensure_node("b");
            let c = g.ensure_node("c");
            let d = g.ensure_node("d");
            g.add_edge(a, b, AttrMap::new());
            g.add_edge(b, c, AttrMap::new());
            g.add_edge(c, d, AttrMap::new());
            g.add_edge(d, a, AttrMap::new());

            layout_graph(&mut g, engine).unwrap();
            assert_eq!(g.bb.ll, Point::new(0.0, 0.0));
            assert!(g.bb.width() > 0.0 && g.bb.height() > 0.0);
            for node in &g.nodes {
                assert!(node.pos.x.is_finite() && node.pos.y.is_finite());
                assert!(node.pos.x >= 0.0 && node.pos.y >= 0.0);
            }
            // patchwork draws no edges, as in C.
            if engine != LayoutEngine::Patchwork {
                for edge in &g.edges {
                    assert!(!edge.spline.segments.is_empty());
                }
            }
        }
    }
}
