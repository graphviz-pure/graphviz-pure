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
//!
//! Every render call and `inspectGraph` take an optional last argument, `options`, the
//! JavaScript form of the facade's `Limits` (docs/roadmap/limits.md): `maxInputBytes`,
//! `maxNodes`, `maxEdges`, `maxSteps`, `maxAllocation`, `maxMemory`, `timeout`, `signal`
//! and `shouldCancel`. Without it a call takes the unlimited path it always took.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use graphviz_pure_core::charset;
use graphviz_pure_core::limits::{self, Limit, Limits};
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
    constructor(message, status, kind, diagnostics, limit) {
        super(message);
        this.name = "GraphvizError";
        this.status = status;
        this.kind = kind;
        this.diagnostics = diagnostics;
        if (limit !== undefined) this.limit = limit;
    }
}
export function graphviz_error(message, status, kind, diagnosticsJson, limit) {
    return new GraphvizError(message, status, kind, JSON.parse(diagnosticsJson), limit);
}
const COUNTS = ["maxInputBytes", "maxNodes", "maxEdges", "maxSteps", "maxAllocation", "maxMemory"];
export function read_options(options) {
    if (options === undefined || options === null) return undefined;
    if (typeof options !== "object") throw new TypeError("options must be an object");
    const counts = COUNTS.map((key) => {
        const v = options[key];
        if (v === undefined) return -1;
        if (!Number.isSafeInteger(v) || v < 0) {
            throw new TypeError(`options.${key} must be a non-negative safe integer`);
        }
        return v;
    });
    const signal = options.signal;
    if (signal !== undefined && (signal === null || typeof signal.aborted !== "boolean")) {
        throw new TypeError("options.signal must be an AbortSignal");
    }
    const should = options.shouldCancel;
    if (should !== undefined && typeof should !== "function") {
        throw new TypeError("options.shouldCancel must be a function");
    }
    const timeout = options.timeout;
    if (timeout !== undefined && !(typeof timeout === "number" && timeout >= 0 && timeout <= Number.MAX_SAFE_INTEGER)) {
        throw new TypeError("options.timeout must be a non-negative number of milliseconds");
    }
    const clock = globalThis.performance ?? Date;
    const end = timeout === undefined ? undefined : clock.now() + timeout;
    const state = {
        counts,
        timedOut: false,
        thrown: undefined,
        polled: signal !== undefined || should !== undefined || end !== undefined,
    };
    // An exception must not unwind through the wasm frames: it cancels the call,
    // and the binding rethrows it once the call has returned.
    state.poll = () => {
        if (state.thrown !== undefined) return true;
        try {
            if (signal !== undefined && signal.aborted) return true;
            if (end !== undefined && clock.now() >= end) {
                state.timedOut = true;
                return true;
            }
            return should !== undefined && Boolean(should());
        } catch (e) {
            state.thrown = { error: e };
            return true;
        }
    };
    return state;
}
export function option_rethrow(state, error) {
    return state !== undefined && state.thrown !== undefined ? state.thrown.error : error;
}
export function option_count(state, i) {
    return state.counts[i];
}
export function option_polled(state) {
    return state.polled;
}
export function option_poll(state) {
    return state.poll();
}
export function option_timed_out(state) {
    return state.timedOut;
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
    fn graphviz_error(
        message: &str,
        status: i32,
        kind: &str,
        diagnostics_json: &str,
        limit: Option<&str>,
    ) -> JsValue;
    #[wasm_bindgen(catch)]
    fn read_options(options: &JsValue) -> Result<JsValue, JsValue>;
    fn option_count(state: &JsValue, i: u32) -> f64;
    fn option_polled(state: &JsValue) -> bool;
    fn option_poll(state: &JsValue) -> bool;
    fn option_timed_out(state: &JsValue) -> bool;
    fn option_rethrow(state: &JsValue, error: JsValue) -> JsValue;
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
    limit: Option<&'static str>,
    diagnostics: Vec<(&'static str, &'a str)>,
}

