//! WebAssembly bindings (`wasm-bindgen`) for running `graphviz-pure` directly in the browser,
//! Node.js, Deno, Bun, and Cloudflare Workers.
//!
//! Failures throw the exported `GraphvizError` class (`extends Error`, with `message`,
//! `status`, `kind` and `diagnostics`), so `instanceof GraphvizError` holds. `status` is
//! currently always 1, the status C's `dot` exits with when one graph fails. `render`
//! returns the output string; `renderWithDiagnostics` returns `{output, diagnostics}`.
//! `supportedEngines` and `supportedFormats` return arrays. `inspectGraph` returns an
//! object whose fields stay snake_case (`width_pt`, `height_pt`). The hand-checked types
//! live in `web/graphviz_pure_wasm.d.ts` and replace the file `wasm-bindgen` generates.

use std::borrow::Cow;

use graphviz_pure_core::charset;
use graphviz_pure_core::{Diagnostic, DiagnosticLevel, GraphvizError, GraphvizErrorKind, Rendered};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Exit status of one failed graph. C's `dot` exits 1 for every kind this package
/// throws. `status` stays a number so a later release can use another code; today
/// every thrown error carries this value.
const FAILURE_STATUS: i32 = 1;

/// The JS object builders. They are the wasm target's glue: a native test that calls
/// them panics, so the tests assert [`thrown`] instead.
///
/// `GraphvizError` is constructed here. `scripts/export-graphviz-error.mjs` re-exports
/// that same class from the glue, which is what `instanceof GraphvizError` checks.
#[wasm_bindgen(inline_js = r#"
export class GraphvizError extends Error {
    constructor(message, status, kind, diagnostics) {
        super(message);
        this.name = "GraphvizError";
        this.status = status;
        this.kind = kind;
        this.diagnostics = diagnostics;
    }
}
export function graphviz_error(message, status, kind, diagnosticsJson) {
    return new GraphvizError(message, status, kind, JSON.parse(diagnosticsJson));
}
export function with_string(output, diagnosticsJson) {
    return { output: output, diagnostics: JSON.parse(diagnosticsJson) };
}
export function with_strings(output, diagnosticsJson) {
    return { output: output, diagnostics: JSON.parse(diagnosticsJson) };
}
export function with_bytes(output, diagnosticsJson) {
    return { output: output, diagnostics: JSON.parse(diagnosticsJson) };
}
export function graph_stats(json) {
    return JSON.parse(json);
}
"#)]
extern "C" {
    fn graphviz_error(message: &str, status: i32, kind: &str, diagnostics_json: &str) -> JsValue;
    fn with_string(output: String, diagnostics_json: &str) -> JsValue;
    fn with_strings(output: Vec<String>, diagnostics_json: &str) -> JsValue;
    fn with_bytes(output: Vec<u8>, diagnostics_json: &str) -> JsValue;
    fn graph_stats(json: &str) -> JsValue;
}

/// What a thrown error carries. `message` is [`GraphvizError`]'s `Display` (the text the
/// command line prints, without the program-name prefix), not the bare detail.
struct Thrown<'a> {
    message: String,
    status: i32,
    kind: &'static str,
    diagnostics: Vec<(&'static str, &'a str)>,
}

fn thrown(error: &GraphvizError) -> Thrown<'_> {
    Thrown {
        message: error.to_string(),
        status: FAILURE_STATUS,
        kind: kind_name(error.kind()),
        diagnostics: error
            .diagnostics()
            .iter()
            .map(|d| (level_name(d.level()), d.message()))
            .collect(),
    }
}

fn throw_js(error: &GraphvizError) -> JsValue {
    let view = thrown(error);
    graphviz_error(
        &view.message,
        view.status,
        view.kind,
        &diagnostics_json(&view.diagnostics),
    )
}

