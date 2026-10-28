import { issues, severityRank, ENGINES, e2eSkips, type Issue } from './data';

const engineRank = (e: string) => {
	const i = ENGINES.indexOf(e);
	return i < 0 ? ENGINES.length : i;
};
export const sortIssues = (list: Issue[]) =>
	[...list].sort((a, b) => severityRank(a.severity) - severityRank(b.severity) || engineRank(a.engine) - engineRank(b.engine) || a.slug.localeCompare(b.slug));

/**
 * The classes a difference may fall into (F2 of docs/roadmap/1.0.md): the reason C's output
 * cannot be matched byte for byte. A difference with no class is an open difference in the
 * port, one the port should fix.
 */
export const CLASSES = [
	{ key: 'c-nondeterminism', label: 'C nondeterminism or undefined behaviour', short: 'C nondeterminism' },
	{ key: 'c-crash', label: 'A C crash or assertion', short: 'C crash' },
	{ key: 'c-error-path', label: "C's error path", short: 'C error path' },
	{ key: 'platform-libm', label: 'The platform libm', short: 'platform libm' },
] as const;
export type ClassKey = (typeof CLASSES)[number]['key'];
export const classLabel = (key?: string) => CLASSES.find((c) => c.key === key)?.label ?? key ?? '';
export const classShort = (key?: string) => CLASSES.find((c) => c.key === key)?.short ?? key ?? '';

/** Open differences in the port: the port should change. */
export const portBugs = sortIssues(issues.filter((i) => !i.class));

/** Accepted differences: C's output is what it is, and the port should not reproduce it. */
export const accepted = sortIssues(issues.filter((i) => i.class));
export const acceptedIn = (key: string) => sortIssues(accepted.filter((i) => i.class === key));

export const skipped = [...e2eSkips].sort((a, b) => a.input.localeCompare(b.input));

/** Files named c.<ext> and rust.<ext> with the same extension, the first pair found. */
export function pair(i: Issue) {
	const order = ['svg', 'png', 'json', 'dot', 'xdot', 'txt', 'plain'];
	const exts = i.files.filter((f) => f.startsWith('c.') && !f.includes('stderr')).map((f) => f.slice(2));
	exts.sort((a, b) => (order.indexOf(a) + 99) % 99 - (order.indexOf(b) + 99) % 99);
	const ext = exts.find((e) => i.files.includes(`rust.${e}`));
	return ext ? { ext, kind: ext === 'svg' ? 'svg' : ext === 'png' ? 'png' : 'text' } as const : undefined;
}
