# Releasing graphviz-pure

To release: dispatch Release (`gh workflow run release.yml [-f version=X.Y.Z]`, or ask any agent); approve the `release` deployment when GitHub notifies you. That's it.

`version` is optional. Empty means the next version: while the version is `0.x`, a pull request labelled `breaking` bumps the minor version and anything else the patch version; from 1.0, `breaking` bumps the major version ([versioning policy](versioning.md)). After a pre-release tag the next version is the version it pre-released. `intro` is optional changelog text; empty means a line counted from the entries (`3 changes: 1 added, 2 fixed.`). `dry_run` (default false) builds, fuzzes, smoke-tests and attests, and does not publish:

```sh
gh workflow run release.yml -f dry_run=true
```

A dry run, or a check that fails, never asks for the `release` environment. A real release uses the concurrency group `release` (one at a time; a second one waits). A dry run uses `release-dry-run`, so dispatching a dry run cannot cancel a real release that is queued or running. A real release has to be dispatched from `master`. The `release` environment allows deployments from `master` only, so a workflow run from any other branch cannot request it.

## Before dispatching

`scripts/release-check.sh` prints the checklist the workflow runs, and changes nothing:

- no open `priority:p1` or `priority:p2` issue, and no open "Cross-platform fails on master";
- the newest `ci.yml` and `cross-platform.yml` runs on master's head succeeded;
- every pull request merged since the last tag has a `## Changelog entry`. An entry marked breaking (under a `### Breaking` heading, or a bullet starting `**Breaking:**` or `**Changed (breaking):**`) has to come from a pull request with the `breaking` label. The labels decide the version;
- the stable Rust APIs (`graphviz-pure` and `graphviz-pure-raster`) are checked against their latest non-yanked stable crates.io releases with cargo-semver-checks 0.51.0. Detected breaks require a merged `breaking` PR and a minor bump while `0.x`, major from `1.0`; this runs before publishing approval;
- CHANGELOG.md's Unreleased section is empty. Entries belong in pull request bodies;
- the version is not tagged yet, and no GitHub release, including a draft, exists for it;
- the release commit merges cleanly onto current master (`git merge-tree --write-tree`). Master moving is fine; a conflict fails the run before any approval is requested;
- every e2e input that was byte-identical to C at the last release still is, and the count is not lower.

A pull request with no entry: add one to its body (`Not user-visible.` when there is nothing to say). An entry marked breaking without the label: label it, or unmark the entry.

`scripts/release-verify.sh X.Y.Z` checks, and prints pass or fail for, each channel: the GitHub
release's assets against their attestations and `SHA256SUMS`; the eight publishable crates on
crates.io (`graphviz-pure-wasm`, `-diff` and `-portable-libm` have `publish = false`), and the
published `graphviz-pure-cli`'s cargo-binstall metadata against the release's archives (and, in
the `verify` job, a pinned cargo-binstall's dry run of every target); JSR's
`meta.json`; the Wasmer package, which must contain the module of the release's attested WASI
archive byte for byte; PyPI's wheels, one per release platform, none yanked, each with its
attestation (`scripts/check-pypi-wheels.py`); the image's attestation, and `X.Y` and `latest` pointing at it where this version
should hold them (not for a backport to an older minor line, nor `latest` for anything but the
newest version); npm's
`latest` dist-tag, an install pinned to the attested release tarball's integrity (`npm ci`
against a lockfile the script writes from it), and `npm audit signatures`; the tap's formula,
the bucket's manifest and
their newest `update.yml` run. Attestations come from `release.yml`. The certificate's source ref is `refs/heads/master`. Its subject digest is the dispatch SHA, master's HEAD when the run started, not the release commit. Releases through 0.2.3 were attested for the tag. The `verify` job runs the script with `--ci` (every channel but the tap and bucket) once the publishing jobs finish, and reports a channel whose publishing job failed without polling it.

### The `breaking` label

A pull request that breaks the Rust or JavaScript API, the command line or a format gets the
`breaking` label when it merges. While the version is `0.x`, a release with one or more such pull
requests bumps the minor version; from 1.0 it bumps the major version. Without the label a
release bumps the patch version
([versioning policy](versioning.md)).

## What the release workflow does

`.github/workflows/release.yml` runs on GitHub-hosted runners, with no caches, the pinned Rust
toolchain and `cargo build --locked`:

