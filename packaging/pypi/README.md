# graphviz-pure

A pure-Rust port of [Graphviz](https://graphviz.org) 16.1.0: the `dot` command line with all eight
layout engines (`dot`, `neato`, `fdp`, `sfdp`, `circo`, `twopi`, `osage`, `patchwork`), as one
self-contained program. It writes the same bytes on every platform, and needs no system Graphviz,
libraries or C compiler. Not affiliated with the Graphviz project.

```sh
pip install graphviz-pure        # or: uv tool install graphviz-pure, pipx install graphviz-pure
graphviz-pure -Tsvg graph.gv -o graph.svg
uvx graphviz-pure -Kneato -Tpng graph.gv -o graph.png
```

This package installs the `graphviz-pure` command, with `dot`'s flags (`-K`, `-T`, `-o`, `-O`,
`-G`/`-N`/`-E`, ...). It contains no Python code. Output formats: `svg`, `png`, `jpeg`, `dot`,
`xdot`, `canon`, `json` and `plain` with their variants (`-T?` lists them); `pdf` and `ps` are not
supported yet.

It does not install a `dot` command, so it never shadows a Graphviz you have. Python libraries
that run `dot` by name (the `graphviz` package, pydot, Sphinx) keep using that one; pydot and
Sphinx can be pointed at `graphviz-pure` (`prog="graphviz-pure"`, `graphviz_dot =
"graphviz-pure"`). How far graphviz-pure is from being a drop-in `dot` for them is tracked in
[docs/python.md](https://github.com/graphviz-pure/graphviz-pure/blob/master/docs/python.md).

The wheels hold the release binaries exactly as built by the release workflow, and each wheel has
a signed build-provenance attestation. To check one:

```sh
pip download graphviz-pure==VERSION --no-deps -d wheels
gh attestation verify wheels/graphviz_pure-*.whl --repo graphviz-pure/graphviz-pure
```

Licence: EPL-2.0 for the port, OFL-1.1 for the bundled Tinos, Arimo and Cousine fonts, MIT for the
CORE-MATH `pow` and expat ports; the licences of the Rust crates compiled in are in
`THIRD-PARTY-LICENSES.md`. All are in the wheel's `licenses/` directory.

Rust crates, the npm package for browsers and Node.js, the container image and other installers
are described at <https://graphviz-pure.github.io/>.
