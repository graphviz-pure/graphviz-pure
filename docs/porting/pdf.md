# PDF output

Owner decision #711, implemented by #714: `-Tpdf` converts the port's existing
SVG drawing through usvg and svg2pdf. It matches C Graphviz's layout under the
reference font setup, **not the bytes of C's cairo PDF**. The other formats keep
their existing fidelity guarantees. This is a vector conversion, not a bitmap
wrapped in a PDF.

The CLI's default `pdf` Cargo feature includes PDF in the release binaries and
source installs. `cargo build -p graphviz-pure-cli --no-default-features` omits it
and rejects `-Tpdf`. Neither the facade library nor WebAssembly enables it. The
CLI forwards the feature to the device registration in `graphviz-pure-render`
and optional conversion in `graphviz-pure-raster`; svg2pdf is absent from normal
library and wasm dependency trees.

Text uses the raster crate's bundled Tinos, Arimo and Cousine fonts and is
converted to outlines; it is not searchable/selectable PDF text. No system
fonts are loaded. Images are omitted, as in raster output. Each input graph
produces one PDF page; separate graphs are separate PDF documents, following
the existing SVG job/output behavior, rather than a combined multipage PDF.
The SVG's point dimensions are preserved (usvg's CSS pixels are converted back
at 96 dpi), including an explicit graph `dpi` scale.

## Reproducible bytes

The existing patched usvg 0.48 stack normalizes the SVG and outlines the text.
svg2pdf 0.13 uses usvg 0.45, so normalized paths are serialized and parsed into
that version without enabling its text, image or filter features. This keeps
font shaping and its portable math in the existing stack.

svg2pdf's resource dictionaries otherwise follow randomized HashMap iteration.
The vendored patch sorts entries by allocated object reference, keeping drawing
order and deterministic resource names. Text embedding is disabled, avoiding
font-subset ordering. Content streams are uncompressed; the ICC profiles use
fixed bundled bytes and miniz_oxide. The writer adds a fixed producer name,
without creation/modification dates or a document ID.

`tests/render/pdf.gv` and `pdf.pdf` are port-owned fixtures, not C reference
outputs. The CLI test renders in three separate processes, compares every byte
to the same committed fixture on the existing Linux/Windows/macOS test matrix,
checks the page size, validates object cross-reference offsets and trailer,
and rejects timestamps, document IDs and embedded fonts. The fixture includes
three font families, gradients, transparency and a dashed edge. An independent
strict PDF parser also validates the page and its content stream during local
verification. Cross-platform CI must pass before claiming verified bytes on
those targets.

As with the existing raster patches, the workspace patch applies to repository
and release builds. Cargo does not propagate patches into a downstream
crates.io source build: those installs use upstream svg2pdf and do not guarantee
reproducible resource ordering. See [the vendor notes](../../vendor/README.md).

To deliberately regenerate the port fixture after reviewing a converter change:

```sh
cargo run -p graphviz-pure-cli -- -Tpdf tests/render/pdf.gv -o tests/render/pdf.pdf
```

Do not regenerate it to hide a cross-platform failure.

## Feature cost (2026-11-05)

Matched clean builds of the same revision on Linux x86-64, with dependencies
already fetched, four build jobs and nice 15. Release profile: opt-level 3,
thin LTO, one codegen unit, stripped CLI; `x86_64-unknown-linux-musl`, baseline
x86-64, without PGO. Each build followed `cargo clean --release`:

```sh
cargo build --release --locked --offline --target x86_64-unknown-linux-musl \
  -p graphviz-pure-cli --no-default-features
cargo build --release --locked --offline --target x86_64-unknown-linux-musl \
  -p graphviz-pure-cli
```

| Metric | PDF disabled | PDF enabled | Cost |
| --- | ---: | ---: | ---: |
| Stripped binary bytes | 13,223,040 | 14,005,440 | +782,400 (+5.92%) |
| Clean build wall seconds | 66.11 | 69.52 | +3.41 (+5.16%) |
| Build user CPU seconds | 178.67 | 190.29 | +11.62 (+6.50%) |

Wall time is one informational pair on a shared machine, not a performance
gate. Both binaries were built offline; timing excludes cargo-slot waiting and
includes compilation/linking. The musl release PDF matches the GNU debug
fixture byte for byte. Normal facade and wasm dependency trees exclude both
svg2pdf and pdf-writer.
