// Hand-checked types for @graphviz-pure/wasm. `web/package.json` `build:wasm` and
// `scripts/site-wasm.sh` copy this file over the `graphviz_pure_wasm.d.ts` that
// wasm-bindgen generates, and that copy is what the npm and JSR packages publish
// (`types` in package.json, `publish.include` in jsr.json). `scripts/wasm-runtimes/types.ts`
// is typechecked against it in CI. Edit this file; do not treat a regenerated one as the source.
/// <reference lib="dom" />

export type InitInput = string | URL | Response | BufferSource | WebAssembly.Module;

export type SyncInitInput = BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
}

/**
 * Instantiates the module. Pass `{ module }` for a compiled `WebAssembly.Module`
 * (Cloudflare Workers). Passing the bytes or the module directly is also accepted.
 */
export function initSync(
  module?: { module: SyncInitInput; memory?: WebAssembly.Memory } | SyncInitInput,
): InitOutput;

/**
 * Loads the module. No argument fetches `graphviz_pure_wasm_bg.wasm` from next to
 * this file. `{ module_or_path }` is a URL, a `Response`, bytes, or a compiled module.
 */
export default function init(module_or_path?: {
  module_or_path?: InitInput | Promise<InitInput>;
  memory?: WebAssembly.Memory;
} | InitInput | Promise<InitInput>): Promise<InitOutput>;

/** `Error`, `Warning`, or `CountedAsError` (`AGMAX`: counted as an error, written as a warning). */
export type DiagnosticLevel = "Error" | "Warning" | "CountedAsError";

/** One `agerr` write. `message` includes the `Error: ` or `Warning: ` prefix and the trailing newline. */
export interface Diagnostic {
  level: DiagnosticLevel;
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
  | "Error";

/**
 * A failed call. `instanceof Error` is true and `name` is `"GraphvizError"`.
 * The constructor is not a package export: catch the object, do not construct it.
 *
 * `message` is the text the command line prints for the failure, without the
 * program-name prefix. `status` is `1`, the status `dot` uses when one graph
 * fails. A successful call that still collected an error-level diagnostic does
 * not throw (`dot` would exit 1); that diagnostic is on `*WithDiagnostics`.
 */
export interface GraphvizError extends Error {
  name: "GraphvizError";
  message: string;
  status: number;
  kind: GraphvizErrorKind;
  diagnostics: Diagnostic[];
}

/**
 * `inspectGraph`. The fields are snake_case, the names they had as JSON keys.
 * Function names are camelCase.
 */
export interface GraphStats {
  name: string;
  directed: boolean;
  strict: boolean;
  nodes: number;
  edges: number;
  subgraphs: number;
  width_pt: number;
  height_pt: number;
}

export interface RenderedText {
  output: string;
  diagnostics: Diagnostic[];
}

export interface RenderedTexts {
  output: string[];
  diagnostics: Diagnostic[];
}

export interface RenderedBytes {
  output: Uint8Array;
  diagnostics: Diagnostic[];
}

/** The first graph's output. Throws a {@link GraphvizError}. Warnings are dropped. */
export function render(dot_source: string, engine: string, format: string): string;

/** {@link render} plus the diagnostics that call collected. */
export function renderWithDiagnostics(
  dot_source: string,
  engine: string,
  format: string,
): RenderedText;

/** One output string per graph. Throws a {@link GraphvizError}. */
export function renderAll(dot_source: string, engine: string, format: string): string[];

/** {@link renderAll} plus the diagnostics of the whole call. */
export function renderAllWithDiagnostics(
  dot_source: string,
  engine: string,
  format: string,
): RenderedTexts;

/**
 * Raw input bytes in, the command line's output bytes out (every graph concatenated).
 * Throws a {@link GraphvizError} only when a failure leaves nothing rendered.
 */
export function renderAllBytes(dot_source: Uint8Array, engine: string, format: string): Uint8Array;

/**
 * {@link renderAllBytes} plus the diagnostics of the whole call. A partial success
 * carries the `agerr` messages, not the syntax error's `message`.
 */
export function renderAllBytesWithDiagnostics(
  dot_source: Uint8Array,
  engine: string,
  format: string,
): RenderedBytes;

/** {@link render} with format `svg`. */
export function renderSvg(dot_source: string, engine: string): string;

/** {@link render} with format `json`. */
export function renderJson(dot_source: string, engine: string): string;

/** Layout statistics. Warnings are not returned. Throws a {@link GraphvizError}. */
export function inspectGraph(dot_source: string, engine: string): GraphStats;

/** `dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage`, `patchwork`. */
export function supportedEngines(): string[];

/** Every format name {@link render} accepts, including aliases. No raster formats. */
export function supportedFormats(): string[];

/** `graphviz version 16.1.0 (graphviz-pure X.Y.Z)`. */
export function version(): string;
