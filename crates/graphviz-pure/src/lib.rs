//! High-level Graphviz orchestration engine (`graphviz-pure`).
//!
//! The functions here take and return ordinary text: DOT source, engine and format names,
//! [`GlobalAttrs`] values, outputs and error messages. The port reads every string in a
//! decoded form ([`charset`]), where U+10FF00..=U+10FFFF stands for an input byte that is not
//! UTF-8; these functions convert at the boundary, so a real U+10FFxx character in the source
//! is that character, as its UTF-8 bytes are to C.
//!
//! Output that is not valid UTF-8 reaches the caller with U+FFFD in place of the bytes that
//! aren't ([`charset::to_text`]); C writes them as they are. Some writers cut a value at a byte
//! count (a `%.80s` excerpt in a message) or split it at a byte (a multi-byte `layersep`).
//!
//! A [`Graph`] is an opaque handle. Parse it, lay it out and render it. Read its structure
//! from the `json` and `json0` output.

use std::borrow::Cow;

pub use graphviz_pure_core::{GraphvizError, Result, charset};
pub use graphviz_pure_layout::{LayoutEngine, LayoutOptions};
pub use graphviz_pure_parser::GlobalAttrs;
pub use graphviz_pure_render::OutputFormat;

use graphviz_pure_core::charset::Decoded;
use graphviz_pure_render::decoded;

/// A parsed graph. It has no public fields: parse it, lay it out and render it. Its
/// structure is the `json` and `json0` output.
#[derive(Clone)]
pub struct Graph {
    inner: graphviz_pure_core::Graph,
}

impl Graph {
    fn from_inner(inner: graphviz_pure_core::Graph) -> Self {
        Self { inner }
    }
}

/// End-to-end DOT rendering pipeline for any supported engine and output format. Renders the
/// first graph in `dot_source`.
pub fn render_dot(dot_source: &str, engine: &str, format: &str) -> Result<String> {
    decoded::render_dot(
        &Decoded::from_text(dot_source),
        &charset::from_text(engine),
        &charset::from_text(format),
    )
    .map(text)
    .map_err(error_text)
}

/// Renders every graph in `dot_source`, applying `globals` (`-G`/`-N`/`-E`) before each graph body.
pub fn render_dot_all(
    dot_source: &str,
    engine: &str,
    format: &str,
    globals: &GlobalAttrs,
) -> Result<Vec<String>> {
    render_dot_all_with_options(
        dot_source,
        engine,
        format,
        globals,
        LayoutOptions::default(),
    )
}