fn thrown(error: &GraphvizError) -> Thrown<'_> {
    Thrown {
        message: error.to_string(),
        status: FAILURE_STATUS,
        kind: kind_name(error.kind()),
        limit: error.limit().map(limit_name),
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
        view.limit,
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
        GraphvizErrorKind::LimitExceeded => "LimitExceeded",
        GraphvizErrorKind::Cancelled => "Cancelled",
        // `GraphvizErrorKind` is non-exhaustive. A kind this binding does not know yet.
        _ => "Error",
    }
}

fn limit_name(limit: Limit) -> &'static str {
    match limit {
        Limit::InputBytes => "InputBytes",
        Limit::Nodes => "Nodes",
        Limit::Edges => "Edges",
        Limit::Deadline => "Deadline",
        Limit::Steps => "Steps",
        Limit::Allocation => "Allocation",
        Limit::Heap => "Memory",
        // `Limit` is non-exhaustive.
        _ => "Other",
    }
}

/// The `options` argument, read. `None` fields are off. `poll` is the JS side's
/// `signal`, `timeout` and `shouldCancel`; `timed_out` says whether the timeout
/// was what fired.
#[derive(Default)]
struct Options {
    input_bytes: Option<u64>,
    nodes: Option<u64>,
    edges: Option<u64>,
    steps: Option<u64>,
    allocation: Option<u64>,
    memory: Option<u64>,
    poll: Option<Rc<dyn Fn() -> bool>>,
    timed_out: Option<Rc<dyn Fn() -> bool>>,
    /// The JS side's state, for [`fail`].
    js: Option<JsValue>,
}

thread_local! {
    /// The polls of the limited calls in progress, innermost last. A JS value is not
    /// `Send`, and `Limits::cancel_when` wants a `Send + Sync` closure, so that closure
    /// holds an index into this stack instead. A `shouldCancel` that calls back in
    /// pushes its own entry above.
    static POLLS: RefCell<Vec<Rc<dyn Fn() -> bool>>> = const { RefCell::new(Vec::new()) };
}

struct PollGuard;

impl Drop for PollGuard {
    fn drop(&mut self) {
        POLLS.with_borrow_mut(|p| p.pop());
    }
}

/// `options` from JS. `undefined` and `null` are `None`: no limit, the old path.
/// A malformed object throws a `TypeError`.
fn js_options(options: &JsValue) -> Result<Option<Options>, JsValue> {
    let state = read_options(options)?;
    if state.is_undefined() {
        return Ok(None);
    }
    let count = |i| {
        let v = option_count(&state, i);
        // A safe integer: `read_options` checked it.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (v >= 0.0).then_some(v as u64)
    };
    let mut out = Options {
        input_bytes: count(0),
        nodes: count(1),
        edges: count(2),
        steps: count(3),
        allocation: count(4),
        memory: count(5),
        js: Some(state.clone()),
        ..Options::default()
    };
    if option_polled(&state) {
        let s = state.clone();
        out.poll = Some(Rc::new(move || option_poll(&s)));
        out.timed_out = Some(Rc::new(move || option_timed_out(&state)));
    }
    Ok(Some(out))
}

/// The value a failed call throws: the exception a `signal` or `shouldCancel`
/// raised, when one did, or the [`GraphvizError`].
fn fail(options: Option<&Options>, error: &GraphvizError) -> JsValue {
    let thrown = throw_js(error);
    match options.and_then(|o| o.js.as_ref()) {
        Some(state) => option_rethrow(state, thrown),
        None => thrown,
    }
}

