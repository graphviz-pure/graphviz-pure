// Typechecks the hand-checked declaration CI copies into the npm and JSR packages.
// `deno check` does not instantiate the module. A generated `any`, a JSON-string
// return, or a missing field fails one of the assertions below.
import init, {
  initSync,
  inspectGraph,
  render,
  renderAll,
  renderAllBytes,
  renderAllBytesWithDiagnostics,
  renderAllWithDiagnostics,
  renderJson,
  renderSvg,
  renderWithDiagnostics,
  supportedEngines,
  supportedFormats,
  version,
  GraphvizError,
  type Diagnostic,
  type GraphStats,
  type GraphvizErrorKind,
  type RenderedBytes,
  type RenderedText,
  type RenderedTexts,
} from "../../web/pkg/graphviz_pure_wasm.d.ts";

type Assert<T extends true> = T;
type IsAny<T> = 0 extends 1 & T ? true : false;
type Eq<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type NotAny<T extends true> = T;

type _render = Assert<NotAny<IsAny<ReturnType<typeof render>> extends false ? true : never>>;
type _renderEq = Assert<Eq<ReturnType<typeof render>, string>>;
type _renderAll = Assert<Eq<ReturnType<typeof renderAll>, string[]>>;
type _renderSvg = Assert<Eq<ReturnType<typeof renderSvg>, string>>;
type _renderJson = Assert<Eq<ReturnType<typeof renderJson>, string>>;
type _engines = Assert<Eq<ReturnType<typeof supportedEngines>, string[]>>;
type _formats = Assert<Eq<ReturnType<typeof supportedFormats>, string[]>>;
type _version = Assert<Eq<ReturnType<typeof version>, string>>;
type _inspect = Assert<Eq<ReturnType<typeof inspectGraph>, GraphStats>>;
type _with = Assert<Eq<ReturnType<typeof renderWithDiagnostics>, RenderedText>>;
type _withAll = Assert<Eq<ReturnType<typeof renderAllWithDiagnostics>, RenderedTexts>>;
type _withBytes = Assert<Eq<ReturnType<typeof renderAllBytesWithDiagnostics>, RenderedBytes>>;
type _bytes = Assert<Eq<ReturnType<typeof renderAllBytes>, Uint8Array>>;

const stats: GraphStats = inspectGraph("digraph G { a -> b }", "dot");
const width: number = stats.width_pt + stats.height_pt + stats.nodes;
const directed: boolean = stats.directed && !stats.strict;
const name: string = stats.name;
const level: Diagnostic["level"] = renderWithDiagnostics("digraph { a -> b }", "dot", "svg").diagnostics[0]?.level ??
  "Warning";
const output: string = renderWithDiagnostics("digraph { a -> b }", "dot", "svg").output;
const many: string[] = renderAllWithDiagnostics("digraph { a -> b }", "dot", "svg").output;
const raw: Uint8Array = renderAllBytesWithDiagnostics(new Uint8Array(), "dot", "svg").output;

const kinds: Record<GraphvizErrorKind, true> = {
  Syntax: true,
  UnsupportedEngine: true,
  UnsupportedFormat: true,
  Layout: true,
  Attribute: true,
  Render: true,
  Error: true,
};

function readError(err: unknown): string {
  if (!(err instanceof GraphvizError)) throw new Error("expected GraphvizError");
  const built = new GraphvizError("syntax error", 1, "Syntax", []);
  const kind: GraphvizErrorKind = built.kind;
  return `${err.name}:${err.status}:${err.kind}:${err.message}:${err.diagnostics.length}:${level}:${kind}:${kinds.Syntax}`;
}

void width;
void directed;
void name;
void output;
void many;
void raw;
void readError;
void init;
void initSync;
void render;
void supportedEngines;
void supportedFormats;
void version;
