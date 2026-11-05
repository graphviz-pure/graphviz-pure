// Hand-checked types for @graphviz-pure/wasm. `web/package.json` `build:wasm` and
// `scripts/site-wasm.sh` copy this file over the `graphviz_pure_wasm.d.ts` that
// wasm-bindgen generates, and that copy is what the npm and JSR packages publish
// (`types` in package.json, `publish.include` in jsr.json). `scripts/check-wasm-dts.mjs`
// fails CI when the generated function names or the init signatures drift from this
// file. `scripts/wasm-runtimes/types.ts` is typechecked against it. Edit this file;
// do not treat a regenerated one as the source.

/** A URL, response, bytes or compiled module accepted by asynchronous initialisation. */
export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

/** Bytes or a compiled module accepted by synchronous initialisation. */
export type SyncInitInput = BufferSource | WebAssembly.Module;

// wasm-bindgen's view of the instance exports. The drift check compares this
// interface with the generated file, comments and whitespace aside.
/** The WebAssembly instance exports returned by initialisation. */
export interface InitOutput {
  /** The instance's linear memory. */
  readonly memory: WebAssembly.Memory;
  /** The raw ABI entry point for graph statistics. */
  readonly inspectGraph: (a: number, b: number, c: number, d: number, e: any) => [number, number, number];
  /** The raw ABI entry point for {@link render}. */
  readonly render: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number, number];
  /** The raw ABI entry point for {@link renderAll}. */
  readonly renderAll: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number, number];
  /** The raw ABI entry point for {@link renderAllBytes}. */
  readonly renderAllBytes: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number, number];
  /** The raw ABI entry point for {@link renderAllBytesWithDiagnostics}. */
  readonly renderAllBytesWithDiagnostics: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number];
  /** The raw ABI entry point for {@link renderAllWithDiagnostics}. */
  readonly renderAllWithDiagnostics: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number];
  /** The raw ABI entry point for {@link renderJson}. */
  readonly renderJson: (a: number, b: number, c: number, d: number, e: any) => [number, number, number, number];
  /** The raw ABI entry point for {@link renderSvg}. */
  readonly renderSvg: (a: number, b: number, c: number, d: number, e: any) => [number, number, number, number];
  /** The raw ABI entry point for {@link renderWithDiagnostics}. */
  readonly renderWithDiagnostics: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number];
  /** The raw ABI entry point for {@link supportedEngines}. */
  readonly supportedEngines: () => [number, number];
  /** The raw ABI entry point for {@link supportedFormats}. */
  readonly supportedFormats: () => [number, number];
  /** The raw ABI entry point for {@link version}. */
  readonly version: () => [number, number];
  /** Stores a thrown JavaScript exception for the caller. */
  readonly __wbindgen_exn_store: (a: number) => void;
  /** Allocates a JavaScript object reference table slot. */
  readonly __externref_table_alloc: () => number;
  /** The table holding JavaScript object references. */
  readonly __wbindgen_externrefs: WebAssembly.Table;
  /** Frees an allocation in linear memory. */
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  /** Drops a slice of JavaScript object references. */
  readonly __externref_drop_slice: (a: number, b: number) => void;
  /** Allocates bytes in linear memory. */
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  /** Resizes an allocation in linear memory. */
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  /** Releases a JavaScript object reference table slot. */
  readonly __externref_table_dealloc: (a: number) => void;
  /** Runs the instance initialisation routine. */
  readonly __wbindgen_start: () => void;
}

/**
 * Instantiates the module. Pass `{ module }` for a compiled `WebAssembly.Module`
 * (Cloudflare Workers). Passing the bytes or the module directly is also accepted.
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * Loads the module. No argument fetches `graphviz_pure_wasm_bg.wasm` from next to
 * this file. `{ module_or_path }` is a URL, a `Response`, bytes, or a compiled module.
 */
