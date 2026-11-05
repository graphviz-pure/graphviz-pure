# Vendored raster crates (#339, #712)

`-Tpng` and `-Tjpeg` rasterize SVG with resvg, which draws through usvg, tiny-skia,
tiny-skia-path, kurbo, svgtypes and skrifa. Those crates call `f32` and `f64`
transcendental methods (`sin`, `cos`, `atan2`, `powf`, ...). With `std` enabled,
which PNG encoding requires, a same-named trait method never overrides the inherent
method, so the calls would otherwise reach the platform libm.

Each of tiny-skia, tiny-skia-path, kurbo, svgtypes and skrifa is the upstream crates.io
source named in its manifest, plus two changes:

- `src/portable_math.rs` implements those operations with the `libm` crate 0.2.16,
  the musl port `graphviz_pure_core::math` already uses for every function except
  `pow` and `hypot`. Layout keeps its own f64 `hypot` (an Apple-libm reproduction)
  and f64 `pow` (CORE-MATH). Raster `hypot` and `powf` are the `libm` crate, so an
  f32 call and an f64 call share one implementation on every platform.
- Call sites in `src/` pass through that trait. `sqrt` and `powi` are left as they
  are: `sqrt` is correctly rounded, and `powi` is multiplication. tiny-skia's
  `f32x8::powf` (`src/wide/f32x8_t.rs`, used from `src/pipeline/highp.rs`) is a
  bit-trick approximation, not libm, and is not rewritten. `Vec2::hypot()` and
  `Vec2::atan2()` in kurbo take no arguments; the float calls inside them are rewritten.

`libm` is a required dependency. The upstream `libm` / `no-std-float` features only
take effect when `std` is off, so enabling them does not change a PNG build.

No math changes are needed in resvg's filters, usvg (its marker `atan2` runs only for
SVG markers, which the port does not emit), read-fonts VARC, harfrust, polycool
(`powi` only) or euclid (not linked). resvg and usvg are nevertheless vendored as
forwarding forks: upstream manifests name the original packages, so their forks rename
only those dependency entries to select the portable-math forks. Without these two
forwarding forks, Cargo would compile upstream raster crates beside the patched ones.

The nested `vendor/Cargo.toml` workspace publishes `graphviz-pure-tiny-skia`,
`graphviz-pure-tiny-skia-path`, `graphviz-pure-kurbo`, `graphviz-pure-svgtypes`,
`graphviz-pure-skrifa`, `graphviz-pure-resvg`, `graphviz-pure-usvg` and
`graphviz-pure-svg2pdf`. Each manifest's description and README names the exact upstream
version. All eight packages share the graphviz-pure release version and exact internal
requirements; `scripts/bump-version.sh` advances both workspaces and their lockfiles together,
so a later patch cannot be skipped as an already-published upstream version. Upstream
authorship, copyright notices and `LICENSE*` files are retained. The release workflow publishes
this workspace before the main workspace, so every fork is available before
`graphviz-pure-raster`.

`graphviz-pure-raster` selects `graphviz-pure-resvg` and `graphviz-pure-svg2pdf` with
path and version requirements. Cargo removes the paths when packaging, leaving the same
prefixed crates.io dependencies. There is no `[patch.crates-io]`: release binaries, wasm,
npm, PyPI, Docker, workspace builds and crates.io source builds all use the same portable
math for PNG/JPEG and deterministic resource ordering for PDF.

`pow` and `hypot` here are the `libm` crate. `graphviz_pure_core::math` uses CORE-MATH
for `pow` and its own `hypot`. Both are deterministic; they are two implementations.
Core's `math` is `f64` only and the crate is the graph model, so these calls stay on
`libm` (`docs/porting/libm.md`).

`scripts/refresh-vendor.sh` re-downloads the pinned upstream versions, drops the files
the build does not need, and re-applies `scripts/vendor-patches`, which contain the math,
PDF ordering and fork packaging edits. `scripts/check-vendor-math.sh` fails if a
transcendental call slips off the math patch. `cargo about` picks up the upstream licences.

## PDF resource ordering (#714, #712)

`svg2pdf` 0.13.0 is vendored with resource dictionaries sorted by allocated
reference instead of randomized HashMap order, plus explicit elided lifetimes
for current Rust. `scripts/vendor-patches/svg2pdf.patch` records those changes and its
`graphviz-pure-svg2pdf` packaging metadata. The CLI enables it without text, image or
filter features. Text is outlined by the existing patched usvg 0.48 stack, then parsed as
paths by svg2pdf's upstream usvg 0.45 stack. No system fonts or font subsets enter the PDF.
Content compression is disabled and the writer adds only a fixed producer, without
timestamps or document IDs. The older usvg/kurbo/svgtypes/tiny-skia-path stack stays
upstream: it receives only those normalised paths and needs no portable-math patch.
