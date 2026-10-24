//! Output renderers for `graphviz-pure` (`svg`, `json`, `json0`, `dot`, `xdot`, `canon`, `plain`).

#![forbid(unsafe_code)]

mod agwrite;
pub mod emit;
pub mod json;
mod svg_engine;
mod xdot_engine;

/// Renders `-Tdot` (`xdot == false`) or `-Txdot`.
pub fn render_dot_positioned(graph: &Graph, xdot: bool) -> String {
    render_graph(
        graph,
        if xdot {
            OutputFormat::Xdot
        } else {
            OutputFormat::Dot
        },
    )
    .unwrap_or_default()
}

/// Renders `-Tplain`.
pub fn render_plain(graph: &Graph) -> String {
    render_graph(graph, OutputFormat::Plain).unwrap_or_default()
}

/// Renders `-Tjson` (`json0 == false`) or `-Tjson0`.
pub fn render_json(graph: &Graph, json0: bool) -> Result<String> {
    render_graph(
        graph,
        if json0 {
            OutputFormat::Json0
        } else {
            OutputFormat::Json
        },
    )
}
pub use emit::{reset_render_state, set_y_invert};

/// Renders `graph` as SVG (`-Tsvg`).
pub fn render_svg(graph: &Graph) -> String {
    graphviz_pure_core::usershape::rendering(&graph.layout.usershapes, || {
        let m = emit::model::Model::build(graph);
        emit::run(&m, svg_engine::SvgEngine::default()).out
    })
}

use graphviz_pure_core::{Graph, GraphvizError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Svg,
    Json,
    Json0,
    Dot,
    Xdot,
    Canon,
    Plain,
}

impl OutputFormat {
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "svg" => Ok(Self::Svg),
            "json" => Ok(Self::Json),
            "json0" => Ok(Self::Json0),
            "dot" | "gv" => Ok(Self::Dot),
            "xdot" | "xdot1.2" | "xdot1.4" => Ok(Self::Xdot),
            "dot_json" | "xdot_json" => Ok(Self::Json),
            "canon" => Ok(Self::Canon),
            "plain" | "plain-ext" => Ok(Self::Plain),
            other => Err(GraphvizError::UnsupportedFormat(other.to_string())),
        }
    }
}

pub fn render_graph(graph: &Graph, format: OutputFormat) -> Result<String> {
    // The format's own name, which the render diagnostics print.
    let name = match format {
        OutputFormat::Svg => "svg",
        OutputFormat::Json => "json",
        OutputFormat::Json0 => "json0",
        OutputFormat::Dot => "dot",
        OutputFormat::Xdot => "xdot",
        OutputFormat::Canon => "canon",
        OutputFormat::Plain => "plain",
    };
    render_graph_named(graph, format, name)
}

/// `agfstnode`/`agfstout` edge order of an unlaid graph.
fn input_edge_order(g: &Graph) -> Vec<usize> {
    let mut out = vec![Vec::new(); g.nodes.len()];
    for (i, e) in g.edges.iter().enumerate() {
        out[e.tail.0 as usize].push(i);
    }
    for l in &mut out {
        l.sort_by_key(|&e| (g.edges[e].head.0, e));
    }
    out.into_iter().flatten().collect()
}

/// `IS_CLUST_NODE(n)` for each node.
fn clust_nodes(g: &Graph) -> Vec<bool> {
    g.nodes.iter().map(|n| n.layout.clust_node).collect()
}

/// Edge iteration order of `agfstnode`/`agfstout` (numbering of `_gvid` in JSON).
fn edge_order(m: &emit::model::Model<'_>) -> Vec<usize> {
    m.out_edges.iter().flatten().copied().collect()
}

