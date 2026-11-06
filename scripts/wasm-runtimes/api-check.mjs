// The JS shapes #523 promises. No Node APIs: Node, Deno, Bun, workerd and the browser all run it.
// `gv` is the package namespace after init / initSync.

export function checkApi(gv) {
  const engines = gv.supportedEngines();
  if (!Array.isArray(engines) || engines[0] !== 'dot' || !engines.includes('patchwork')) {
    throw new Error(`supportedEngines: ${JSON.stringify(engines)}`);
  }
  const formats = gv.supportedFormats();
  if (!Array.isArray(formats) || !formats.includes('svg') || !formats.includes('plain-ext')) {
    throw new Error(`supportedFormats: ${JSON.stringify(formats)}`);
  }
  if (formats.includes('png')) throw new Error('png is not a wasm format');

  const svg = gv.render('digraph { a -> b }', 'dot', 'svg');
  if (typeof svg !== 'string' || !svg.includes('<svg')) throw new Error('render did not return a string');
  if (!Array.isArray(gv.renderAll('digraph { a -> b }\ndigraph { c -> d }', 'dot', 'svg')) ||
      gv.renderAll('digraph { a -> b }\ndigraph { c -> d }', 'dot', 'svg').length !== 2) {
    throw new Error('renderAll did not return one string per graph');
  }

  const warned = gv.render('digraph { a [shape=only_a] }', 'dot', 'svg');
  if (typeof warned !== 'string' || !warned.includes('<svg')) {
    throw new Error('a warning changed render() off a string');
  }
  const both = gv.renderWithDiagnostics('digraph { a [shape=only_a] }', 'dot', 'svg');
  if (typeof both.output !== 'string' || !both.output.includes('<svg') || !Array.isArray(both.diagnostics)) {
    throw new Error(`renderWithDiagnostics: ${typeof both.output}`);
  }
  const warning = both.diagnostics.find((d) => d.level === 'Warning');
  if (!warning || warning.message !== 'Warning: using box for unknown shape only_a\n') {
    throw new Error(`warning diagnostic ${JSON.stringify(both.diagnostics)}`);
  }

  // `dot` exits 1 and still writes the picture. The call does not throw.
  const badLabel = 'digraph { b [shape=record,label="{a|"] }';
  const bad = gv.renderWithDiagnostics(badLabel, 'dot', 'svg');
  if (typeof bad.output !== 'string' || !bad.output.includes('<svg')) {
    throw new Error('an error diagnostic threw or dropped the output');
  }
  if (!bad.diagnostics.some((d) => d.level === 'Error' && d.message.includes('bad label format'))) {
    throw new Error(`error diagnostic ${JSON.stringify(bad.diagnostics)}`);
  }
  if (typeof gv.render(badLabel, 'dot', 'svg') !== 'string') throw new Error('render threw on a bad label');

  const stats = gv.inspectGraph('digraph G { a -> b }', 'dot');
  for (const key of ['name', 'directed', 'strict', 'nodes', 'edges', 'subgraphs', 'width_pt', 'height_pt']) {
    if (stats == null || typeof stats !== 'object' || !(key in stats)) throw new Error(`inspectGraph missing ${key}`);
  }
  if (stats.name !== 'G' || stats.directed !== true || stats.strict !== false) {
    throw new Error(`inspectGraph ${JSON.stringify(stats)}`);
  }
  if (stats.nodes !== 2 || stats.edges !== 1 || typeof stats.width_pt !== 'number' || typeof stats.height_pt !== 'number') {
    throw new Error(`inspectGraph counts ${JSON.stringify(stats)}`);
  }
  if (gv.inspectGraph('digraph { a [shape=only_a] }', 'dot').nodes !== 1) {
    throw new Error('inspectGraph threw on a warning');
  }

  const syntax = 'digraph {';
  for (const fn of [
    () => gv.render(syntax, 'dot', 'svg'),
    () => gv.renderWithDiagnostics(syntax, 'dot', 'svg'),
    () => gv.renderAll(syntax, 'dot', 'svg'),
    () => gv.renderAllWithDiagnostics(syntax, 'dot', 'svg'),
    () => gv.renderAllBytes(new TextEncoder().encode(syntax), 'dot', 'svg'),
    () => gv.renderAllBytesWithDiagnostics(new TextEncoder().encode(syntax), 'dot', 'svg'),
    () => gv.renderSvg(syntax, 'dot'),
    () => gv.renderJson(syntax, 'dot'),
    () => gv.inspectGraph(syntax, 'dot'),
  ]) {
    expectThrow(gv, fn, 'Syntax', 'syntax error');
  }
  expectThrow(gv, () => gv.render('digraph { a -> b }', 'nope', 'svg'), 'UnsupportedEngine', 'nope');
  expectThrow(gv, () => gv.render('digraph { a -> b }', 'dot', 'png'), 'UnsupportedFormat', 'png');
  expectThrow(gv, () => gv.renderAll('digraph { a -> b }', 'nope', 'svg'), 'UnsupportedEngine', 'nope');
  expectThrow(gv, () => gv.inspectGraph('digraph { a -> b }', 'nope'), 'UnsupportedEngine', 'nope');

  // Warnings during the layout, then a layout failure. Render is the raster
  // crate's kind; this package has no raster format, so nothing here throws it.
  const layout = `digraph G {
rank=sink;
subgraph cluster0 { a; b; c; d }
subgraph cluster_1 { e; subgraph cluster_2 { f } }
subgraph cluster_3 { }
{ d }
{ e }
{ }
f -> e;
g -> h [minlen=2];
d -> c;
i -> h [label=x];
subgraph s { rank=source; a; h; subgraph cluster_in { } }
}`;
  expectThrow(gv, () => gv.render(layout, 'dot', 'svg'), 'Layout', 'Layout computation error', true);
  expectThrow(gv, () => gv.renderWithDiagnostics(layout, 'dot', 'svg'), 'Layout', 'Layout computation error', true);
  expectThrow(gv, () => gv.inspectGraph(layout, 'dot'), 'Layout', 'Layout computation error', true);

  const bytes = gv.renderAllBytesWithDiagnostics(new TextEncoder().encode('digraph { a -> b }'), 'dot', 'svg');
  if (!(bytes.output instanceof Uint8Array) || bytes.output.length === 0 || !Array.isArray(bytes.diagnostics)) {
    throw new Error('renderAllBytesWithDiagnostics');
  }
  if (typeof gv.renderSvg('digraph { a -> b }', 'dot') !== 'string') throw new Error('renderSvg');
  if (typeof gv.renderJson('digraph { a -> b }', 'dot') !== 'string') throw new Error('renderJson');
  if (typeof gv.version() !== 'string' || !gv.version().startsWith('graphviz version 16.1.0')) {
    throw new Error(`version ${gv.version()}`);
  }
  checkLimits(gv);
}