/// The instance's linear memory in bytes. It only grows, so `maxMemory` counts
/// what earlier calls grew it to.
fn memory_bytes() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) * 65536
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// `f` under `options`, or plainly when there are none. A timeout reports
/// `LimitExceeded` with `Limit::Deadline`, as `Limits::deadline` does natively;
/// `wasm32-unknown-unknown` has no `Instant`, so it is polled as a cancel.
fn limited<T>(
    options: Option<&Options>,
    f: impl FnOnce() -> Result<T, GraphvizError>,
) -> Result<T, GraphvizError> {
    let Some(o) = options else {
        return f();
    };
    let mut limits = Limits::new();
    if let Some(v) = o.input_bytes {
        limits = limits.max_input_bytes(v);
    }
    if let Some(v) = o.nodes {
        limits = limits.max_nodes(v);
    }
    if let Some(v) = o.edges {
        limits = limits.max_edges(v);
    }
    if let Some(v) = o.steps {
        limits = limits.max_steps(v);
    }
    if let Some(v) = o.allocation {
        limits = limits.max_allocation(v);
    }
    if let Some(v) = o.memory {
        limits = limits.heap_limit(v, memory_bytes);
    }
    let mut _guard = None;
    if let Some(poll) = &o.poll {
        let index = POLLS.with_borrow_mut(|p| {
            p.push(Rc::clone(poll));
            p.len() - 1
        });
        _guard = Some(PollGuard);
        limits = limits.cancel_when(move || {
            // Cloned out first: the poll may call back in and push its own entry.
            let poll = POLLS.with_borrow(|p| p.get(index).cloned());
            poll.is_some_and(|poll| poll())
        });
    }
    let result = limits.run(f);
    match result {
        Err(e)
            if e.kind() == GraphvizErrorKind::Cancelled
                && o.timed_out.as_ref().is_some_and(|t| t()) =>
        {
            let diagnostics = e.diagnostics().to_vec();
            Err(
                GraphvizError::limit_exceeded(Limit::Deadline, "the deadline passed")
                    .__set_diagnostics(diagnostics),
            )
        }
        other => other,
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
    options: Option<&Options>,
) -> Result<Rendered<String>, GraphvizError> {
    limited(options, || {
        graphviz_pure::render_dot(dot_source, engine, format)
    })
}

/// [`render_all`] on the facade.
fn render_all_text(
    dot_source: &str,
    engine: &str,
    format: &str,
    options: Option<&Options>,
) -> Result<Rendered<Vec<String>>, GraphvizError> {
    let globals = graphviz_pure::GlobalAttrs::default();
    limited(options, || {
        graphviz_pure::render_dot_all(dot_source, engine, format, &globals)
    })
}

/// `renderAllBytes` on decoded input. Output bytes stay encoded. Diagnostics are
/// converted once here: this path does not go through the facade, because the
/// facade would decode the bytes again.
///
/// Under `options` a limit or a cancel throws even when some graphs were
/// rendered: a limited call returns its whole output or the error.
fn render_all_bytes_decoded(
    dot_source: &[u8],
    engine: &str,
    format: &str,
    options: Option<&Options>,
) -> Result<(Vec<u8>, Vec<Diagnostic>), GraphvizError> {
    limited(options, || {
        render_all_bytes_unlimited(dot_source, engine, format, options.is_some())
    })
}

fn render_all_bytes_unlimited(
    dot_source: &[u8],
    engine: &str,
    format: &str,
    checked: bool,
) -> Result<(Vec<u8>, Vec<Diagnostic>), GraphvizError> {
    let src = charset::Decoded::decode_slice(dot_source);
    let engine = charset::from_text(engine);
    let format = charset::from_text(format);
    let ((parts, err), diagnostics) = graphviz_pure_core::diag::capture(|| {
        if checked {
            let start = limits::check().and_then(|()| limits::check_input_bytes(dot_source.len()));
            if let Err(e) = start {
                return (Vec::new(), Some(e));
            }
        }
        let (parts, err) = graphviz_pure_render::decoded::render_dot_all_partial(
            &src,
            &engine,
            &format,
            &graphviz_pure_parser::GlobalAttrs::default(),
            graphviz_pure_layout::LayoutOptions::default(),
        );
        match (limits::error(), err) {
            (Some(stop), _) if checked => (Vec::new(), Some(stop)),
            // The parser's size limits return their error instead of tripping.
            (_, Some(e))
                if matches!(
                    e.kind(),
                    GraphvizErrorKind::LimitExceeded | GraphvizErrorKind::Cancelled
                ) =>
            {
                (Vec::new(), Some(e))
            }
            (_, err) => (parts, err),
        }
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
fn inspect_text(
    dot_source: &str,
    engine: &str,
    options: Option<&Options>,
) -> Result<GraphStats, GraphvizError> {
    limited(options, || inspect_unlimited(dot_source, engine))
}

fn inspect_unlimited(dot_source: &str, engine: &str) -> Result<GraphStats, GraphvizError> {
    let src = charset::from_text(dot_source);
    let engine = charset::from_text(engine);
    let (result, diagnostics) =
        graphviz_pure_core::diag::capture(|| -> Result<_, GraphvizError> {
            // Outside a `Limits::run` these are no-ops.
            limits::check()?;
            limits::check_input_bytes(dot_source.len())?;
            let layout_engine = graphviz_pure_layout::LayoutEngine::parse(&engine)?;
            let mut graph = graphviz_pure_parser::parse_dot(&src)?;
            let laid = graphviz_pure_layout::layout_graph(&mut graph, layout_engine);
            limits::settle(laid)?;
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
pub fn render(
    dot_source: &str,
    engine: &str,
    format: &str,
    options: JsValue,
) -> Result<String, JsValue> {
    let options = js_options(&options)?;
    match render_text(dot_source, engine, format, options.as_ref()) {
        Ok(rendered) => Ok(rendered.into_output()),
        Err(error) => Err(fail(options.as_ref(), &error)),
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
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let options = js_options(&options)?;
    match render_text(dot_source, engine, format, options.as_ref()) {
        Ok(rendered) => Ok(ok_string(rendered)),
        Err(error) => Err(fail(options.as_ref(), &error)),
    }
}

/// Renders every graph in a DOT string, one output string per graph, as `dot`
/// does for a multi-graph file. Same failure object as [`render`].
#[wasm_bindgen(js_name = renderAll)]
pub fn render_all(
    dot_source: &str,
    engine: &str,
    format: &str,
    options: JsValue,
) -> Result<Vec<String>, JsValue> {
    let options = js_options(&options)?;
    match render_all_text(dot_source, engine, format, options.as_ref()) {
        Ok(rendered) => Ok(rendered.into_output()),
        Err(error) => Err(fail(options.as_ref(), &error)),
    }
}

/// [`render_all`] plus the diagnostics of the whole call. `output` is the
/// array [`render_all`] returns.
#[wasm_bindgen(js_name = renderAllWithDiagnostics)]
pub fn render_all_with_diagnostics(
    dot_source: &str,
    engine: &str,
    format: &str,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let options = js_options(&options)?;
    match render_all_text(dot_source, engine, format, options.as_ref()) {
        Ok(rendered) => Ok(ok_strings(rendered)),
        Err(error) => Err(fail(options.as_ref(), &error)),
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
///
/// A limit or a cancel from `options` always throws, even after some graphs
/// were rendered: unlike a syntax error, it returns no partial output.
#[wasm_bindgen(js_name = renderAllBytes)]
pub fn render_all_bytes(
    dot_source: &[u8],
    engine: &str,
    format: &str,
    options: JsValue,
) -> Result<Vec<u8>, JsValue> {
    let options = js_options(&options)?;
    match render_all_bytes_decoded(dot_source, engine, format, options.as_ref()) {
        Ok((bytes, _)) => Ok(bytes),
        Err(error) => Err(fail(options.as_ref(), &error)),
    }
}

/// [`render_all_bytes`] plus the diagnostics of the whole call. `output` is
/// the `Uint8Array` [`render_all_bytes`] returns. A partial success (some
/// graphs rendered, then a syntax error) carries the `agerr` messages, not the
/// syntax error's `message`. A limit or a cancel always throws.
#[wasm_bindgen(js_name = renderAllBytesWithDiagnostics)]
pub fn render_all_bytes_with_diagnostics(
    dot_source: &[u8],
    engine: &str,
    format: &str,
    options: JsValue,
) -> Result<JsValue, JsValue> {
    let options = js_options(&options)?;
    match render_all_bytes_decoded(dot_source, engine, format, options.as_ref()) {
        Ok((bytes, diagnostics)) => Ok(with_bytes(bytes, &diagnostics_of(&diagnostics))),
        Err(error) => Err(fail(options.as_ref(), &error)),
    }
}

/// [`render`] with the format `svg`.
#[wasm_bindgen(js_name = renderSvg)]
pub fn render_svg(dot_source: &str, engine: &str, options: JsValue) -> Result<String, JsValue> {
    render(dot_source, engine, "svg", options)
}

/// [`render`] with the format `json`.
#[wasm_bindgen(js_name = renderJson)]
pub fn render_json(dot_source: &str, engine: &str, options: JsValue) -> Result<String, JsValue> {
    render(dot_source, engine, "json", options)
}

/// Parses and lays out a graph.
///
/// Returns `{name, directed, strict, nodes, edges, subgraphs, width_pt, height_pt}`.
/// The fields are snake_case, the names they had when this returned JSON. The
/// function names stay camelCase. Warnings are not returned.
#[wasm_bindgen(js_name = inspectGraph)]
pub fn inspect_graph(dot_source: &str, engine: &str, options: JsValue) -> Result<JsValue, JsValue> {
    let options = js_options(&options)?;
    match inspect_text(dot_source, engine, options.as_ref()) {
        Ok(stats) => {
            let json = serde_json::to_string(&stats)
                .map_err(|e| throw_js(&GraphvizError::render(e.to_string())))?;
            Ok(graph_stats(&json))
        }
        Err(error) => Err(fail(options.as_ref(), &error)),
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
                "ps",
                "ps2",
                "eps",
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
            let out = render_text("digraph { a -> b }", "dot", &format, None)
                .unwrap()
                .into_output();
            assert!(!out.is_empty(), "{format}");
        }
    }

    #[test]
    fn a_failure_carries_status_kind_and_diagnostics() {
        let err = render_text("digraph {", "dot", "svg", None).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "Syntax");
        assert!(view.message.contains("syntax error"), "{}", view.message);

        let err = render_text("digraph { a -> b }", "nope", "svg", None).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "UnsupportedEngine");
        assert_eq!(view.message, "Unsupported layout engine: 'nope'");
        assert!(view.diagnostics.is_empty());

        let err = render_text("digraph { a -> b }", "dot", "png", None).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.kind, "UnsupportedFormat");
        assert_eq!(view.status, 1);
        assert_eq!(view.message, "Unsupported output format: 'png'");
    }

    #[test]
    fn a_warning_stays_on_success_and_an_error_diagnostic_does_not_throw() {
        let rendered = render_text("digraph { a [shape=only_a] }", "dot", "svg", None).unwrap();
        assert!(rendered.output().contains("<svg"));
        assert_eq!(rendered.diagnostics()[0].level(), DiagnosticLevel::Warning);
        assert_eq!(
            rendered.diagnostics()[0].message(),
            "Warning: using box for unknown shape only_a\n"
        );
        let again = render_text("digraph { a [shape=only_a] }", "dot", "svg", None).unwrap();
        assert_eq!(
            again.diagnostics()[0].message(),
            rendered.diagnostics()[0].message()
        );

        // `dot` exits 1 for this and still writes the SVG. The library call
        // returns the SVG; the error diagnostic is not a throw.
        let rendered = render_text(
            "digraph { b [shape=record,label=\"{a|\"] }",
            "dot",
            "svg",
            None,
        )
        .unwrap();
        assert!(rendered.output().contains("<svg"));
        assert!(rendered.diagnostics().iter().any(|d| {
            d.level() == DiagnosticLevel::Error && d.message().contains("bad label format")
        }));
    }

    #[test]
    fn inspect_returns_the_fields_and_drops_warnings() {
        let stats = inspect_text("digraph G { a -> b }", "dot", None).unwrap();
        assert_eq!(stats.name, "G");
        assert!(stats.directed);
        assert!(!stats.strict);
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.subgraphs, 0);
        assert!(stats.width_pt.is_finite() && stats.width_pt > 0.0);
        assert!(stats.height_pt.is_finite() && stats.height_pt > 0.0);

        let strict = inspect_text("strict graph { a -- b }", "dot", None).unwrap();
        assert!(strict.strict);
        assert!(!strict.directed);
        assert_eq!(strict.edges, 1);

        let warned = inspect_text("digraph { a [shape=only_a] }", "dot", None).unwrap();
        assert_eq!(warned.nodes, 1);
    }

    #[test]
    fn bytes_keep_a_partial_render_and_throw_only_when_nothing_was_rendered() {
        let (bytes, diagnostics) = render_all_bytes_decoded(
            b"digraph { a [shape=only_a] }\ndigraph {",
            "dot",
            "svg",
            None,
        )
        .unwrap();
        assert!(bytes.windows(4).any(|w| w == b"<svg"));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message().contains("unknown shape only_a"))
        );

        let err = render_all_bytes_decoded(b"digraph {", "dot", "svg", None).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.status, 1);
        assert_eq!(view.kind, "Syntax");

        let err = render_all_bytes_decoded(b"digraph { a }", "dot", "nope", None).unwrap_err();
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
        let err = render_text(src, "dot", "svg", None).unwrap_err();
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

        let err = render_all_text("digraph {", "dot", "svg", None).unwrap_err();
        assert_eq!(thrown(&err).kind, "Syntax");
        assert_eq!(thrown(&err).status, 1);
        let err = inspect_text("digraph {", "dot", None).unwrap_err();
        assert_eq!(thrown(&err).kind, "Syntax");
        let err = inspect_text("digraph { a -> b }", "nope", None).unwrap_err();
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        assert_eq!(thrown(&err).status, 1);
    }

    // A JS string is text, so a real U+10FF41 in it is not one of the port's raw
    // bytes: it goes in and comes back out as itself, in output and in errors.
    #[test]
    fn real_chars_of_the_raw_byte_range_cross_the_js_boundary() {
        let src = "digraph { \"x\u{10ff41}y\" -> b }";
        let out = render_text(src, "dot", "plain", None)
            .unwrap()
            .into_output();
        assert!(out.contains("node x\u{10ff41}y "), "{out}");
        assert!(!out.contains("xAy"), "{out}");
        let outs = render_all_text(&format!("{src}\n{src}"), "dot", "canon", None)
            .unwrap()
            .into_output();
        assert_eq!(outs.len(), 2);
        assert!(
            outs.iter().all(|o| o.contains("\tx\u{10ff41}y -> b;")),
            "{outs:?}"
        );

        let err = render_text(src, "x\u{10ff41}", "plain", None).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("x\u{10ff41}") && !message.contains("xA"),
            "{message}"
        );
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        let err = render_all_text(src, "dot", "x\u{10ff41}", None).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("x\u{10ff41}") && !message.contains("xA"),
            "{message}"
        );

        let stats = inspect_text("digraph \"x\u{10ff41}\" { a }", "dot", None).unwrap();
        assert_eq!(stats.name, "x\u{10ff41}");
        let err = inspect_text(src, "x\u{10ff41}", None).unwrap_err();
        assert_eq!(err.to_string(), "Unsupported layout engine: 'x\u{10ff41}'");
        assert_eq!(thrown(&err).kind, "UnsupportedEngine");
        assert_eq!(
            charset::to_text(&charset::decode(b"caf\xe9".to_vec())),
            "caf\u{fffd}"
        );
    }

    fn polled(poll: impl Fn() -> bool + 'static, timed_out: bool) -> Options {
        Options {
            poll: Some(Rc::new(poll)),
            timed_out: Some(Rc::new(move || timed_out)),
            ..Options::default()
        }
    }

    #[test]
    fn options_that_are_not_reached_keep_the_bytes() {
        let src = "digraph { a -> b -> c; a -> c }";
        let options = Options {
            input_bytes: Some(1 << 20),
            nodes: Some(100),
            edges: Some(100),
            steps: Some(u64::MAX / 2),
            allocation: Some(1 << 30),
            ..polled(|| false, false)
        };
        let plain = render_text(src, "dot", "svg", None).unwrap().into_output();
        let capped = render_text(src, "dot", "svg", Some(&options))
            .unwrap()
            .into_output();
        assert_eq!(plain, capped);
        let (plain, _) = render_all_bytes_decoded(src.as_bytes(), "neato", "plain", None).unwrap();
        let (capped, _) =
            render_all_bytes_decoded(src.as_bytes(), "neato", "plain", Some(&options)).unwrap();
        assert_eq!(plain, capped);
        assert_eq!(inspect_text(src, "dot", Some(&options)).unwrap().nodes, 3);
    }

    #[test]
    fn each_path_reports_the_limit_it_reached() {
        let src = "digraph { a -> b; b -> c; c -> d }";
        let nodes = Options {
            nodes: Some(2),
            ..Options::default()
        };
        let err = render_text(src, "dot", "svg", Some(&nodes)).unwrap_err();
        assert_eq!(thrown(&err).kind, "LimitExceeded");
        assert_eq!(thrown(&err).limit, Some("Nodes"));
        assert!(
            thrown(&err).message.starts_with("Limit exceeded: "),
            "{err}"
        );
        let err = render_all_text(src, "dot", "svg", Some(&nodes)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("Nodes"));
        let err = inspect_text(src, "dot", Some(&nodes)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("Nodes"));

        let bytes = Options {
            input_bytes: Some(4),
            ..Options::default()
        };
        let err = render_all_bytes_decoded(src.as_bytes(), "dot", "svg", Some(&bytes)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("InputBytes"));
        let err = inspect_text(src, "dot", Some(&bytes)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("InputBytes"));

        // A limit throws even after a first graph rendered: no partial output.
        let two = format!("digraph {{ x }}\n{src}");
        let err = render_all_bytes_decoded(two.as_bytes(), "dot", "svg", Some(&nodes)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("Nodes"));

        let steps = Options {
            steps: Some(1),
            ..Options::default()
        };
        let err = render_text(src, "neato", "svg", Some(&steps)).unwrap_err();
        assert_eq!(thrown(&err).limit, Some("Steps"));
        let syntax = render_text("digraph {", "dot", "svg", Some(&steps)).unwrap_err();
        assert!(thrown(&syntax).limit.is_none());
    }

    #[test]
    fn a_poll_cancels_and_a_timeout_is_a_deadline() {
        let src = "digraph { a -> b }";
        let err = render_text(src, "dot", "svg", Some(&polled(|| true, false))).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.kind, "Cancelled");
        assert_eq!(view.limit, None);
        assert_eq!(view.message, "Cancelled: the call was cancelled");
        let err =
            render_all_bytes_decoded(src.as_bytes(), "dot", "svg", Some(&polled(|| true, false)))
                .unwrap_err();
        assert_eq!(thrown(&err).kind, "Cancelled");

        let err = render_text(src, "dot", "svg", Some(&polled(|| true, true))).unwrap_err();
        let view = thrown(&err);
        assert_eq!(view.kind, "LimitExceeded");
        assert_eq!(view.limit, Some("Deadline"));
        assert_eq!(view.message, "Limit exceeded: the deadline passed");
        assert!(POLLS.with_borrow(Vec::is_empty));
    }

    // A `shouldCancel` that renders again: the inner call runs under the outer
    // limits too, and each call polls its own callback.
    #[test]
    fn a_poll_may_call_back_in() {
        let inner_ok = Rc::new(std::cell::Cell::new(None));
        let seen = Rc::clone(&inner_ok);
        let outer = polled(
            move || {
                if seen.get().is_none() {
                    let r = render_text(
                        "digraph { x }",
                        "dot",
                        "plain",
                        Some(&polled(|| false, false)),
                    );
                    seen.set(Some(r.is_ok()));
                }
                false
            },
            false,
        );
        let svg = render_text("digraph { a -> b }", "dot", "svg", Some(&outer)).unwrap();
        assert!(svg.output().contains("<svg"));
        assert_eq!(inner_ok.get(), Some(true));
        assert!(POLLS.with_borrow(Vec::is_empty));
    }
}
