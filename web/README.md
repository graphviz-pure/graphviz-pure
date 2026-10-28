# @graphviz-pure/wasm

A Rust + WebAssembly port of [Graphviz](https://graphviz.org) 16.1.0: the DOT parser, all eight
layout engines (dot, neato, fdp, sfdp, twopi, circo, osage, patchwork) and the svg, json, dot,
xdot and plain renderers, with no C. Every build of graphviz-pure computes with the same math
library, so its output is the same bytes in every browser, in Node.js, Deno and Bun, and as the
native `graphviz-pure` command on Linux, Windows and macOS. graphviz-pure is an independent port, not affiliated with or endorsed by the Graphviz
project.

```js
import init, { render } from '@graphviz-pure/wasm';

await init();
const svg = render('digraph G { a -> b [label="fast"]; }', 'dot', 'svg');
```

SVG from untrusted DOT is untrusted markup: like C Graphviz, `URL`/`href` attributes can carry `javascript:` links, so sanitise it before embedding it inline (including with `innerHTML`), or display it with `<img>`. Layout cost is unbounded, as in C; services must impose input-size, time and memory limits in an isolated worker or process with a timeout.

`init()` loads the WebAssembly module (1.9 MB, about 650 KB gzipped) once; call it before
anything else. Where it finds the module depends on the runtime. CI runs each setup below
(`scripts/wasm-runtimes/`).

**Browser.** `init()` fetches `pkg/graphviz_pure_wasm_bg.wasm` from next to the JavaScript
module, so serve both, the `.wasm` as `application/wasm`. If your bundler does not copy the
`.wasm` file, copy it yourself and pass its URL:
`await init({ module_or_path: '/assets/graphviz_pure_wasm_bg.wasm' })`.

**Node.js.** `init()` reads the file from the installed package. To load it yourself, pass the
bytes:

```js
import { readFile } from 'node:fs/promises';
import init from '@graphviz-pure/wasm';

const url = new URL(import.meta.resolve('@graphviz-pure/wasm/graphviz_pure_wasm_bg.wasm'));
await init({ module_or_path: await readFile(url) });
```

**Deno.** Import from JSR or npm, then `await init()`:

```js
import init, { render } from 'jsr:@graphviz-pure/wasm'; // init() fetches the .wasm: --allow-net=jsr.io
import init, { render } from 'npm:@graphviz-pure/wasm'; // init() reads the .wasm: --allow-read
```

**Bun.** `bun add @graphviz-pure/wasm`, then `await init()`.

**Cloudflare Workers.** Import the `.wasm` file as a module (wrangler compiles it) and
instantiate it with `initSync`:

```js
import { initSync, render } from '@graphviz-pure/wasm';
import wasm from '@graphviz-pure/wasm/graphviz_pure_wasm_bg.wasm';

initSync({ module: wasm });

export default {
  async fetch(request) {
    const svg = render(await request.text(), 'dot', 'svg');
    return new Response(svg, { headers: { 'content-type': 'image/svg+xml' } });
  },
};
```

Before serving untrusted requests with this Worker, cap the request-body size and configure
a [CPU timeout](https://developers.cloudflare.com/workers/wrangler/configuration/#limits),
for example `limits.cpu_ms = 1000` in `wrangler.toml` on a paid plan (enforced after deployment);
the Free plan allows only [10 ms of CPU time per request](https://developers.cloudflare.com/workers/platform/limits/#cpu-time).
`render` is synchronous, so `setTimeout` or `Promise.race` on the same thread cannot interrupt it.

`render(dot, engine, format)` lays out the first graph and returns the output string.
`renderAll` returns one string per graph. `renderAllBytes` takes and returns raw bytes
(`Uint8Array`), including non-UTF-8 input, as the command line does. `renderSvg` and
`renderJson` are `render` with the format filled in.

`renderWithDiagnostics`, `renderAllWithDiagnostics` and `renderAllBytesWithDiagnostics`
return `{ output, diagnostics }`. `output` is what the plain function returns.
`diagnostics` is an array of `{ level, message }`. `level` is `"Error"`, `"Warning"` or
`"CountedAsError"`. `message` is the text of that one message, including the `Error: ` or
`Warning: ` prefix and the trailing newline. A warning does not throw. A layout that
succeeds and still reports an error-level diagnostic does not throw either: `dot` would
exit 1, and the diagnostic is on this object. `render` drops the diagnostics and still
returns the string.

A failure throws `GraphvizError`, a class the package exports (`extends Error`), so
`instanceof GraphvizError` is true. `message` is the text the command line prints,
without the program-name prefix. `status` is a number and is currently always `1`,
the status C's `dot` exits with when one graph fails. `kind` is `"Syntax"`,
`"UnsupportedEngine"`, `"UnsupportedFormat"`, `"Layout"`, `"Attribute"` or `"Render"`.
`diagnostics` is the `agerr` messages emitted around the failure. The failure's own
text is `message`, not one of those entries. The plain functions and the
`*WithDiagnostics` functions throw the same object.

`supportedEngines()` and `supportedFormats()` return arrays of strings.
`supportedFormats()` lists every format name `render` accepts, including aliases:
`svg`, `svg_inline`, `cmapx`, `json`, `json0`, `dot`, `xdot`, `canon`, `plain`,
`cmapx_np`, `gv`, `xdot1.2`, `xdot1.4`, `dot_json`, `xdot_json` and `plain-ext`.
Raster formats (`png`, `jpg`, `jpeg`) are not available in this package.

`inspectGraph(dot, engine)` returns
`{ name, directed, strict, nodes, edges, subgraphs, width_pt, height_pt }`.
The fields are snake_case, the names they had as JSON keys. The function names are
camelCase. `width_pt` and `height_pt` are the laid-out size in points. Warnings from
the layout are not on this object.

`version()` returns `graphviz version 16.1.0 (graphviz-pure X.Y.Z)`, where `X.Y.Z` is the
package version compiled into the WebAssembly module. This follows the CLI's `-V` format
without the command name prefix.

`pkg/graphviz_pure_wasm.d.ts` is hand-checked and is the file the npm `types` entry and
the JSR package publish. CI typechecks a sample against it (`deno check`, Deno 2.9.6).

Each release is built by GitHub Actions and published with npm provenance; check it with
`npm audit signatures`. Documentation, fidelity evidence and the source:
https://graphviz-pure.github.io/ and https://github.com/graphviz-pure/graphviz-pure. Licence: EPL-2.0 AND MIT.
The package ships `NOTICE` and the licence texts in `LICENSES/`.
