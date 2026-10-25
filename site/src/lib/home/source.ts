/**
 * The front page's graph, as DOT. The same function runs at build time (for the static
 * drawing every visitor sees first) and in the browser (for every layout after that), so
 * the page never draws anything the port did not lay out.
 *
 * Everything that varies is a Graphviz attribute: the theme is bgcolor/color/fontcolor,
 * the viewport's shape is rankdir, the selected engine is a fillcolor, a node you have
 * dragged is pos="x,y!" with style=bold.
 */

export const ENGINES = ['dot', 'neato', 'fdp', 'sfdp', 'twopi', 'circo', 'osage', 'patchwork'] as const;
export type Engine = (typeof ENGINES)[number];

export interface Figures {
	/** files byte-identical to C computing with the same math functions, of `total` */
	identical: number;
	total: number;
	/** files byte-identical to stock C on macOS (Apple's libm), of `stockTotal`; informational */
	stock: number;
	stockTotal: number;
	/** root-relative URLs for the link nodes, already under the site's base */
	links: Record<Exclude<LinkId, 'platforms'>, string> & { platforms?: string };
}
export type LinkId = 'fidelity' | 'gallery' | 'verify' | 'playground' | 'left' | 'api' | 'notes' | 'platforms';

export interface SourceOptions {
	engine: Engine;
	rankdir: 'LR' | 'TB';
	theme: 'light' | 'dark';
	/** Every node's position, in points, when neato is following a drag. */
	positions?: Record<string, [number, number]>;
	/** Nodes held in place: pos="x,y!" and drawn bold. */
	pinned?: Iterable<string>;
}

/** X11 colour names, the way the Graphviz gallery uses them. Dark mode inverts the sheet. */
const PALETTE = {
	light: {
		bg: 'white',
		ink: 'black',
		spoke: 'gray60',
		cFill: 'lightgrey',
		cNode: 'white',
		rsPen: 'blue',
		rsFill: 'lightblue2',
		ask: 'lightyellow',
		pick: 'lightgoldenrod1',
		kPen: 'gray50',
	},
	dark: {
		bg: '#15181d',
		ink: 'gray88',
		spoke: 'gray40',
		cFill: 'gray22',
		cNode: 'gray12',
		rsPen: 'royalblue1',
		rsFill: 'skyblue4',
		ask: 'lightgoldenrod4',
		pick: 'goldenrod3',
		kPen: 'gray45',
	},
};

const n = (v: number) => v.toLocaleString('en-US');
/**
 * A DOT quoted string: backslashes are escaped first, then the quotes, so nothing in the
 * argument can end the string early. Every argument is a constant from this file or a link path
 * fixed at build time, never user input. The labels' line breaks are DOT's `\n`, written by
 * `lines()` between the escaped parts, never inside one.
 */
const esc = (s: string) => s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
const q = (s: string) => `"${esc(s)}"`;
const lines = (...parts: readonly string[]) => `"${parts.map(esc).join('\\n')}"`;

/** The ids of the nodes that are controls: link nodes navigate, engine nodes switch -K. */
export const LINK_NODES: Record<string, LinkId> = {
	Fidelity: 'fidelity',
	Gallery: 'gallery',
	Verify: 'verify',
	Playground: 'playground',
	left: 'left',
	API: 'api',
	notes: 'notes',
	xp: 'platforms',
};
export const engineNode = (e: string) => `K_${e}`;
export const nodeEngine = (name: string) => (name.startsWith('K_') ? (name.slice(2) as Engine) : undefined);

