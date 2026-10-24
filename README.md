<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/icon-dark.svg">
  <img src="assets/icon.svg" width="64" height="64" alt="">
</picture>

# graphviz-pure

[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/graphviz-pure/graphviz-pure/badge)](https://scorecard.dev/viewer/?uri=github.com/graphviz-pure/graphviz-pure)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/15718/badge)](https://www.bestpractices.dev/projects/15718)

**A pure-Rust port of Graphviz 16.1.0, with the same output on every platform** ([documentation and evidence](https://graphviz-pure.github.io/)) (`#![forbid(unsafe_code)]`, no C or FFI dependencies). It runs natively and in the browser (`wasm32-unknown-unknown`), and every build computes with the same math library, so it writes one set of bytes on Linux, Windows, macOS and in the browser. Given the same math functions, C Graphviz writes those bytes too: the port is byte-identical to C on every stable input in the project's test corpora.

graphviz-pure is an independent port. It is not affiliated with or endorsed by the Graphviz project; "Graphviz" in the name says what it is a port of.

---

## Status

- All eight layout engines (`dot`, `neato`, `fdp`, `sfdp`, `twopi`, `circo`, `osage`, `patchwork`) are function-by-function ports of the Graphviz 16.1.0 C code, together with the libraries they use: network simplex, `pathplan`, `ortho`, `pack`, `vpsc`, the `neatogen` solvers, `lib/label` and the shape, label and HTML-table code. `docs/porting/` maps each Rust module to its C file and lists every known difference.
- Output formats: `svg`, `json`, `json0`, `dot`, `xdot`, `canon`, `plain`, `plain-ext`, and the raster formats `png` and `jpg` (`jpeg`, `jpe`). `-n`/`-n2` (use positions from the input) is supported.
- Raster output is the SVG drawn by [resvg](https://github.com/linebender/resvg) (`crates/graphviz-pure-raster`), at Graphviz's sizes (96 dpi, or the graph's `dpi`). Its layout is the port's, so it matches C under the reference setup; its pixels do not match C's, which draws with cairo and measures text with Pango and the installed fonts. Text is drawn with bundled Tinos, Arimo and Cousine (the metrics of Times, Arial and Courier), never with system fonts. JPEG output is based in part on the work of the Independent JPEG Group. PDF output is not supported yet.
- The renderers, devices, layouts and image loaders are built in, and `-P` and `-v` report them as a statically linked C build (`dot_static`) reports its own: no plugin library, demand loading disabled, packages in its built-in order. The packages are `core`, `dot_layout`, `neato_layout`, and `cairo` for the raster devices (`-T png:cairo`). `-P` draws the plugin graph; `-v` reports each plugin looked up and lists them, and the layouts write C's progress messages (#416), with UTC time stamps.
- Text is measured with Graphviz's built-in font metric tables, which is what C does when built without Pango. The reference outputs come from C 16.1.0 run that way (see `scripts/gen-golden.sh`).
- C behaviour is reproduced where output depends on it, including C bugs and platform details: the macOS `qsort` and `rand`, `drand48`, clang's fused multiply-adds, cdt's splay trees, and C's error counting and exit status. `docs/c-bugs-ledger.md` lists the reproduced C bugs.

### Fidelity against C Graphviz 16.1.0

Measured with `gv-diff`, which `cargo test --workspace` runs as ratchets that fail on any regression. The reference is C Graphviz computing with the port's math functions (`scripts/dot-portable-libm.sh` interposes them into the stock `dot`), so the comparison measures the port, not the platform's libm:

| Check | Every platform (Linux, Windows, macOS, WASM) |
| :--- | :---: |
| Full `-Tsvg` byte identity (`gv-diff e2e`), 9 corpora (14 inputs skipped: C's own output changes between runs on 11, and C crashes on 3) | 2,595 / 2,595 |
| Layout geometry, main corpus (566 files: wild graphs, upstream fixtures, fuzz reproducers) | 566 / 566 exact |
| Per-engine corpora, exact | neato 442/445, sfdp 297/297, circo 282/282, fdp 386/388, osage 180/180, patchwork 141/141, twopi 281/289 |
| Renderers given C's layout (`gv-diff render`, 587 layouts, 7 formats) | svg, xdot, dot, json0 581/587; json 545/551; plain and plain-ext 569/577 |
| C's own test graphs (`graphs/`, `tests/graphs/`, `tests/regression_tests/`), dot with base, LR, ortho, concentrate and newrank variants | 379 / 379 per variant |
| Differential fuzzing (6,000 random graphs, all engines, final round) | 98.8% identical |

- **One math library.** Every build calls the same pure-Rust math functions (`graphviz_pure_core::math`), on every target; no build option brings back a platform libm, and clippy rejects code that would. (PNG and JPEG are rasterized from the SVG by resvg and tiny-skia, whose own `f32` math is outside this guarantee; #339.) Stock C on macOS calls Apple's libm, whose `pow`, `sin`, `cos` and `atan2` are closed source and are not reproduced bit for bit, so against stock C on macOS the port matches 2,474 of 2,594 inputs: all 120 others differ only through the libm (the port equals C with the same math on every one; `gv-diff e2e --stock` lists them). That comparison is informational. See `docs/porting/libm.md`. For comparison, C Graphviz built on Linux matches C on macOS on only 2,005 of 2,586.
- **neato and fdp:** neato's 3 inexact files are inputs C crashes on, so there is no C layout to compare with. fdp's 2 are layouts that change from run to run in C.
- **Renderers:** the 6 files that differ in every format are fuzz reproducers whose `neato -n2` rendering misses label placement (#142, #266).
- **plain/plain-ext:** in two Latin-1 files C prints an empty label where the port prints the text, because C reads the label from freed memory (`lib/common/output.c`); the same C bug makes 10 other references unstable.
- **twopi:** the 8 inexact files are a floating-point tie in edge routing that C itself breaks differently from run to run.
- **Fuzzing:** the remaining differences are documented in `docs/porting/fuzzing.md`. Most are C nondeterminism (results that depend on memory addresses) and C crashes.

## Checking fidelity yourself

```sh
cargo test --workspace                                         # every ratchet
cargo run --release -p graphviz-pure-diff -- e2e --stock                  # informational: against stock C on macOS
cargo run --release -p graphviz-pure-diff -- e2e --list                   # full-SVG identity, list differences
cargo run --release -p graphviz-pure-diff -- --filter clust --worst 20    # layout geometry, a subset
cargo run --release -p graphviz-pure-diff -- render                       # renderers on C's own layouts
cargo run --release -p graphviz-pure-diff -- structure                    # same nodes, edges and labels as C, on canvas
node scripts/wasm-e2e.mjs --pkg <wasm-bindgen nodejs output>   # the same check under WASM
scripts/update-baselines.sh                                    # after an improvement
```

Regenerating the C references needs Graphviz 16.1.0 installed and the interposer built (`cargo build --release -p graphviz-pure-portable-libm`): `scripts/gen-golden.sh`, `scripts/gen-render-golden.sh` and `GVBINDIR=<pango-free plugin dir> gv-diff e2e --generate --runs 5`.

## Performance

Measured with one process per run on both sides on the same machine (Apple M-series), taking the minimum of 3 runs of `-Tsvg`. The machine was shared with other work, so treat these as rough:

| Graph | C `dot` | `graphviz-pure` |
| :--- | ---: | ---: |
| `b100` (Graphviz test graph) | 4.5 s | 4.7 s |
| `b103` | 0.73 s | 0.69 s |
| random DAG, 2,000 nodes | 0.56 s | 0.46 s |
| random DAG, 8,000 nodes | 24.5 s | 20.0 s |
| `b100` with `splines=ortho` | over 300 s | 20.1 s |

Output is identical in every row. The ortho speedup comes from data structures with better complexity than C's in a few places, not from changing the algorithm. The WASM package is 1.7 MB (about 700 KB gzipped).

---

## Crates

- [`graphviz-pure-core`](crates/graphviz-pure-core): graph IR, geometry, font metric tables, colour tables with `colorxlate` semantics, charset handling, and the math module (`math`: the same pure-Rust functions on every target).
- [`graphviz-pure-parser`](crates/graphviz-pure-parser): DOT lexer and parser with cgraph's attribute, subgraph and ordering semantics, and the canonical writer.
- [`graphviz-pure-geom`](crates/graphviz-pure-geom): node sizing (`shapes.c`, records, HTML tables, `labels.c`) and `pathplan`.
- [`graphviz-pure-layout`](crates/graphviz-pure-layout): the engines (`dotgen`, `neatogen`, `fdpgen`, `sfdpgen`, `twopigen`, `circogen`, `osage`, `patchwork`), the shared `lib/common` code, plus `ortho`, `pack`, `vpsc`, `sparse` and no-layout mode.
- [`graphviz-pure-render`](crates/graphviz-pure-render): a port of C's emit code and output plugins, and of cgraph's writer.
- [`graphviz-pure`](crates/graphviz-pure): the pipeline (`render_dot`, `render_dot_all`, `render_dot_all_partial`).
- [`graphviz-pure-cli`](crates/graphviz-pure-cli): `graphviz-pure`, a drop-in `dot` command line (`-T`, `-K`, `-o`, `-O`, `-G`/`-N`/`-E`, `-n`, several files, every graph in a file, C's exit status).
- [`gv-diff`](crates/graphviz-pure-diff): the differential harness and ratchets.
- [`graphviz-pure-wasm`](crates/graphviz-pure-wasm): `wasm-bindgen` bindings.

---

## Browser usage (`@graphviz-pure/wasm`)

```js
import init, { render, renderAll } from '@graphviz-pure/wasm';

await init();
const svg = render('digraph G { a -> b [label="fast"]; }', 'dot', 'svg');
document.getElementById('diagram').innerHTML = svg;
```

`render` lays out the first graph in the input. `renderAll` returns one output per graph, and `renderAllBytes` takes and returns raw bytes the way the command line does, including non-UTF-8 input. Each call starts from fresh state, so repeated calls give identical output. `web/index.html` is a playground: `cd web && npm run build:wasm && npm run serve`.

The same `await init()` works in Node.js, Bun and Deno (`jsr:@graphviz-pure/wasm` or `npm:@graphviz-pure/wasm`). Cloudflare Workers import the `.wasm` file as a module and call `initSync({ module })` instead; [web/README.md](web/README.md) has the setup for each runtime, and CI runs every one of them (`scripts/wasm-runtimes/`).

## Licence

[EPL-2.0](LICENSE), the licence of Graphviz. The port follows the Graphviz C sources closely and is a derivative work of Graphviz; [NOTICE](NOTICE) credits the Graphviz authors and lists the third-party code in the math module (MIT), whose provenance is detailed in `docs/porting/libm.md`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), the [code of conduct](CODE_OF_CONDUCT.md), the [changelog](CHANGELOG.md) and the [versioning policy](docs/versioning.md). Security problems: [SECURITY.md](SECURITY.md).
