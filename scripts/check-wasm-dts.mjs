#!/usr/bin/env node
// Fail when wasm-bindgen's generated declarations drift from the hand-checked
// web/graphviz_pure_wasm.d.ts: exported function, class and const names, plus
// the InitInput, SyncInitInput, InitOutput, initSync and default init signatures.
// GraphvizError is declared by hand and is the one class the generator does not emit.
import { readFileSync } from 'node:fs';

const [generatedPath, handPath] = process.argv.slice(2);
if (!generatedPath || !handPath) {
  console.error('usage: check-wasm-dts.mjs <generated.d.ts> <web/graphviz_pure_wasm.d.ts>');
  process.exit(2);
}

const strip = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const norm = (s) =>
  strip(s).replace(/__wbg_init/g, 'init').replace(/\s+/g, ' ').replace(/ (\()/g, '$1').trim();

function names(text) {
  const src = strip(text);
  const grab = (re) => [...src.matchAll(re)].map((m) => m[1]);
  return {
    functions: grab(/^export function (\w+)/gm),
    classes: grab(/^export class (\w+)/gm),
    consts: grab(/^export const (\w+)/gm),
    hasDefault: /^export default function /m.test(src),
  };
}

function declaration(text, kind, name) {
  const src = strip(text);
  const start = kind === 'interface' || kind === 'type'
    ? src.search(new RegExp(`^export ${kind} ${name}\\b`, 'm'))
    : src.search(new RegExp(`^export function ${name}\\b`, 'm'));
  if (start < 0) return null;
  if (kind === 'function') {
    const end = src.indexOf(';', start);
    return end < 0 ? null : src.slice(start, end + 1);
  }
  let depth = 0;
  let seen = false;
  for (let i = start; i < src.length; i++) {
    if (src[i] === '{') {
      depth++;
      seen = true;
    } else if (src[i] === '}') {
      depth--;
      if (seen && depth === 0) return src.slice(start, i + 1);
    } else if (!seen && src[i] === ';') {
      return src.slice(start, i + 1);
    }
  }
  return null;
}

function defaultInit(text) {
  const src = strip(text);
  const start = src.search(/^export default function /m);
  if (start < 0) return null;
  const end = src.indexOf(';', start);
  return end < 0 ? null : src.slice(start, end + 1);
}

const generated = readFileSync(generatedPath, 'utf8');
const hand = readFileSync(handPath, 'utf8');
const g = names(generated);
const h = names(hand);
const problems = [];

function sameSet(label, left, right) {
  const missing = left.filter((n) => !right.includes(n));
  const extra = right.filter((n) => !left.includes(n));
  if (missing.length || extra.length) {
    problems.push(
      `${label}: generated ${left.join(', ') || '(none)'}; hand-checked ${right.join(', ') || '(none)'}`,
    );
  }
}

sameSet('functions', g.functions, h.functions);
sameSet('consts', g.consts, h.consts);
const handClasses = h.classes.filter((n) => n !== 'GraphvizError');
sameSet('classes', g.classes, handClasses);
if (!h.classes.includes('GraphvizError')) problems.push('hand-checked d.ts does not export class GraphvizError');
if (g.hasDefault !== h.hasDefault) problems.push('default init export differs');

for (const [kind, name] of [
  ['type', 'InitInput'],
  ['type', 'SyncInitInput'],
  ['interface', 'InitOutput'],
]) {
  const a = declaration(generated, kind, name);
  const b = declaration(hand, kind, name);
  if (a == null || b == null || norm(a) !== norm(b)) {
    problems.push(`${name} signature differs\n  generated: ${a}\n  hand:      ${b}`);
  }
}
const initA = declaration(generated, 'function', 'initSync');
const initB = declaration(hand, 'function', 'initSync');
if (initA == null || initB == null || norm(initA) !== norm(initB)) {
  problems.push(`initSync signature differs\n  generated: ${initA}\n  hand:      ${initB}`);
}
const defA = defaultInit(generated);
const defB = defaultInit(hand);
if (defA == null || defB == null || norm(defA) !== norm(defB)) {
  problems.push(`default init signature differs\n  generated: ${defA}\n  hand:      ${defB}`);
}

if (problems.length) {
  console.error(problems.join('\n\n'));
  process.exit(1);
}
console.log('hand-checked wasm types match wasm-bindgen exports');
