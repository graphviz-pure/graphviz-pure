# Releasing graphviz-pure

A release is prepared, checked and announced by an agent. The owner does four things: approve the
release pull request, sign and push the tag, approve one `release` deployment, and approve the
staged npm version. Everything the owner runs arrives in one issue, "Release X.Y.Z ready".

## The owner's four steps

1. **Approve the release pull request** (`gh pr review N --approve`). The agent adds it to the
   merge queue.
2. **Sign and push the tag** on the merge commit, which the agent comments in the issue:
   `git tag -s vX.Y.Z -m vX.Y.Z SHA && git push origin vX.Y.Z`. Only maintainers can create
   `v*` tags (tag ruleset), and the workflow refuses a tag that is not on `master` or not signed
   with a key in [`.github/allowed_signers`](../.github/allowed_signers) (read from `master`, not
   from the tagged commit). To sign with SSH:
   `git config gpg.format ssh`, `git config user.signingkey ~/.ssh/id_ed25519.pub`; add the key to
   GitHub as a signing key and, in a reviewed pull request, to `.github/allowed_signers`.
3. **Approve the `release` deployment** of the release run, once: every publishing job waits for
   it together. The agent comments the run's link and a `gh api` command that approves it.
4. **Approve the staged npm version** with 2FA: `npm stage approve <id>` (the agent comments the
   id from the npm job's log), then `npm logout`.

## What the agent does

The agent pushes and opens pull requests and issues as a bot account, never as the owner
(GitHub does not let anyone approve their own pull request): `GH` names that account's gh in
every command below.

1. **When the release's gates are met** (no open `priority:p1` or `priority:p2` issue, master's
   CI green), preview the release: `scripts/release-prep.sh --dry-run`. It reads the
   `## Changelog entry` of every pull request merged since the last tag and prints the version,
   the CHANGELOG section and what is missing:
   - a pull request with no entry: add one to its body (`Not user-visible.` when there is
     nothing to say), with `$GH pr edit N --body-file ...`;
   - an entry marked breaking (under a `### Breaking` heading, or a bullet starting
     `**Breaking:**` or `**Changed (breaking):**`) from a pull request without the `breaking`
     label: label it, or unmark the entry. The labels decide the version;
   - prose beside an entry's bullets, which is not copied: check that nothing is lost.

   Entries that a later pull request superseded are fixed in the CHANGELOG on the release branch.
2. **Open the release pull request:** `GH=... scripts/release-prep.sh` (or with the version, which
   must be the one the labels ask for). It writes the entries under "Unreleased", runs
   `scripts/bump-version.sh X.Y.Z --pr` (sets the version in `Cargo.toml`, `Cargo.lock`,
   `web/package.json` and `web/jsr.json`, dates the section, pushes `release/vX.Y.Z`, opens the
   pull request with `$GH`), and starts the release workflow's dry run and the fuzzing workflow on
   the branch. Write the release's intro paragraph where the script left its placeholder and fix
   superseded entries, in one push. A push restarts CI but not the dry run or fuzzing: start
   `release.yml` and `fuzz.yml` on the branch again (`$GH workflow run release.yml --ref
   release/vX.Y.Z`, the same for `fuzz.yml`). Fuzzing is the release's pre-release step: the
   only fuzzing CI does, once per release, over everything merged since the last one. The first
   run takes over an hour; a later dispatch whose code is unchanged (a changelog-only push)
   reuses its result in a minute. `release-prep.sh` and `release-ping.sh` refuse to write as the
   owner.
3. **Check readiness:** `scripts/release-ready.sh X.Y.Z PR` prints a Markdown checklist and exits
   0 only when it all passes: no open `priority:p1`/`priority:p2` issue and no open
   "Cross-platform fails on master" issue; the newest runs of `ci.yml`, `cross-platform.yml`,
   `fuzz.yml` and the release dry run on the pull request's head succeeded; nothing merged to
   master after the branch's base (else run `release-prep.sh` again); every pull request since
   the last tag has an entry and the labels ask for X.Y.Z; CHANGELOG.md has the section and its
   intro; the three manifests say X.Y.Z; every e2e input identical to C at the last release
   still is.
