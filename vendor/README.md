# Vendored raster crates (#339)

`-Tpng` and `-Tjpeg` rasterize SVG with resvg, which draws through usvg, tiny-skia,
tiny-skia-path, kurbo, svgtypes and skrifa. Those crates call `f32` and `f64`
transcendental methods (`sin`, `cos`, `atan2`, `powf`, ...). With `std` enabled,
which PNG encoding requires, a same-named trait method never overrides the inherent
method, so the calls reach the platform libm.

Each crate here is the crates.io source named in the heading of its `Cargo.toml`,
plus two changes:

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

Not vendored, because `-Tpng` does not call their transcendental code: resvg's filters,
usvg (its marker `atan2` runs only for SVG markers, which the port does not emit),
read-fonts VARC, harfrust, polycool (`powi` only) and euclid (not linked).

Release binaries, wasm, npm, PyPI, Docker and builds from this repository use the
portable math for PNG/JPEG; a crates.io source build (`cargo install`) still gets the
platform libm in raster output until the forks or upstream features land. The `[patch]`
applies to this checkout only.

`pow` and `hypot` here are the `libm` crate. `graphviz_pure_core::math` uses CORE-MATH
for `pow` and its own `hypot`. Both are deterministic; they are two implementations.
Core's `math` is `f64` only and the crate is the graph model, so these calls stay on
`libm` (`docs/porting/libm.md`).

`scripts/refresh-vendor.sh` re-downloads the pinned versions, drops the files the build
does not need, and re-applies `scripts/vendor-patches`. `scripts/check-vendor-math.sh`
fails if a transcendental call slips off that patch. Licences are the upstream
`LICENSE*` files, picked up by `cargo about`.
