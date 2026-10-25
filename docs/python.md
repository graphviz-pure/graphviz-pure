# graphviz-pure as a backend for Python

Python's Graphviz libraries need a C Graphviz installed separately: `pip install graphviz` gives
a wrapper that fails with `ExecutableNotFound` until the system has a `dot` on `PATH`. This page
looks at what it would take for graphviz-pure to be that backend, installable with `pip`, and at
what stands in the way. Researched on 2026-10-24 against graphviz-pure 0.2.0 and the projects'
public repositories; the measurements are from macOS arm64.

## Summary

- **Most Python tools find `dot` by name on `PATH`.** A wheel that installs a `dot` command into
  the virtual environment is picked up by the `graphviz` package, pydot, Sphinx, pyreverse,
  diagrams, dask, Airflow, torchviz and others without any change to them.
- **Shipping the binary in wheels is cheap** (the release archives repackaged, OIDC publishing).
  [Option (a)](#a-a-wheel-that-puts-the-binary-on-path) is the way in.
- **Being a drop-in `dot` is not yet.** With graphviz-pure linked as `dot`, the `graphviz`
  package's own executable tests pass 11 of 34 (C passes 34). With a C-style `-V` line, 16 of 34
  pass; the rest need `pdf`, `ps`, `svg_inline` and the `unflatten` tool. pydot's default format
  (`ps`), Sphinx's HTML image maps (`cmapx`) and anything that runs `neato`, `fdp`, ... as
  commands also need work in the port first.
- **In-process (PyO3) and pyodide** are a second step, worth it for notebooks in the browser
  (JupyterLite), where no subprocess exists.

## How the Python libraries call Graphviz

| library or tool | how it runs Graphviz | can the program be chosen? | formats it needs |
| :--- | :--- | :--- | :--- |
| [`graphviz`](https://github.com/xflr6/graphviz) (PyPI) | `dot -K<engine> -T<fmt>[:renderer[:formatter]]` from `PATH` (`backend/dot_command.py`); `pipe()` on stdin and stdout; `unflatten()` runs the `unflatten` program; `version()` parses `dot -V` | no: no option or environment variable; only monkey-patching `graphviz.backend.dot_command.DOT_BINARY` | default `pdf`; Jupyter display `svg` |
| [pydot](https://github.com/pydot/pydot) | `[prog, -T<fmt>, file]`; `prog` is a name, a path or a list (`set_prog`) | yes | default `ps` |
| [pygraphviz](https://github.com/pygraphviz/pygraphviz) | links libcgraph and libgvc (SWIG); runs `tred`, `unflatten`, `gvpr`, `acyclic`, `ccomps`, `sccmap`, `gc`, `gvcolor` as programs | no | any, in process |
| NetworkX | `nx_agraph` through pygraphviz; `nx_pydot` runs `prog` (default `neato`) through pydot | `prog` | — |
| Sphinx `sphinx.ext.graphviz` | `[graphviz_dot, -T<fmt>, -o<out>]`; the `:layout:` option replaces the program name | yes: `graphviz_dot` | HTML: `png` plus `-Tcmapx`, or `svg`; LaTeX: `pdf` |
| pylint `pyreverse` | `shutil.which("dot")`, parses `dot -T?` | no | any |
| diagrams, torchviz, dask, Airflow | the `graphviz` package | no | `png`, `svg`, `pdf`, ... |
| Keras `plot_model` | pydot; its availability check renders `-Tps` | through pydot | `ps`, then `png` |
| objgraph, pycallgraph, markdown-inline-graphviz | the program by name (`dot`, or the engine's name) | pycallgraph: `--tool` | `png` |
| PlantUML (Sphinx and MkDocs plugins) | `GRAPHVIZ_DOT` | yes | `svg`; reads the version with `(\d+)\.(\d\d?)` |

## What the `dot` command line lacks for them

Measured by linking `target/release/graphviz-pure` as `dot` in an otherwise Graphviz-free `PATH`
and running the `graphviz` package's test suite (`python run-tests.py --only-exe`, xflr6/graphviz
at `6ef1ce2`, pytest 8.0 as its `test` group pins): **11 passed, 16 failed, 7 errors**, against 34
passed with C `dot` 16.1.0. In the order of the tests they unlock:

1. **`-V`.** graphviz-pure prints `graphviz-pure - graphviz-pure version 0.2.0 (Rust port of
   Graphviz 16.1.0)`. The `graphviz` package's `version()` searches for
   `graphviz version N.N[.N]` and raises `RuntimeError` on anything else; its tests use
   `version()` in skip conditions, which is where the 7 errors come from. PlantUML reads the first
   `N.NN` and would take 0.2 for the Graphviz version. A line like C's, naming the program as it
   was called, keeps graphviz-pure's own version in the build field:
   `dot - graphviz version 16.1.0 (graphviz-pure 0.2.0)`. With that line (tried through a wrapper
   script) the suite passes 16 of 34.
2. **`pdf`** (10 of the remaining 18 failures; the default format of `graphviz` and Sphinx's LaTeX
   output) and **`ps`** (1; pydot's default, and the check Keras runs). C draws both with cairo,
   so byte identity with C is out of reach for `pdf`; Graphviz's own `-Tps` (the `core` renderer)
   is text and could be ported like the SVG writer.
3. **`unflatten`** (4): a separate Graphviz program (`cmd/tools/unflatten.c`, 124 lines) that the
   `graphviz` package runs by name.
4. **`svg_inline`** (3): SVG without the XML declaration and doctype, a format of C's core SVG
   plugin.
5. Outside this suite: **`cmapx`** (Sphinx's HTML output with PNG images, the default), and
   **the command's name choosing the engine**. C lays out with `neato` when it runs as `neato`
   (`dotneato_args_initialize` takes the default layout from the program name), and pydot,
   NetworkX, Sphinx's `:layout:` and pycallgraph rely on that; graphviz-pure always defaults to
   `dot` ([packaging/README.md](../packaging/README.md#the-dot-command)). Following C here is a
   fidelity fix, not a new feature.

The renderer and formatter suffixes of the formats graphviz-pure writes are accepted
(`-Tplain:dot:core`, `-Tpng:cairo`); C's other pairings (`-Tsvg:cairo`, ...) are refused with
C's "not recognized" message. The remaining C formats (`gif`, `bmp`, `tiff`, `webp`, `eps`, `fig`, `pic`, `vml`,
`imap`, ...) are rarely asked for by these tools.

## The options

### (a) A wheel that puts the binary on `PATH`

How ruff and uv ship: a wheel whose `.data/scripts/` holds the binary, which pip and uv install
into the environment's `bin/` (`Scripts\` on Windows). Nothing is imported; `uvx graphviz-pure`
and `pipx` work too.

- **Build:** repackage the six attested release archives into six wheels in `release.yml` (a short
  script; no second compile, so the wheel's binary is the attested one byte for byte). maturin's
  `bindings = "bin"` would compile again on each platform instead.
- **Tags:** the Linux binaries are static (musl), so one wheel per architecture can carry both
  `manylinux_2_17` and `musllinux_1_1` tags; macOS `macosx_11_0_arm64` and `macosx_10_12_x86_64`
  (the binaries' minimum OS versions); `win_amd64` and `win_arm64`. Tag `py3-none`: the binary
  does not depend on the Python version. No sdist at first (a platform without a wheel gets a
  clear "no matching distribution"); a maturin sdist that compiles with cargo can come later for
  other platforms.
- **Size:** about 6 MB per wheel (the release archives are 5.7 to 6.3 MB; the binary is 12 MB
  uncompressed, fonts included). C Graphviz with cairo, pango and their libraries is far larger.
- **Publishing:** PyPI trusted publishing (OIDC), bound to the `release` environment, by a
  seventh publishing job with the same `needs`. The PyPI project name `graphviz-pure` is free.
  `uv publish --trusted-publishing always` does the exchange; the organization's action list does
  not include `pypa/gh-action-pypi-publish`, so uv would be installed pinned by checksum, like
  Deno.
- **The `dot` question.** A wheel that installs only `graphviz-pure` changes nothing for the
  `graphviz` package. A wheel that also installs `dot` (and `neato`, ...) makes
  `pip install graphviz graphviz-pure` work, but shadows a system Graphviz inside that
  environment, and fails the tools above until the gaps in the previous section are closed. A
  middle way: the wheel `graphviz-pure` installs `graphviz-pure` only, and a second, tiny wheel
  (say `graphviz-pure-dot`) depends on it and adds the `dot` and engine commands, once `-V` and
  the engine-from-name behaviour match C.

### (b) A native extension (PyO3)

`graphviz_pure.render(source, engine="dot", format="svg") -> bytes` in process: no subprocess,
no start-up cost, and the only route that also works in pyodide (below). It is a new crate
(`graphviz-pure-python`, PyO3 with `abi3`, built by maturin) and a compile per platform in the
release. Libraries would have to call it; the `graphviz` package would not, since it is a
subprocess wrapper by design.

### (c) A backend hook in the `graphviz` package

The package has no hook, and its maintainer has turned down the nearby requests:
[#65](https://github.com/xflr6/graphviz/issues/65) (a path to the executables: set `PATH`),
[#197](https://github.com/xflr6/graphviz/issues/197) (JupyterLite and pyodide: out of scope for a
subprocess wrapper), and [#246](https://github.com/xflr6/graphviz/issues/246) (2026-07, an extra
that bundles `dot`). In #246 he wrote that he might be open to endorsing, or integrating, "a
well-maintained and regularly updated Python package" that installs Graphviz "on all three
platforms in a robust and conformant way". That is the realistic target: wheels from (a) that
pass the package's test suite, offered for a mention in its documentation. A hook in the package
itself is unlikely to be accepted.

### (d) pygraphviz and pydot

- **pydot** needs nothing from upstream: `prog` accepts a path, so `Dot.create(prog="graphviz-pure",
  format="svg")` works today. Its default format `ps` and NetworkX's default `prog="neato"` need
  items 2 and 5 above.
- **pygraphviz** links C Graphviz. Since 2.0 (2026-06) its wheels bundle Graphviz's libraries,
  plugins and helper programs, but not `dot`. Replacing them would take a C ABI shim of about 50
  cgraph functions (`agopen`, `agread`, `agnode`, `agedge`, `agattr`, `agsafeset`, `agnxtnode`,
  ...), the gvc layout and render calls (`gvContext`, `gvLayout`, `gvRender`, `gvRenderData`,
  ...), C struct layouts (`Agsym_t`, `Agdesc_t`) and `FILE*` I/O, in a separate crate allowed
  `unsafe` code. Large, and only for one library: not now. The same C API would serve vcpkg and
  Conan.

## Pyodide and the browser

Pyodide has no subprocesses, so neither the `graphviz` package nor pydot can render there
(`graphviz` #197, pygraphviz #453). Two routes:

- **JavaScript:** a pure-Python package that calls `@graphviz-pure/wasm` through pyodide's `js`
  module (`pyodide.code.run_js` or `registerJsModule`), awaiting its `init()`. Works with the npm
  package as it is.
- **A pyodide wheel of (b):** PyO3 builds for `wasm32-unknown-emscripten` with pyodide-build;
  Rust needs pyodide's toolchain settings (`-Zemscripten-wasm-eh` on nightly, or their prebuilt
  sysroot). Packages can then be added to pyodide's distribution.

## What graphviz-pure offers Python users

- **`pip install` and nothing else:** no system Graphviz, no C compiler, no Homebrew, apt or
  Chocolatey step, and the same install on every CI image (the `graphviz` package's own CI
  installs Graphviz with apt and Chocolatey, and skips the executable tests on macOS).
- **The same bytes on every platform.** C Graphviz's output depends on the platform's libm, and
  its PNG text on the installed fonts and Pango; graphviz-pure computes with one math library and
  draws text with bundled fonts. Snapshot tests of rendered diagrams (documentation builds, CI
  golden files) stop flaking between macOS, Linux and Windows.
- **Fast start-up.** Python tools typically render many small graphs, one process each. On a
  four-node graph: C `dot` 16.1.0 (Homebrew) about 50 ms per run (median of 30), graphviz-pure
  2.5 ms for `-Tsvg` and 4.5 ms for `-Tpng`, because C loads its plugins, fontconfig and Pango
  first. On large graphs the two are close, with graphviz-pure faster on some
  ([README](../README.md#performance)).
- **Memory safety:** no `unsafe` code in the layout and render crates; Graphviz's own README warns
  against exposing it to untrusted input.
- **Reproducible and attested:** each release binary has a signed build-provenance attestation
  that `gh attestation verify` checks.
- **WebAssembly:** the same engines in the browser, so a pyodide route exists at all.

And honestly what it does not do (yet): `pdf`, `ps`, `cmapx`, `svg_inline`, `gif` and the other
cairo and gd formats; the other Graphviz programs (`unflatten`, `tred`, `gvpr`, `ccomps`, ...);
plugins; engine selection from the command's name; PNG pixels that match C's (layouts match, the
rasterizer differs: resvg and bundled fonts, not cairo and Pango).

## Test plan

1. **The `graphviz` package's suite** in CI, against each wheel: install the wheel and
   `graphviz` into a fresh environment, put a `dot` link to the installed binary first on `PATH`
   with no other Graphviz, run `python run-tests.py --only-exe` from a checkout of xflr6/graphviz
   at a pinned commit (pytest pinned below 8.1, as its `test` group requires). Track the pass
   count as a ratchet: 11/34 today, 16/34 with the `-V` change.
2. **pydot's and Sphinx's tests** that render, the same way (Sphinx: `tests/test_extensions/test_ext_graphviz.py`
   with `graphviz_dot` pointing at the binary).
3. **The wheels themselves:** in the release workflow, install each wheel with pip and with uv on
   its platform (the same runners as the binaries) and run `scripts/release-smoke.py` on the
   installed command; check that the wheel's binary is the attested archive's byte for byte.
4. **pyodide** (when (b) exists): the gallery samples through the pyodide wheel in Node, like
   `scripts/wasm-runtimes/`.

## Who decides

From the projects' public repositories (commit history, maintainer lists, recipe files). All of
them take proposals through public issue trackers; nobody has been contacted.

| project | people (public handles) | where to raise it | what to ask, once there is something to show |
| :--- | :--- | :--- | :--- |
| `graphviz` (PyPI) | Sebastian Bank, `xflr6`: sole maintainer (about 1,300 commits); MIT; about one release a year | [issues](https://github.com/xflr6/graphviz/issues) (no Discussions, no CONTRIBUTING) | a mention in its installation docs, following up #246 with test-suite results |
| pydot | Łukasz Łapiński, `lkk7` (maintainer); `ferdnyc` active; `prmtl`, `peternowee` earlier | [issues](https://github.com/pydot/pydot/issues) | a documented way to name the program (it exists: `prog`) and an install hint |
| pygraphviz | authors Aric Hagberg (`hagberg`), Dan Schult (`dschult`); releases by `jarrodmillman`, `MridulS`, `rossbar`; wheels by `larsoner` (most also maintain NetworkX) | [issues](https://github.com/pygraphviz/pygraphviz/issues), [mailing list](https://groups.google.com/forum/#!forum/pygraphviz-discuss) | only with a C ABI; the pyodide issue #453 is the open thread |
| NetworkX | the same core group | [issues](https://github.com/networkx/networkx/issues) | `nx_pydot` with `prog` set, once engine names work |
| conda-forge `graphviz-feedstock` | `jakirkham`, `ccordoba12`, `ocefpaf`, `johanneskoester`, `nehaljwani` | [feedstock issues](https://github.com/conda-forge/graphviz-feedstock/issues) | nothing to change there: graphviz-pure goes in as its own feedstock (`packaging/conda-forge/`) |
| conda-forge `python-graphviz`, `pygraphviz`, `pydot` | `jakirkham`, `scopatz`; `nehaljwani`, `mgorny`; `nehaljwani`, `leycec`, `bollwyvl` | their feedstocks' issues | — |
| Graphviz | Matthew Fernandez (`smattr`, most commits this year), Magnus Jacobsson (`magjac`, d3-graphviz, the forum), Mark Hansen (`markhansen`), Stephen North (`scnorth`) | [forum.graphviz.org](https://forum.graphviz.org), [GitLab issues](https://gitlab.com/graphviz/graphviz/-/issues) | a listing on graphviz.org's Resources page, which lists ports such as @hpcc-js/wasm and viz.js (a merge request to `graphviz/graphviz.gitlab.io`, `content/en/resources.md`) |
| Sphinx | the Sphinx team | [issues](https://github.com/sphinx-doc/sphinx/issues) | nothing: `graphviz_dot` already takes a path |

How to approach them: lead with something they can run (wheels on PyPI, the test-suite numbers
above, the one-math-library argument with a reproducible diff), keep it to their project's
benefit, say plainly that graphviz-pure is a port not affiliated with Graphviz, and ask for one
small thing (a docs mention, a listing). The Graphviz forum has received ports kindly before
(threads [1079](https://forum.graphviz.org/t/1079) and [3388](https://forum.graphviz.org/t/3388)).

## Next steps

1. Ship the wheel channel ((a), `graphviz-pure` only).
2. In the port: the `-V` line, the engine from the command's name, `svg_inline`, then `ps`,
   `cmapx`, `unflatten`; each moves toward C and raises the test-suite count.
3. Then the `dot` wheel, the CI ratchet on the `graphviz` suite, and the #246 follow-up.
4. Later: (b) and the pyodide wheel.