fn kind_name(kind: GraphvizErrorKind) -> &'static str {
    match kind {
        GraphvizErrorKind::Syntax => "Syntax",
        GraphvizErrorKind::UnsupportedEngine => "UnsupportedEngine",
        GraphvizErrorKind::UnsupportedFormat => "UnsupportedFormat",
        GraphvizErrorKind::Layout => "Layout",
        GraphvizErrorKind::Attribute => "Attribute",
        GraphvizErrorKind::Render => "Render",
        // `GraphvizErrorKind` is non-exhaustive. A kind this binding does not know yet.
        _ => "Error",
    }
}

fn level_name(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Error => "Error",
        DiagnosticLevel::Warning => "Warning",
        DiagnosticLevel::CountedAsError => "CountedAsError",
        // `DiagnosticLevel` is non-exhaustive.
        _ => "Error",
    }
}

fn diagnostics_json(diagnostics: &[(&str, &str)]) -> String {
    let list: Vec<_> = diagnostics
        .iter()
        .map(|(level, message)| serde_json::json!({ "level": level, "message": message }))
        .collect();
    serde_json::to_string(&list).expect("diagnostic messages are strings")
}

fn diagnostics_of(diagnostics: &[Diagnostic]) -> String {
    let pairs: Vec<(&str, &str)> = diagnostics
        .iter()
        .map(|d| (level_name(d.level()), d.message()))
        .collect();
    diagnostics_json(&pairs)
}

/// One `to_text` of a decoded string. The facade has already done this to its
/// `Display` and its diagnostics, and a second pass would turn a real
/// `U+10FF41` back into the byte it stands for.
fn text(s: String) -> String {
    match charset::to_text(&s) {
        Cow::Borrowed(_) => s,
        Cow::Owned(t) => t,
    }
}

fn rewrite(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|d| d.__map_message(text))
        .collect()
}

/// The decoded path's error and its captured messages, each converted once.
fn as_text(error: GraphvizError, diagnostics: Vec<Diagnostic>) -> GraphvizError {
    error.__set_diagnostics(diagnostics).__map_text(text)
}

