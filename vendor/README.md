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

The workspace `[patch.crates-io]` applies to the git checkout, so CI, release binaries
and the wasm build use these copies. It is not applied when a downstream crate depends
on graphviz-pure from crates.io. Publishing renamed forks is a separate release change.

Refresh: replace a directory with the same crates.io version unpacked from the
registry, re-apply `portable_math.rs` and the `libm` dependency, and re-run the call
rewrite. Licenses are the upstream `LICENSE*` files, picked up by `cargo about`.