export function homeSource(f: Figures, o: SourceOptions): string {
	const p = PALETTE[o.theme];
	const pinned = new Set(o.pinned ?? []);
	const pw = o.engine === 'patchwork';
	// patchwork gives every node `area` square inches; size them so their labels fit.
	const area = (label: string) => (pw ? `area=${Math.max(0.5, ((label.replace(/\\n/g, '').length * 5.4 + 16) / 72) ** 2 * 1.4).toFixed(2)}` : '');
	// patchwork draws every node as a plain rectangle, so a record's fields read as one line.
	const rec = (a: string, b: string): readonly string[] => (pw ? [a, b] : [`{${a}|${b}}`]);
	const bold = (name: string) => (pinned.has(name) ? ', penwidth=2' : '');
	const at = (name: string) => {
		const xy = o.positions?.[name];
		return xy ? `, pos="${xy[0].toFixed(2)},${xy[1].toFixed(2)}${pinned.has(name) ? '!' : ''}"` : '';
	};
	const nd = (name: string, attrs: string, label?: readonly string[]) =>
		`${q(name)} [${[attrs, label ? `label=${lines(...label)}` : '', area(label ? label.join('\\n') : name), bold(name), at(name)]
			.map((s) => s.replace(/^, /, ''))
			.filter(Boolean)
			.join(', ')}]`;
	const link = (name: string, label: string, id: LinkId) => nd(name, `URL=${q(f.links[id] ?? '')}, tooltip=${q(label)}`, [label]);

	const engines = ENGINES.map((e) =>
		nd(engineNode(e), e === o.engine ? `style=filled, fillcolor=${p.pick}, fontcolor=${o.theme === 'dark' ? 'black' : p.ink}` : 'style=solid', [e]),
	).join('; ');

	const dragging = !!o.positions;
	const portrait = o.rankdir === 'TB' && o.engine === 'dot';
	return `digraph graphviz_pure {
  graph [rankdir=${o.rankdir}, bgcolor=${q(p.bg)}, fontname="Times-Roman", fontsize=14, fontcolor=${q(p.ink)}, pad="0.2",
         nodesep=0.22, ranksep=0.42, overlap=${dragging ? 'true' : 'false'}, splines=true, outputorder=edgesfirst${dragging ? ', inputscale=72' : ''}]
  node [fontname="Times-Roman", fontsize=${pw ? 10 : 14}, height=0.4, color=${q(p.ink)}, fontcolor=${q(p.ink)}]
  edge [len=${o.rankdir === 'TB' ? 1.6 : 2.2}, fontname="Times-Roman", fontsize=11, arrowsize=0.75, color=${q(p.ink)}, fontcolor=${q(p.ink)}]

  ${nd('input', 'shape=note', ['input.gv'])}
  subgraph cluster_c {
    label="C Graphviz 16.1.0"; style=filled; color=${q(p.cFill)};
    node [style=filled, fillcolor=${q(p.cNode)}, color=${q(p.cNode)}];
    ${nd('c', '', ['dot -Tsvg'])}
  }
  subgraph cluster_rs {
    label="graphviz-pure"; color=${q(p.rsPen)};
    node [style=filled, fillcolor=${q(p.rsFill)}];
    ${nd('rs', '', ['graphviz-pure -Tsvg'])}
    ${nd('wasm', '', ['graphviz_pure_wasm, in this tab'])}
    subgraph cluster_k {
      label="-K"; style=dashed; color=${q(p.kPen)}; fontsize=11;
      node [fontsize=${pw ? 10 : 12}, height=0.3, width=0.6, margin="0.05,0.02"];
      ${engines}
    }
  }
  ${nd('same', `shape=diamond, style=filled, fillcolor=${q(p.ask)}`, ['same', 'bytes?'])}
  ${nd('eq', 'shape=record', rec('C with the same math, every platform', `${n(f.identical)} of ${n(f.total)}`))}
  ${nd('stock', 'shape=record', rec('stock C on macOS', `${n(f.stock)} of ${n(f.stockTotal)}`))}

  input -> c; input -> rs;
  rs -> {${ENGINES.map((e) => engineNode(e)).join(' ')}} [arrowhead=none, color=${q(p.spoke)}];
  {${ENGINES.map((e) => engineNode(e)).join(' ')}} -> wasm [arrowhead=none, color=${q(p.spoke)}];
  c -> same; wasm -> same;
  same -> eq [label="same math"];
  same -> stock [label="Apple libm"];

  node [shape=box, style=rounded, fontsize=${pw ? 10 : 13}];
  ${link('Fidelity', 'Fidelity', 'fidelity')}
  ${link('Gallery', 'Gallery', 'gallery')}
  ${link('Verify', 'Verify it yourself', 'verify')}
  ${link('Playground', 'Playground', 'playground')}
  ${link('left', "What's left", 'left')}
  ${link('API', 'API', 'api')}
  ${link('notes', 'Porting notes', 'notes')}
  ${f.links.platforms ? `${link('xp', 'Cross-platform', 'platforms')}
  eq -> xp;` : ''}
  eq -> Fidelity; stock -> Fidelity; eq -> left; same -> Gallery; same -> Verify;
  wasm -> Playground; rs -> API; rs -> notes;
${portrait ? `
  // Portrait only: push siblings into lower ranks so the drawing fits a phone's width.
  K_dot -> K_sfdp -> K_osage [style=invis]; K_neato -> K_twopi -> K_patchwork [style=invis]; K_fdp -> K_circo [style=invis];
  wasm -> c [style=invis]; Gallery -> Verify [style=invis]; eq -> stock [style=invis]; API -> notes [style=invis];${f.links.platforms ? ' Verify -> xp [style=invis];' : ''}` : ''}
}
`;
}
