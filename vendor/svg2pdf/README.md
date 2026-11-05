# graphviz-pure-svg2pdf

Patched fork of upstream [svg2pdf 0.13.0](https://crates.io/crates/svg2pdf/0.13.0), from [typst/svg2pdf](https://github.com/typst/svg2pdf), for graphviz-pure. This is not an upstream release.

Resource dictionaries are sorted by allocated reference instead of randomized `HashMap` order, making PDF bytes deterministic. Explicit elided lifetimes keep the source current with Rust. The Rust library target remains `svg2pdf`.

Upstream authorship, copyright notices, `NOTICE`, and the Apache-2.0 and MIT licence texts in `LICENSE-APACHE` and `LICENSE-MIT` are retained.
