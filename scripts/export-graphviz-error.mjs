#!/usr/bin/env node
// wasm-bindgen imports the snippet that constructs GraphvizError and does not
// re-export the class. Append that re-export so `instanceof GraphvizError`
// sees the same function the throw uses. Idempotent.
import { readFileSync, writeFileSync } from 'node:fs';

const file = process.argv[2];
if (!file) {
  console.error('usage: export-graphviz-error.mjs <graphviz_pure_wasm.js>');
  process.exit(2);
}
let text = readFileSync(file, 'utf8');
if (/^exports\.GraphvizError\s*=/m.test(text) || /^export\s*\{\s*GraphvizError\s*\}/m.test(text)) {
  process.exit(0);
}
const esm = text.match(/from (['"])(\.\/snippets\/[^'"]*inline0\.js)\1/);
const cjs = text.match(/require\(String\.raw`(\.\/snippets\/[^`]*inline0\.js)`\)/);
if (esm) {
  text += `\nexport { GraphvizError } from ${esm[1]}${esm[2]}${esm[1]};\n`;
} else if (cjs) {
  text += `\nexports.GraphvizError = require(String.raw\`${cjs[1]}\`).GraphvizError;\n`;
} else {
  console.error(`export-graphviz-error: no inline snippet import in ${file}`);
  process.exit(1);
}
writeFileSync(file, text);
