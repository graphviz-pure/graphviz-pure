//! Shared bodies of the fuzz targets in `fuzz_targets/`. Each target must never panic, hang
//! or run out of memory on any input; an `Err` from the library is a normal result.

/// Inputs longer than this are skipped. The seed corpus and `-max_len` stay below it.
pub const MAX_INPUT: usize = 8192;

/// Graphs larger than these are skipped by the render targets: layout is superlinear (network
/// simplex, crossing minimisation, stress majorisation), so a big enough graph is slow in C
/// too, and libFuzzer would report it as a timeout.
pub const MAX_NODES: usize = 50;
pub const MAX_EDGES: usize = 100;
pub const MAX_SUBGRAPHS: usize = 25;

/// DOT is read as bytes and decoded as the CLI decodes a file ([`graphviz_pure_core::charset::decode_slice`]:
/// bytes that are not UTF-8 stay raw bytes). The targets parse it as the CLI reads a file
/// ([`graphviz_pure_parser::reading_file`]: `fgets`, where a NUL hides the rest of its line),
/// not as a library string, which ends at its first NUL.
fn source(data: &[u8]) -> Option<graphviz_pure_core::charset::Decoded> {
    (data.len() <= MAX_INPUT).then(|| graphviz_pure_core::charset::Decoded::decode_slice(data))
}

/// The DOT parser and the canonical writer on every graph read before an error.
pub fn parse(data: &[u8]) {
    let Some(src) = source(data) else { return };
    let (graphs, _err) = graphviz_pure_parser::reading_file("fuzz", || {
        graphviz_pure_parser::parse_dot_all_partial(
            src.as_str(),
            &graphviz_pure_parser::GlobalAttrs::default(),
        )
    });
    for graph in &graphs {
        let _ = graphviz_pure_parser::write_canon(graph);
    }
}

/// The HTML-label parser (`label=<...>`) on the text between the outer delimiters.
pub fn parse_html(data: &[u8]) {
    let Some(src) = source(data) else { return };
    let _ = graphviz_pure_geom::html_label::parse_html(src.as_str());
}

/// Parse, lay out with `engine` (or the graph's own `layout`) and render SVG.
pub fn render(data: &[u8], engine: &str) {
    let Some(src) = source(data) else { return };
    graphviz_pure_parser::reading_file("fuzz", || render_source(&src, engine));
}

fn render_source(src: &graphviz_pure_core::charset::Decoded, engine: &str) {
    let Ok(graph) = graphviz_pure_render::decoded::parse_dot(src) else {
        return;
    };
    if graph.nodes.len() > MAX_NODES
        || graph.edges.len() > MAX_EDGES
        || graph.subgraphs.len() > MAX_SUBGRAPHS
    {
        return;
    }
    let _ = graphviz_pure_render::decoded::render_dot(src, engine, "svg");
}