/// `cgraph` with layout attributes and, for xdot, draw operations attached. `cgraph` is the
/// model's graph with what earlier jobs set on it. The third value is what the xdot job,
/// run with `opts`, wrote straight into its output file.
fn attached(
    m: &emit::model::Model<'_>,
    cgraph: &Graph,
    xdot: bool,
    fixed_xdot: Option<&str>,
    opts: emit::RunJob,
) -> (Graph, Vec<std::collections::BTreeSet<String>>, String) {
    let (mut g, _, _, mut forced) = emit::output::attach_attrs(m, cgraph);
    let mut output_file = String::new();
    if xdot {
        // `agget(g, "xdotversion")` reads what an earlier xdot job set (#362).
        let eng = xdot_engine::XdotEngine::new(
            cgraph.graph_attrs.get("xdotversion").map(String::as_str),
            fixed_xdot,
            emit::y_offset(m.bb),
        );
        let (eng, text) = emit::run_job(m, eng, opts);
        output_file = text;
        for (kind, name, value) in eng.sets {
            let attrs = match kind {
                emit::job::ObjKind::Root => &mut g.graph_attrs,
                emit::job::ObjKind::Cluster(c) => &mut g.subgraphs[m.clusters[c].sub].graph_attrs,
                emit::job::ObjKind::Node(n) => &mut g.nodes[n].attrs,
                emit::job::ObjKind::Edge(e) => &mut g.edges[e].attrs,
            };
            if let emit::job::ObjKind::Cluster(c) = kind {
                forced[m.clusters[c].sub].insert(name.to_string());
            }
            // `xd->version_s = agget(g, "xdotversion")` is set back as it is, flag included.
            let root = &cgraph.graph_attrs;
            let html = name == "xdotversion"
                && fixed_xdot.is_none()
                && root.get(name) == Some(&value)
                && root.is_html(name);
            attrs.set(name, value, html);
        }
    }
    (g, forced, output_file)
}

/// Records the attributes the layout and renderers set in cgraph's dictionaries:
/// `safe_dcl` for node and edge attributes, and graph values as local symbols (`agxset`).
/// `set` holds, per subgraph, the graph attributes a writer set with `agxset`
/// (`rec_attach_bb`, xdot's `_draw_`), which become local values even when they equal the
/// inherited ones.
fn sync_dicts(g: &mut Graph, orig: &Graph, set: &[std::collections::BTreeSet<String>]) {
    use graphviz_pure_core::AttrSym;
    let none = std::collections::BTreeSet::new();
    let set_graph = |d: &mut std::collections::BTreeMap<String, AttrSym>,
                     now: &graphviz_pure_core::AttrMap,
                     before: &graphviz_pure_core::AttrMap,
                     set: &std::collections::BTreeSet<String>| {
        for (k, v) in now {
            if before.get(k) != Some(v) || before.is_html(k) != now.is_html(k) || set.contains(k) {
                let sym = d.entry(k.clone()).or_default();
                sym.defval = v.clone();
                sym.html_defval = now.is_html(k);
            }
        }
    };
    let (root_now, root_before) = (g.graph_attrs.clone(), &orig.graph_attrs);
    // safe_dcl declares graph attributes on the root before setting them.
    for k in root_now.keys() {
        if !root_before.contains_key(k) {
            g.dicts[0].graph.entry(k.clone()).or_default();
        }
    }
    set_graph(&mut g.dicts[0].graph, &root_now, root_before, &none);
    for i in 0..g.subgraphs.len() {
        let now = g.subgraphs[i].graph_attrs.clone();
        for k in now.keys() {
            if !orig.subgraphs[i].graph_attrs.contains_key(k) {
                g.dicts[0].graph.entry(k.clone()).or_default();
            }
        }
        set_graph(
            &mut g.dicts[i + 1].graph,
            &now,
            &orig.subgraphs[i].graph_attrs,
            set.get(i).unwrap_or(&none),
        );
    }
    let mut node_keys = std::collections::BTreeSet::new();
    for n in &g.nodes {
        node_keys.extend(n.attrs.keys().cloned());
    }
    for k in node_keys {
        // Every subgraph's symbols are also in the root's dictionary.
        g.dicts[0].node.entry(k).or_default();
    }
    let mut edge_keys = std::collections::BTreeSet::new();
    for e in &g.edges {
        edge_keys.extend(e.attrs.keys().cloned());
    }
    for k in edge_keys {
        // Every subgraph's symbols are also in the root's dictionary.
        g.dicts[0].edge.entry(k).or_default();
    }
}

/// Renders `graph`; `name` is the requested format name (distinguishes `plain-ext`).
/// The output is the writer's alone: there is no output file for the text C writes past
/// the renderer ([`GraphJobs::write_output_file_text`]).
pub fn render_graph_named(graph: &Graph, format: OutputFormat, name: &str) -> Result<String> {
    GraphJobs::new(graph).render(format, name)
}

