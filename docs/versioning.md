# Versioning

graphviz-pure follows [Semantic Versioning](https://semver.org/). All release channels share
one release version, set in `[workspace.package]` in `Cargo.toml`, `web/package.json` and
`web/jsr.json`, with the channel-specific forms below.

## What the version covers

- **The Rust API** of the stable crates (`graphviz-pure` and `graphviz-pure-raster`), and
  **the JavaScript API** of `@graphviz-pure/wasm`.
  `graphviz-pure-core`, `graphviz-pure-parser`, `graphviz-pure-geom`, `graphviz-pure-layout`
  and `graphviz-pure-render` are implementation crates with no semver promise.
  The types the facade re-exports (`GraphvizError`, `GraphvizErrorKind`, `Diagnostic`,
  `DiagnosticLevel`, `Rendered`, `Result`, `charset`, `LayoutEngine`, `LayoutOptions`,
  `LayoutOptionsBuilder`, `GlobalAttrs`, `OutputFormat`, `Limits`, `Limit`, `CancelToken`)
  are covered by the facade's promise even though they are defined in an implementation
  crate. `graphviz-pure-raster` re-exports `GraphvizError`, `GraphvizErrorKind`,
  `Diagnostic`, `DiagnosticLevel`, `Rendered`, `Limits`, `Limit` and `CancelToken`, and
  its `RasterFormat` is part of that same promise.
  `GraphvizError::__map_text`, `GraphvizError::__set_diagnostics`,
  `Diagnostic::__map_message` and `Rendered::__from_parts` are outside it: hidden hooks
  the facade and the raster crate use, free to change in any release.
  JavaScript function names are camelCase. `inspectGraph()` keeps the field names
  `name`, `directed`, `strict`, `nodes`, `edges`, `subgraphs`, `width_pt` and
  `height_pt`. A failure throws the exported `GraphvizError` class (`extends Error`)
  with `message`, `status` (currently always 1, as C's `dot` exits), `kind` and
  `diagnostics`. `render()` returns the output;
  `renderWithDiagnostics()` returns `{output, diagnostics}`.
- **The command line:** options, accepted `-T` formats, exit status, and the `-V` line's shape:
  `graphviz-pure - graphviz version 16.1.0 (graphviz-pure X.Y.Z)`. Not ported: C's remaining
  core-plugin formats (`cmap`, `eps`, `fig`, `imap`, `imap_np`, `ismap`, `pic`, `pov`, `ps`,
  `ps2`, `svgz`, `tk`) and the cairo, gd and terminal formats (`pdf`, `gif`, `vt`, `kitty`, …).
- **Output.** Because the project exists to reproduce Graphviz's output exactly, every change
  to the bytes the port writes for an existing input is listed in the changelog, whatever its
  size. Every build, including a crates.io source build, computes with the same math library,
  so the bytes are the same on every platform.

## Release channels

The [release workflow](../.github/workflows/release.yml) publishes the channels listed in
[channels.md](channels.md#shipped). Each version identifies the same graphviz-pure release:

| channel | what the version means |
| :--- | :--- |
| GitHub releases | `vX.Y.Z` identifies the release; its native and WASI archives carry `X.Y.Z` in their names. |
| crates.io | `X.Y.Z` versions the stable Rust API of the facade (`graphviz-pure`) and `graphviz-pure-raster`, and the CLI in `graphviz-pure-cli`. The facade pins its implementation crates with exact `=` requirements; those crates have no semver promise. The eight prefixed rendering forks are implementation dependencies with no graphviz-pure API promise; they share the graphviz-pure release version, and each description names its upstream version. |
| npm and JSR `@graphviz-pure/wasm` | `X.Y.Z` versions the JavaScript API and its output on both registries. JSR publishes the files from the npm tarball. |
| PyPI `graphviz-pure` wheels | The bundled CLI's release version, mapped to PEP 440: stable `X.Y.Z` stays `X.Y.Z`; accepted pre-release suffixes `-alpha.N`, `-beta.N` and `-rc.N` become `aN`, `bN` and `rcN`, respectively (for example, `0.3.0-rc.1` becomes `0.3.0rc1`; [releasing.md](releasing.md)). |
| Wasmer `graphviz-pure/graphviz-pure` and WASI archives | The release version of the WASI CLI module. Wasmer packages the module from the `wasm32-wasip1` release archive and publishes stable versions only. Publishing is skipped if `WASMER_TOKEN` is unset during initial setup; after setup, a missing token for a new version fails the job. Other WASI runtimes can use the archive directly. |
| ghcr.io `graphviz-pure/graphviz-pure` | `X.Y.Z` identifies the CLI image for that release. `X.Y` follows the highest stable `X.Y.*` image version, and `latest` follows the highest stable image version across all majors. Pre-releases get only their full version tag. The workflow does not push an `X` tag (for example, `:1`). |
| Homebrew tap `graphviz-pure/homebrew-tap` | The formula pins the CLI release version, archive URLs and SHA-256 checksums. Install with `brew install graphviz-pure/tap/graphviz-pure`. |
| Scoop bucket `graphviz-pure/scoop-bucket` | The manifest pins the CLI release version, archive URLs and SHA-256 checksums. Run `scoop bucket add graphviz-pure https://github.com/graphviz-pure/scoop-bucket`, then `scoop install graphviz-pure/graphviz-pure`. |
| cargo-binstall | The `graphviz-pure-cli` crate version selects the matching GitHub release archive; its source-build fallback uses the same portable raster math as the archive. |

All CLI distributions above carry the same CLI and are covered by the command-line and output
promises, including the `-V` line's shape. The Homebrew tap and Scoop bucket updates are
dispatched after the GitHub release is published and also run on a 6-hour schedule. Both select
GitHub's latest release and verify its archive attestations and checksums before regenerating
the formula or manifest. Releases marked as pre-releases on GitHub are excluded. The update
scripts do not filter version suffixes themselves. `release-ship.sh` marks a version with a
pre-release suffix (`-alpha.N`, `-beta.N` or `-rc.N`) as a GitHub pre-release, on the draft and
again when that draft is published, so GitHub does not select it as latest.

ghcr's moving tags use versions already pushed to the image registry and only move forward;
`latest` can cross a major-version boundary. Pin a full version to select a particular release.

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

## Security support across 1.0

Before 1.0.0, only the latest release receives security fixes. In 1.x, security fixes ship as
patch releases of the latest 1.x minor only; earlier minors are unsupported. After 1.0.0,
0.x receives no security fixes. This is the policy in [SECURITY.md](../SECURITY.md#supported-versions).

## What "identical to C" means

Fidelity means identical to C Graphviz when C computes with the same math. Floating-point
functions such as `pow`, `sin` and `atan2` differ in the last bit from one platform's libm to
another, and C Graphviz uses whatever libm its platform has, so stock C itself writes different
bytes on macOS and on Linux. graphviz-pure's layout instead computes with one math library on
every platform (`graphviz_pure_core::math`), and is compared with C running those same functions
(`scripts/dot-portable-libm.sh`; see [porting/libm.md](porting/libm.md)).

- An output difference that comes only from a platform's libm is not a bug: for example, stock
  C on macOS calls Apple's libm, and differs from the port on the inputs that
  `gv-diff e2e --stock` lists as libm-only.
- Nor is a difference between C builds that fuse multiply-adds differently: the reference is
  Homebrew's macOS arm64 build, which fuses them, while the usual x86-64 Linux package fuses
  none, and it is not always a last digit (on #846's graph the reference loses an edge the
  unfused build draws; [porting/libm.md](porting/libm.md), "Fused multiply-adds").
- SVG and layout outputs always use portable math, including in crates.io source builds.
- PNG and JPEG output uses the same portable math in every build too. The five math-patched
  raster crates and the resvg/usvg forwarding forks are published under `graphviz-pure-`
  prefixes, so crates.io dependencies select them directly without workspace patches.
  `graphviz-pure-svg2pdf` likewise gives crates.io source builds deterministic PDF resource
  ordering; its older path-only usvg stack stays upstream ([vendor/README.md](../vendor/README.md), #712).

## Making a release

Every pull request carries its changelog entry in its body, under `## Changelog entry`, and one
that makes a breaking change gets the `breaking` label. The release workflow collects the entries
merged since the last release into CHANGELOG.md and picks the version from the labels (minor
while `0.x` if any pull request is `breaking`, else patch). See [releasing.md](releasing.md).
