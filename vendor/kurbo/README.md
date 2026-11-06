# graphviz-pure-kurbo

Patched fork of upstream [kurbo 0.13.1](https://crates.io/crates/kurbo/0.13.1), from [linebender/kurbo](https://github.com/linebender/kurbo), for graphviz-pure. This is not an upstream release.

Transcendental floating-point calls use `libm` so raster output is portable across platforms. The Rust library target remains `kurbo`.

Upstream copyright notices and the Apache-2.0 and MIT licence texts in `LICENSE-APACHE` and `LICENSE-MIT` are retained.
