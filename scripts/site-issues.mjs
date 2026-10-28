#!/usr/bin/env node
// Copies the open-issue manifest into the site.
//
//   node scripts/site-issues.mjs [--manifest docs/issues/manifest.json] [--filed docs/issues/filed.json]
//
// Reads the manifest (an array of {slug, dir, title, labels, engine, category, severity,
// seeds, case_count, upstream_c, close_as_wontfix, body}), writes it to
// site/src/data/issues.json with `files` (what was copied) and, when filed.json maps the
// slug to a URL, `issue_url`, and copies each entry's directory into
// site/public/whats-left/<slug>/.
//
// It also copies docs/issues/e2e-skips.json (the corpus inputs `gv-diff e2e` skips, each with
// its cause) to site/src/data/e2e-skips.json, and checks that the list names exactly the
// inputs the golden records as unstable, so the page cannot drift from the check.
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opt = (name, dflt) => {
	const i = args.indexOf(name);
	return i >= 0 ? resolve(args[i + 1]) : dflt;
};

const manifestPath = opt('--manifest', join(root, 'docs/issues/manifest.json'));
const filedPath = opt('--filed', join(root, 'docs/issues/filed.json'));
const skipsPath = opt('--skips', join(root, 'docs/issues/e2e-skips.json'));
const openPath = opt('--open', join(root, 'docs/issues/open-fidelity.json'));
const goldenPath = opt('--golden', join(root, 'tests/golden/e2e.libm-matched.json'));
const outData = join(root, 'site/src/data/issues.json');
const outSkips = join(root, 'site/src/data/e2e-skips.json');
const outOpen = join(root, 'site/src/data/open-fidelity.json');
const outPublic = join(root, 'site/public/whats-left');

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
if (!Array.isArray(manifest)) throw new Error(`${manifestPath}: expected an array`);
const filed = existsSync(filedPath) ? JSON.parse(readFileSync(filedPath, 'utf8')) : {};

const required = ['slug', 'dir', 'title', 'body'];
const seen = new Set();
rmSync(outPublic, { recursive: true, force: true });
const out = manifest.map((issue) => {
	for (const k of required) if (!issue[k]) throw new Error(`issue ${issue.slug ?? '?'}: missing ${k}`);
	if (!/^[a-z0-9][a-z0-9-]*$/.test(issue.slug)) throw new Error(`bad slug ${issue.slug}`);
	if (seen.has(issue.slug)) throw new Error(`duplicate slug ${issue.slug}`);
	seen.add(issue.slug);
	const src = resolve(root, issue.dir);
	const dst = join(outPublic, issue.slug);
	mkdirSync(dst, { recursive: true });
	const files = existsSync(src)
		? readdirSync(src).filter((f) => !f.startsWith('.') && statSync(join(src, f)).isFile()).sort()
		: [];
	if (!files.length) console.warn(`warning: ${issue.slug}: no files in ${issue.dir}`);
	for (const f of files) copyFileSync(join(src, f), join(dst, f));
	// The body is written for GitHub; on the site its own page is the visual diff.
	const body = issue.body.replace(/^Visual diff: \{\{SITE\}\}\S*\n?/m, '').replace(/\{\{SITE\}\}/g, '/graphviz-pure');
	const entry = { ...issue, body, files };
	if (filed[issue.slug]) entry.issue_url = filed[issue.slug];
	return entry;
});

mkdirSync(dirname(outData), { recursive: true });
writeFileSync(outData, JSON.stringify(out, null, 1) + '\n');

// The inputs `gv-diff e2e` skips, with the cause of each. The list must name exactly the
// inputs the golden records as unstable: a skipped input without a cause is an F2 gap.
const skips = JSON.parse(readFileSync(skipsPath, 'utf8'));
if (!Array.isArray(skips)) throw new Error(`${skipsPath}: expected an array`);
const named = skips.map((s) => s.input);
if (new Set(named).size !== named.length) throw new Error(`${skipsPath}: duplicate input`);
if (existsSync(goldenPath)) {
	const golden = JSON.parse(readFileSync(goldenPath, 'utf8'));
	const unstable = new Set(golden.unstable ?? []);
	const missing = [...unstable].filter((i) => !named.includes(i));
	const extra = named.filter((i) => !unstable.has(i));
	if (missing.length || extra.length) {
		throw new Error(
			`${relative(root, skipsPath)} does not match the ${unstable.size} unstable inputs of ` +
				`${relative(root, goldenPath)}` +
				(missing.length ? `; without a cause: ${missing.join(', ')}` : '') +
				(extra.length ? `; not skipped by gv-diff e2e: ${extra.join(', ')}` : ''),
		);
	}
}
writeFileSync(outSkips, JSON.stringify(skips, null, 1) + '\n');

// docs/porting/fuzzing.md lists the same inputs with their causes; the two must agree.
const fuzzing = readFileSync(join(root, 'docs/porting/fuzzing.md'), 'utf8');
const unlisted = named.filter((i) => !fuzzing.includes('`' + i + '`'));
if (unlisted.length) {
	throw new Error(
		`docs/porting/fuzzing.md does not list these skipped inputs: ${unlisted.join(', ')}`,
	);
}

console.log(
	`site-issues: ${out.length} issues from ${relative(root, manifestPath)}` +
		`${Object.keys(filed).length ? `, ${out.filter((i) => i.issue_url).length} filed` : ''}` +
		`, ${skips.length} skipped e2e inputs`,
);

// The open issues labelled `fidelity`, so that the page lists every one of them (F5).
const open = JSON.parse(readFileSync(openPath, 'utf8'));
if (!Array.isArray(open.issues)) throw new Error(`${openPath}: expected an "issues" array`);
writeFileSync(outOpen, JSON.stringify(open.issues, null, 1) + '\n');
console.log(`site-issues: ${open.issues.length} open fidelity issues`);