// The `options` argument (#798): the Rust `Limits` as plain fields, `signal` and
// `shouldCancel` for cancellation.
function checkLimits(gv) {
  const src = 'digraph { a -> b; b -> c; c -> d; d -> a }';
  const bytes = new TextEncoder().encode(src);
  const plain = gv.render(src, 'neato', 'svg');
  const roomy = {
    maxInputBytes: 1 << 20, maxNodes: 100, maxEdges: 100, maxSteps: Number.MAX_SAFE_INTEGER,
    maxAllocation: 1 << 30, maxMemory: Number.MAX_SAFE_INTEGER, timeout: 60_000,
    signal: new AbortController().signal, shouldCancel: () => false,
  };
  for (const options of [undefined, null, {}, roomy]) {
    if (gv.render(src, 'neato', 'svg', options) !== plain) {
      throw new Error(`options ${JSON.stringify(options)} changed the bytes`);
    }
  }
  if (gv.renderAllBytes(bytes, 'dot', 'plain', roomy).join() !== gv.renderAllBytes(bytes, 'dot', 'plain').join()) {
    throw new Error('renderAllBytes options changed the bytes');
  }

  const calls = (options) => [
    () => gv.render(src, 'dot', 'svg', options),
    () => gv.renderWithDiagnostics(src, 'dot', 'svg', options),
    () => gv.renderAll(src, 'dot', 'svg', options),
    () => gv.renderAllWithDiagnostics(src, 'dot', 'svg', options),
    () => gv.renderAllBytes(bytes, 'dot', 'svg', options),
    () => gv.renderAllBytesWithDiagnostics(bytes, 'dot', 'svg', options),
    () => gv.renderSvg(src, 'dot', options),
    () => gv.renderJson(src, 'dot', options),
    () => gv.inspectGraph(src, 'dot', options),
  ];
  for (const fn of calls({ maxNodes: 2 })) expectLimit(gv, fn, 'Nodes');
  for (const fn of calls({ maxEdges: 1 })) expectLimit(gv, fn, 'Edges');
  for (const fn of calls({ maxInputBytes: 4 })) expectLimit(gv, fn, 'InputBytes');
  for (const fn of calls({ timeout: 0 })) expectLimit(gv, fn, 'Deadline');
  expectLimit(gv, () => gv.render(src, 'neato', 'svg', { maxSteps: 1 }), 'Steps');
  // A partial multi-graph render is not returned under a limit.
  expectLimit(gv, () => gv.renderAllBytes(new TextEncoder().encode(`digraph { x }\n${src}`), 'dot', 'svg', { maxNodes: 2 }), 'Nodes');

  // The work budget is deterministic: the same budget stops at the same point.
  const outcome = (steps) => {
    try {
      return gv.render(src, 'neato', 'svg', { maxSteps: steps });
    } catch (e) {
      return `${e.kind}:${e.limit}`;
    }
  };
  for (const steps of [1, 50, 500, 5000]) {
    if (outcome(steps) !== outcome(steps)) throw new Error(`maxSteps ${steps} is not deterministic`);
  }

  const aborted = new AbortController();
  aborted.abort();
  for (const fn of calls({ signal: aborted.signal })) expectThrow(gv, fn, 'Cancelled', 'Cancelled: the call was cancelled');
  for (const fn of calls({ shouldCancel: () => true })) expectThrow(gv, fn, 'Cancelled', 'cancelled');

  // Aborted while the call runs, from a callback inside it.
  const controller = new AbortController();
  let polls = 0;
  expectThrow(gv, () => gv.render(src, 'neato', 'svg', {
    signal: controller.signal,
    shouldCancel: () => {
      if (++polls === 2) controller.abort();
      return false;
    },
  }), 'Cancelled', 'cancelled');
  if (polls < 2) throw new Error(`shouldCancel polled ${polls} times`);

  // A callback may call back in, and its exception is rethrown as itself.
  let inner;
  const outer = gv.render(src, 'dot', 'svg', {
    shouldCancel: () => {
      inner ??= gv.render('digraph { x }', 'dot', 'plain', { maxNodes: 5 });
      return false;
    },
  });
  if (!outer.includes('<svg') || !inner?.startsWith('graph ')) throw new Error('re-entrant render');
  const boom = new Error('boom');
  try {
    gv.render(src, 'dot', 'svg', { shouldCancel: () => { throw boom; } });
    throw new Error('a throwing shouldCancel did not throw');
  } catch (e) {
    if (e !== boom) throw new Error(`a throwing shouldCancel threw ${e}`);
  }
  if (gv.render(src, 'neato', 'svg') !== plain) throw new Error('state leaked after a throw');

  for (const bad of ['x', 3, { maxNodes: -1 }, { maxNodes: 1.5 }, { maxSteps: '9' }, { timeout: -1 },
    { signal: 5 }, { signal: null }, { shouldCancel: 1 }]) {
    try {
      gv.render(src, 'dot', 'svg', bad);
    } catch (e) {
      if (e instanceof TypeError) continue;
      throw new Error(`options ${JSON.stringify(bad)} threw ${e}`);
    }
    throw new Error(`options ${JSON.stringify(bad)} did not throw`);
  }
}

