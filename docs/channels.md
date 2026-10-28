# Release channels

Where graphviz-pure is published, where it could be, and what each channel costs. The channels
already shipped are described in [releasing.md](releasing.md); this page ranks the others. The
survey was made on 2026-10-24, when the project was twelve days old with one star; several
channels have adoption thresholds that rule them out until later. Facts marked *unverified* could
not be confirmed from the channel's documentation.

## Principles

- **No long-lived tokens in this repository where a channel offers OIDC** (trusted publishing). A
  token that cannot be avoided is a secret of the `release` environment, read only by its
  publishing job after the release is approved.
- **Prefer pull over push.** A separate repository that pulls each release, verifies its
  attestations and commits (the Homebrew tap, the Scoop bucket) keeps credentials out of this
  repository entirely. Ecosystem bots (nixpkgs' r-ryantm, conda-forge's autotick, Homebrew's
  autobump) are the cheapest pull of all.
- **One approval.** Every publishing job in `release.yml` has the same `needs`, so the `release`
  environment's single approval starts them all.
- **Same bytes.** Channels that repackage the release archives keep the attested binaries as they
  are; channels that build from source (nixpkgs, conda-forge, homebrew-core, AUR `graphviz-pure`)
  compute with the same portable math library, so they write the same output.

## Shipped

