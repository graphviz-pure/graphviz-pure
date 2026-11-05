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
  type GraphvizLimit,
  type InitOutput,
  type RenderOptions,
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
  LimitExceeded: true,
  Cancelled: true,
  Error: true,
};

const options: RenderOptions = {
  maxInputBytes: 1 << 20,
  maxNodes: 10_000,
  maxEdges: 50_000,
  maxSteps: 5_000_000,
  maxAllocation: 512 << 20,
  maxMemory: 1 << 30,
  timeout: 5_000,
  signal: new AbortController().signal,
  shouldCancel: () => false,
};
const limited: string = render("digraph { a -> b }", "dot", "svg", options) +
  renderSvg("digraph { a -> b }", "dot", {}) + renderJson("digraph { a -> b }", "dot", undefined);
const limitedAll: string[] = renderAll("digraph { a -> b }", "dot", "svg", { maxNodes: 1 });
const limitedBytes: Uint8Array = renderAllBytes(new Uint8Array(), "dot", "svg", { timeout: 0 });
const limitedStats: GraphStats = inspectGraph("digraph { a -> b }", "dot", { maxSteps: 1 });
const nulled: string = render("digraph { a -> b }", "dot", "svg", null) + renderSvg("digraph { a -> b }", "dot", null);
const nulledStats: GraphStats = inspectGraph("digraph { a -> b }", "dot", null);
// Raw InitOutput calls written before `options` existed still typecheck.
function rawCalls(out: InitOutput): void {
  out.render(0, 0, 0, 0, 0, 0);
  out.renderSvg(0, 0, 0, 0);
  out.inspectGraph(0, 0, 0, 0);
  out.render(0, 0, 0, 0, 0, 0, undefined);
}
const limits: Record<GraphvizLimit, true> = {
  InputBytes: true,
  Nodes: true,
  Edges: true,
  Deadline: true,
  Steps: true,
  Allocation: true,
  Memory: true,
  Other: true,
};

function readError(err: unknown): string {
  if (!(err instanceof GraphvizError)) throw new Error("expected GraphvizError");
  const built = new GraphvizError("syntax error", 1, "Syntax", []);
  const kind: GraphvizErrorKind = built.kind;
  const limit: GraphvizLimit | undefined = new GraphvizError("x", 1, "LimitExceeded", [], "Nodes").limit ?? err.limit;
  void limit;
  return `${err.name}:${err.status}:${err.kind}:${err.message}:${err.diagnostics.length}:${level}:${kind}:${kinds.Syntax}`;
}

void width;
void limited;
void limitedAll;
void limitedBytes;
void limitedStats;
void limits;
void nulled;
void nulledStats;
void rawCalls;
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