- it builds `graphviz-pure` for six targets (Linux x86-64 and arm64 as static musl binaries,
  macOS arm64 and x86-64, Windows x86-64 and arm64), a WASI module (`wasm32-wasip1`, packaged as
  `graphviz-pure-X.Y.Z-wasm32-wasip1.tar.gz` holding `graphviz-pure.wasm`) and the npm package;
- it requires every binary, the WASI module (run by a pinned, checksum-verified Wasmer) and the
  npm package to reproduce the gallery samples byte for byte (`scripts/release-smoke.py`,
  `scripts/release-smoke-wasm.mjs`);
- it packages deterministic archives (`scripts/release-package.py`), writes `SHA256SUMS` and
  attests the build provenance of every file (Sigstore, stored on GitHub);
- it checks that the command line crate's `[package.metadata.binstall]` leads cargo-binstall to
  every archive and the binary in it (`scripts/check-binstall.py`), so
  `cargo binstall graphviz-pure-cli` installs the release's archive instead of compiling;
- it repackages each archive's binary as a Python wheel (`scripts/release-wheel.py`: `py3-none`
  wheels whose scripts directory holds the binary byte for byte), installs it with pip into a
  fresh environment on the same runner and runs the smoke test on the installed command, and
  attests the six wheels (they go to PyPI only, not to the GitHub release). The macOS wheels'
  tags carry the binaries' minimum OS version, read from their Mach-O headers;
- it checks that every archive and the npm tarball (and so the JSR package) carry LICENSE,
  NOTICE, THIRD-PARTY-LICENSES.md and LICENSES/, and every crate LICENSE and NOTICE
  (`scripts/check-package-notices.py`, `scripts/check-crate-notices.sh`; CI runs the same
  checks on every change; the crates' directories hold copies that must match the root files);
- it builds the container image for linux/amd64 and linux/arm64 from the two musl archives
  (`scripts/release-image.sh`, no second compile; with the same BuildKit, commit time and
  revision, the same archives give the same digest) and runs the version check and the gallery
  smoke test through it on both platforms;
- it installs the Windows zips with Scoop on x86-64 and on arm64, from a manifest written like
  the bucket's, checks that each got its own architecture's exe, and runs the same checks
  through Scoop's shim;
- it dry-runs the JSR and npm publishes, and builds and runs the Wasmer package
  (`scripts/wasmer-package.sh`, `wasmer package build`; `wasmer publish --dry-run` needs a login).

On a real run, the seven publishing jobs then wait together for one approval of the `release`
environment, and each publishes on its own:

- `github-release` creates the GitHub release with all files;
- `crates-io` publishes the eight crates to crates.io (trusted publishing,
  `cargo publish --workspace`, leaving out any crate whose version is already there);
- `npm` publishes `@graphviz-pure/wasm` (`npm publish --provenance --access public`). The
  trusted publisher has to allow `npm publish`. The version is live when the job finishes;
- `pypi` publishes the six attested wheels to PyPI as `graphviz-pure` with a pinned uv
  (trusted publishing, OIDC: the job exchanges the run's token for a short-lived PyPI token, as
  PyPI documents, and `uv publish` uses it). PyPI has no staging: the version is live when the
  job finishes. A pre-release version becomes PEP 440's form (`0.3.0-rc.1` → `0.3.0rc1`). Until
  the trusted publisher is set up, PyPI answers `invalid-publisher`: the job then publishes
  nothing and succeeds, and `verify` leaves the channel out;
- `jsr` publishes the files of the npm tarball (which carries `web/jsr.json`) to JSR with a
  pinned Deno (`deno publish`, OIDC). JSR has no staging: the version is live as soon as the job
  finishes, so the `release` approval is its only gate;
- `ghcr` pushes the smoke-tested image to `ghcr.io/graphviz-pure/graphviz-pure` as `X.Y.Z`,
  attests its digest (the attestation is stored next to the image in the registry), then points
  `X.Y` and `latest` at it if it is the newest stable `X.Y.*` and the newest stable version among
  the image's tags (the registry's tags, not the GitHub release). A pre-release gets only its own
  tag;
