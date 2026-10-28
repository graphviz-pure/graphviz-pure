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

The Worker uploads at about 1.9 MB, far inside the 64 MiB size limit. Most graphs lay out in a
few milliseconds, but large ones take tens, more than the Free plan's 10 ms of CPU per request.

`render(dot, engine, format)` lays out the first graph; `renderAll` returns one output per
graph; `renderAllBytes` takes and returns raw bytes, including non-UTF-8 input, as the
command line does.

Each release is built by GitHub Actions and published with npm provenance; check it with
`npm audit signatures`. Documentation, fidelity evidence and the source:
https://graphviz-pure.github.io/ and https://github.com/graphviz-pure/graphviz-pure. Licence: EPL-2.0 AND MIT.
The package ships `NOTICE` and the licence texts in `LICENSES/`.
