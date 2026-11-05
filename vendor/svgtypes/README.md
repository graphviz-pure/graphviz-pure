# graphviz-pure-svgtypes

Patched fork of upstream [svgtypes 0.16.1](https://crates.io/crates/svgtypes/0.16.1), from [linebender/svgtypes](https://github.com/linebender/svgtypes), for graphviz-pure. This is not an upstream release.

Transcendental floating-point calls use `libm` so raster output is portable across platforms. The `kurbo` dependency is renamed to `graphviz-pure-kurbo`; the Rust library target remains `svgtypes`.

Upstream copyright notices and the Apache-2.0 and MIT licence texts in `LICENSE-APACHE` and `LICENSE-MIT` are retained.