/// A render plugin that draws nothing (`gvrender_core_dot.c`'s `dot_engine` and
/// `gvrender_core_json.c`'s `json_engine`): `emit_graph` still walks the graph for it, with
/// its features, and reports what the walk finds (styles, layers).
struct NullEngine<'a> {
    features: emit::job::Features,
    langname: &'a str,
    render_type: &'static str,
}

impl emit::job::Engine for NullEngine<'_> {
    fn features(&self) -> emit::job::Features {
        self.features
    }
    fn output_langname(&self) -> &str {
        self.langname
    }
    fn render_type(&self) -> &str {
        self.render_type
    }
}

/// `render_features_dot` (`dot`, `canon`, `plain`, `plain-ext`).
const DOT_FEATURES: emit::job::Features = emit::job::Features {
    flags: emit::job::GVRENDER_DOES_TRANSFORM,
    default_pad: 0.0,
    knowncolors: &[],
    rgba: false,
};

/// `render_features_json` (`json`, `json0`, `dot_json`, `xdot_json`).
const JSON_FEATURES: emit::job::Features = emit::job::Features {
    flags: emit::job::GVRENDER_DOES_TRANSFORM
        | emit::job::GVRENDER_DOES_MAPS
        | emit::job::GVRENDER_DOES_TARGETS
        | emit::job::GVRENDER_DOES_TOOLTIPS,
    default_pad: 0.0,
    knowncolors: &[],
    rgba: false,
};

/// Runs `emit_graph` for a render plugin that draws nothing, of `render_type` (`dot` with
/// [`DOT_FEATURES`], `json` with [`JSON_FEATURES`]); returns what it wrote straight into
/// the output file.
fn emit_null(
    m: &emit::model::Model<'_>,
    features: emit::job::Features,
    render_type: &'static str,
    langname: &str,
    opts: emit::RunJob,
) -> String {
    let eng = NullEngine {
        features,
        langname,
        render_type,
    };
    emit::run_job(m, eng, opts).1
}

/// The graph as cgraph holds it for the writers that do not attach layout attributes
/// (`canon`, `dot_json`, `xdot_json`): fdp's render view is not part of it.
fn cgraph_view(graph: &Graph) -> Graph {
    let mut g = graph.clone();
    g.render_view = None;
    g
}

/// One laid-out graph as `gvRenderJobs` renders it: one job per output format, each running
/// `emit_graph` (which reports render-time diagnostics for every format, drawing or not)
/// and then its writer. C's jobs share one cgraph: what a job sets on it with `agxset`
/// (`attach_attrs`, xdot's `_draw_` and `xdotversion`) is what every later job reads and
/// writes, over the input's values.
pub struct GraphJobs<'g> {
    /// The layout's graph: the jobs' models are built from it.
    graph: &'g Graph,
    /// The cgraph's attributes after the jobs so far; `None` while no job has set any.
    cgraph: Option<Graph>,
    /// Per subgraph, the graph attributes earlier jobs set with `agxset` (see
    /// [`sync_dicts`]): they stay local values of the subgraph for every later writer.
    forced: Vec<std::collections::BTreeSet<String>>,
    /// What jobs wrote to the standard output rather than to their own output: the xdot
    /// job of `-Tjson` (`json_begin_graph`'s `gvRender` to no file).
    stdout: String,
    /// Whether the jobs keep the text C writes past the renderer, straight into the output
    /// file (`epsf_gencode`'s PostScript call).
    output_file: bool,
}

impl<'g> GraphJobs<'g> {
    pub fn new(graph: &'g Graph) -> Self {
        GraphJobs {
            graph,
            cgraph: None,
            forced: Vec::new(),
            stdout: String::new(),
            output_file: false,
        }
    }

    /// Keeps (`true`) the text C writes with `fprintf(job->output_file, ...)`, past the
    /// renderer, as the CLI does: inside the SVG, before the other writers' output, and
    /// for `-Tjson` on the standard output too ([`Self::take_stdout`]). Off by default, so
    /// that the output of the library is the writer's document alone.
    pub fn write_output_file_text(&mut self, on: bool) {
        self.output_file = on;
    }

    /// Takes what the jobs run so far wrote to the standard output ([`Self::render`]
    /// returns only the job's own output).
    pub fn take_stdout(&mut self) -> String {
        std::mem::take(&mut self.stdout)
    }