/// [`render_dot_all`] with engine options such as `-n` (keep input positions).
pub fn render_dot_all_with_options(
    dot_source: &str,
    engine: &str,
    format: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> Result<Vec<String>> {
    decoded::render_dot_all_with_options(
        &Decoded::from_text(dot_source),
        &charset::from_text(engine),
        &charset::from_text(format),
        &globals_from_text(globals),
        options,
    )
    .map(|outs| outs.into_iter().map(text).collect())
    .map_err(error_text)
}

/// [`render_dot_all_with_options`] that renders the graphs read before a syntax error
/// and returns that error alongside, as `dot` does.
pub fn render_dot_all_partial(
    dot_source: &str,
    engine: &str,
    format: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> (Vec<String>, Option<GraphvizError>) {
    let (outs, err) = decoded::render_dot_all_partial(
        &Decoded::from_text(dot_source),
        &charset::from_text(engine),
        &charset::from_text(format),
        &globals_from_text(globals),
        options,
    );
    (outs.into_iter().map(text).collect(), err.map(error_text))
}

/// Lays out and renders every graph in `dot_source` as SVG, for raster output: each SVG comes
/// with whether its graph sets `dpi` (or `resolution`), since Graphviz then scales the drawing
/// to that resolution instead of drawing raster formats at 96 dpi. Like
/// [`render_dot_all_partial`], the graphs read before a syntax error are returned with it.
pub fn render_svg_all_partial(
    dot_source: &str,
    engine: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> (Vec<(String, bool)>, Option<GraphvizError>) {
    let (outs, err) = decoded::render_svg_all_partial(
        &Decoded::from_text(dot_source),
        &charset::from_text(engine),
        &globals_from_text(globals),
        options,
    );
    let outs = outs
        .into_iter()
        .map(|(svg, dpi)| (text(svg), dpi))
        .collect();
    (outs, err.map(error_text))
}

/// Parses the first graph in `dot_source`.
pub fn parse_dot(dot_source: &str) -> Result<Graph> {
    decoded::parse_dot(&Decoded::from_text(dot_source))
        .map(Graph::from_inner)
        .map_err(error_text)
}

/// Parses every graph in `dot_source`, applying `globals` before each graph body.
pub fn parse_dot_all(dot_source: &str, globals: &GlobalAttrs) -> Result<Vec<Graph>> {
    decoded::parse_dot_all(&Decoded::from_text(dot_source), &globals_from_text(globals))
        .map(|graphs| graphs.into_iter().map(Graph::from_inner).collect())
        .map_err(error_text)
}

/// Lays `graph` out with `engine`.
pub fn layout_graph(graph: &mut Graph, engine: LayoutEngine) -> Result<()> {
    graphviz_pure_layout::layout_graph(&mut graph.inner, engine).map_err(error_text)
}

/// [`layout_graph`] with engine options such as `-n` (keep input positions).
pub fn layout_graph_with_options(
    graph: &mut Graph,
    engine: LayoutEngine,
    options: LayoutOptions,
) -> Result<()> {
    graphviz_pure_layout::layout_graph_with_options(&mut graph.inner, engine, options)
        .map_err(error_text)
}

/// [`layout_graph_with_options`] inside a run of several layouts, which share the generator
/// states and `Nop`.
pub fn layout_graph_in_run(
    graph: &mut Graph,
    engine: LayoutEngine,
    options: &mut LayoutOptions,
) -> Result<()> {
    graphviz_pure_layout::layout_graph_in_run(&mut graph.inner, engine, options).map_err(error_text)
}

/// Renders a laid-out graph in `format`.
pub fn render_graph(graph: &Graph, format: OutputFormat) -> Result<String> {
    graphviz_pure_render::render_graph(&graph.inner, format)
        .map(text)
        .map_err(error_text)
}

/// [`render_graph`] for the format named `name` (an alias such as `gv` for `dot`), which the
/// render diagnostics print.
pub fn render_graph_named(graph: &Graph, format: OutputFormat, name: &str) -> Result<String> {
    graphviz_pure_render::render_graph_named(&graph.inner, format, &charset::from_text(name))
        .map(text)
        .map_err(error_text)
}

/// `globals` in decoded form.
fn globals_from_text(globals: &GlobalAttrs) -> GlobalAttrs {
    globals.map_strings(charset::from_text)
}

/// Decoded output as text for the caller.
fn text(s: String) -> String {
    match charset::to_text(&s) {
        Cow::Borrowed(_) => s,
        Cow::Owned(t) => t,
    }
}

/// An error whose messages are text for the caller.
fn error_text(e: GraphvizError) -> GraphvizError {
    use GraphvizError::*;
    match e {
        SyntaxError {
            line,
            col,
            message,
            file,
        } => SyntaxError {
            line,
            col,
            message: text(message),
            file: file.map(text),
        },
        UnsupportedEngine(s) => UnsupportedEngine(text(s)),
        UnsupportedFormat(s) => UnsupportedFormat(text(s)),
        LayoutError { stage, message } => LayoutError {
            stage,
            message: text(message),
        },
        AttributeError { attr, message } => AttributeError {
            attr: text(attr),
            message: text(message),
        },
        RenderError(s) => RenderError(text(s)),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn repeated_renders_are_identical() {
        // Gradient and HTML anchor ids are per-process statics in C; a library call must
        // not see the counts left by an earlier call.
        let src = r#"digraph { a [style=filled, fillcolor="red:blue"];
            b [label=<<TABLE><TR><TD HREF="x">c</TD></TR></TABLE>>]; a -> b }"#;
        let first = render_dot(src, "dot", "svg").unwrap();
        assert_eq!(first, render_dot(src, "dot", "svg").unwrap());
        assert!(first.contains("_l_0"));
    }

    use super::*;
    use graphviz_pure_layout::cutil::{RandState, with_rand_state};
    use graphviz_pure_render::decoded::{self, Decoded, GraphJobs, LayoutJobs};
    use std::fs;
    use std::path::PathBuf;

    // Issue #290: the graphs of one call share the C library's generator state, as in one C
    // process; each call (and each layout on its own) starts fresh, so results don't depend
    // on earlier calls; a caller can carry a state across calls through `LayoutJobs`.
    #[test]
    fn one_call_is_one_process_for_the_random_state() {
        let g = "graph { dim=3; a [pos=\"10,20,5\" z=\"3\"]; b [pos=\"30,40\"]; a -- b }\n";
        let opts = LayoutOptions {
            nop: 1,
            ..LayoutOptions::default()
        };
        let both = render_dot_all_with_options(
            &g.repeat(2),
            "neato",
            "dot",
            &GlobalAttrs::default(),
            opts,
        )
        .unwrap();
        assert!(both[0].contains("z=57.090927") && both[1].contains("z=121.029893"));
        let again = render_dot_all_with_options(
            &g.repeat(2),
            "neato",
            "dot",
            &GlobalAttrs::default(),
            opts,
        )
        .unwrap();
        assert_eq!(both, again);
        // Inside a caller's run, every entry point continues the caller's state and leaves it
        // advanced.
        let mut state = RandState::default();
        let outer = with_rand_state(&mut state, || {
            render_dot_all_with_options(&g.repeat(2), "neato", "dot", &GlobalAttrs::default(), opts)
        })
        .unwrap();
        assert_eq!(outer, both);
        assert_ne!(state, RandState::default());
        let (third, _) = with_rand_state(&mut state, || {
            render_dot_all_partial(g, "neato", "dot", &GlobalAttrs::default(), opts)
        });
        assert!(!third[0].contains("z=57.090927") && !third[0].contains("z=121.029893"));
        let (svgs, _) = with_rand_state(&mut RandState::default(), || {
            render_svg_all_partial(&g.repeat(2), "neato", &GlobalAttrs::default(), opts)
        });
        assert_eq!(svgs.len(), 2);

        let mut jobs = LayoutJobs::new(LayoutEngine::Neato, opts);
        let mut first = parse_dot(g).unwrap();
        jobs.layout(&mut first.inner).unwrap();
        let mut graph = parse_dot(g).unwrap();
        let mut carried = LayoutJobs::continuing(LayoutEngine::Neato, opts, jobs.rand_state());
        carried.layout(&mut graph.inner).unwrap();
        let z = |g: &Graph| g.inner.nodes[1].attrs["z"].clone();
        assert_eq!(z(&graph), "121.029893");
        let mut alone = parse_dot(g).unwrap();
        layout_graph_with_options(&mut alone, LayoutEngine::Neato, opts).unwrap();
        assert_eq!(z(&alone), "57.090927");
    }

    // Issue #332: `Nop` belongs to the run, as C's global: after a `nop` engine
    // (`gvlayout_neato_layout.c:48-57`) or fdp with splines (`fdpgen/layout.c:1057`), `-n2`
    // no longer applies to the call's later graphs, nor to a `LayoutJobs`' later layouts.
    #[test]
    fn nop_engines_clear_the_runs_nop() {
        let opts = LayoutOptions {
            nop: 2,
            ..LayoutOptions::default()
        };
        let b = "graph B { layout=neato; b; c; b -- c }\n";
        let laid_out = "node b 0.375 0.39777 0.75 0.5 b solid ellipse black lightgrey\n";
        for a in [
            "graph A { layout=nop2; a [pos=\"0,0\"] }\n",
            "graph A { layout=fdp; splines=true; a -- x }\n",
        ] {
            let src = format!("{a}{b}");
            let all =
                render_dot_all_with_options(&src, "neato", "plain", &GlobalAttrs::default(), opts)
                    .unwrap();
            assert!(all[1].contains(laid_out), "{}", all[1]);
            let (all, _) =
                render_dot_all_partial(&src, "neato", "plain", &GlobalAttrs::default(), opts);
            assert!(all[1].contains(laid_out), "{}", all[1]);
            let (svgs, _) = render_svg_all_partial(&src, "neato", &GlobalAttrs::default(), opts);
            let (alone, _) = render_svg_all_partial(
                b,
                "neato",
                &GlobalAttrs::default(),
                LayoutOptions::default(),
            );
            assert_eq!(svgs[1], alone[0]);

            let mut jobs = LayoutJobs::new(LayoutEngine::Neato, opts);
            let mut parsed = parse_dot(a).unwrap();
            jobs.layout(&mut parsed.inner).unwrap();
            let mut graph = parse_dot(b).unwrap();
            jobs.layout(&mut graph.inner).unwrap();
            assert!(graph.inner.nodes[0].pos != graph.inner.nodes[1].pos);
        }
        // fdp with line edges leaves `Nop`.
        let src = format!("graph A {{ layout=fdp; a -- x }}\n{b}");
        let (all, _) = graphviz_pure_core::diag::quietly(|| {
            render_dot_all_partial(&src, "neato", "plain", &GlobalAttrs::default(), opts)
        });
        assert!(all[1].contains("node b 0 0 "), "{}", all[1]);
    }

    #[test]
    fn smoke_render_svg() {
        let svg = render_dot("digraph G { a -> b; }", "dot", "svg").unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn render_dot_all_renders_every_graph_with_globals() {
        let mut globals = GlobalAttrs::default();
        globals.node.insert("shape".into(), "box".into());
        let outs = render_dot_all(
            "digraph G { a -> b }\ndigraph H { c -> d }",
            "dot",
            "svg",
            &globals,
        )
        .unwrap();
        assert_eq!(outs.len(), 2);
        assert!(outs[0].contains("<title>a</title>") && !outs[0].contains("<ellipse"));
        assert!(outs[1].contains("<title>c</title>"));
    }

    // Issue #378: the root functions take and return text. A real U+10FF41 is the four bytes
    // F4 8F BD 81 to C, which measures each and writes them unchanged (C 16.1.0, -Tplain). The
    // decoded entry points read the same string as the raw byte 0x41, `A`.
    #[test]
    fn text_entry_points_keep_a_real_raw_byte_range_char() {
        let src = "digraph { \"x\u{10FF41}y\" -> b [label=\"\u{10FF41}\"] }";
        let want = "graph 1 0.84307 1.7333\n\
             node x\u{10FF41}y 0.42153 1.4833 0.84307 0.5 x\u{10FF41}y solid ellipse black lightgrey\n\
             node b 0.42153 0.25 0.75 0.5 b solid ellipse black lightgrey\n\
             edge x\u{10FF41}y b 4 0.42153 1.2313 0.42153 1.0689 0.42153 0.84989 0.42153 0.66136 \
             \u{10FF41} 0.51876 0.86667 solid black\n\
             stop\n";
        assert_eq!(render_dot(src, "dot", "plain").unwrap(), want);
        // the command line's path: the file's bytes, decoded, rendered and encoded
        let file = Decoded::decode_slice(src.as_bytes());
        let out = decoded::render_dot(&file, "dot", "plain").unwrap();
        assert_eq!(charset::encode(&out), want.as_bytes());
        let raw = Decoded::from_decoded(src.to_string());
        let raw = decoded::render_dot(&raw, "dot", "plain").unwrap();
        assert!(charset::encode(&raw).starts_with(b"graph 1 0.76854 1.7333\nnode xAy "));
        let opts = LayoutOptions::default();
        let no_globals = GlobalAttrs::default();
        let all = render_dot_all_with_options(src, "dot", "plain", &no_globals, opts);
        assert_eq!(all.unwrap(), [want]);
        let (all, err) = render_dot_all_partial(src, "dot", "plain", &no_globals, opts);
        assert!(err.is_none());
        assert_eq!(all, [want]);
        let (svgs, err) = render_svg_all_partial(src, "dot", &no_globals, opts);
        assert!(err.is_none());
        assert!(svgs[0].0.contains(">x\u{10FF41}y</text>"), "{}", svgs[0].0);
        // globals are text too
        let mut globals = GlobalAttrs::default();
        globals.node.insert("label".into(), "\u{10FF41}".into());
        let out = render_dot_all("digraph { a }", "dot", "plain", &globals).unwrap();
        assert!(out[0].contains(" 0.5 \u{10FF41} solid "), "{}", out[0]);
        let (out, _) = render_dot_all_partial("digraph { a }", "dot", "plain", &globals, opts);
        assert!(out[0].contains(" 0.5 \u{10FF41} solid "), "{}", out[0]);
        let (svgs, _) = render_svg_all_partial("digraph { a }", "dot", &globals, opts);
        assert!(svgs[0].0.contains(">\u{10FF41}</text>"), "{}", svgs[0].0);
        let g = &parse_dot_all("digraph { a }", &globals).unwrap()[0];
        let canon = render_graph(g, OutputFormat::Canon).unwrap();
        assert!(canon.contains("\tnode [label=\u{10FF41}];\n"), "{canon}");
        // a parsed graph holds the decoded form, and renders back to text
        let mut g = parse_dot(src).unwrap();
        assert_eq!(
            charset::encode(&g.inner.nodes[0].name),
            "x\u{10FF41}y".as_bytes()
        );
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        let plain = render_graph_named(&g, OutputFormat::Plain, "plain").unwrap();
        assert_eq!(plain, want);
        // error messages are text: they quote the caller's names as given
        let err = render_dot(src, "x\u{10FF41}", "plain").unwrap_err();
        assert_eq!(err.to_string(), "Unsupported layout engine: 'x\u{10FF41}'");
        let src = "digraph { layout=\"x\u{10FF41}\"; a }";
        let err = render_dot(src, "dot", "plain").unwrap_err();
        assert_eq!(err.to_string(), "Unsupported layout engine: 'x\u{10FF41}'");
        let err = render_dot(src, "dot", "x\u{10FF41}").unwrap_err();
        assert_eq!(err.to_string(), "Unsupported output format: 'x\u{10FF41}'");
    }

    #[test]
    fn test_multigraph_and_duplicate_cluster_scopes() {
        // 1. Multi-graph DOT stream (`multi.gv`)
        let multi = "digraph G { a -> b; }\ndigraph H { c -> d; }\n";
        let svg = render_dot(multi, "dot", "svg").unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<title>a</title>"));

        // 2. Same cluster name in different parent scopes (`b7.gv`)
        let b7 = r#"
            digraph G {
                subgraph cluster_0 {
                    subgraph cluster_1 {}
                    a -> c;
                }
                subgraph cluster_1 {
                    y -> z;
                    y -> q;
                }
                y -> b;
            }
        "#;
        let g = parse_dot(b7).unwrap();
        assert_eq!(g.inner.subgraphs.len(), 3);
        let j0 = render_dot(b7, "dot", "json0").unwrap();
        assert!(j0.contains("\"_subgraph_cnt\": 3"));
    }

    #[test]
    fn test_wild_corpus_renders_without_error() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let wild_dir = manifest_dir.join("../../tests/wild");
        let mut entries: Vec<PathBuf> = fs::read_dir(&wild_dir)
            .expect("tests/wild directory must exist")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "gv"))
            .collect();
        entries.sort();
        assert!(
            entries.len() >= 30,
            "expected at least 30 wild corpus fixtures"
        );

        for path in entries {
            // read as the command line reads a file: nine fixtures are not UTF-8
            let src = Decoded::decode(fs::read(&path).unwrap());
            let svg = decoded::render_dot(&src, "dot", "svg")
                .unwrap_or_else(|e| panic!("Failed rendering SVG for {}: {e}", path.display()));
            assert!(svg.contains("<svg"), "Missing <svg in {}", path.display());
            assert!(
                !svg.contains("NaN") && !svg.contains("Infinity"),
                "Non-finite coordinate in {}",
                path.display()
            );

            let j0 = decoded::render_dot(&src, "dot", "json0")
                .unwrap_or_else(|e| panic!("Failed rendering json0 for {}: {e}", path.display()));
            assert!(j0.contains("\"bb\":"), "Missing bb in {}", path.display());
        }
    }

    /// A scratch directory with a 40x20 PNG header and an EPS file whose `%%BoundingBox` is
    /// 100x50, and the paths of the two.
    fn image_dir(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("gvp-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_png(&dir.join("a.png"), 40, 20);
        fs::write(dir.join("box.eps"), "%%BoundingBox: 10 20 110 70\n").unwrap();
        (dir.join("a.png"), dir.join("box.eps"))
    }

    fn write_png(path: &std::path::Path, w: u32, h: u32) {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend(w.to_be_bytes());
        png.extend(h.to_be_bytes());
        png.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        fs::write(path, png).unwrap();
    }

    /// #355: the raster job is C's cairo job. With no maps there is no object id, so no `id`
    /// on the groups, and cairo draws an aligned edge label as plain text at its point: no
    /// `textPath` along a `<path id="_p">`, which every edge would share. The device has no
    /// layers, so the whole graph is drawn once, and colour names resolve through X11
    /// (`green` is #00ff00, SVG's is #008000). The SVG job keeps C's SVG.
    #[test]
    fn raster_svg_is_drawn_as_cairo_draws() {
        let svg_and_raster = |src: &str| {
            let mut g = parse_dot(src).unwrap();
            layout_graph(&mut g, LayoutEngine::Dot).unwrap();
            let svg = GraphJobs::new(&g.inner)
                .render(OutputFormat::Svg, "svg")
                .unwrap();
            let raster = GraphJobs::new(&g.inner)
                .render_raster_svg_as(0, "png")
                .unwrap();
            (svg, raster)
        };
        let (svg, raster) = svg_and_raster(
            "digraph { a -> b [label=x labelaligned=true]; c -> d [label=y labelaligned=true] }",
        );
        assert!(svg.contains("<textPath xlink:href=\"#edge1_p\""), "{svg}");
        assert!(svg.contains("<g id=\"node1\" class=\"node\">"), "{svg}");
        assert!(
            !raster.contains("textPath") && !raster.contains("_p\""),
            "{raster}"
        );
        assert!(
            !raster.contains("<a ") && !raster.contains(" id=\""),
            "{raster}"
        );
        assert!(raster.contains("<g class=\"node\">"), "{raster}");

        let (svg, raster) = svg_and_raster(
            "digraph { layers=\"a:b\"; x [layer=a]; y [layer=b style=filled fillcolor=green]; x -> y }",
        );
        assert!(svg.contains("class=\"layer\""), "{svg}");
        assert!(svg.contains("fill=\"green\""), "{svg}");
        assert!(!raster.contains("class=\"layer\""), "{raster}");
        for node in [
            "<title>x</title>",
            "<title>y</title>",
            "<title>x&#45;&gt;y</title>",
        ] {
            assert_eq!(raster.matches(node).count(), 1, "{node}: {raster}");
        }
        assert!(raster.contains("fill=\"#00ff00\""), "{raster}");
    }

    /// `-Tdot` of the epsf graph `src` as the CLI writes it, with C's PostScript call, in a
    /// run of its own.
    fn epsf_run(src: &str) -> String {
        graphviz_pure_render::reset_render_state();
        let mut g = parse_dot(src).unwrap();
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        let mut jobs = GraphJobs::new(&g.inner);
        jobs.write_output_file_text(true);
        jobs.render(OutputFormat::Dot, "dot").unwrap()
    }

    // #377: where C writes an xdot key with no value (invalid JSON), the library keeps one
    // JSON document: the value is `""`. Only the CLI writes C's bytes.
    #[test]
    fn json_with_an_unparsable_xdot_value_parses() {
        let src = "digraph { a [_tldraw_=\"zz\"]; a -> b [_hdraw_=\"zz\"] }";
        for format in ["json", "xdot_json", "json0", "dot_json"] {
            let out = render_dot(src, "dot", format).unwrap();
            let doc: serde_json::Value =
                serde_json::from_str(&out).unwrap_or_else(|e| panic!("{format}: {e}\n{out}"));
            let nodes = doc["objects"].as_array().unwrap();
            let a = nodes.iter().find(|o| o["name"] == "a").unwrap();
            let want = if format.contains("json0") || format == "dot_json" {
                "zz"
            } else {
                ""
            };
            assert_eq!(a["_tldraw_"], want, "{format}");
        }
    }

    // #347 review: a call is one C process. The files it read and the epsf files it
    // numbered are its own, and another thread has its own runs.
    #[test]
    fn usershape_state_is_per_run() {
        let (png, eps) = image_dir("lib-runs");
        let src = format!("digraph {{ a [image=\"{}\" label=\"\"] }}", png.display());
        let first = render_dot(&src, "dot", "dot").unwrap();
        assert!(first.contains("height=0.5,"), "{first}");
        write_png(&png, 400, 200);
        let second = render_dot(&src, "dot", "dot").unwrap();
        assert!(second.contains("height=2.9856,"), "{second}");
        let src = format!(
            "digraph {{ e [shape=epsf shapefile=\"{}\"] }}",
            eps.display()
        );
        for _ in 0..2 {
            render_dot(&src, "dot", "dot").unwrap();
        }
        let call = "-10 -20 translate newpath user_shape_0\n";
        assert!(epsf_run(&src).starts_with(call));
        let other = std::thread::spawn(move || epsf_run(&src)).join().unwrap();
        assert!(other.starts_with(call), "{other}");
    }

    // #347 review: the renderer draws the images the layout found, from the graph, whatever
    // was laid out since and on whichever thread it draws.
    #[test]
    fn images_are_drawn_from_the_layout_result() {
        let (png, _) = image_dir("lib-render");
        let src = format!("digraph {{ a [image=\"{}\"] }}", png.display());
        let mut a = parse_dot(&src).unwrap();
        layout_graph(&mut a, LayoutEngine::Dot).unwrap();
        let mut b = parse_dot("digraph { imagepath=\"nowhere\"; x [image=\"none.png\"] }").unwrap();
        layout_graph(&mut b, LayoutEngine::Dot).unwrap();
        let image = format!("<image xlink:href=\"{}\" width=\"40px\"", png.display());
        let svg = render_graph(&a, OutputFormat::Svg).unwrap();
        assert!(svg.contains(&image), "{svg}");
        let svg = std::thread::spawn(move || render_graph(&a, OutputFormat::Svg).unwrap())
            .join()
            .unwrap();
        assert!(svg.contains(&image), "{svg}");
    }

    // #347 review: each job keeps its own loadimage plugin. The PostScript image comes first,
    // so neither svg job has a plugin for it yet, and neither draws it.
    #[test]
    fn jobs_keep_their_own_loadimage_plugin() {
        let (png, eps) = image_dir("lib-jobs");
        let src = format!(
            "digraph {{ a [image=\"{}\"]; b [image=\"{}\"] }}",
            eps.display(),
            png.display()
        );
        graphviz_pure_render::reset_render_state();
        let mut g = parse_dot(&src).unwrap();
        layout_graph(&mut g, LayoutEngine::Dot).unwrap();
        let mut jobs = GraphJobs::new(&g.inner);
        let first = jobs.render_as(0, OutputFormat::Svg, "svg").unwrap();
        let second = jobs.render_as(1, OutputFormat::Svg, "svg").unwrap();
        assert_eq!(first, second);
        assert_eq!(first.matches("<image ").count(), 1, "{first}");
    }

    // #347 review: the library's output is the writer's document: no PostScript call from
    // `epsf_gencode`, which C writes into its output file.
    #[test]
    fn epsf_output_is_a_document() {
        let (_, eps) = image_dir("lib-doc");
        let src = format!(
            "digraph {{ e [shape=epsf shapefile=\"{}\"] }}",
            eps.display()
        );
        let json = render_dot(&src, "dot", "json").unwrap();
        serde_json::from_str::<serde_json::Value>(&json).unwrap();
        assert!(
            render_dot(&src, "dot", "dot")
                .unwrap()
                .starts_with("digraph {")
        );
        assert!(!render_dot(&src, "dot", "svg").unwrap().contains("newpath"));
    }
}
