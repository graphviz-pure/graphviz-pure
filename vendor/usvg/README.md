# graphviz-pure-usvg

Repackaged fork of upstream [usvg 0.49.0](https://crates.io/crates/usvg/0.49.0), from [linebender/resvg](https://github.com/linebender/resvg), for graphviz-pure. This is not an upstream release.

The SVG simplification source is unchanged. Its `kurbo`, `skrifa`, `svgtypes` and `tiny-skia-path` dependencies select the graphviz-pure-prefixed forks, so the complete raster path uses portable math. The Rust library target remains `usvg`.

Upstream copyright notices and the Apache-2.0 and MIT licence texts in `LICENSE-APACHE` and `LICENSE-MIT` are retained.