- `wasmer` publishes the attested WASI archive's module to the Wasmer registry as
  `graphviz-pure/graphviz-pure` (`scripts/wasmer-package.sh` adds `packaging/wasmer/wasmer.toml`
  and its README). Wasmer has no trusted publishing, so this job uses an API token, the
  `release` environment's `WASMER_TOKEN` secret ([setup](#wasmer-namespace-graphviz-pure)), and no
  staging: the version is live when the job finishes. Without the secret, and for a pre-release
  (the registry would show it as the latest version), the job publishes nothing and succeeds,
  and `verify` leaves the channel out.

Then `verify` checks what they published (`scripts/release-verify.sh --ci`) and closes a milestone
titled X.Y.Z if one is open and every publishing job that ran succeeded. A skipped job does not
block that; a failed one leaves the milestone open. The same approval's `github-release` job
dispatches `update.yml` in `graphviz-pure/homebrew-tap` and `graphviz-pure/scoop-bucket`; those
workflows verify each asset and commit the new formula and manifest. Their verifiers accept an
attestation whose source ref is the tag or `refs/heads/master` (the dispatch SHA is the subject
digest).
`graphviz-pure/graphviz-pure.github.io` deploys the site from `master` builds the same way. No
repository holds a long-lived credential for another: the dispatch uses a GitHub App token minted
in the `github-release` job and discarded with it.

`gh workflow run release.yml -f dry_run=true` builds, fuzzes, smoke-tests and attests everything,
the container image included, without publishing anything.

### One approval

Through 0.1.2, the registries' jobs waited for the GitHub release, so a release took two approvals.
One approval now starts all seven publishing jobs at once. The checks that used to sit between
those approvals run first: the readiness checklist, fuzzing, and every smoke test. npm publishes
in that same approval. What is given up: a registry can publish even if the GitHub release fails,
and nothing is left to stop between the GitHub release and the registries. Keep
`environment: release` on every publishing job (the crates.io, npm and PyPI trusted publishers
are bound to it) and keep its approval manual: JSR and PyPI have no staging. Its deployment
branch policy stays `master` only.

Before that approval, `prepare` checks the release can land, with the job's read-only token:
`git merge-tree --write-tree` of current master and the release commit is clean, `refs/tags/vX.Y.Z`
does not exist, and no GitHub release, including a draft, exists for that tag. A missing App
secret, or a ruleset bypass that does not let the App push, cannot be checked then. Those are
confirmed once, when the environment secrets and the ruleset bypass are applied.

### If a publishing job fails

The publishing jobs are independent, so a failure leaves the other channels published. Fix the
cause (an expired trusted-publisher configuration, a registry outage), then **Re-run failed
jobs** on the same run. A re-run uses that dispatch: the workflow file and the SHA of master
when the run was started. There is no tag until `github-release` creates it, and the re-run
does not look up a pre-existing tag to decide what to build. A bug in the workflow itself is
fixed on `master` and ships with the next dispatch. A re-run asks for the approval again.

- `crates-io`: safe to re-run, also after a partial publish. The job asks crates.io which
  crates already have the version (`scripts/crates-to-publish.py`) and publishes only the
  others, in dependency order; if all are there, it publishes nothing.