4. **Ping the owner:** `GH=... scripts/release-ping.sh X.Y.Z PR` opens "Release X.Y.Z ready",
   assigned to the owner and mentioning them, with the pull request, the dry run, the checklist and
   the four steps.
5. **Drive it through**, adding each next command to the issue as the owner gets to it:
   - after the approval, add the pull request to the merge queue; once it merged,
     `scripts/release-ready.sh X.Y.Z PR` again (it now checks the merge commit: master's own
     `ci.yml` and `cross-platform.yml` runs on it decide; for fuzzing and the dry run, the pull
     request head's runs count when the trees are the same), then
     `GH=... scripts/release-ping.sh X.Y.Z tag` comments the tag command with the merge commit;
   - when the release run waits for the environment, `GH=... scripts/release-ping.sh X.Y.Z deploy`;
   - when the npm job has staged the package, `GH=... scripts/release-ping.sh X.Y.Z npm`;
   - after the owner's npm approval, start the tap and bucket updates rather than waiting for
     their 6-hourly schedule: `$GH workflow run update.yml --repo graphviz-pure/homebrew-tap`, and
     the same for `graphviz-pure/scoop-bucket`. When they finish,
     `GH=... scripts/release-ping.sh X.Y.Z done` runs `scripts/release-verify.sh X.Y.Z`, comments
     the result and closes the issue if every channel passes.

`scripts/release-verify.sh X.Y.Z` checks, and prints pass or fail for, each channel: the GitHub
release's assets against their attestations and `SHA256SUMS`; the eight publishable crates on
crates.io (`graphviz-pure-wasm`, `-diff` and `-portable-libm` have `publish = false`); JSR's
`meta.json`; the Wasmer package, which must contain the module of the release's attested WASI
archive byte for byte; the image's attestation, and `X.Y` and `latest` pointing at it where this version
should hold them (not for a backport to an older minor line, nor `latest` for anything but the
newest version); npm's
`latest` dist-tag, an install pinned to the attested release tarball's integrity (`npm ci`
against a lockfile the script writes from it), and `npm audit signatures`; the tap's formula,
the bucket's manifest and
their newest `update.yml` run. Attestations must come from `release.yml` run for the tag, not a
dry run. The release workflow's `verify` job runs it with `--ci` (all but npm and the tap and
bucket) once the publishing jobs finish, and reports a channel whose publishing job failed
without polling it.

### The `breaking` label

A pull request that breaks the Rust or JavaScript API, the command line or a format gets the
`breaking` label when it merges. While the version is `0.x`, a release with one or more such pull
requests bumps the minor version and any other release the patch version
([versioning policy](versioning.md)); `scripts/release-prep.sh` creates the label if it is
missing.

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

On a tag push, the six publishing jobs then wait together for one approval of the `release`
environment, and each publishes on its own:

- `github-release` creates the GitHub release with all files;
- `crates-io` publishes the eight crates to crates.io (trusted publishing,
  `cargo publish --workspace`, leaving out any crate whose version is already there);
- `npm` stages `@graphviz-pure/wasm` on npm (trusted publishing, with npm provenance). The
  trusted publisher may only stage, so the version goes live only when a maintainer approves it
  with 2FA: the package's **Staged Packages** tab on npmjs.com, or `npm stage approve <stage-id>`;
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

Then `verify` checks what they published (`scripts/release-verify.sh --ci`). Within hours,
`graphviz-pure/homebrew-tap` and `graphviz-pure/scoop-bucket` pull the release, verify each
asset's attestation and checksum, and commit the new formula and manifest.
`graphviz-pure/graphviz-pure.github.io` deploys the site from `master` builds the same way. No
repository holds a credential for another.

