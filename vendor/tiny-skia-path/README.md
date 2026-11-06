# graphviz-pure-tiny-skia-path

Patched fork of upstream [tiny-skia-path 0.12.0](https://crates.io/crates/tiny-skia-path/0.12.0), from [linebender/tiny-skia](https://github.com/linebender/tiny-skia/tree/master/path), for graphviz-pure. This is not an upstream release.

Transcendental floating-point calls use `libm` so raster output is portable across platforms. The Rust library target remains `tiny_skia_path`.

Upstream authorship, copyright notices and the BSD-3-Clause licence in `LICENSE` are retained.
