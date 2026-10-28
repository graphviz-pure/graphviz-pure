/**
 * The front page's live graph. Every frame on screen is one of three things: a layout the
 * port's WASM build just computed, an interpolation between two such layouts, or the
 * static drawing the build rendered. Grabbing a node hands the layout to neato with
 * pos="x,y!" on the node, re-run for as long as the pointer moves.
 */
import { latest, render } from './client';
import { between, css, drawCanvas, fontFamily, inkBox, pathData, sceneFromJson, translate, type Item, type Prim, type Scene } from './scene';
import { ENGINES, engineNode, homeSource, nodeEngine, type Engine, type Figures, type SourceOptions } from './source';

const SVGNS = 'http://www.w3.org/2000/svg';
/** Graphviz draws 1pt as 4/3 px (96 dpi); the sheet never magnifies beyond that. */
const MAX_SCALE = 4 / 3;
/** -Tpng draws at this multiple of the screen's pixel density, then downsamples. */
const RASTER_SUPERSAMPLE = 2;
/** Largest side of the -Tpng canvas, in canvas pixels. */
const RASTER_MAX_PX = 6144;
const PAD = 14.4; // the graph's pad="0.2", in points
const PORTRAIT = '(max-width: 45rem)';

type Box = [number, number, number, number]; // x, y, w, h

interface Drag {
	name: string;
	pointerId: number;
	/** pointer minus node centre, world units */
	grab: [number, number];
	start: [number, number];
	moved: boolean;
	at: [number, number];
}

const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const easeOut = (t: number) => 1 - (1 - t) ** 3;
const fmt = (n: number) => n.toLocaleString('en-US');
const ms = (v: number) => `${v < 10 ? v.toFixed(1) : Math.round(v)} ms`;

