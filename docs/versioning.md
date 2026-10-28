# Versioning

graphviz-pure follows [Semantic Versioning](https://semver.org/). All crates and the npm package
share one version number, set in `[workspace.package]` in `Cargo.toml` and in
`web/package.json`, and are released together.

## What the version covers

- **The Rust API** of the stable crates (`graphviz-pure` and `graphviz-pure-raster`), and
  **the JavaScript API** of `@graphviz-pure/wasm`.
  `graphviz-pure-core`, `graphviz-pure-parser`, `graphviz-pure-geom`, `graphviz-pure-layout`
  and `graphviz-pure-render` are implementation crates with no semver promise.
  The types the facade re-exports (`GraphvizError`, `GraphvizErrorKind`, `Result`,
  `charset`, `LayoutEngine`, `LayoutOptions`, `LayoutOptionsBuilder`, `GlobalAttrs`,
  `OutputFormat`) are covered by the facade's promise even though they are defined in an
  implementation crate.
- **The command line:** options, accepted `-T` formats, exit status, and the `-V` line's shape:
  `graphviz-pure - graphviz version 16.1.0 (graphviz-pure X.Y.Z)`. Not ported: C's remaining
  core-plugin formats (`cmap`, `eps`, `fig`, `imap`, `imap_np`, `ismap`, `pic`, `pov`, `ps`,
  `ps2`, `svgz`, `tk`) and the cairo, gd and terminal formats (`pdf`, `gif`, `vt`, `kitty`, …).
- **Output.** Because the project exists to reproduce Graphviz's output exactly, every change
  to the bytes the port writes for an existing input is listed in the changelog, whatever its
  size. Every build computes with the same math library, so the bytes are the same on every
  platform.

## What each kind of change bumps

While the version is `0.x`, a breaking change bumps the minor version (`0.1` → `0.2`) and
everything else bumps the patch version (`0.1.0` → `0.1.1`), as Cargo treats it.

From `1.0`:

| change | bump |
| :--- | :--- |
| Removing or changing a public Rust or JavaScript API, a command-line option or a format | major |
| A fix that makes output closer to C Graphviz (bytes change for some inputs) | minor |
| A new format, engine option or API | minor |
| Raising `rust-version` (at most current stable minus two) | any release; not a semver promise |
| A fix that does not change output for any input in the test corpora; dependency updates; documentation | patch |
| Updating the reference to a newer Graphviz release | major |

`rust-version` is at most current stable minus two. Any release may raise it; it is not a
semver promise. CI checks the workspace with exactly that toolchain.

Output that moves *away* from C Graphviz is a bug, never a release decision: the fidelity
ratchets in CI refuse it.

## What "identical to C" means

Fidelity means identical to C Graphviz when C computes with the same math. Floating-point
functions such as `pow`, `sin` and `atan2` differ in the last bit from one platform's libm to
another, and C Graphviz uses whatever libm its platform has, so stock C itself writes different
bytes on macOS and on Linux. graphviz-pure instead computes with one math library on every
platform (`graphviz_pure_core::math`), and is compared with C running those same functions
(`scripts/dot-portable-libm.sh`; see [porting/libm.md](porting/libm.md)).

- An output difference that comes only from a platform's libm is not a bug: for example, stock
  C on macOS calls Apple's libm, and differs from the port on the inputs that
  `gv-diff e2e --stock` lists as libm-only.
- The port never follows a platform libm. No build or feature switches to one.

## Making a release

Every pull request carries its changelog entry in its body, under `## Changelog entry`, and one
that makes a breaking change gets the `breaking` label. The release workflow collects the entries
merged since the last release into CHANGELOG.md and picks the version from the labels (minor
while `0.x` if any pull request is `breaking`, else patch). See [releasing.md](releasing.md).
