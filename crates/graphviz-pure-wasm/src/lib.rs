//! WebAssembly bindings (`wasm-bindgen`) for running `graphviz-pure` directly in the browser,
//! Node.js, Deno, Bun, and Cloudflare Workers.

use graphviz_pure_core::charset;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// A decoded error's message as text for JS ([`charset::to_text`]).
fn error_text(e: impl std::fmt::Display) -> String {
    charset::to_text(&e.to_string()).into_owned()
}

/// A decoded error for JS ([`error_text`]).
fn js_error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error_text(e))
}

/// [`render`] without the `JsValue`: the root `graphviz_pure` functions take and return text.
fn render_text(dot_source: &str, engine: &str, format: &str) -> Result<String, String> {
    graphviz_pure::render_dot(dot_source, engine, format).map_err(|e| e.to_string())
}

/// [`render_all`] as [`render_text`] is [`render`].
fn render_all_text(dot_source: &str, engine: &str, format: &str) -> Result<Vec<String>, String> {
    let globals = graphviz_pure::GlobalAttrs::default();
    graphviz_pure::render_dot_all(dot_source, engine, format, &globals).map_err(|e| e.to_string())
}

#[derive(Serialize)]
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

/// Renders a DOT string using the specified layout engine (`dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage`, `patchwork`)
/// and output format (`svg`, `svg_inline`, `cmapx`, `json`, `json0`, `dot`, `xdot`, `canon`, `plain`).
#[wasm_bindgen]
pub fn render(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> std::result::Result<String, JsValue> {
    render_text(dot_source, engine, format).map_err(|e| JsValue::from_str(&e))
}

/// Renders every graph in a DOT string (a file may hold several), one output string per graph,
/// like the `dot` command line does for a multi-graph input file.
#[wasm_bindgen(js_name = renderAll)]
pub fn render_all(
    dot_source: &str,
    engine: &str,
    format: &str,
) -> std::result::Result<Vec<String>, JsValue> {
    render_all_text(dot_source, engine, format).map_err(|e| JsValue::from_str(&e))
}

/// [`render_all`] for a file's raw bytes, as the `dot` command line reads them: bytes that are
/// not UTF-8 (a `charset=latin1` file, say) are kept rather than replaced, and the result is
/// the bytes the command line writes, every graph's output concatenated. A JS string cannot
/// carry such bytes, so `renderAll` cannot reproduce C on these inputs.
///
/// Like `dot`, the graphs read before a syntax error are still rendered and returned; it
/// throws only when the error leaves nothing to render.
#[wasm_bindgen(js_name = renderAllBytes)]
pub fn render_all_bytes(
    dot_source: &[u8],
    engine: &str,
    format: &str,
) -> std::result::Result<Vec<u8>, JsValue> {
    let src = charset::Decoded::decode_slice(dot_source);
    let (engine, format) = (charset::from_text(engine), charset::from_text(format));
    let (parts, err) = graphviz_pure_render::decoded::render_dot_all_partial(
        &src,
        &engine,
        &format,
        &graphviz_pure_parser::GlobalAttrs::default(),
        graphviz_pure_layout::LayoutOptions::default(),
    );
    match err {
        Some(e) if parts.is_empty() => Err(js_error(e)),
        _ => Ok(charset::encode(&parts.concat()).into_owned()),
    }
}

/// Convenience helper to render a DOT graph directly to an SVG string using the specified layout engine.
#[wasm_bindgen(js_name = renderSvg)]
pub fn render_svg(dot_source: &str, engine: &str) -> std::result::Result<String, JsValue> {
    render_text(dot_source, engine, "svg").map_err(|e| JsValue::from_str(&e))
}

/// Convenience helper to render a DOT graph to Graphviz `json` format.
#[wasm_bindgen(js_name = renderJson)]
pub fn render_json(dot_source: &str, engine: &str) -> std::result::Result<String, JsValue> {
    render_text(dot_source, engine, "json").map_err(|e| JsValue::from_str(&e))
}

/// Parses and lays out a DOT graph and returns JSON statistics (`nodes`, `edges`, `subgraphs`, `width_pt`, `height_pt`).
#[wasm_bindgen(js_name = inspectGraph)]
pub fn inspect_graph(dot_source: &str, engine: &str) -> std::result::Result<String, JsValue> {
    inspect_text(dot_source, engine).map_err(|e| JsValue::from_str(&e))
}

/// [`inspect_graph`] without the `JsValue`.
fn inspect_text(dot_source: &str, engine: &str) -> Result<String, String> {
    let layout_engine =
        graphviz_pure_layout::LayoutEngine::parse(engine).map_err(|e| e.to_string())?;
    // `parse_dot` on the facade converts text first; a real char of the raw-byte range is
    // its UTF-8 bytes, and `to_text` below turns the graph's name back into that char.
    let src = charset::from_text(dot_source);
    let mut graph = graphviz_pure_parser::parse_dot(&src).map_err(error_text)?;
    graphviz_pure_layout::layout_graph(&mut graph, layout_engine).map_err(error_text)?;
    let stats = GraphStats {
        // the parsed graph holds the decoded form
        name: charset::to_text(&graph.name).into_owned(),
        directed: graph.kind == graphviz_pure_core::GraphKind::Directed,
        strict: graph.strict,
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        subgraphs: graph.subgraphs.len(),
        width_pt: graph.bb.width(),
        height_pt: graph.bb.height(),
    };
    serde_json::to_string(&stats).map_err(|e| e.to_string())
}

/// Returns the list of supported layout engines as a JSON array.
#[wasm_bindgen(js_name = supportedEngines)]
pub fn supported_engines() -> String {
    r#"["dot","neato","fdp","sfdp","twopi","circo","osage","patchwork"]"#.to_string()
}

/// Returns the list of supported output formats as a JSON array.
#[wasm_bindgen(js_name = supportedFormats)]
pub fn supported_formats() -> String {
    r#"["svg","svg_inline","cmapx","json","json0","dot","xdot","canon","plain"]"#.to_string()
}

/// Returns the version of `graphviz-pure` (compatible with Graphviz 16.1.0).
#[wasm_bindgen]
pub fn version() -> String {
    "graphviz-pure 0.1.0 (Rust port of Graphviz 16.1.0)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue #151: a JS string is text, so a real U+10FF41 in it is not one of the port's raw
    // bytes: it goes in and comes back out as itself, in output and in error messages.
    #[test]
    fn real_chars_of_the_raw_byte_range_cross_the_js_boundary() {
        let src = "digraph { \"x\u{10ff41}y\" -> b }";
        let out = render_text(src, "dot", "plain").unwrap();
        assert!(out.contains("node x\u{10ff41}y "), "{out}");
        assert!(!out.contains("xAy"), "{out}");
        let outs = render_all_text(&format!("{src}\n{src}"), "dot", "canon").unwrap();
        assert_eq!(outs.len(), 2);
        assert!(
            outs.iter().all(|o| o.contains("\tx\u{10ff41}y -> b;")),
            "{outs:?}"
        );

        // Engine and format names are text too, and error messages name them.
        let err = render_text(src, "x\u{10ff41}", "plain").unwrap_err();
        assert!(err.contains("x\u{10ff41}") && !err.contains("xA"), "{err}");
        let err = render_all_text(src, "dot", "x\u{10ff41}").unwrap_err();
        assert!(err.contains("x\u{10ff41}") && !err.contains("xA"), "{err}");
        // inspectGraph: the graph's name and an error that quotes the engine are text.
        let stats = inspect_text("digraph \"x\u{10ff41}\" { a }", "dot").unwrap();
        assert!(stats.starts_with("{\"name\":\"x\u{10ff41}\","), "{stats}");
        let err = inspect_text(src, "x\u{10ff41}").unwrap_err();
        assert_eq!(err, "Unsupported layout engine: 'x\u{10ff41}'");
        // A message that holds a raw byte is still text: the byte becomes U+FFFD.
        assert_eq!(
            error_text(charset::decode(b"caf\xe9".to_vec())),
            "caf\u{fffd}"
        );
    }
}
