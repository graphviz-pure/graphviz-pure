#!/usr/bin/env node
// JSR accepts one SPDX identifier, not an SPDX expression. Deno's publish dry run
// does not enforce this server-side rule. Keep the npm expression in package.json.
import { readFileSync } from 'node:fs';

const file = process.argv[2] ?? 'web/jsr.json';
const { license } = JSON.parse(readFileSync(file, 'utf8'));
if (typeof license !== 'string' || !/^[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*$/.test(license)) {
  console.error(`${file}: JSR license must be a single SPDX identifier, not an expression: ${JSON.stringify(license)}`);
  process.exit(1);
}
console.log(`${file}: single JSR license identifier (${license})`);