/// [`render`] on the facade. The strings are already text.
fn render_text(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> Result<Rendered<String>, GraphvizError> {
    graphviz_pure::render_dot(dot_source, engine, format)
}

/// [`render_all`] on the facade.
fn render_all_text(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> Result<Rendered<Vec<String>>, GraphvizError> {
    let globals = graphviz_pure::GlobalAttrs::default();
    graphviz_pure::render_dot_all(dot_source, engine, format, &globals)
}

/// `renderAllBytes` on decoded input. Output bytes stay encoded. Diagnostics are
/// converted once here: this path does not go through the facade, because the
/// facade would decode the bytes again.
fn render_all_bytes_decoded(
    dot_source: &[u8],
    engine: &str,
    format: &str,
) -> Result<(Vec<u8>, Vec<Diagnostic>), GraphvizError> {
    let src = charset::Decoded::decode_slice(dot_source);
    let engine = charset::from_text(engine);
    let format = charset::from_text(format);
    let ((parts, err), diagnostics) = graphviz_pure_core::diag::capture(|| {
        graphviz_pure_render::decoded::render_dot_all_partial(
            &src,
            &engine,
            &format,
            &graphviz_pure_parser::GlobalAttrs::default(),
            graphviz_pure_layout::LayoutOptions::default(),
        )
    });
    match err {
        // Graphs read before a syntax error are still returned, as `dot` writes
        // them. The syntax error's own text is `Display`, not an `agerr` line, so
        // a partial success does not carry it. Nothing rendered: throw.
        Some(error) if parts.is_empty() => Err(as_text(error, diagnostics)),
        _ => Ok((
            charset::encode(&parts.concat()).into_owned(),
            rewrite(diagnostics),
        )),
    }
}

#[derive(Debug, Serialize)]
struct GraphStats {
    name: String,
    directed: bool,
    strict: bool,
    nodes: usize,
    edges: usize,
    subgraphs: usize,
    width_pt: f64,
    height_pt: f64,
}

/// [`inspect_graph`] without the JS object. Warnings are dropped: there is no
/// `inspectGraphWithDiagnostics`.
fn inspect_text(dot_source: &str, engine: &str) -> Result<GraphStats, GraphvizError> {
    let src = charset::from_text(dot_source);
    let engine = charset::from_text(engine);
    let (result, diagnostics) =
        graphviz_pure_core::diag::capture(|| -> Result<_, GraphvizError> {
            let layout_engine = graphviz_pure_layout::LayoutEngine::parse(&engine)?;
            let mut graph = graphviz_pure_parser::parse_dot(&src)?;
            graphviz_pure_layout::layout_graph(&mut graph, layout_engine)?;
            Ok(graph)
        });
    match result {
        Ok(graph) => Ok(GraphStats {
            name: text(graph.name),
            directed: graph.kind == graphviz_pure_core::GraphKind::Directed,
            strict: graph.strict,
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
            subgraphs: graph.subgraphs.len(),
            width_pt: graph.bb.width(),
            height_pt: graph.bb.height(),
        }),
        Err(error) => Err(as_text(error, diagnostics)),
    }
}

fn ok_string(rendered: Rendered<String>) -> JsValue {
    let (output, diagnostics) = rendered.into_parts();
    with_string(output, &diagnostics_of(&diagnostics))
}

fn ok_strings(rendered: Rendered<Vec<String>>) -> JsValue {
    let (output, diagnostics) = rendered.into_parts();
    with_strings(output, &diagnostics_of(&diagnostics))
}

/// Renders the first graph in `dot_source`.
///
/// Returns the output string. Warnings are not on this string;
/// [`render_with_diagnostics`] returns them. A layout that succeeds and still
/// emitted an error-level diagnostic does not throw: `dot` would exit 1, and
/// the diagnostic is on that object.
///
/// A failure throws the exported `GraphvizError` class. `message` is the text
/// the command line prints. `status` is currently always 1, the status C's
/// `dot` exits with when one graph fails. `kind` is `Syntax`,
/// `UnsupportedEngine`, `UnsupportedFormat`, `Layout`, `Attribute` or `Render`.
/// `diagnostics` is the `agerr` messages around the failure. The failure's own
/// text is `message`, not one of those entries.
///
/// `engine` is `dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage` or
/// `patchwork`. `format` is a name from [`supported_formats`].
#[wasm_bindgen]
pub fn render(dot_source: &str, engine: &str, format: &str) -> Result<String, JsValue> {
    match render_text(dot_source, engine, format) {
        Ok(rendered) => Ok(rendered.into_output()),
        Err(error) => Err(throw_js(&error)),
    }
}

/// [`render`] plus the diagnostics that call collected.
///
/// `{ output, diagnostics }`. `output` is the string [`render`] returns.
/// `diagnostics` is `{ level, message }` in order. `level` is `Error`,
/// `Warning` or `CountedAsError`. `message` is that one `agerr` write,
/// including its `Error: ` or `Warning: ` prefix and its trailing newline.
#[wasm_bindgen(js_name = renderWithDiagnostics)]
pub fn render_with_diagnostics(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> Result<JsValue, JsValue> {
    match render_text(dot_source, engine, format) {
        Ok(rendered) => Ok(ok_string(rendered)),
        Err(error) => Err(throw_js(&error)),
    }
}

/// Renders every graph in a DOT string, one output string per graph, as `dot`
/// does for a multi-graph file. Same failure object as [`render`].
#[wasm_bindgen(js_name = renderAll)]
pub fn render_all(dot_source: &str, engine: &str, format: &str) -> Result<Vec<String>, JsValue> {
    match render_all_text(dot_source, engine, format) {
        Ok(rendered) => Ok(rendered.into_output()),
        Err(error) => Err(throw_js(&error)),
    }
}

/// [`render_all`] plus the diagnostics of the whole call. `output` is the
/// array [`render_all`] returns.
#[wasm_bindgen(js_name = renderAllWithDiagnostics)]
pub fn render_all_with_diagnostics(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> Result<JsValue, JsValue> {
    match render_all_text(dot_source, engine, format) {
        Ok(rendered) => Ok(ok_strings(rendered)),
        Err(error) => Err(throw_js(&error)),
    }
}

/// [`render_all`] for a file's raw bytes, as `dot` reads them. Bytes that are
/// not UTF-8 (a `charset=latin1` file) are kept, and the result is the bytes
/// the command line writes, every graph's output concatenated. A JS string
/// cannot carry those bytes, so [`render_all`] cannot.
///
/// Graphs read before a syntax error are still returned. This throws only when
/// the error leaves nothing to render, and then with the same `GraphvizError`
/// as [`render`].
#[wasm_bindgen(js_name = renderAllBytes)]
pub fn render_all_bytes(dot_source: &[u8], engine: &str, format: &str) -> Result<Vec<u8>, JsValue> {
    match render_all_bytes_decoded(dot_source, engine, format) {
        Ok((bytes, _)) => Ok(bytes),
        Err(error) => Err(throw_js(&error)),
    }
}

/// [`render_all_bytes`] plus the diagnostics of the whole call. `output` is
/// the `Uint8Array` [`render_all_bytes`] returns. A partial success (some
/// graphs rendered, then a syntax error) carries the `agerr` messages, not the
/// syntax error's `message`.
#[wasm_bindgen(js_name = renderAllBytesWithDiagnostics)]
pub fn render_all_bytes_with_diagnostics(
    dot_source: &[u8],
    engine: &str,
    format: &str,
) -> Result<JsValue, JsValue> {
    match render_all_bytes_decoded(dot_source, engine, format) {
        Ok((bytes, diagnostics)) => Ok(with_bytes(bytes, &diagnostics_of(&diagnostics))),
        Err(error) => Err(throw_js(&error)),
    }
}

/// [`render`] with the format `svg`.
#[wasm_bindgen(js_name = renderSvg)]
pub fn render_svg(dot_source: &str, engine: &str) -> Result<String, JsValue> {
    render(dot_source, engine, "svg")
}

/// [`render`] with the format `json`.
#[wasm_bindgen(js_name = renderJson)]
pub fn render_json(dot_source: &str, engine: &str) -> Result<String, JsValue> {
    render(dot_source, engine, "json")
}

/// Parses and lays out a graph.
///
/// Returns `{name, directed, strict, nodes, edges, subgraphs, width_pt, height_pt}`.
/// The fields are snake_case, the names they had when this returned JSON. The
/// function names stay camelCase. Warnings are not returned.
#[wasm_bindgen(js_name = inspectGraph)]
pub fn inspect_graph(dot_source: &str, engine: &str) -> Result<JsValue, JsValue> {
    match inspect_text(dot_source, engine) {
        Ok(stats) => {
            let json = serde_json::to_string(&stats)
                .map_err(|e| throw_js(&GraphvizError::render(e.to_string())))?;
            Ok(graph_stats(&json))
        }
        Err(error) => Err(throw_js(&error)),
    }
}

/// The layout engines [`render`] accepts, in this order: `dot`, `neato`, `fdp`,
/// `sfdp`, `twopi`, `circo`, `osage`, `patchwork`.
#[wasm_bindgen(js_name = supportedEngines)]
pub fn supported_engines() -> Vec<String> {
    [
        "dot",
        "neato",
        "fdp",
        "sfdp",
        "twopi",
        "circo",
        "osage",
        "patchwork",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Every output format name [`render`] accepts, including aliases. Raster
/// formats (`png`, `jpg`, `jpeg`) are not in this package.
#[wasm_bindgen(js_name = supportedFormats)]
pub fn supported_formats() -> Vec<String> {
    graphviz_pure::OutputFormat::names()
        .map(str::to_string)
        .collect()
}

/// `graphviz version 16.1.0 (graphviz-pure X.Y.Z)`, where `X.Y.Z` is this
/// package's version. The CLI's `-V` line, without the command name.
#[wasm_bindgen]
pub fn version() -> String {
    concat!(
        "graphviz version 16.1.0 (graphviz-pure ",
        env!("CARGO_PKG_VERSION"),
        ")"
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_reports_the_package_version() {
        assert_eq!(
            version(),
            format!(
                "graphviz version 16.1.0 (graphviz-pure {})",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn engines_and_formats_are_the_lists_render_accepts() {
        assert_eq!(
            supported_engines(),
            [
                "dot",
                "neato",
                "fdp",
                "sfdp",
                "twopi",
                "circo",
                "osage",
                "patchwork",
            ]
        );
        let formats = supported_formats();
        assert_eq!(
            formats,
            [
                "svg",
                "svg_inline",
                "cmapx",
                "json",
                "json0",
                "dot",
                "xdot",
                "canon",
                "plain",
                "cmapx_np",
                "gv",
                "xdot1.2",
                "xdot1.4",
                "dot_json",
                "xdot_json",
                "plain-ext",
            ]
        );
        for format in formats {
            let out = render("digraph { a -> b }", "dot", &format).unwrap();
            assert!(!out.is_empty(), "{format}");
        }
    }

    #[test]
    fn a_failure_carries_status_kind_and_diagnostics() {
        let err = render_text("digraph {", "dot", "svg").unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "Syntax");
        assert!(view.message.contains("syntax error"), "{}", view.message);

        let err = render_text("digraph { a -> b }", "nope", "svg").unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "UnsupportedEngine");
        assert_eq!(view.message, "Unsupported layout engine: 'nope'");
        assert!(view.diagnostics.is_empty());

        let err = render_text("digraph { a -> b }", "dot", "png").unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.kind, "UnsupportedFormat");
        assert_eq!(view.status, 1);
        assert_eq!(view.message, "Unsupported output format: 'png'");
    }

    #[test]
    fn a_warning_stays_on_success_and_an_error_diagnostic_does_not_throw() {
        let rendered = render_text("digraph { a [shape=only_a] }", "dot", "svg").unwrap();
        assert!(rendered.output().contains("<svg"));
        assert_eq!(rendered.diagnostics()[0].level(), DiagnosticLevel::Warning);
        assert_eq!(
            rendered.diagnostics()[0].message(),
            "Warning: using box for unknown shape only_a\n"
        );
        let again = render_text("digraph { a [shape=only_a] }", "dot", "svg").unwrap();
        assert_eq!(
            again.diagnostics()[0].message(),
            rendered.diagnostics()[0].message()
        );

        // `dot` exits 1 for this and still writes the SVG. The library call
        // returns the SVG; the error diagnostic is not a throw.
        let rendered =
            render_text("digraph { b [shape=record,label=\"{a|\"] }", "dot", "svg").unwrap();
        assert!(rendered.output().contains("<svg"));
        assert!(rendered.diagnostics().iter().any(|d| {
            d.level() == DiagnosticLevel::Error && d.message().contains("bad label format")
        }));
    }

    #[test]
    fn inspect_returns_the_fields_and_drops_warnings() {
        let stats = inspect_text("digraph G { a -> b }", "dot").unwrap();
        assert_eq!(stats.name, "G");
        assert!(stats.directed);
        assert!(!stats.strict);
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.subgraphs, 0);
        assert!(stats.width_pt.is_finite() && stats.width_pt > 0.0);
        assert!(stats.height_pt.is_finite() && stats.height_pt > 0.0);

        let strict = inspect_text("strict graph { a -- b }", "dot").unwrap();
        assert!(strict.strict);
        assert!(!strict.directed);
        assert_eq!(strict.edges, 1);

        let warned = inspect_text("digraph { a [shape=only_a] }", "dot").unwrap();
        assert_eq!(warned.nodes, 1);
    }

    #[test]
    fn bytes_keep_a_partial_render_and_throw_only_when_nothing_was_rendered() {
        let (bytes, diagnostics) =
            render_all_bytes_decoded(b"digraph { a [shape=only_a] }\ndigraph {", "dot", "svg")
                .unwrap();
        assert!(bytes.windows(4).any(|w| w == b"<svg"));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message().contains("unknown shape only_a"))
        );

        let err = render_all_bytes_decoded(b"digraph {", "dot", "svg").unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "Syntax");

        let err = render_all_bytes_decoded(b"digraph { a }", "dot", "nope").unwrap_err();
        assert_eq!(thrown(&err).kind, "UnsupportedFormat");
    }

    // `Render` is raised by the raster crate, and this package does not rasterize.
    // `Attribute` has no constructor call outside the error type's own tests.
    #[test]
    fn a_layout_failure_throws_and_keeps_the_diagnostics_around_it() {
        let src = "digraph G {\n\
            rank=sink;\n\
            subgraph cluster0 { a; b; c; d }\n\
            subgraph cluster_1 { e; subgraph cluster_2 { f } }\n\
            subgraph cluster_3 { }\n\
            { d }\n\
            { e }\n\
            { }\n\
            f -> e;\n\
            g -> h [minlen=2];\n\
            d -> c;\n\
            i -> h [label=x];\n\
            subgraph s { rank=source; a; h; subgraph cluster_in { } }\n\
            }\n";
        let err = render_text(src, "dot", "svg").unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.kind, "Layout");
        assert_eq!(view.status, 1);
        assert!(
            view.message.contains("Layout computation error"),
            "{}",
            view.message
        );
        assert!(
            view.diagnostics
                .iter()
                .any(|(_, message)| !message.is_empty()),
            "{:?}",
            view.diagnostics
        );

        let err = render_all_text("digraph {", "dot", "svg").unwrap_err();
        assert_eq!(thrown(&err).kind, "Syntax");
        assert_eq!(thrown(&err).status, 1);
        let err = inspect_text("digraph {", "dot").unwrap_err();
        assert_eq!(thrown(&err).kind, "Syntax");
        let err = inspect_text("digraph { a -> b }", "nope").unwrap_err();
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        assert_eq!(thrown(&err).status, 1);
    }

    // A JS string is text, so a real U+10FF41 in it is not one of the port's raw
    // bytes: it goes in and comes back out as itself, in output and in errors.
    #[test]
    fn real_chars_of_the_raw_byte_range_cross_the_js_boundary() {
        let src = "digraph { \"x\u{10ff41}y\" -> b }";
        let out = render_text(src, "dot", "plain").unwrap().into_output();
        assert!(out.contains("node x\u{10ff41}y "), "{out}");
        assert!(!out.contains("xAy"), "{out}");
        let outs = render_all_text(&format!("{src}\n{src}"), "dot", "canon")
            .unwrap()
            .into_output();
        assert_eq!(outs.len(), 2);
        assert!(
            outs.iter().all(|o| o.contains("\tx\u{10ff41}y -> b;")),
            "{outs:?}"
        );

        let err = render_text(src, "x\u{10ff41}", "plain").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("x\u{10ff41}") && !message.contains("xA"),
            "{message}"
        );
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        let err = render_all_text(src, "dot", "x\u{10ff41}").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("x\u{10ff41}") && !message.contains("xA"),
            "{message}"
        );

        let stats = inspect_text("digraph \"x\u{10ff41}\" { a }", "dot").unwrap();
        assert_eq!(stats.name, "x\u{10ff41}");
        let err = inspect_text(src, "x\u{10ff41}").unwrap_err();
        assert_eq!(err.to_string(), "Unsupported layout engine: 'x\u{10ff41}'");
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        assert_eq!(
            charset::to_text(&charset::decode(b"caf\xe9".to_vec())),
            "caf\u{fffd}"
        );
    }
}
