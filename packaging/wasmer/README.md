# graphviz-pure

A pure-Rust port of [Graphviz](https://graphviz.org) 16.1.0: the `dot` command line with all eight
layout engines, compiled to WebAssembly (WASI). It writes the same bytes on every platform. Not
affiliated with the Graphviz project.

```sh
echo 'digraph { a -> b }' | wasmer run graphviz-pure/graphviz-pure -- -Tsvg > graph.svg
```

The program sees only the directories you map into it. Map one with `--volume` and name files by
their path inside it:

```sh
wasmer run --volume .:/data graphviz-pure/graphviz-pure -- -Kneato -Tpng -o /data/graph.png /data/graph.gv
```

The flags are `dot`'s (`-K`, `-T`, `-o`, `-G`/`-N`/`-E`, ...). Output formats: `svg`, `png`, `jpeg`,
`dot`, `xdot`, `canon`, `json` and `plain` (with their variants; `-T?` lists them all). Text in PNG
and JPEG is drawn with the bundled Tinos, Arimo and Cousine fonts.

SVG from untrusted DOT is untrusted markup: like C Graphviz, `URL`/`href` attributes can carry `javascript:` links, so sanitise it before embedding it inline, or display it with `<img>`. Layout cost is unbounded, as in C; services must impose input-size, time and memory limits in an isolated worker or process with a timeout.

The same build is attached to each [GitHub release](https://github.com/graphviz-pure/graphviz-pure/releases)
as `graphviz-pure-VERSION-wasm32-wasip1.tar.gz`, with a signed build-provenance attestation, for
other WASI runtimes: `wasmtime run --dir . graphviz-pure.wasm -Tsvg -o graph.svg graph.gv` (very
deeply nested graphs need a larger stack there: `-W max-wasm-stack=8388608`).

Native binaries, the Rust crates and the npm package for browsers are described at
<https://graphviz-pure.github.io/>. The licences (EPL-2.0 for the port, OFL-1.1 for the fonts, MIT
for the CORE-MATH and expat ports, and those of the Rust crates compiled in) are in `LICENSE` and
in `/usr/share/doc/graphviz-pure` inside the package.
