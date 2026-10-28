// A Cloudflare Worker using the package as the README shows: wrangler bundles the .wasm as a
// compiled WebAssembly.Module, and initSync instantiates it. POST a DOT file to /?engine=NAME
// and get the SVG back. GET /?check=1 runs the JS shape checks.
import * as gv from '@graphviz-pure/wasm';
import wasm from '@graphviz-pure/wasm/graphviz_pure_wasm_bg.wasm';
import { checkApi } from '../api-check.mjs';

gv.initSync({ module: wasm });

let apiFailure = null;
try {
  checkApi(gv);
} catch (e) {
  apiFailure = e instanceof Error ? e.message : String(e);
}

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.searchParams.has('check')) {
      return new Response(apiFailure ?? 'ok', { status: apiFailure ? 500 : 200 });
    }
    if (apiFailure) return new Response(apiFailure, { status: 500 });
    const engine = url.searchParams.get('engine') ?? 'dot';
    try {
      const svg = gv.renderAllBytes(new Uint8Array(await request.arrayBuffer()), engine, 'svg');
      return new Response(svg, { headers: { 'content-type': 'image/svg+xml' } });
    } catch (e) {
      // The full error goes to the Worker's log; the response carries only its message.
      console.error(e);
      return new Response(e instanceof Error ? e.message : 'render failed', { status: 500 });
    }
  },
};