| channel | how | credential |
| :--- | :--- | :--- |
| GitHub releases | `release.yml`, attested archives and `SHA256SUMS` | job token |
| crates.io | `release.yml` | OIDC |
| npm `@graphviz-pure/wasm` | `release.yml`, `npm publish --provenance` | OIDC |
| JSR `@graphviz-pure/wasm` | `release.yml` | OIDC |
| ghcr.io image | `release.yml` | job token |
| Homebrew tap, Scoop bucket | pull repositories | none here |
| Wasmer `graphviz-pure/graphviz-pure` (from 0.2.1, #464) | `release.yml` (WASI build) | `WASMER_TOKEN`, `release` environment secret: Wasmer has no OIDC |
| PyPI `graphviz-pure` (from 0.2.1, #466) | `release.yml`, wheels repacked from the attested archives | OIDC |
| cargo-binstall (from 0.2.1, #465) | metadata in `crates/graphviz-pure-cli/Cargo.toml`, checked by `scripts/check-binstall.py` | none |

### Free with what is shipped (document only)

- **mise:** `mise use github:graphviz-pure/graphviz-pure` works with no registry entry. mise's
  `github:` backend picks the archive for the host, checks the GitHub API's digest and verifies
  the build-provenance attestation (`github.github_attestations` defaults to on). A fresh release
  is hidden for a while by mise's default `minimum_release_age`.
- **eget:** `eget graphviz-pure/graphviz-pure` picks the archive by target name.
- **npm CDNs:** replace `X.Y.Z` with the release version. jsDelivr (`https://cdn.jsdelivr.net/npm/@graphviz-pure/wasm@X.Y.Z/+esm`,
  `https://esm.run/@graphviz-pure/wasm@X.Y.Z`), unpkg
  (`https://unpkg.com/@graphviz-pure/wasm@X.Y.Z/pkg/graphviz_pure_wasm_bg.wasm`), esm.sh
  (`https://esm.sh/@graphviz-pure/wasm@X.Y.Z`) and JSR's own URLs serve the npm and JSR packages,
  `.wasm` as `application/wasm`. jsDelivr's `/gh/` mirror serves files of the repository at a tag,
  not release assets.
- **Deno:** `npm:` and `jsr:` specifiers already work.

## Ranked

The AUR, nixpkgs and conda-forge files in `packaging/` are submission templates, not published
channels. Their example versions, hashes and maintainer placeholders are filled in from a
verified release at submission time, following [packaging/README.md](../packaging/README.md).
The release workflow does not bump these templates. The AUR update script refuses to finish
while TODOs or placeholder checksums remain; nixpkgs and conda-forge are completed by hand.

Rank **A**: worth doing now. **B**: worth doing once eligible or once the effort pays. **C**: low
reach or high cost. **X**: not a fit.

| channel | reach | needs | automation | credential | rank |
| :--- | :--- | :--- | :--- | :--- | :--- |
| aqua registry | medium (aqua; the backend mise recommends for tools without packslips) | one PR to `aquaproj/aqua-registry`, scaffolded with `argd s`, signed commits | none per release: entries are version-agnostic | none; users verify the attestations (`github_artifact_attestations`) | **A** |
| winget | high on Windows | first manifest by hand (`komac new`), Microsoft's validation and moderation | `komac update --submit`, from a pull repository (the token stays out of this repository) | **classic** PAT with `public_repo` (fine-grained tokens cannot open the PR); no OIDC | **A** |
| nixpkgs | high among Nix users | one PR; `packaging/nixpkgs/` has a submission template | r-ryantm opens the bumps | none | **A** |
| conda-forge | high (Python, data science, pixi) | one PR to `staged-recipes`; `packaging/conda-forge/` has a submission template | autotick bot | none | **A** |
| AUR `graphviz-pure-bin`, `graphviz-pure` | medium-high on Arch | an AUR account and SSH key; `packaging/aur/` has a submission template | `packaging/aur/update.sh` per release, or a pull repository holding the key | SSH key (no expiry); no OIDC | **A** |
| mise packslip | medium (mise's top-ranked backend) | signed release manifests ([packslip](https://mise.jdx.dev/dev-tools/backends/packslip.html), keyless OIDC signing) published with each release | `release.yml` | OIDC (keyless) | **B** |
| GitHub Action `setup-graphviz-pure` | medium (CI) | a public repository with `action.yml` at its root; the Marketplace developer agreement | none per release: the action downloads the version asked for and runs `gh attestation verify` | none | **B** |
| homebrew-core | very high on macOS | a from-source formula; notability: 75 stars (or 30 forks or watchers) for a third-party submission, 225 stars (or 90 forks or watchers) for the author's own; the repository at least 30 days old | autobump every 3 h | none | **B** (not eligible yet) |
| Scoop `extras` / `main` | medium on Windows | extras: 100 stars and/or 50 forks, a package-request issue; main: 500 stars and 150 forks | Excavator (`checkver`/`autoupdate`) | none | **B** (not eligible yet) |
| Cloudsmith (apt, rpm) | medium on Debian, Ubuntu, Fedora | the open-source plan; `.deb`/`.rpm` built from the musl binaries (nfpm) | `release.yml` | OIDC (service account; *unverified* on the open-source plan) | **B** |
| Docker Hub mirror | medium | a `graphvizpure` namespace (no hyphens allowed) | `release.yml` (`skopeo copy` from ghcr.io) | OIDC only on Team, Business, Docker-Sponsored Open Source and Hardened Images subscriptions; otherwise an access token | C |
| Chocolatey | medium, shrinking | a package and human moderation | AU from a pull repository | API key (lifetime *unverified*); no OIDC | C |
| Snap Store | medium on Ubuntu | `snapcraft.yaml` | `snapcraft upload` | exported login with `--expires`; no OIDC found | C |
| Fedora COPR | low-medium | a spec file | webhook builds on tags | API token, 180 days | C |
| Launchpad PPA | low-medium | a Debian source package with vendored crates (builders are offline) | signed `dput` | GPG key | C |
| openSUSE OBS | low-medium | a spec and `.obs/workflows.yml` | OBS pulls on tag | OBS token and a GitHub token held by OBS | C |
| self-hosted apt repository (Pages) | low-medium | reprepro/aptly in a pull repository | pull | a long-lived GPG key | C |
| MacPorts | low-medium | a Portfile (cargo portgroup) | by hand | none | C |
| Alpine aports | low-medium | an APKBUILD in `testing`, a maintainer | by hand | none | C |
| pkgx pantry | low | `package.yml` | pkgx builds | none | C |
| Spack | low (HPC) | `package.py` in `spack/spack-packages` (`CargoPackage`) | by hand | none | C |
| Gentoo GURU, FreeBSD ports, Termux | low | an ebuild / a port / a build recipe; Termux takes "active, well-known" projects | by hand / Termux's bot | none | C |
| asdf plugin | low (mise no longer takes new `asdf:` entries) | a plugin repository | none | none | C |
| NuGet (`dotnet tool` or a Wasmtime.NET wrapper) | low | a new wrapper package | push | OIDC (trusted publishing) | C |
| Maven Central (a Chicory-based JVM wrapper of the WASI build) | low-medium: a pure-JVM Graphviz is a real niche | a new wrapper, a Central Portal namespace, GPG signing | push | user token and GPG key (OIDC *unverified*) | C |
| Debian/Ubuntu archives (ITP), Void | high long-term | every crate packaged through debcargo and a sponsor; Void wants a mature release history | by hand | none | X for now |
| Flathub | — | "Console softwares will not be accepted" | — | — | X |
| vcpkg, Conan | — | C and C++ libraries only: would need a C API (see [python.md](python.md#d-pygraphviz-and-pydot)) | — | — | X |
| Homebrew cask, Microsoft Store, `go install`, Volta | — | wrong kind of package manager | — | — | X |

## Notes on the A and B channels

**aqua.** The registry has no popularity bar; its rejection reasons are duplicates, tools that
are not a single binary on `PATH`, unsigned commits, and hand-written entries (use `argd s
graphviz-pure/graphviz-pure` and do not amend the scaffold's commit). An entry shaped like the one
below installed and verified the attestation through mise's aqua backend from a local registry
file; `argd s` produces the final form:

```yaml
packages:
  - type: github_release
    repo_owner: graphviz-pure
    repo_name: graphviz-pure
    description: A dot-compatible command line for the pure-Rust port of Graphviz
    asset: graphviz-pure-{{trimV .Version}}-{{.Arch}}-{{.OS}}.{{.Format}}
    format: tar.gz
    files:
      - name: graphviz-pure
        src: graphviz-pure-{{trimV .Version}}-{{.Arch}}-{{.OS}}/graphviz-pure
    replacements:
      amd64: x86_64
      arm64: aarch64
      darwin: apple-darwin
      linux: unknown-linux-musl
      windows: pc-windows-msvc
    overrides:
      - goos: windows
        format: zip
    checksum:
      type: github_release
      asset: SHA256SUMS
      algorithm: sha256
    github_artifact_attestations:
      signer_workflow: graphviz-pure/graphviz-pure/.github/workflows/release.yml
```

With the aqua entry, `mise use aqua:graphviz-pure/graphviz-pure` works. A `graphviz-pure`
shorthand in mise's own registry needs its popularity bar (normally thousands of stars), whatever
the backend.

**winget.** Microsoft's pipeline validates each pull request to `microsoft/winget-pkgs` (schema,
URL reputation, antivirus scans, a silent install) and a moderator reviews it; there is no
popularity bar. Each Windows zip holds one portable `graphviz-pure.exe`, which komac detects as a
nested portable installer. komac and WinGet Releaser need a **classic** token with the
`public_repo` scope ([komac#310](https://github.com/russellbanks/Komac/issues/310)), which is
broad. Keep it out of this repository: a pull repository like the tap (owned by a dedicated bot
account that is not an organization member and owns the `winget-pkgs` fork) runs
`komac update --submit` on a schedule after verifying the release's attestations. Unsigned
binaries can draw SmartScreen or antivirus flags in validation (*unverified* for this binary).

**nixpkgs, conda-forge, AUR.** The recipes and their submission steps are in
[packaging/README.md](../packaging/README.md). After the first merge, nixpkgs and conda-forge
update themselves. The AUR has no bot: `packaging/aur/update.sh` verifies the release and writes
both PKGBUILDs; the push needs an SSH key of the AUR account.

**GitHub Action.** A repository `graphviz-pure/setup-graphviz-pure` with an `action.yml` that
downloads `graphviz-pure-VERSION-TARGET` for the runner, checks `SHA256SUMS` and runs
`gh attestation verify --repo graphviz-pure/graphviz-pure --signer-workflow
graphviz-pure/graphviz-pure/.github/workflows/release.yml`, then adds it to `PATH`. Marketplace
listing needs a public repository, a unique name, the developer agreement accepted by an
organization owner and 2FA; there is no review. It needs no credential and no work per release
(it resolves `latest` itself). Users of a pinned action also get the action's own immutable
releases.

**homebrew-core, Scoop main/extras.** Not eligible yet (stars, forks, age). When they are, a
third party submitting the homebrew-core formula has the lower bar; the tap's formula is a
starting point, but homebrew-core builds from source (`cargo install` with `std_cargo_args`).

**Cloudsmith.** The cheapest way to `apt install` and `dnf install`: `.deb` and `.rpm` packages of
the static musl binaries, built with nfpm in `release.yml` and pushed with Cloudsmith's CLI under
OIDC (a service account, short-lived tokens). Ask Cloudsmith whether OIDC is available on the
open-source plan before building it.

## cargo-binstall signatures

cargo-binstall verifies minisign signatures only, not GitHub attestations. Its documented
"just-in-time" scheme generates a key pair in the release job, signs the archives, writes the
public key into `graphviz-pure-cli`'s manifest before `cargo publish` and throws the secret key
away: no stored key, and crates.io's trusted publishing protects the public key. It costs an edit
of the manifest in CI (the published crate then differs from the tag) and a `.sig` file per
archive, so it is left out while binstall users can verify the archives with `gh attestation
verify`.