function expectLimit(gv, fn, limit) {
  try {
    fn();
  } catch (e) {
    assertGraphvizError(gv, e, 'LimitExceeded', 'Limit exceeded: ', false);
    if (e.limit !== limit) throw new Error(`limit ${e.limit}, wanted ${limit}: ${e.message}`);
    return;
  }
  throw new Error(`${limit} did not throw`);
}

function expectThrow(gv, fn, kind, messagePart, wantsDiagnostics = false) {
  try {
    fn();
  } catch (e) {
    assertGraphvizError(gv, e, kind, messagePart, wantsDiagnostics);
    return;
  }
  throw new Error(`${kind} did not throw`);
}

function assertGraphvizError(gv, e, kind, messagePart, wantsDiagnostics) {
  if (typeof gv.GraphvizError !== 'function') throw new Error('GraphvizError is not exported');
  if (!(e instanceof gv.GraphvizError)) throw new Error(`${kind}: not instanceof GraphvizError`);
  if (!(e instanceof Error)) throw new Error(`${kind}: not an Error (${typeof e})`);
  if (e.name !== 'GraphvizError') throw new Error(`${kind}: name ${e.name}`);
  if (e.status !== 1) throw new Error(`${kind}: status ${e.status}`);
  if (e.kind !== kind) throw new Error(`${kind}: kind ${e.kind} message ${e.message}`);
  if (typeof e.message !== 'string' || !e.message.includes(messagePart)) {
    throw new Error(`${kind}: message ${e.message}`);
  }
  if (!Array.isArray(e.diagnostics)) throw new Error(`${kind}: diagnostics`);
  if (wantsDiagnostics && e.diagnostics.length === 0) {
    throw new Error(`${kind}: expected diagnostics, got none`);
  }
}
