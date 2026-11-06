# graphviz-pure-resvg

Repackaged fork of upstream [resvg 0.49.0](https://crates.io/crates/resvg/0.49.0), from [linebender/resvg](https://github.com/linebender/resvg), for graphviz-pure. This is not an upstream release.

The rendering source is unchanged. Its `svgtypes`, `tiny-skia` and `usvg` dependencies select the graphviz-pure-prefixed forks, so the complete raster path uses portable math. The Rust library target remains `resvg`.

Upstream copyright notices and the Apache-2.0 and MIT licence texts in `LICENSE-APACHE` and `LICENSE-MIT` are retained.
