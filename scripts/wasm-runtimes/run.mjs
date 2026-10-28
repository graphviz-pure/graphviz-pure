// Checks that the npm package works in every runtime its description names: it packs web/ (as
// `npm publish` would), installs the tarball into node_modules, loads it the way a user of each
// runtime would, and renders every gallery sample,
// byte for byte against site/public/samples/NAME/rust.svg (as scripts/release-smoke-wasm.mjs
// does for Node). Deno, Bun, wrangler (workerd) and the Playwright driver come from this
// directory's package-lock.json; the browser is the installed Google Chrome, or CHROME_PATH.
//
// Build the package first (web/package.json `build:wasm`), then:
//   cd scripts/wasm-runtimes && npm ci && node run.mjs [node|deno|bun|workers|browser ...]
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';
import { differing, loadSamples } from './samples.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const bin = (name) => path.join(here, 'node_modules/.bin', name);
const installed = path.join(here, 'node_modules/@graphviz-pure/wasm');

// npm pack, so the check sees exactly the published files (`files` and `exports` included).
function installPackage() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'gvwasm-pack-'));
  const pack = spawnSync('npm', ['pack', path.join(root, 'web'), '--pack-destination', tmp, '--json'], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (pack.status !== 0) throw new Error('npm pack failed');
  const tarball = path.join(tmp, JSON.parse(pack.stdout)[0].filename);
  fs.rmSync(installed, { recursive: true, force: true });
  fs.mkdirSync(installed, { recursive: true });
  const tar = spawnSync('tar', ['-xzf', tarball, '-C', installed, '--strip-components', '1'], { stdio: 'inherit' });
  if (tar.status !== 0) throw new Error('tar failed');
  fs.rmSync(tmp, { recursive: true, force: true });
}

// One process per load form (local.mjs). Returns a list of failures.
function local(label, cmd, args) {
  const failures = [];
  for (const [how, extra] of args) {
    const r = spawnSync(cmd, [...extra, 'local.mjs', how], {
      cwd: here,
      encoding: 'utf8',
      env: { ...process.env, DENO_NO_UPDATE_CHECK: '1', NO_COLOR: '1' },
    });
    const out = (r.stdout + r.stderr).trim();
    console.log(`  ${label} ${out.split('\n').join('\n    ')}`);
    if (r.status !== 0) failures.push(`${label} ${how}`);
  }
  return failures;
}

const checks = {
  node: () => local('node', process.execPath, [['npm', []], ['bytes', []]]),
  // --allow-read: Deno's fetch of a file: URL, and node:fs, need it.
  deno: () => local('deno', bin('deno'), [['jsr', ['run', '--allow-read', '--no-lock']], ['npm', ['run', '--allow-read', '--no-lock']]]),
  bun: () => local('bun', bin('bun'), [['npm', ['run']], ['jsr', ['run']]]),

  async workers(samples) {
    const { unstable_startWorker } = await import('wrangler');
    const worker = await unstable_startWorker({
      config: path.join(here, 'worker/wrangler.jsonc'),
      dev: { server: { hostname: '127.0.0.1', port: 0 }, inspector: false, watch: false, logLevel: 'warn' },
    });
    try {
      await worker.ready;
      const check = await worker.fetch('http://localhost/?check=1');
      if (!check.ok) throw new Error(`worker api: ${await check.text()}`);
      let slowest = 0;
      const bad = await differing(samples, async (input, engine) => {
        const t = performance.now();
        const res = await worker.fetch(`http://localhost/?engine=${engine}`, { method: 'POST', body: input });
        slowest = Math.max(slowest, performance.now() - t);
        if (!res.ok) throw new Error(`worker: ${res.status} ${await res.text()}`);
        return new Uint8Array(await res.arrayBuffer());
      });
      console.log(`  workers: ${samples.length - bad.length}/${samples.length} samples byte-identical (slowest request ${slowest.toFixed(0)} ms wall)`);
      if (bad.length) console.log(`    differ: ${bad.join(', ')}`);
      return bad.length ? ['workers'] : [];
    } finally {
      await worker.dispose();
    }
  },

  async browser(samples) {
    const { chromium } = await import('playwright-core');
    const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm' };
    const server = http.createServer((req, res) => {
      const url = new URL(req.url, 'http://x').pathname;
      const file = url === '/' ? path.join(here, 'browser.html') : url === '/api-check.mjs' ? path.join(here, 'api-check.mjs') : url.startsWith('/pkg/') ? path.join(installed, url.slice(5)) : null;
      if (url === '/favicon.ico') return res.writeHead(204).end();
      if (!file || !file.startsWith(here) || !fs.existsSync(file)) return res.writeHead(404).end();
      res.writeHead(200, { 'content-type': types[path.extname(file)] ?? 'application/octet-stream' });
      fs.createReadStream(file).pipe(res);
    });
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: 'chrome' });
    try {
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', (e) => errors.push(String(e)));
      page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(() => document.title === 'ready' || null, null, { timeout: 30000 }).catch(() => {
        throw new Error(`browser: the package did not load: ${errors.join('; ')}`);
      });
      const bad = await differing(samples, async (input, engine) =>
        Uint8Array.from(await page.evaluate(([i, e]) => window.gvRender(i, e), [Array.from(input), engine])),
      );
      console.log(`  browser (${browser.version()}): ${samples.length - bad.length}/${samples.length} samples byte-identical`);
      if (bad.length) console.log(`    differ: ${bad.join(', ')}`);
      if (errors.length) console.log(`    page errors: ${errors.join('; ')}`);
      return bad.length || errors.length ? ['browser'] : [];
    } finally {
      await browser.close();
      server.close();
    }
  },
};

const wanted = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(checks);
for (const w of wanted) if (!checks[w]) throw new Error(`unknown runtime ${w}; one of ${Object.keys(checks).join(', ')}`);

installPackage();
const wasm = fs.readFileSync(path.join(installed, 'pkg/graphviz_pure_wasm_bg.wasm'));
const kb = (n) => `${(n / 1024).toFixed(0)} KiB`;
console.log(`package: ${kb(wasm.length)} wasm, ${kb(zlib.gzipSync(wasm, { level: 9 }).length)} gzipped`);

const samples = loadSamples();
const failures = [];
for (const w of wanted) {
  console.log(w);
  try {
    failures.push(...(await checks[w](samples)));
  } catch (e) {
    console.log(`  ${w}: ${e.stack ?? e}`);
    failures.push(w);
  }
}
if (failures.length) {
  console.error(`FAILED: ${failures.join(', ')}`);
  process.exit(1);
}
console.log(`all runtimes passed: ${wanted.join(', ')}`);
