# graphviz-pure-tiny-skia

Patched fork of upstream [tiny-skia 0.12.0](https://crates.io/crates/tiny-skia/0.12.0), from [linebender/tiny-skia](https://github.com/linebender/tiny-skia), for graphviz-pure. This is not an upstream release.

Transcendental floating-point calls use `libm` so PNG and JPEG output is portable across platforms. The `tiny-skia-path` dependency is renamed to `graphviz-pure-tiny-skia-path`; the Rust library target remains `tiny_skia`.

Upstream authorship, copyright notices and the BSD-3-Clause licence in `LICENSE` are retained.