export default function init(module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

/** `Error`, `Warning`, or `CountedAsError` (`AGMAX`: counted as an error, written as a warning). */
export type DiagnosticLevel = "Error" | "Warning" | "CountedAsError";

/** One `agerr` write. `message` includes the `Error: ` or `Warning: ` prefix and the trailing newline. */
export interface Diagnostic {
  /** The severity of this diagnostic. */
  level: DiagnosticLevel;
  /** The complete diagnostic text, including its prefix and trailing newline. */
  message: string;
}

/**
 * Why a call failed. `Error` is a kind this version of the types does not name yet
 * (`GraphvizErrorKind` can grow).
 */
export type GraphvizErrorKind =
  | "Syntax"
  | "UnsupportedEngine"
  | "UnsupportedFormat"
  | "Layout"
  | "Attribute"
  | "Render"
  | "LimitExceeded"
  | "Cancelled"
  | "Error";

/**
 * Which limit a `LimitExceeded` failure reached: `maxInputBytes`, `maxNodes`,
 * `maxEdges`, `timeout`, `maxSteps`, `maxAllocation`, `maxMemory`. `Other` is a
 * limit this version of the types does not name yet.
 */
export type GraphvizLimit =
  | "InputBytes"
  | "Nodes"
  | "Edges"
  | "Deadline"
  | "Steps"
  | "Allocation"
  | "Memory"
  | "Other";

/**
 * Limits and cancellation for one call, the last argument of every render call and
 * of {@link inspectGraph}. Every field is optional and off when absent; omitting the
 * object, or passing `undefined` or `null`, runs the call exactly as without it.
 *
 * A call stops at its next checkpoint (the parser's statements, the layout loops,
 * the emitter) and throws a {@link GraphvizError} of kind `LimitExceeded` (with
 * `limit`) or `Cancelled`. It returns its whole output, the same bytes as an
 * unlimited call, or throws: never a partial drawing. The checks are cooperative,
 * so a call can run past a limit until its next checkpoint; a service should still
 * run untrusted input in a worker it can terminate.
 *
 * Counts must be non-negative safe integers; anything else throws a `TypeError`.
 */
export interface RenderOptions {
  /** The longest DOT source accepted, in UTF-8 bytes. */
  maxInputBytes?: number;
  /** The most nodes one graph may declare. */
  maxNodes?: number;
  /** The most edges one graph may declare. */
  maxEdges?: number;
  /**
   * A deterministic work budget: the same input, version and budget always stop at
   * the same point. The unit is not stable across releases.
   */
  maxSteps?: number;
  /** The largest single input-sized buffer (polygon vertices, neato's model, dot's arenas), in bytes. */
  maxAllocation?: number;
  /**
   * The largest size of this instance's linear memory, in bytes, read at checkpoints.
   * WebAssembly memory never shrinks, so it counts what earlier calls grew it to.
   */
  maxMemory?: number;
  /**
   * Milliseconds from the start of the call, read from `performance.now()` at
   * checkpoints. Reaching it throws `LimitExceeded` with `limit` `Deadline`.
   * Cloudflare Workers stop the clock while code runs, so there it fires only for `0`.
   */
  timeout?: number;
  /**
   * Cancels the call when aborted: throws kind `Cancelled`. It is read when the call
   * starts and at every checkpoint. The call is synchronous, so nothing on this
   * thread's event loop can abort it once it runs (an `AbortSignal.timeout` fires only
   * after the call returns); an aborted signal stops calls before they start, and
   * `shouldCancel` or `timeout` stop a running one.
   */
  signal?: AbortSignal;
  /**
   * Polled at checkpoints; returning `true` cancels the call (kind `Cancelled`). It
   * runs inside the call, so it can read a clock or an `Atomics` flag that another
   * worker sets in a `SharedArrayBuffer`. It may call this package again: that call
   * runs under these limits too.
   */
  shouldCancel?: () => boolean;
}

/**
 * A failed call. The package exports this class, and a thrown failure is
 * `instanceof GraphvizError` (and `instanceof Error`).
 *
 * `message` is the text the command line prints for the failure, without the
 * program-name prefix. `status` is a number and is currently always `1`, the
 * status C's `dot` exits with when one graph fails. A successful call that
 * still collected an error-level diagnostic does not throw (`dot` would exit
 * 1); that diagnostic is on `*WithDiagnostics`.
 */
export class GraphvizError extends Error {
  /** The error class name. */
  readonly name: "GraphvizError";
  /** The command line exit status for this failure. */
  readonly status: number;
  /** The category of failure. */
  readonly kind: GraphvizErrorKind;
  /** Diagnostics collected before the call failed. */
  readonly diagnostics: Diagnostic[];
  /** The limit reached, only on a `LimitExceeded` failure. */
  readonly limit?: GraphvizLimit;
  /** Creates an error with its exit status, category, collected diagnostics and limit. */
  constructor(
    message: string,
    status: number,
    kind: GraphvizErrorKind,
    diagnostics: Diagnostic[],
    limit?: GraphvizLimit,
  );
}

/**
 * `inspectGraph`. The fields are snake_case, the names they had as JSON keys.
 * Function names are camelCase.
 */
export interface GraphStats {
  /** The graph name. */
  name: string;
  /** Whether the graph is directed. */
  directed: boolean;
  /** Whether parallel edges are forbidden. */
  strict: boolean;
  /** The number of nodes. */
  nodes: number;
  /** The number of edges. */
  edges: number;
  /** The number of subgraphs. */
  subgraphs: number;
  /** The laid-out graph width in points. */
  width_pt: number;
  /** The laid-out graph height in points. */
  height_pt: number;
}

/** The first graph's rendered text and the diagnostics collected by its call. */
export interface RenderedText {
  /** The first graph's rendered text. */
  output: string;
  /** Diagnostics collected by the call. */
  diagnostics: Diagnostic[];
}

/** Rendered text for every graph and the diagnostics of the whole call. */
export interface RenderedTexts {
  /** One rendered text string per graph. */
  output: string[];
  /** Diagnostics collected across all graphs. */
  diagnostics: Diagnostic[];
}

/** Concatenated rendered bytes and the diagnostics of the whole call. */
export interface RenderedBytes {
  /** The rendered bytes of all graphs concatenated. */
  output: Uint8Array;
  /** Diagnostics collected across all graphs. */
  diagnostics: Diagnostic[];
}

/** The first graph's output. Throws a {@link GraphvizError}. Warnings are dropped. */
export function render(dot_source: string, engine: string, format: string, options?: RenderOptions): string;

/** {@link render} plus the diagnostics that call collected. */
export function renderWithDiagnostics(
  dot_source: string,
  engine: string,
  format: string,
  options?: RenderOptions,
): RenderedText;

/** One output string per graph. Throws a {@link GraphvizError}. */
export function renderAll(dot_source: string, engine: string, format: string, options?: RenderOptions): string[];

/** {@link renderAll} plus the diagnostics of the whole call. */
export function renderAllWithDiagnostics(
  dot_source: string,
  engine: string,
  format: string,
  options?: RenderOptions,
): RenderedTexts;

/**
 * Raw input bytes in, the command line's output bytes out (every graph concatenated).
 * Throws a {@link GraphvizError} only when a failure leaves nothing rendered.
 */
export function renderAllBytes(dot_source: Uint8Array, engine: string, format: string, options?: RenderOptions): Uint8Array;

/**
 * {@link renderAllBytes} plus the diagnostics of the whole call. A partial success
 * carries the `agerr` messages, not the syntax error's `message`.
 */
export function renderAllBytesWithDiagnostics(
  dot_source: Uint8Array,
  engine: string,
  format: string,
  options?: RenderOptions,
): RenderedBytes;

/** {@link render} with format `svg`. */
export function renderSvg(dot_source: string, engine: string, options?: RenderOptions): string;

/** {@link render} with format `json`. */
export function renderJson(dot_source: string, engine: string, options?: RenderOptions): string;

/** Layout statistics. Warnings are not returned. Throws a {@link GraphvizError}. */
export function inspectGraph(dot_source: string, engine: string, options?: RenderOptions): GraphStats;

/** `dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage`, `patchwork`. */
export function supportedEngines(): string[];

/** Every format name {@link render} accepts, including aliases. No raster formats. */
export function supportedFormats(): string[];

/** `graphviz version 16.1.0 (graphviz-pure X.Y.Z)`. */
export function version(): string;
