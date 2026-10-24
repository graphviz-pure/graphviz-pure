# Contributing to graphviz-pure

Thanks for helping. graphviz-pure is a function-by-function port of Graphviz 16.1.0, and the
bar for every change is the same: the output must stay byte-identical to C wherever it is
today, and should get closer wherever it is not.

## Reporting a problem

- **Different output from C Graphviz:** use the "Output differs from C Graphviz" issue form.
  Include the smallest DOT input you can, the command, and both outputs. Compare against C
  set up as described in [Verify it yourself](https://graphviz-pure.github.io/verify/): a stock
  `dot` measures text with Pango and the installed fonts, so its label sizes differ from both
  the reference and the port.
- **Crash, hang or wrong behaviour:** use the bug report form.
- **Security problems:** report them privately (see [SECURITY.md](SECURITY.md)), not in an
  issue.

Known differences are tracked in the [What's left](https://graphviz-pure.github.io/whats-left/)
list; check it first.

## Building and testing

You need a recent stable Rust (`rustup` installs it). Then:

```sh
cargo test --workspace                      # everything, as CI runs it on every platform
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
```

`cargo test` runs the fidelity ratchets: they compare the port with C Graphviz's committed
outputs on about 2,600 inputs, computed by C with the port's own math functions (every build
uses the same math library; see [docs/porting/libm.md](docs/porting/libm.md)), and fail if any file that matched C stops matching. They need
no C installation. Regenerating the C references does; see
[tests/golden/README.md](tests/golden/README.md). CI also runs `scripts/panic-sites.sh`, which
fails if the layout or geom crates gain an `unwrap`, `expect`, index or integer-overflow site
([the panic-site ratchet](docs/porting/crashes.md#the-panic-site-ratchet)).

## Making a change

- **Follow the C.** The port mirrors Graphviz's C sources function by function. Name the C
  function and file you are porting or fixing (`lib/dotgen/position.c:make_lrvn`) in comments
  and in the pull request, and keep the structure recognisable.
- **Prove it.** A fix for a difference from C comes with the input that showed it, ideally added
  to a corpus under `tests/`, and the ratchet baselines refreshed if more files now match
  (`scripts/update-baselines.sh`). A change must not
  make any file that matched C stop matching.
- **Where C crashes, the port must not.** Fix a panic at its cause, not at the line that
  panics, and group crashes that share a cause into one fix
  ([how crashes are fixed](docs/porting/crashes.md)).
- **No `unsafe`.** The workspace forbids it; the only exception is the test-only
  `graphviz-pure-portable-libm` crate.
- **Dependencies** are added only when necessary, and must pass `cargo deny check`
  (`deny.toml`: licences, advisories, sources). After any change to `Cargo.lock` (a new,
  removed or updated dependency), regenerate the licence notices that ship with the binaries
  and the wasm package with `scripts/third-party-licenses.sh` and commit
  `THIRD-PARTY-LICENSES.md`; it needs `cargo install --locked --features cli cargo-about@0.9.2`,
  and CI fails when the file is stale. A licence added to `deny.toml`'s allow list goes into
  `about.toml`'s `accepted` list too.
- **Licence files in the crates:** each crate published to crates.io holds copies of `LICENSE`
  and `NOTICE` (and `graphviz-pure-core` of `LICENSES/MIT-CORE-MATH.txt`, `graphviz-pure-geom` of
  `LICENSES/MIT-EXPAT.txt`), because a crate cannot include files from outside its directory.
  When you change a root file, copy it over the crates' copies
  (`for d in crates/*/LICENSE; do cp LICENSE "$d"; done`, likewise for `NOTICE`, for
  `LICENSES/MIT-CORE-MATH.txt` in `crates/graphviz-pure-core/LICENSES/` and for
  `LICENSES/MIT-EXPAT.txt` in `crates/graphviz-pure-geom/LICENSES/`); CI
  (`scripts/check-crate-notices.sh`) fails when one differs.
- **Changelog:** write the entry in the pull request's body, under `## Changelog entry`, for
  anything a user would notice, including changes to output bytes: one Keep a Changelog bullet
  per change, under `### Fixed`, `### Changed` and so on if it has more than one kind, or
  `Not user-visible.` Don't edit CHANGELOG.md: the release collects the entries from the bodies.
  A breaking change gets the `breaking` label.
- **Pull requests** need the required CI checks to pass (tests on Linux, Windows and macOS
  arm64, the supply-chain check, and the cross-platform check that those platforms and WASM
  write the same bytes) and all review threads resolved. macOS x86-64 is checked after the
  merge, on each push to master, and fuzzing runs once a day when the code changed; either
  opens an issue when it fails. Rebase only when the pull request conflicts with `master`: the
  merge queue tests each pull request merged onto `master` and the ones ahead of it. The "PR
  risk report" job lists what a maintainer will read most closely:
  changes to workflows, scripts, build scripts and dependencies
  ([how pull requests are reviewed](docs/pull-requests.md)).

## Review and maintainers

- **CI waits for a maintainer.** If you are not a maintainer, each CI run on your pull request
  starts only after a maintainer has read the diff and approved it, and that applies to every
  push, not just the first. A pull request with no checks yet is waiting for that, not broken.
- **Some areas are maintainer-only.** Changes from contributors to the workflows in `.github/`,
  the release pipeline, `deny.toml`, `CODEOWNERS`, `SECURITY.md` or the licence are declined
  unless a maintainer asked for them. Open an issue instead.
- **Maintainers** are [@cloudbring](https://github.com/cloudbring). Maintainers' CI runs start
  without approval, but their changes still go through pull requests with the same required
  checks; nobody pushes to `master` directly.
- **Automated agents** working for the maintainer open pull requests as
  [@slystation](https://github.com/slystation). It can push branches but is not a maintainer:
  its pull requests need a maintainer's approval like anyone else's.
- **Becoming a maintainer** is by invitation, after a track record of substantial merged pull
  requests that follow the C and prove it, and of helpful reviews or issue reports. The
  organisation requires two-factor authentication for everyone with access.

## Licence

graphviz-pure is a derivative work of Graphviz and is distributed under the
[Eclipse Public License 2.0](LICENSE). By submitting a contribution you agree that it is
licensed under the same terms.

## Conduct

Be adults: see the [code of conduct](CODE_OF_CONDUCT.md). It is four lines.
