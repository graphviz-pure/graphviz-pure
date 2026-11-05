//! The entry points for text already in the port's decoded form
//! ([`charset`]), which a [`Decoded`] holds: file bytes read with [`Decoded::decode`], or
//! text converted with [`Decoded::from_text`]. Engine and format names, [`GlobalAttrs`] values
//! and a [`Graph`]'s contents are decoded too, and so is what these functions return, errors
//! included: [`charset::encode`] gives the bytes C writes, [`charset::to_text`] ordinary text.
//! The command line and the wasm bindings' byte entry point use these, so nothing is
//! converted twice.
//!
//! [`charset`]: graphviz_pure_core::charset
//! [`charset::encode`]: graphviz_pure_core::charset::encode
//! [`charset::to_text`]: graphviz_pure_core::charset::to_text

use crate::OutputFormat;
use crate::plugins::{LAYOUT_PLUGINS, Plugins, layout_select, plugin_list};
use graphviz_pure_core::{Graph, GraphvizError, Result};
use graphviz_pure_layout::cutil::{RandState, in_rand_run, with_rand_state};
use graphviz_pure_layout::{LayoutEngine, LayoutOptions};
use graphviz_pure_parser::GlobalAttrs;

pub use crate::{GraphJobs, render_graph, render_graph_named};
pub use graphviz_pure_core::charset::Decoded;
pub use graphviz_pure_layout::{layout_graph, layout_graph_in_run, layout_graph_with_options};
/// The graphs of a decoded input one at a time ([`graph_stream`]).
pub use graphviz_pure_parser::GraphStream;

/// `graphviz_pure::render_dot` on decoded text: renders the first graph in `dot_source`.
pub fn render_dot(dot_source: &Decoded, engine: &str, format: &str) -> Result<String> {
    let output_format = OutputFormat::parse(format)?;
    // Each call is one C process: per-process counters (gradient and anchor ids) restart.
    crate::reset_render_state();
    let graph = parse_dot(dot_source)?;
    render_parsed(
        graph,
        engine,
        output_format,
        format,
        &mut LayoutOptions::default(),
    )
}