Run the workflow by hand (Actions → Release → Run workflow, or
`$GH workflow run release.yml --ref BRANCH`) to build, smoke-test and attest everything, the
container image included, without publishing anything: the publishing jobs run only on a tag
push, and the `release` environment accepts only `v*` tags.

### One approval: the trade-off

Through 0.1.2, the registries' jobs waited for the GitHub release, so a release took two approvals:
the GitHub release, then the publishing jobs. The owner could look at the published GitHub
release before anything irreversible went to crates.io or JSR. Now one approval starts all six
jobs at once, and the checks that the second look stood for run before the approval instead:
every smoke test of the run, a dry run of the same workflow on the same commit, and the readiness
checklist. npm still needs its own 2FA approval. What is given up: a registry can publish even if
the GitHub release fails, and nothing is left to stop between the GitHub release and the
registries. Keep `environment: release` on every publishing job (the crates.io and npm trusted
publishers are bound to it) and keep its approval manual: JSR has no staging.

### If a publishing job fails

The publishing jobs are independent, so a failure leaves the other channels published. Fix the
cause (an expired trusted-publisher configuration, a registry outage), then **Re-run failed
jobs** on the same run: never push a new tag for the same version. A re-run runs `release.yml`
as it was in the tagged commit, and the release's tag cannot move, so a bug in the workflow
itself is fixed on `master` and ships with the next version. A re-run asks for the approval
again.

- `crates-io`: safe to re-run, also after a partial publish. The job asks crates.io which
  crates already have the version (`scripts/crates-to-publish.py`) and publishes only the
  others, in dependency order; if all are there, it publishes nothing.
- `npm`, `jsr`: a re-run is safe if the job failed before publishing. If the version went up,
  there is nothing to re-run.
- `wasmer`: safe to re-run. A version already in the registry is left as it is (versions there
  cannot be replaced), and `release-verify.sh` checks that it holds the attested module.
- `ghcr`: safe to re-run. Pushing the same image again changes nothing, and `X.Y`/`latest`
  only ever move forward: they follow the highest stable version among the image's tags on
  ghcr.io, so a `v*` tag whose release was rejected never holds them.
- `github-release`: the release is immutable once published. If the job failed before
  publishing, delete the draft release, then re-run.
- `verify`: re-run it after fixing a channel, or run `scripts/release-verify.sh X.Y.Z` locally.

**First release from a new trusted-publisher configuration:** npm drops a configuration that
has not published within 48 hours of its creation. If it is older than that, delete and recreate
it (same fields, see [npm](#npm-organization-graphviz-pure)) just before pushing the tag.

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
   actions**, leave both `npm publish` and `npm dist-tag` unchecked: the workflow can then only
   stage a release, and each one waits for your 2FA approval.
4. Package settings → Publishing access → **"Require two-factor authentication and disallow
   tokens"**. `npm logout`.

Since 2026-10-02 npm expires a trusted-publisher configuration 48 hours after it is created
unless it has published by then; the first successful (staged) publish validates it for good
([changelog](https://github.blog/changelog/2026-10-02-unvalidated-npm-trusted-publishing-configurations-now-expire/)).
If the configuration was created well before the first release, delete it and recreate it with
the same fields just before pushing the first tag, then release within 48 hours.

### JSR (scope `@graphviz-pure`)

1. Log in to jsr.io with GitHub and create the scope `@graphviz-pure`, then the package `wasm`.
2. Package settings → **GitHub repository**: link `graphviz-pure/graphviz-pure`. JSR accepts an
   OIDC publish only from a workflow in the linked repository.
3. Scope settings: keep publishing from GitHub Actions restricted to scope members (on by
   default), so a release publishes only when a member pushed the tag. Once the first version
   is out, also consider requiring every version to be published from CI.

JSR has no staged-approval step like npm's: `deno publish` makes the version live, and the
`release` environment's approval is the only gate. The JSR package holds the npm tarball's
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

Nothing to set up: the tap and bucket repositories update themselves after each release.
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