async function sha256(bytes: Uint8Array) {
	const d = await crypto.subtle.digest('SHA-256', bytes as BufferSource);
	return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export function mountLive(root: HTMLElement) {
	const figures: Figures = JSON.parse(root.dataset.figures!);
	const builds: Record<string, { bytes: number; sha256: string }> = JSON.parse(root.dataset.builds || '{}');
	const buildHost = root.dataset.host!;
	const sheet = root.querySelector<HTMLElement>('[data-sheet]')!;
	const status = root.querySelector<HTMLElement>('[data-status]')!;
	const proof = root.querySelector<HTMLElement>('[data-proof]')!;
	const srcOut = root.querySelector<HTMLElement>('[data-src]')!;
	const unpin = root.querySelector<HTMLButtonElement>('[data-unpin]')!;
	const kButtons = [...root.querySelectorAll<HTMLButtonElement>('[data-k]')];
	const tButtons = [...root.querySelectorAll<HTMLButtonElement>('[data-t]')];
	const reduce = matchMedia('(prefers-reduced-motion: reduce)');
	const portrait = matchMedia(PORTRAIT);

	const svg = document.createElementNS(SVGNS, 'svg');
	svg.setAttribute('class', 'live');
	svg.setAttribute('tabindex', '-1');
	svg.setAttribute('role', 'group');
	svg.setAttribute('aria-label', 'The graph, laid out in this tab by the WASM build');
	const sceneG = document.createElementNS(SVGNS, 'g');
	sceneG.setAttribute('class', 'scene');
	const ringG = document.createElementNS(SVGNS, 'rect');
	ringG.setAttribute('class', 'ring');
	ringG.setAttribute('visibility', 'hidden');
	svg.append(sceneG, ringG);
	const canvas = document.createElement('canvas');
	canvas.className = 'raster';
	canvas.setAttribute('aria-hidden', 'true');

	const state = {
		engine: 'dot' as Engine,
		format: 'svg' as 'svg' | 'png',
		theme: (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark') as 'light' | 'dark',
		rankdir: (portrait.matches ? 'TB' : 'LR') as 'LR' | 'TB',
		/** true once a drag has handed the layout to neato with positions */
		free: false,
		pins: new Map<string, [number, number]>(),
		lastMs: 0,
		runs: 0,
	};
	let to: Scene | null = null;
	let from: Scene | null = null;
	let shown: Scene | null = null;
	let t0 = 0;
	let dur = 0;
	let view: Box = [0, 0, 1, 1];
	let viewFrom: Box = view;
	let viewTo: Box = view;
	let drag: Drag | null = null;
	let focused: string | null = null;
	let suppressClick = false;
	let raf = 0;
	let live = false;
	/** set when a drag or nudge starts from a plain engine layout */
	let slack = false;
	let slackUntil = 0;
	/** the next layout lands without animation (a theme switch: the page around it is instant) */
	let instant = false;

	/* ---------- Layout requests ---------- */

	const opts = (extra: Partial<SourceOptions> = {}): SourceOptions => ({ engine: state.engine, rankdir: state.rankdir, theme: state.theme, ...extra });

	/** A plain layout of the source by the selected engine, centred on the origin. */
	async function layoutEngine(animate = true) {
		const src = homeSource(figures, opts());
		srcOut.textContent = src;
		const r = await latest('layout', { op: 'render', src, engine: state.engine, format: 'json' });
		if (!r.ok) {
			if (!('stale' in r)) fail(r.error);
			return;
		}
		state.lastMs = r.ms;
		state.runs++;
		const s = sceneFromJson(r.out as string);
		const [x0, y0, x1, y1] = s.bb;
		setTarget(translate(s, -(x0 + x1) / 2, -(y0 + y1) / 2), animate ? 760 : 0, true);
		say();
	}

	/** neato from the current positions, with every pinned node held where it is. */
	async function layoutFree(refit: boolean) {
		const base = to;
		if (!base) return;
		const positions: Record<string, [number, number]> = {};
		for (const it of base.items) if (it.kind === 'node') positions[it.name] = [it.cx, -it.cy];
		for (const [n, [x, y]] of state.pins) positions[n] = [x, -y];
		const anchor = drag?.name ?? [...state.pins.keys()].pop();
		const src = homeSource(figures, opts({ engine: 'neato', positions, pinned: state.pins.keys() }));
		srcOut.textContent = src;
		const r = await latest('layout', { op: 'render', src, engine: 'neato', format: 'json' });
		if (!r.ok) {
			if (!('stale' in r)) fail(r.error);
			return;
		}
		state.lastMs = r.ms;
		state.runs++;
		let s = sceneFromJson(r.out as string);
		// neato moves its output to the origin; put the pinned node back where it was pinned.
		const a = anchor && s.items.find((it) => it.name === anchor);
		const want = anchor && state.pins.get(anchor);
		if (a && want) s = translate(s, want[0] - a.cx, want[1] - a.cy);
		// The first neato run after a plain layout is the hierarchy going slack: let it show.
		const now = performance.now();
		if (slack) slackUntil = now + 520;
		slack = false;
		setTarget(s, Math.max(drag ? 90 : 320, slackUntil - now), refit);
		say();
	}

	function relayout(animate = true) {
		if (state.free && state.pins.size) layoutFree(true);
		else layoutEngine(animate);
	}

	/* ---------- Animation ---------- */

	function fitBox(s: Scene): Box {
		// What is drawn, not the engine's bb: with pinned positions neato's bb can reach the origin.
		const [x0, y0, x1, y1] = inkBox(s);
		const bx = x0 - PAD,
			by = y0 - PAD;
		const bw = x1 + PAD - bx,
			bh = y1 + PAD - by;
		const cw = sheet.clientWidth,
			ch = sheet.clientHeight;
		const k = Math.min(cw / bw, ch / bh, MAX_SCALE);
		const w = cw / k,
			h = ch / k;
		return [bx + bw / 2 - w / 2, by + bh / 2 - h / 2, w, h];
	}

	function setTarget(s: Scene, duration: number, refit: boolean) {
		const now = performance.now();
		from = shown ?? s;
		to = s;
		t0 = now;
		dur = reduce.matches || instant ? 0 : duration;
		instant = false;
		viewFrom = view;
		viewTo = refit && !drag ? fitBox(s) : view;
		if (!shown) view = viewFrom = viewTo;
		kick();
	}

	function kick() {
		if (!raf) raf = requestAnimationFrame(frame);
	}

	function frame(now: number) {
		raf = 0;
		if (!to || !from || drawing) return;
		const t = dur ? Math.min(1, (now - t0) / dur) : 1;
		const e = drag ? easeOut(t) : ease(t);
		shown = between(from, to, e);
		if (drag?.moved) shown = holdDragged(shown, drag);
		view = viewFrom.map((v, i) => v + (viewTo[i] - v) * e) as Box;
		draw(shown);
		if (t < 1 || drag?.moved) kick();
		else if (shown === to) from = to;
	}

	/** The node under the pointer is drawn at the pointer, not where the last layout put it. */
	function holdDragged(s: Scene, d: Drag): Scene {
		const it = s.items.find((x) => x.key === `n:${d.name}`);
		if (!it) return s;
		const dx = d.at[0] - d.grab[0] - it.cx,
			dy = d.at[1] - d.grab[1] - it.cy;
		if (Math.abs(dx) < 0.01 && Math.abs(dy) < 0.01) return s;
		const moved = translate({ items: [it], bb: s.bb }, dx, dy).items[0];
		return { ...s, items: s.items.map((x) => (x === it ? moved : x)) };
	}

	/* ---------- Drawing ---------- */

	const groups = new Map<string, SVGGraphicsElement>();

	function groupFor(it: Item): SVGGraphicsElement {
		let g = groups.get(it.key);
		const wantA = !!it.href;
		if (g && (g.tagName === 'a') !== wantA) {
			g.remove();
			groups.delete(it.key);
			g = undefined;
		}
		if (!g) {
			g = document.createElementNS(SVGNS, wantA ? 'a' : 'g') as SVGGraphicsElement;
			g.setAttribute('class', it.kind);
			g.dataset.key = it.key;
			if (it.kind === 'node') {
				g.dataset.name = it.name;
				const hit = document.createElementNS(SVGNS, 'rect');
				hit.setAttribute('class', 'hit');
				g.append(hit);
				const eng = nodeEngine(it.name);
				if (wantA) {
					g.setAttribute('href', it.href!);
					g.setAttribute('aria-label', `${it.tooltip ?? it.name}. Drag to move, or press the arrow keys.`);
				} else if (eng) {
					g.setAttribute('role', 'button');
					g.setAttribute('aria-label', `Lay out with ${eng}`);
				}
			}
			groups.set(it.key, g);
		}
		return g;
	}

	function primEl(g: Element, i: number, p: Prim): SVGElement {
		const tag = p.t === 'text' ? 'text' : p.t === 'ell' ? 'ellipse' : 'path';
		let el = g.children[i] as SVGElement | undefined;
		if (!el || el.tagName !== tag) {
			const n = document.createElementNS(SVGNS, tag);
			if (el) el.replaceWith(n);
			else g.append(n);
			el = n;
		}
		return el;
	}

	let drawing = false;
	function draw(s: Scene) {
		if (drawing) return;
		drawing = true;
		try {
			paint(s);
		} finally {
			drawing = false;
		}
	}

	function paint(s: Scene) {
		svg.setAttribute('viewBox', view.map((v) => v.toFixed(2)).join(' '));
		const seen = new Set<string>();
		let prev: Element | null = null;
		for (const it of s.items) {
			seen.add(it.key);
			const g = groupFor(it);
			if (g.previousElementSibling !== prev || g.parentNode !== sceneG) {
				if (prev) prev.after(g);
				else sceneG.prepend(g);
			}
			prev = g;
			g.setAttribute('opacity', it.alpha >= 0.999 ? '1' : it.alpha.toFixed(3));
			const off = it.kind === 'node' ? 1 : 0;
			if (off) {
				const hit = g.children[0] as SVGRectElement;
				hit.setAttribute('x', (it.cx - it.hw).toFixed(2));
				hit.setAttribute('y', (it.cy - it.hh).toFixed(2));
				hit.setAttribute('width', (2 * it.hw).toFixed(2));
				hit.setAttribute('height', (2 * it.hh).toFixed(2));
				g.classList.toggle('pinned', state.pins.has(it.name));
			}
			it.prims.forEach((p, i) => {
				const el = primEl(g, i + off, p);
				if (p.t === 'text') {
					el.setAttribute('x', p.x.toFixed(2));
					el.setAttribute('y', p.y.toFixed(2));
					el.setAttribute('text-anchor', p.anchor < 0 ? 'start' : p.anchor > 0 ? 'end' : 'middle');
					el.setAttribute('font-family', fontFamily(p.face));
					el.setAttribute('font-size', p.size.toFixed(2));
					el.setAttribute('fill', css(p.color));
					if (el.textContent !== p.text) el.textContent = p.text;
					return;
				}
				if (p.t === 'ell') {
					el.setAttribute('cx', p.cx.toFixed(2));
					el.setAttribute('cy', p.cy.toFixed(2));
					el.setAttribute('rx', p.rx.toFixed(2));
					el.setAttribute('ry', p.ry.toFixed(2));
				} else el.setAttribute('d', pathData(p));
				el.setAttribute('fill', css(p.fill));
				el.setAttribute('stroke', css(p.stroke));
				if (p.w !== 1) el.setAttribute('stroke-width', p.w.toFixed(2));
				else el.removeAttribute('stroke-width');
				if (p.dash) el.setAttribute('stroke-dasharray', p.dash === 1 ? '5,2' : '1,5');
				else el.removeAttribute('stroke-dasharray');
			});
			while (g.children.length > it.prims.length + off) g.lastElementChild!.remove();
		}
		for (const [k, g] of groups)
			if (!seen.has(k)) {
				if (g.contains(document.activeElement)) continue;
				g.remove();
				groups.delete(k);
			}
		// Focus ring: Graphviz's own dashed box, drawn around the focused node.
		const f = focused && s.items.find((x) => x.key === `n:${focused}`);
		if (f) {
			ringG.setAttribute('x', (f.cx - f.hw - 4).toFixed(2));
			ringG.setAttribute('y', (f.cy - f.hh - 4).toFixed(2));
			ringG.setAttribute('width', (2 * f.hw + 8).toFixed(2));
			ringG.setAttribute('height', (2 * f.hh + 8).toFixed(2));
			ringG.setAttribute('visibility', 'visible');
		} else ringG.setAttribute('visibility', 'hidden');
		if (state.format === 'png') raster(s);
	}

	/**
	 * -Tpng: the same frame rasterised on a canvas at twice the screen's pixel density, then
	 * shown at the sheet's size, so the browser averages four drawn pixels into each screen pixel.
	 */
	function raster(s: Scene) {
		const dpr = window.devicePixelRatio || 1;
		// Capped, so a drag still renders each frame in time on large, dense screens.
		const k = Math.min((sheet.clientWidth / view[2]) * dpr * RASTER_SUPERSAMPLE, RASTER_MAX_PX / view[2], RASTER_MAX_PX / view[3]);
		const cw = Math.max(1, Math.round(view[2] * k)),
			ch = Math.max(1, Math.round(view[3] * k));
		if (canvas.width !== cw || canvas.height !== ch) {
			canvas.width = cw;
			canvas.height = ch;
		}
		const ctx = canvas.getContext('2d')!;
		ctx.imageSmoothingEnabled = true;
		ctx.imageSmoothingQuality = 'high';
		const bgPrim = s.items[0]?.kind === 'graph' ? s.items[0].prims.find((p) => p.t === 'poly') : undefined;
		const bg = bgPrim && bgPrim.t === 'poly' ? bgPrim.fill : null;
		ctx.setTransform(1, 0, 0, 1, 0, 0);
		ctx.fillStyle = css(bg) === 'none' ? getComputedStyle(sheet).backgroundColor : css(bg);
		ctx.fillRect(0, 0, cw, ch);
		ctx.setTransform(k, 0, 0, k, -view[0] * k, -view[1] * k);
		// Hairlines stay one screen pixel wide: RASTER_SUPERSAMPLE canvas pixels.
		drawCanvas(ctx, s, RASTER_SUPERSAMPLE / k);
		ctx.setTransform(1, 0, 0, 1, 0, 0);
	}

	/* ---------- Words ---------- */

	function say(extra?: string) {
		const pins = state.pins.size;
		const what = state.free
			? `neato, from the positions on screen${pins ? `, ${pins} node${pins > 1 ? 's' : ''} pinned with pos="x,y!"` : ''}`
			: `${state.engine}${state.format === 'png' ? ', drawn as -Tpng' : ''}`;
		status.textContent = extra ?? `Laid out by ${what}: ${ms(state.lastMs)} in this tab, layout ${fmt(state.runs)}.`;
		unpin.hidden = !pins;
		// The command line says what ran: a drag is -Kneato.
		const ran = state.free ? 'neato' : state.engine;
		for (const b of kButtons) b.setAttribute('aria-pressed', String(b.dataset.k === ran));
	}

	function fail(error: string) {
		status.textContent = `The WASM build failed: ${error}. The drawing stays as it was.`;
	}

	/* ---------- Input ---------- */

	const toWorld = (cx: number, cy: number): [number, number] => {
		const r = svg.getBoundingClientRect();
		return [view[0] + ((cx - r.left) / r.width) * view[2], view[1] + ((cy - r.top) / r.height) * view[3]];
	};
	const nodeAt = (el: EventTarget | null) => (el instanceof Element ? el.closest<SVGGraphicsElement>('.node[data-name]') : null);

	svg.addEventListener('pointerdown', (ev) => {
		const g = nodeAt(ev.target);
		if (!g || !to || ev.button > 0) return;
		const name = g.dataset.name!;
		const it = (shown ?? to).items.find((x) => x.key === `n:${name}`);
		if (!it) return;
		const at = toWorld(ev.clientX, ev.clientY);
		drag = { name, pointerId: ev.pointerId, grab: [at[0] - it.cx, at[1] - it.cy], start: [ev.clientX, ev.clientY], moved: false, at };
		svg.setPointerCapture(ev.pointerId);
		suppressClick = false;
	});
	svg.addEventListener('pointermove', (ev) => {
		if (!drag || ev.pointerId !== drag.pointerId) return;
		if (!drag.moved && Math.hypot(ev.clientX - drag.start[0], ev.clientY - drag.start[1]) < 4) return;
		if (!drag.moved) {
			drag.moved = true;
			suppressClick = true;
			root.classList.add('dragging');
			if (!state.free) {
				state.free = true;
				state.pins.clear();
				slack = true;
			}
		}
		drag.at = toWorld(ev.clientX, ev.clientY);
		state.pins.set(drag.name, [drag.at[0] - drag.grab[0], drag.at[1] - drag.grab[1]]);
		layoutFree(false);
		kick();
	});
	const endDrag = (ev: PointerEvent) => {
		if (!drag || ev.pointerId !== drag.pointerId) return;
		const d = drag;
		drag = null;
		root.classList.remove('dragging');
		if (d.moved) layoutFree(true);
		else {
			const eng = nodeEngine(d.name);
			if (eng) choose(eng);
			else follow(d.name, ev);
		}
	};
	// Pointer capture retargets the click to the <svg>, so a link node's <a> may never see it:
	// a press without a drag follows the link here, and the native click is swallowed.
	const follow = (name: string, ev: PointerEvent) => {
		const g = svg.querySelector<SVGGraphicsElement>(`.node[data-name="${CSS.escape(name)}"]`);
		const a = g?.closest('a') ?? g?.querySelector('a');
		const href = a?.getAttribute('href') ?? a?.getAttribute('xlink:href');
		if (!href) return;
		suppressClick = true;
		if (ev.metaKey || ev.ctrlKey || ev.shiftKey) window.open(href, '_blank', 'noopener');
		else location.assign(href);
	};
	svg.addEventListener('pointerup', endDrag);
	svg.addEventListener('pointercancel', endDrag);
	svg.addEventListener('click', (ev) => {
		if (suppressClick) {
			ev.preventDefault();
			suppressClick = false;
		}
	});
	// Touch: a finger on a node drags it (nodes have touch-action: none); anywhere else scrolls.

	// Removing a focused element fires focusout in the middle of draw(): only schedule here.
	svg.addEventListener('focusin', (ev) => {
		focused = nodeAt(ev.target)?.dataset.name ?? null;
		kick();
	});
	svg.addEventListener('focusout', () => {
		focused = null;
		kick();
	});
	svg.addEventListener('keydown', (ev) => {
		const g = nodeAt(ev.target);
		if (!g || !to) return;
		if (ev.key === 'Escape' && state.pins.size) {
			ev.preventDefault();
			clearPins();
			return;
		}
		const step = ev.shiftKey ? 48 : 12;
		const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[ev.key];
		if (!d) return;
		ev.preventDefault();
		const name = g.dataset.name!;
		const it = to.items.find((x) => x.key === `n:${name}`)!;
		const cur = state.pins.get(name) ?? [it.cx, it.cy];
		if (!state.free) {
			state.free = true;
			state.pins.clear();
			slack = true;
		}
		state.pins.set(name, [cur[0] + d[0], cur[1] + d[1]]);
		layoutFree(true);
	});

	const touched = () => root.classList.add('touched');
	svg.addEventListener('pointerdown', touched, { once: true });
	svg.addEventListener('keydown', touched, { once: true });

	function choose(e: Engine) {
		touched();
		state.engine = e;
		state.free = false;
		state.pins.clear();
		layoutEngine();
	}
	function clearPins() {
		state.pins.clear();
		state.free = false;
		state.engine = 'neato';
		layoutEngine();
	}
	for (const b of kButtons) b.addEventListener('click', () => choose(b.dataset.k as Engine));
	unpin.addEventListener('click', clearPins);
	for (const b of tButtons)
		b.addEventListener('click', () => {
			state.format = b.dataset.t as 'svg' | 'png';
			for (const x of tButtons) x.setAttribute('aria-pressed', String(x === b));
			root.dataset.format = state.format;
			if (shown) draw(shown);
			say();
		});

	new MutationObserver(() => {
		const th = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
		if (th !== state.theme) {
			state.theme = th;
			instant = true;
			if (live) relayout(false);
		}
	}).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
	portrait.addEventListener('change', () => {
		state.rankdir = portrait.matches ? 'TB' : 'LR';
		if (live && !state.free) layoutEngine();
	});
	new ResizeObserver(() => {
		if (!to || drag) return;
		viewFrom = viewTo = view = fitBox(to);
		if (shown) draw(shown);
	}).observe(sheet);

	/* ---------- Start ---------- */

	async function start() {
		status.textContent = 'Loading the WASM build…';
		const src = homeSource(figures, opts());
		srcOut.textContent = src;
		const [j, s] = await Promise.all([render(src, 'dot', 'json'), render(src, 'dot', 'svg')]);
		if (!j.ok || !s.ok) {
			root.dataset.state = 'failed';
			fail(!j.ok ? j.error : !s.ok ? s.error : '');
			return;
		}
		state.lastMs = j.ms;
		state.runs = 1;
		const sc = sceneFromJson(j.out as string);
		const [x0, y0, x1, y1] = sc.bb;
		to = from = shown = null;
		sheet.append(canvas, svg);
		setTarget(translate(sc, -(x0 + x1) / 2, -(y0 + y1) / 2), 0, true);
		frame(performance.now());
		root.dataset.state = 'live';
		live = true;
		for (const b of [...kButtons, ...tButtons]) b.disabled = false;
		say();
		// A phone has no room for the hint on the sheet; it goes in the first status line.
		if (portrait.matches) status.textContent += ' Drag any node; tap a box to follow it.';
		// The proof: this tab's bytes against the bytes the build wrote for the same source.
		const variant = `${state.theme}-${state.rankdir}`;
		const want = builds[variant];
		const bytes = new TextEncoder().encode(s.out as string);
		const hex = await sha256(bytes);
		if (want && hex === want.sha256) {
			proof.dataset.state = 'same';
			proof.innerHTML = `The drawing you first saw was rendered when this site was built, on ${buildHost}. Your browser rendered it again: the same <span class="num">${fmt(bytes.length)}</span> bytes, sha256 <code>${hex.slice(0, 16)}…</code>`;
		} else if (want) {
			proof.dataset.state = 'differs';
			proof.textContent = `Your browser's -Tsvg differs from the build's (${fmt(bytes.length)} bytes against ${fmt(want.bytes)}). That should not happen; please report it.`;
		}
		// Sit back until something changes; a theme switch during loading is picked up here.
		if (variant !== `${document.documentElement.dataset.theme === 'light' ? 'light' : 'dark'}-${state.rankdir}`) {
			state.theme = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
			relayout(false);
		}
	}

	let started = false;
	const go = () => {
		if (started) return;
		started = true;
		start().catch((e) => {
			root.dataset.state = 'failed';
			fail(String(e));
		});
	};
	// Lazily: after first paint, or as soon as someone reaches for the drawing.
	sheet.addEventListener('pointerenter', go, { once: true });
	sheet.addEventListener('focusin', go, { once: true });
	if ('requestIdleCallback' in window) requestIdleCallback(go, { timeout: 1500 });
	else setTimeout(go, 600);

	return { engines: ENGINES, engineNode };
}