- `pypi`: safe to re-run. A re-run uploads the same files (the run's attested artifact), and
  `uv publish --check-url https://pypi.org/simple/` first asks PyPI which files it already has:
  identical ones are skipped (PyPI itself answers a re-upload with 400), so only what is missing
  goes up, and a file that differs from PyPI's fails the job.
- `npm`, `jsr`: a re-run is safe if the job failed before publishing. If the version went up,
  there is nothing to re-run: the registries keep a published version.
- `wasmer`: safe to re-run. A version already in the registry is left as it is (versions there
  cannot be replaced), and `release-verify.sh` checks that it holds the attested module.
- `ghcr`: safe to re-run. Pushing the same image again changes nothing, and `X.Y`/`latest`
  only ever move forward: they follow the highest stable version among the image's tags on
  ghcr.io, so a `v*` tag whose release was rejected never holds them.
- `github-release`: the release is immutable once published. If the job failed before
  publishing, delete the draft release, then re-run. A re-run fast-forwards or merges the same
  release commit, and leaves a tag that already points at it. A conflict with commits that landed
  on master during the run fails the job before it pushes, tags or publishes.
- `verify`: re-run it after fixing a channel, or run `scripts/release-verify.sh X.Y.Z` locally.

**First release from a new trusted-publisher configuration:** npm drops a configuration that
has not published within 48 hours of its creation. If it is older than that, delete and recreate
it (same fields, see [npm](#npm-organization-graphviz-pure)) just before dispatching the release.

## Verifying a download

```sh
gh attestation verify graphviz-pure-0.1.0-x86_64-unknown-linux-musl.tar.gz \
  --repo graphviz-pure/graphviz-pure --signer-workflow graphviz-pure/graphviz-pure/.github/workflows/release.yml
sha256sum --check --ignore-missing SHA256SUMS
npm audit signatures   # in a project that depends on @graphviz-pure/wasm
gh attestation verify oci://ghcr.io/graphviz-pure/graphviz-pure:0.1.0 \
  --repo graphviz-pure/graphviz-pure --signer-workflow graphviz-pure/graphviz-pure/.github/workflows/release.yml
```

The image verification checks the multi-platform index digest that the tag points to, so
verify by tag and then pull by digest (`docker pull ghcr.io/graphviz-pure/graphviz-pure@sha256:...`)
if you need the check and the pull to refer to the same bytes.

## Using the container image

The image holds the static binary and the release archive's licence files (LICENSE, NOTICE,
THIRD-PARTY-LICENSES.md and LICENSES/, in `/usr/share/doc/graphviz-pure`), nothing else (no
shell), and runs as
`nobody`:

```sh
docker run --rm -i ghcr.io/graphviz-pure/graphviz-pure -Tsvg < graph.dot > graph.svg
docker run --rm -v "$PWD:/data" --user "$(id -u):$(id -g)" ghcr.io/graphviz-pure/graphviz-pure \
  -Kneato -Tpng -o graph.png graph.dot
```

## One-time setup

Done once per channel by a maintainer. Nothing here needs a long-lived token in GitHub.

### GitHub (organization `graphviz-pure`)

- Organization settings → Authentication security → **Require two-factor authentication**
  (the only setting the API cannot change).
- CI and release builds run on GitHub-hosted runners, which are free for a public repository.
  Releases must stay there: npm trusted publishing rejects self-hosted runners, and
  GitHub-hosted builders are the documented basis for SLSA Build Level 3.
- Already configured by script (see the transfer PR): Actions restricted to GitHub-owned
  actions plus `Swatinem/rust-cache` and `rust-lang/crates-io-auth-action`, full-SHA pinning
  required, read-only default workflow token, members cannot create repositories. The
  organization's list has since added `ossf/scorecard-action` and the ClusterFuzzLite
  `google/clusterfuzzlite/actions/build_fuzzers` and `run_fuzzers` (fuzz.yml).

### crates.io

Trusted publishing can only be configured for a crate that exists, so the names are reserved
once with placeholder releases:

1. Log in to crates.io with GitHub; verify your email address.
2. Account settings → API tokens → new token, scope **publish-new**, crates `graphviz-pure*`,
   expiry 1 day. Run `cargo login` and paste it there (not in chat).
3. `scripts/reserve-crates.sh` publishes `0.0.0` placeholders of the eight crate names.
4. For each crate: Settings → Trusted Publishing → add GitHub: owner `graphviz-pure`,
   repository `graphviz-pure`, workflow `release.yml`, environment `release`. Then enable
   **"Require trusted publishing"** for the crate.
5. Delete the API token; `cargo logout`.

### npm (organization `graphviz-pure`)

1. Two-factor authentication on your account (done).
2. `npm login`, then `scripts/reserve-npm.sh` publishes a `0.0.0` placeholder of
   `@graphviz-pure/wasm`, and deprecates it.
3. Package settings → Trusted publishers → GitHub Actions: organization `graphviz-pure`,
   repository `graphviz-pure`, workflow `release.yml`, environment `release`. Under **Allowed
   actions**, allow `npm publish`. Leave `npm dist-tag` unchecked: `npm publish` sets the
   `latest` tag for a stable version. The `release` environment's approval is the only gate.
4. Package settings → Publishing access → **"Require two-factor authentication and disallow
   tokens"**. `npm logout`.

Since 2026-10-02 npm expires a trusted-publisher configuration 48 hours after it is created
unless it has published by then; the first successful publish validates it for good
([changelog](https://github.blog/changelog/2026-10-02-unvalidated-npm-trusted-publishing-configurations-now-expire/)).
If the configuration was created well before the first release, delete it and recreate it with
the same fields just before dispatching, then release within 48 hours.

### PyPI (project `graphviz-pure`)

1. Create an account at https://pypi.org/account/register/, verify the email address, and turn
   on two-factor authentication (Account settings → **Two factor authentication**; PyPI requires
   it before publishing).
2. https://pypi.org/manage/account/publishing/ → **Add a new pending publisher** → **GitHub** tab:
   PyPI Project Name `graphviz-pure`, Owner `graphviz-pure`, Repository name `graphviz-pure`,
   Workflow name `release.yml`, Environment name `release` → **Add**. The first publish creates
   the project and turns the pending publisher into a normal one. A pending publisher does not
   reserve the name, so release soon after.
3. Once the project exists: https://pypi.org/manage/project/graphviz-pure/collaboration/ → add
   the other maintainers as owners. Create no API tokens for it.

Until step 2 is done, the `pypi` job publishes nothing and succeeds with a notice ("PyPI's
trusted publisher is not set up"; once the project exists, a publisher that no longer matches
fails the job instead), `verify` leaves the channel out, and
`scripts/release-verify.sh` reports it as not set up while PyPI has no `graphviz-pure` project.
To publish a release that ran before, open its run's **Publish to PyPI** job and choose **Re-run
this job** (not "Re-run all jobs": the other channels are already published).

Users run `pip install graphviz-pure` (or `uvx graphviz-pure`, `pipx install graphviz-pure`);
the wheels install the `graphviz-pure` command, never `dot` ([python.md](python.md)). The wheel
description is `packaging/pypi/README.md`.

### JSR (scope `@graphviz-pure`)

1. Log in to jsr.io with GitHub and create the scope `@graphviz-pure`, then the package `wasm`.
2. Package settings → **GitHub repository**: link `graphviz-pure/graphviz-pure`. JSR accepts an
   OIDC publish only from a workflow in the linked repository.
3. Scope settings: publishing from GitHub Actions can stay restricted to scope members. The
   workflow run's actor is whoever dispatched it. A dispatch by the release App is that App,
   which is not a scope member, so JSR refuses the publish until the restriction is off or the
   App is accepted. Once the first version is out, also consider requiring every version to be
   published from CI.

`deno publish` makes the version live, and the `release` environment's approval is the only
gate. The JSR package holds the npm tarball's
files as listed in `web/jsr.json` (README.md, LICENSE, NOTICE, THIRD-PARTY-LICENSES.md,
`LICENSES/MIT-CORE-MATH.txt`, `LICENSES/MIT-EXPAT.txt` and `pkg/`), so both registries show `web/README.md`; npm's
`package.json` is not part of it.
In Deno, `init()` fetches the `.wasm` next to the module, so a script importing
`jsr:@graphviz-pure/wasm` needs `--allow-net=jsr.io` (or passes the bytes to `init`).

### Container image (`ghcr.io/graphviz-pure/graphviz-pure`)

No token: the `ghcr` job pushes with its own `GITHUB_TOKEN` (`packages: write` in that job
only). GitHub's documentation gives two answers for the visibility of a package first pushed
from a workflow: private, like every new package, or inherited from the repository. Check
after the first release:

1. `docker logout ghcr.io && docker pull ghcr.io/graphviz-pure/graphviz-pure:0.1.0`. If that
   fails as unauthorized, the package is private: Organization → Packages → `graphviz-pure` →
   Package settings → Danger Zone → **Change visibility** → Public. This cannot be undone. If
   Public is not offered, allow it under Organization settings → Packages → Package creation.
2. In the same settings, **Manage Actions access** should list only
   `graphviz-pure/graphviz-pure` (a publishing repository gets the Admin role automatically), and
   the package should show this repository as its source (the image's
   `org.opencontainers.image.source` label).

### Wasmer (namespace `graphviz-pure`)

Wasmer has no trusted publishing (OIDC), so this is the one channel with a stored token. It is a
secret of the `release` environment: only the `wasmer` job's publish step reads it, and only
after the release is approved.

1. Create an account at https://wasmer.io/signup (signing in with GitHub works). A dedicated
   account for releases is better than a personal one: Wasmer's tokens are not limited to one
   namespace.
2. Create the namespace with the Wasmer CLI (`brew install wasmer`, or https://docs.wasmer.io/install):
   `wasmer login` (opens the browser), then `wasmer namespace create graphviz-pure`, and check it
   with `wasmer namespace get graphviz-pure`. The package `graphviz-pure/graphviz-pure` is
   created by the first publish.
3. https://wasmer.io/settings/access-tokens → create a token named `graphviz-pure release`. Copy
   it; do not paste it anywhere else.
4. GitHub: https://github.com/graphviz-pure/graphviz-pure/settings/environments → **release** →
   **Environment secrets** → **Add environment secret** → name `WASMER_TOKEN`, value the token →
   **Add secret**. (Or `gh secret set WASMER_TOKEN --env release --repo graphviz-pure/graphviz-pure`,
   which prompts for the value without echoing it.)
5. Renew it before it expires, or when a maintainer leaves: create the new token, replace the
   secret, delete the old token.

Until the secret exists, the `wasmer` job publishes nothing and succeeds with a notice ("Wasmer
is not set up"), and `verify` leaves the channel out; once the registry has the package, a
missing secret fails the job instead. `scripts/release-verify.sh` reports the channel as not set
up for as long as the registry has no `graphviz-pure/graphviz-pure`. Once the secret is set,
the next release publishes; to publish a release that already ran, open its run's
**Publish to Wasmer** job and choose **Re-run this job** (not "Re-run all jobs": the other
channels are already published). Pre-releases are never published to Wasmer.

Users run `wasmer run graphviz-pure/graphviz-pure -- -Tsvg < graph.gv > graph.svg`; the program
sees only directories mapped with `--volume` ([`packaging/wasmer/README.md`](../packaging/wasmer/README.md)).
Other WASI runtimes take the release's `graphviz-pure-X.Y.Z-wasm32-wasip1.tar.gz`.

### Homebrew and Scoop

Nothing to set up in this repository. After the GitHub release is published, the release
workflow dispatches `update.yml` in the tap and the bucket; both also run on a 6-hour
schedule. Their update scripts accept an attestation whose source ref is the tag or
`refs/heads/master`. The certificate's subject digest is the dispatch SHA.
Users run `brew install graphviz-pure/tap/graphviz-pure` and
`scoop bucket add graphviz-pure https://github.com/graphviz-pure/scoop-bucket` then
`scoop install graphviz-pure/graphviz-pure`. Submitting to homebrew-core and the Scoop main
bucket comes later; both require a project with some adoption, and homebrew-core builds from
source.

The release workflow's `scoop-smoke` job installs each release's x86-64 and arm64 zips (on an
x86-64 and an arm64 runner) with a manifest written the way the bucket's `scripts/update.sh`
writes it, using Scoop v0.6.0 at a pinned commit. If the bucket's manifest format changes, change that job to match.

### Later channels

[channels.md](channels.md) surveys and ranks every other channel; [python.md](python.md) covers
PyPI and Python's Graphviz libraries. The ones closest to ready:

| channel | what it needs |
| :--- | :--- |
| winget | a manifest pull request to `microsoft/winget-pkgs` per release, made with komac (the first one by hand, `komac new`). komac needs a **classic** token with the `public_repo` scope (fine-grained tokens cannot open the pull request, [komac#310](https://github.com/russellbanks/Komac/issues/310)), so it is kept out of this repository: a pull repository like the tap, owned by a dedicated bot account that owns the `winget-pkgs` fork, runs `komac update --submit` after verifying the release ([channels.md](channels.md#notes-on-the-a-and-b-channels)). Each Windows zip holds one portable `graphviz-pure.exe`, which komac detects as the nested installer |
| Debian/Ubuntu and Fedora repositories | Cloudsmith open-source plan (supports GitHub OIDC, so no stored token), or Fedora COPR and a Launchpad PPA, which build from source |
| AUR | an AUR account and SSH key; the `graphviz-pure-bin` (release binaries) and `graphviz-pure` (from source) PKGBUILDs are in [`packaging/aur/`](../packaging/aur), filled in from a verified release by `packaging/aur/update.sh VERSION` ([steps](../packaging/README.md#aur)) |
| nixpkgs, conda-forge | pull requests to their repositories; no account beyond GitHub. The recipes are in [`packaging/nixpkgs/`](../packaging/nixpkgs) and [`packaging/conda-forge/`](../packaging/conda-forge) ([steps](../packaging/README.md#nixpkgs)) |
| macOS notarization, Windows code signing | an Apple Developer ID (99 USD/year) and Azure Trusted Signing or an OV certificate; until then, binaries downloaded through a browser show a Gatekeeper or SmartScreen warning (Homebrew and Scoop installs do not) |