/// `graphviz_pure::render_dot_all` on decoded text.
pub fn render_dot_all(
    dot_source: &Decoded,
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

/// `graphviz_pure::render_dot_all_with_options` on decoded text.
pub fn render_dot_all_with_options(
    dot_source: &Decoded,
    engine: &str,
    format: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> Result<Vec<String>> {
    let output_format = OutputFormat::parse(format)?;
    crate::reset_render_state();
    let graphs = parse_dot_all(dot_source, globals)?;
    // One call is one C process: its graphs share the C library's generator states (the
    // caller's, inside a `with_rand_state`) and `Nop`.
    let mut options = options;
    let mut ps = crate::PostscriptJob::default();
    ps.set_format(ps_format(output_format));
    in_rand_run(|| {
        let mut out: Vec<String> = graphs
            .into_iter()
            .map(|graph| render_one(graph, engine, output_format, format, &mut options, &mut ps))
            .collect::<Result<_>>()?;
        if let Some(last) = out.last_mut() {
            last.push_str(&ps.finish());
        }
        Ok(out)
    })
}

/// One graph of a multi-graph run. The `-Tps` family shares `ps` across the graphs:
/// the paging devices (`ps`, `ps2`) keep one document whose trailer the caller appends
/// after the last graph, while `eps` ends its job after every graph, so each graph's
/// string is a complete document of its own. Concatenated, the strings are C's output.
fn render_one(
    graph: Graph,
    engine: &str,
    output_format: OutputFormat,
    format: &str,
    options: &mut LayoutOptions,
    ps: &mut crate::PostscriptJob,
) -> Result<String> {
    if matches!(
        output_format,
        OutputFormat::Ps | OutputFormat::Ps2 | OutputFormat::Eps
    ) {
        let text = render_parsed_ps(graph, engine, options, ps)?;
        if output_format == OutputFormat::Eps {
            // `eps:ps` has no `GVDEVICE_DOES_PAGES`, so C ends its job after every graph:
            // each graph is a complete document of its own.
            return Ok(text + &ps.finish());
        }
        Ok(text)
    } else {
        render_parsed(graph, engine, output_format, format, options)
    }
}

/// The device of the `-Tps` family that `format` selects.
fn ps_format(output_format: OutputFormat) -> crate::PsFormat {
    match output_format {
        OutputFormat::Ps2 => crate::PsFormat::Ps2,
        OutputFormat::Eps => crate::PsFormat::Eps,
        _ => crate::PsFormat::Ps,
    }
}

/// `graphviz_pure::render_dot_all_partial` on decoded text.
pub fn render_dot_all_partial(
    dot_source: &Decoded,
    engine: &str,
    format: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> (Vec<String>, Option<GraphvizError>) {
    let output_format = match OutputFormat::parse(format) {
        Ok(f) => f,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let mut ps = crate::PostscriptJob::default();
    ps.set_format(ps_format(output_format));
    let (mut out, err) = render_partial(dot_source, globals, options, |graph, options| {
        render_one(graph, engine, output_format, format, options, &mut ps)
    });
    if let Some(last) = out.last_mut() {
        last.push_str(&ps.finish());
    }
    (out, err)
}

/// `graphviz_pure::render_svg_all_partial` on decoded text.
pub fn render_svg_all_partial(
    dot_source: &Decoded,
    engine: &str,
    globals: &GlobalAttrs,
    options: LayoutOptions,
) -> (Vec<(String, bool)>, Option<GraphvizError>) {
    render_partial(dot_source, globals, options, |graph, options| {
        let explicit_dpi = graphviz_pure_core::cnum::drawing_dpi(&graph.graph_attrs) > 0.0;
        render_parsed(graph, engine, OutputFormat::Svg, "svg", options).map(|s| (s, explicit_dpi))
    })
}

/// Renders with `render` each graph read before a syntax error, in one run, and returns the
/// outputs with the first error (a render error before the syntax error).
fn render_partial<T>(
    dot_source: &Decoded,
    globals: &GlobalAttrs,
    options: LayoutOptions,
    mut render: impl FnMut(Graph, &mut LayoutOptions) -> Result<T>,
) -> (Vec<T>, Option<GraphvizError>) {
    crate::reset_render_state();
    let (graphs, parse_err) =
        graphviz_pure_parser::parse_dot_all_partial(dot_source.as_str(), globals);
    let mut out = Vec::with_capacity(graphs.len());
    // One call is one C process: its graphs share the C library's generator states (the
    // caller's, inside a `with_rand_state`) and `Nop`.
    let mut options = options;
    let err = in_rand_run(|| {
        for graph in graphs {
            match render(graph, &mut options) {
                // Not in C: a graph a limit stopped is not returned, nor any after it.
                Ok(_) if graphviz_pure_core::limits::stopped() => {
                    return graphviz_pure_core::limits::error();
                }
                Ok(s) => out.push(s),
                Err(e) => return Some(e),
            }
        }
        None
    });
    (out, err.or(parse_err))
}

/// A [`GraphStream`] over `dot_source`, applying `globals` before each graph body.
pub fn graph_stream<'a>(dot_source: &'a Decoded, globals: &GlobalAttrs) -> GraphStream<'a> {
    GraphStream::new(dot_source.as_str(), globals)
}

/// `graphviz_pure::parse_dot` on decoded text.
pub fn parse_dot(dot_source: &Decoded) -> Result<Graph> {
    graphviz_pure_parser::parse_dot(dot_source.as_str())
}

/// `graphviz_pure::parse_dot_all` on decoded text.
pub fn parse_dot_all(dot_source: &Decoded, globals: &GlobalAttrs) -> Result<Vec<Graph>> {
    graphviz_pure_parser::parse_dot_all(dot_source.as_str(), globals)
}

/// `gvplugin_graph`: the graph `dot -P` lays out and renders instead of its inputs, on the
/// `-G`, `-N`, `-E` and `-A` attributes `globals` (see [`crate::plugins`]).
pub fn plugin_graph(plugins: &Plugins<'_>, globals: &GlobalAttrs) -> Option<Graph> {
    crate::plugins::plugin_graph(plugins, globals)
}

fn render_parsed(
    mut graph: Graph,
    engine: &str,
    output_format: OutputFormat,
    format_name: &str,
    options: &mut LayoutOptions,
) -> Result<String> {
    let engine = LayoutEngine::parse(engine)?;
    // As in C (`gvLayoutJobs`), a graph's `layout` attribute overrides the requested engine.
    let layout_engine = match graph.graph_attrs.get("layout") {
        Some(name) if !name.trim().is_empty() => LayoutEngine::parse(name.trim())?,
        _ => engine,
    };
    // `dot` lays the graph out for every format, `canon` and the json formats without
    // layout attributes included: they write the graph as the layout left it.
    layout_graph_in_run(&mut graph, layout_engine, options)?;
    render_graph_named(&graph, output_format, format_name)
}

/// gvc's layout state for a run of `dot`: the engine selected (`gvc->layout`), which a
/// graph's `layout` attribute replaces for that graph and the ones after it, and the C
/// library's generator states, which carry over from one graph's layout to the next as they
/// do in the C process ([`graphviz_pure_layout::cutil`]).
#[derive(Debug, Clone)]
pub struct LayoutJobs {
    engine: Option<LayoutEngine>,
    options: LayoutOptions,
    rand: RandState,
}

impl LayoutJobs {
    /// The state after `-K` (or the default engine) selected `engine`.
    pub fn new(engine: LayoutEngine, options: LayoutOptions) -> Self {
        LayoutJobs {
            engine: Some(engine),
            options,
            rand: RandState::default(),
        }
    }

    /// [`LayoutJobs::new`] whose layouts continue from the generator state `rand`.
    pub fn continuing(engine: LayoutEngine, options: LayoutOptions, rand: RandState) -> Self {
        LayoutJobs {
            rand,
            ..Self::new(engine, options)
        }
    }

    /// The generator state the layouts so far left.
    pub fn rand_state(&self) -> RandState {
        self.rand
    }

    /// `gvLayoutJobs`: lays `graph` out. `Ok(false)` when no layout was done (an unknown
    /// `layout` reported here, or one reported for an earlier graph), which `gvRenderJobs`
    /// then reports as `Layout was not done`.
    pub fn layout(&mut self, graph: &mut Graph) -> Result<bool> {
        if let Some(name) = root_attr(graph, "layout") {
            self.engine = layout_select(name);
            if self.engine.is_none() {
                graphviz_pure_core::diag::agerrorf(&format!(
                    "Layout type: \"{name}\" not recognized. Use one of:{}\n",
                    plugin_list(&LAYOUT_PLUGINS, name)
                ));
                return Ok(false);
            }
        }
        let Some(engine) = self.engine else {
            return Ok(false);
        };
        // `Nop` is the run's, as C's global: a `nop` engine or fdp's splines clear it.
        let options = &mut self.options;
        with_rand_state(&mut self.rand, || {
            layout_graph_in_run(graph, engine, options)
        })?;
        Ok(true)
    }
}

/// `agget(g, name)` on the root graph: its value, the declared default, or `None` when the
/// attribute is not declared.
fn root_attr<'g>(graph: &'g Graph, name: &str) -> Option<&'g str> {
    graph.graph_attrs.get(name).map(String::as_str).or_else(|| {
        graph
            .dicts
            .first()
            .and_then(|d| d.graph.get(name))
            .map(|sym| sym.defval.as_str())
    })
}

/// C: lib/gvc/gvlayout.c:gvLayoutJobs; lib/gvc/gvrender.c:gvrender_begin_job
fn render_parsed_ps(
    mut graph: Graph,
    engine: &str,
    options: &mut LayoutOptions,
    ps: &mut crate::PostscriptJob,
) -> Result<String> {
    let engine = LayoutEngine::parse(engine)?;
    let engine = match graph.graph_attrs.get("layout") {
        Some(name) if !name.trim().is_empty() => LayoutEngine::parse(name.trim())?,
        _ => engine,
    };
    layout_graph_in_run(&mut graph, engine, options)?;
    ps.render(&graph, 0)
}