    /// Keeps what a job set on the cgraph for the jobs after it.
    fn keep(&mut self, g: &Graph, forced: &[std::collections::BTreeSet<String>]) {
        self.cgraph = Some(g.clone());
        if self.forced.len() < forced.len() {
            self.forced.resize(forced.len(), Default::default());
        }
        for (into, from) in self.forced.iter_mut().zip(forced) {
            into.extend(from.iter().cloned());
        }
    }

    /// Runs the job for `format` as the run's first job ([`Self::render_as`]).
    pub fn render(&mut self, format: OutputFormat, name: &str) -> Result<String> {
        self.render_as(0, format, name)
    }

    /// Runs the job for `format`; `name` is the `-T` name (`job->output_langname`), which
    /// distinguishes `plain-ext`, `dot_json` and the xdot versions, and `job` the job's
    /// place in the run's job list, whose state it keeps from graph to graph. The images
    /// and epsf shapes are those the layout found (`GraphLayout::usershapes`).
    pub fn render_as(&mut self, job: usize, format: OutputFormat, name: &str) -> Result<String> {
        let graph = self.graph;
        graphviz_pure_core::usershape::rendering(&graph.layout.usershapes, || {
            self.render_job(job, format, name)
        })
    }

    fn render_job(&mut self, job: usize, format: OutputFormat, name: &str) -> Result<String> {
        let opts = emit::RunJob {
            job: Some(job),
            output_file: self.output_file,
        };
        let graph = self.graph;
        let cgraph = self.cgraph.as_ref().unwrap_or(graph);
        graphviz_pure_core::charset::begin_render(
            graph.root_name(),
            graph.graph_attrs.get("charset").map(String::as_str),
        );
        let lname = name.to_ascii_lowercase();
        // `canon` and the json formats without layout attributes write the cgraph as the
        // layout left it, with what earlier jobs attached.
        let unattached =
            format == OutputFormat::Canon || lname == "dot_json" || lname == "xdot_json";
        if unattached {
            let m = emit::model::Model::build(graph);
            let pre = if format == OutputFormat::Canon {
                emit_null(&m, DOT_FEATURES, "dot", name, opts)
            } else {
                emit_null(&m, JSON_FEATURES, "json", name, opts)
            };
            let mut g = cgraph_view(cgraph);
            if format == OutputFormat::Canon {
                if g.dicts.len() == g.subgraphs.len() + 1 {
                    sync_dicts(&mut g, graph, &self.forced);
                    return Ok(pre + &agwrite::agwrite(&g));
                }
                return Ok(pre + &graphviz_pure_parser::write_canon_forced(&g, &self.forced));
            }
            // `write_attrs` reads the root's dictionaries, as `agwrite` does.
            if g.dicts.len() == g.subgraphs.len() + 1 {
                sync_dicts(&mut g, graph, &self.forced);
            }
            return Ok(pre
                + &json::write_json(
                    &g,
                    lname == "xdot_json",
                    &input_edge_order(graph),
                    &clust_nodes(graph),
                ));
        }
        // Renderers other than dot, xdot, json and canon run before undoClusterEdges.
        let drawn = match (format, graph.render_view.as_deref()) {
            (OutputFormat::Svg, Some(v)) => v,
            _ => graph,
        };
        let m = emit::model::Model::build(drawn);
        // what `emit_graph` wrote straight into the output file comes before the writer's
        // output, which the writers below write at the end
        let mut pre = String::new();
        let text = match format {
            OutputFormat::Svg => {
                emit::run_job(&m, svg_engine::SvgEngine::default(), opts)
                    .0
                    .out
            }
            OutputFormat::Json | OutputFormat::Json0 => {
                // `json_begin_graph`: json renders the graph as xdot first (a job of its own,
                // which writes to the standard output), json0 attaches the layout
                // attributes; then `emit_graph` runs for the json job itself.
                let nested = emit::RunJob { job: None, ..opts };
                let (mut g, forced, stdout) =
                    attached(&m, cgraph, format == OutputFormat::Json, None, nested);
                self.stdout.push_str(&stdout);
                self.keep(&g, &forced);
                if g.dicts.len() == g.subgraphs.len() + 1 {
                    sync_dicts(&mut g, graph, &self.forced);
                }
                pre = emit_null(&m, JSON_FEATURES, "json", name, opts);
                json::write_json(
                    &g,
                    format == OutputFormat::Json,
                    &edge_order(&m),
                    &clust_nodes(graph),
                )
            }
            OutputFormat::Dot | OutputFormat::Xdot => {
                let xdot = format == OutputFormat::Xdot;
                let (mut g, forced, text) = attached(
                    &m,
                    cgraph,
                    xdot,
                    match lname.as_str() {
                        "xdot1.2" => Some("1.2"),
                        "xdot1.4" => Some("1.4"),
                        _ => None,
                    },
                    opts,
                );
                pre = text;
                if !xdot {
                    pre = emit_null(&m, DOT_FEATURES, "dot", name, opts);
                }
                self.keep(&g, &forced);
                if g.dicts.len() == g.subgraphs.len() + 1 {
                    sync_dicts(&mut g, graph, &self.forced);
                    agwrite::agwrite(&g)
                } else {
                    emit::output::propagate_inherited(&mut g, graph, &self.forced);
                    graphviz_pure_parser::write_canon_forced(&g, &self.forced)
                }
            }
            OutputFormat::Plain => {
                pre = emit_null(&m, DOT_FEATURES, "dot", name, opts);
                let zoom = emit::job::Job::new(&m, DOT_FEATURES, 0).zoom;
                emit::output::write_plain(
                    &m,
                    zoom,
                    name.eq_ignore_ascii_case("plain-ext"),
                    self.cgraph.as_ref(),
                )
            }
            OutputFormat::Canon => unreachable!(),
        };
        Ok(pre + &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{BezierSegment, BoundingBox, Point, Spline};
    use graphviz_pure_parser::parse_dot;

    // The model accepts only clusters below a graph from its exported `GD_clust` list,
    // once each below the root: a bad list from a caller that skipped layout cannot
    // make the cluster tree cyclic or multiply it.
    #[test]
    fn cluster_list_keeps_only_descendants() {
        let mut g = parse_dot(
            "digraph G { subgraph cluster_a { a subgraph cluster_c { c } } subgraph cluster_b { b } }",
        )
        .expect("parse");
        let idx = |name: &str| g.subgraphs.iter().position(|s| s.name == name).unwrap();
        let (a, c, b) = (idx("cluster_a"), idx("cluster_c"), idx("cluster_b"));
        g.bb = BoundingBox::new(0.0, 0.0, 100.0, 100.0);
        for s in &mut g.subgraphs {
            s.bb = BoundingBox::new(10.0, 10.0, 50.0, 50.0);
        }
        g.subgraphs[a].layout.clusters = Some(vec![a, c, c, b, 99]);
        let svg = render_graph(&g, OutputFormat::Svg).expect("render");
        let titles: Vec<&str> = svg
            .split("class=\"cluster\">\n<title>")
            .skip(1)
            .map(|s| &s[..s.find("</title>").unwrap()])
            .collect();
        assert_eq!(titles, ["cluster_a", "cluster_c", "cluster_b"]);
    }

    // Issue #324: the JSON writer names the attributes from the root's dictionaries; a graph
    // built through the API has none, and falls back to its defaults and objects' values.
    #[test]
    fn json_of_a_graph_without_dictionaries_names_its_values() {
        let mut g = Graph::new("G", graphviz_pure_core::GraphKind::Directed, false);
        let a = g.ensure_node("a");
        let b = g.ensure_node("b");
        let mut attrs = graphviz_pure_core::AttrMap::new();
        attrs.insert("label".into(), "ab".into());
        attrs.insert("color".into(), "red".into());
        g.add_edge(a, b, attrs);
        g.nodes[0].pos = Point::new(27.0, 90.0);
        g.nodes[1].pos = Point::new(27.0, 18.0);
        g.bb = BoundingBox::new(0.0, 0.0, 54.0, 108.0);
        assert!(g.dicts.is_empty());
        let json = render_graph(&g, OutputFormat::Json0).expect("render");
        for key in [
            "\"color\": \"red\"",
            "\"label\": \"ab\"",
            "\"pos\": \"27,90\"",
        ] {
            assert!(json.contains(key), "{key} in {json}");
        }

        // With dictionaries, a declared edge `label` no edge sets is written empty.
        let g = parse_dot("digraph { a -> b; {} -> c [label=x] }").expect("parse");
        let json = render_graph(&g, OutputFormat::Json0).expect("render");
        assert!(json.contains("\"label\": \"\""), "{json}");
    }

    fn sample_laid_out_graph() -> Graph {
        let mut g = parse_dot(
            r##"
            digraph G {
                label="Sample Graph";
                subgraph cluster_0 {
                    label="Cluster A";
                    style=filled;
                    fillcolor="#eeeeee";
                    a [label="Node A", shape=box, style="filled,rounded", fillcolor=lightblue];
                    b [label="{f0|f1|f2}", shape=record];
                }
                c [label=<<b>HTML</b><br/>Node>, shape=diamond];
                a -> b [label="ab"];
                b -> c [arrowhead=vee, style=dashed];
            }
            "##,
        )
        .expect("parse sample graph");

        g.bb = BoundingBox::new(0.0, 0.0, 220.0, 180.0);

        if let Some(sub) = g.subgraphs.first_mut() {
            sub.bb = BoundingBox::new(16.0, 64.0, 204.0, 168.0);
        }

        if let Some(a) = g.nodes.get_mut(0) {
            a.pos = Point::new(60.0, 110.0);
            a.width = 1.0;
            a.height = 0.5;
        }
        if let Some(b) = g.nodes.get_mut(1) {
            b.pos = Point::new(150.0, 110.0);
            b.width = 1.2;
            b.height = 0.5;
        }
        if let Some(c) = g.nodes.get_mut(2) {
            c.pos = Point::new(150.0, 36.0);
            c.width = 1.1;
            c.height = 0.6;
        }

        if let Some(e0) = g.edges.get_mut(0) {
            e0.spline = Spline {
                segments: vec![BezierSegment {
                    points: vec![
                        Point::new(96.0, 110.0),
                        Point::new(102.0, 110.0),
                        Point::new(105.0, 110.0),
                        Point::new(108.0, 110.0),
                    ],
                    start_arrow: None,
                    end_arrow: Some(Point::new(114.0, 110.0)),
                    sflag: 0,
                    eflag: graphviz_pure_core::arrows::ARR_TYPE_NORM,
                }],
            };
            e0.label_pos = Some(Point::new(105.0, 122.0));
        }

        if let Some(e1) = g.edges.get_mut(1) {
            e1.spline = Spline {
                segments: vec![BezierSegment {
                    points: vec![
                        Point::new(150.0, 92.0),
                        Point::new(150.0, 80.0),
                        Point::new(150.0, 70.0),
                        Point::new(150.0, 62.0),
                    ],
                    start_arrow: None,
                    end_arrow: Some(Point::new(150.0, 58.0)),
                    sflag: 0,
                    eflag: graphviz_pure_core::arrows::ARR_TYPE_NORM,
                }],
            };
        }

        g
    }

    #[test]
    fn svg_renderer_emits_graphviz_structure_shapes_and_edges() {
        let g = sample_laid_out_graph();
        let svg = render_graph(&g, OutputFormat::Svg).expect("render svg");

        assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>"));
        assert!(svg.contains("<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\""));
        assert!(svg.contains("<svg width=\"228pt\" height=\"188pt\""));
        assert!(svg.contains("viewBox=\"0.00 0.00 228.00 188.00\""));
        assert!(svg.contains(
            "<g id=\"graph0\" class=\"graph\" transform=\"scale(1 1) rotate(0) translate(4 184)\">"
        ));
        assert!(svg.contains("<g id=\"clust1\" class=\"cluster\">"));
        assert!(svg.contains("<g id=\"node1\" class=\"node\">"));
        assert!(svg.contains("<g id=\"node2\" class=\"node\">"));
        assert!(svg.contains("<g id=\"node3\" class=\"node\">"));
        assert!(svg.contains("<g id=\"edge1\" class=\"edge\">"));
        assert!(svg.contains("<g id=\"edge2\" class=\"edge\">"));
        assert!(svg.contains("<title>a&#45;&gt;b</title>"));
        assert!(svg.contains("<path fill=\"none\""));
        assert!(svg.contains("font-weight=\"bold\""));
    }

    #[test]
    fn json_and_json0_render_graphviz_schema() {
        let g = sample_laid_out_graph();
        let json0_str = render_graph(&g, OutputFormat::Json0).expect("render json0");
        let v0: serde_json::Value = serde_json::from_str(&json0_str).expect("valid json0");
        assert_eq!(v0["name"], "G");
        assert_eq!(v0["directed"], true);
        assert_eq!(v0["strict"], false);
        assert_eq!(v0["bb"], "0,0,220,180");
        assert_eq!(v0["_subgraph_cnt"], 1);
        assert!(v0["objects"].is_array());
        assert_eq!(v0["objects"].as_array().unwrap().len(), 4);
        assert!(v0["objects"][0].get("_draw_").is_none());
        assert_eq!(v0["edges"].as_array().unwrap().len(), 2);
        assert!(
            v0["edges"][0]["pos"]
                .as_str()
                .unwrap()
                .starts_with("e,114,110 ")
        );

        let json_str = render_graph(&g, OutputFormat::Json).expect("render json");
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid json");
        assert!(v["_draw_"].is_array());
        assert!(v["objects"][0]["_draw_"].is_array());
        assert!(v["objects"][1]["_draw_"].is_array());
        assert!(v["edges"][0]["_draw_"].is_array());
        assert!(v["edges"][0]["_hdraw_"].is_array());
    }

    // Issue #362: `xdot_begin_graph` reads the `xdotversion` an earlier xdot job set.
    #[test]
    fn a_later_xdot_job_keeps_the_version_an_earlier_one_set() {
        let g = sample_laid_out_graph();
        let mut jobs = GraphJobs::new(&g);
        let first = jobs.render(OutputFormat::Xdot, "xdot1.2").unwrap();
        let second = jobs.render(OutputFormat::Xdot, "xdot").unwrap();
        assert!(first.contains("xdotversion=1.2"), "{first}");
        assert!(second.contains("xdotversion=1.2"), "{second}");
        let json = jobs.render(OutputFormat::Json, "json").unwrap();
        assert!(json.contains("\"xdotversion\": \"1.2\""), "{json}");
        let alone = GraphJobs::new(&g)
            .render(OutputFormat::Xdot, "xdot")
            .unwrap();
        assert!(alone.contains("xdotversion=1.7"), "{alone}");
    }

    // Issue #367: what an earlier job set with `agxset` replaces the input's value of the
    // attribute for every later job, also where the input declares it.
    #[test]
    fn an_earlier_jobs_draw_wins_over_the_declared_input_value() {
        let mut g = sample_laid_out_graph();
        g.graph_attrs.set("_draw_", String::new(), false);
        let root_draw = |out: &str| {
            let at = out.find("graph [_draw_=\"").expect(out) + "graph [_draw_=\"".len();
            out[at..at + out[at..].find('"').unwrap()].to_string()
        };
        for (first, first_name) in [(OutputFormat::Xdot, "xdot"), (OutputFormat::Json, "json")] {
            let mut jobs = GraphJobs::new(&g);
            jobs.render(first, first_name).unwrap();
            let xdot = GraphJobs::new(&g)
                .render(OutputFormat::Xdot, "xdot")
                .unwrap();
            for (fmt, name) in [(OutputFormat::Dot, "dot"), (OutputFormat::Canon, "canon")] {
                let out = jobs.render(fmt, name).unwrap();
                assert!(!root_draw(&xdot).is_empty());
                assert_eq!(
                    root_draw(&out),
                    root_draw(&xdot),
                    "{name} after {first_name}: {out}"
                );
                // the cluster's `_draw_`, which the input does not declare on it
                assert_eq!(
                    out.matches("_draw_=").count(),
                    xdot.matches("_draw_=").count()
                );
            }
        }
    }

    #[test]
    fn dot_xdot_and_plain_outputs_include_layout_positions() {
        let g = sample_laid_out_graph();

        let dot = render_graph(&g, OutputFormat::Dot).expect("render dot");
        assert!(dot.contains("bb=\"0,0,220,180\""));
        assert!(dot.contains("pos=\"60,110\""));
        assert!(dot.contains("e,114,110 96,110 102,110 105,110 108,110"));
        assert!(!dot.contains("_draw_"));

        let xdot = render_graph(&g, OutputFormat::Xdot).expect("render xdot");
        assert!(xdot.contains("xdotversion=1.7"));
        assert!(xdot.contains("_draw_="));
        assert!(xdot.contains("_ldraw_="));
        assert!(xdot.contains("_hdraw_="));

        let plain = render_graph(&g, OutputFormat::Plain).expect("render plain");
        assert!(plain.starts_with("graph 1 "));
        assert!(plain.contains("\nnode a "));
        assert!(plain.contains("\nnode b "));
        assert!(plain.contains("\nedge a b 4 "));
        assert!(plain.ends_with("stop\n"));
    }
}
