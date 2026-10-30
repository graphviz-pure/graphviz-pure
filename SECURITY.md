# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's [private vulnerability
reporting](https://github.com/graphviz-pure/graphviz-pure/security/advisories/new) ("Report a
vulnerability" on the Security tab), not in a public issue. You should get a reply within a week. Once a fix is
released, the advisory is published with credit to the reporter unless you prefer otherwise.

Relevant reports include infinite loops, panics or unbounded memory use on small crafted
inputs, memory-safety problems, and anything in the build and distribution pipeline. The
project's own crates forbid `unsafe`, except the test-only `graphviz-pure-portable-libm`
crate. That restriction does not cover dependencies: the vendored resvg stack used for raster
output contains `unsafe`.

## Untrusted input and output

Like C Graphviz, layout cost can be superlinear and is unbounded. Services laying out
untrusted DOT must impose their own input-size, time and memory limits, using an isolated
worker or process that can be terminated on timeout. A finite but superlinear layout is not
a vulnerability; an infinite loop, a panic or unbounded memory use on small input is. There
are no built-in limits and no cancellation API; that work is planned after 1.0.

SVG from untrusted DOT is untrusted markup. As in C, escaping quotes and markup does not
sanitise `URL`/`href` values or filter schemes: the output can contain `javascript:` links.
Sanitise the SVG before embedding it inline (including with `innerHTML`), or display it as an
image with `<img>` instead of inserting it into the document.

## Supported versions

Before 1.0.0, only the latest release receives security fixes. In 1.x, security fixes ship as
patch releases of the latest 1.x minor only; earlier minors are unsupported. After 1.0.0,
0.x receives no security fixes.

## Supply chain

- External GitHub Actions are pinned to full commit SHAs. They are GitHub-owned actions,
  `Swatinem/rust-cache`, `ossf/scorecard-action`, `rust-lang/crates-io-auth-action` and the
  ClusterFuzzLite actions (`google/clusterfuzzlite`). Local actions and reusable workflows
  come from the workflow's commit. The ClusterFuzzLite build image is pinned by digest.
  Workflow tokens are read-only by default; jobs request additional permissions where needed.
  CI lints the workflows with zizmor.
- Registry Rust dependencies come from crates.io through `Cargo.lock`, with local vendored
  raster overrides in `Cargo.toml`. CI runs `cargo deny` (RustSec advisories, licences,
  sources) and checks the bundled licence notices. The npm runtime-test dependencies are
  installed from `package-lock.json` without lifecycle scripts, their registry signatures
  are verified, and only then are their install scripts run.
- Dependabot proposes updates with a 7-day cooldown.
- CI runs on pull requests, merge-queue groups and pushes to `master` and some other branches;
  build and test jobs can reuse a successful result for identical inputs, but the supply-chain
  job always runs.
- ClusterFuzzLite runs weekly on Monday at 04:41 UTC, on manual dispatch and as a release
  gate, not on pull requests. Weekly runs always fuzz and refresh the retained corpus;
  dispatches and releases can reuse a successful fuzzed-inputs marker. Fuzzing failures fail
  the run; corpus-pruning failures warn and prevent a new marker, but do not fail the gate.
  A failed weekly run opens or updates an issue.

## Releases

A real release starts from the owner's workflow dispatch on `master`. `prepare` rejects an unset
`JSR_PUBLISH_ACTOR`, a mismatched actor or a bot before any publishing approval. The workflow
prepares the version and changelog commit locally, then builds from it on GitHub-hosted runners without build
caches, using a pinned Rust toolchain and locked Rust dependencies. Readiness checks, smoke
tests, the `attest` job and the fuzzing gate must pass before the seven publishing jobs wait
together for one approval of the `release` environment. A dry run builds and runs `attest`
without requesting that environment or publishing. Publishing jobs run independently after
approval; a registry can publish even if the GitHub release job fails. npm stages the tarball
and then requires the owner's passkey approval on npmjs.com before it becomes live.

Before approval, the `attest` job creates signed Sigstore build-provenance attestations for
`dist/*` (release archives, the npm tarball and `SHA256SUMS`) and the PyPI wheels. The container
image is attested inside the `ghcr` publishing job, after approval of the `release` environment.
That job is skipped on a dry run, so a dry run does not attest the image. Verify a downloaded
file with:

    gh attestation verify FILE --repo graphviz-pure/graphviz-pure

For the container, use `gh attestation verify oci://ghcr.io/graphviz-pure/graphviz-pure:VERSION
--repo graphviz-pure/graphviz-pure`. The attestation identifies the workflow's source ref
(`refs/heads/master`) and dispatch SHA, rather than the prepared release commit; its subject
digest identifies the artifact bytes. Releases through 0.2.3 were attested for their tag.
The release tag is annotated and unsigned. The workflow performs no macOS Developer ID
signing or notarisation and no Windows Authenticode signing; Sigstore attestations are
separate from platform code signing.

crates.io, npm, PyPI and JSR use OIDC publishing without stored registry tokens. npm stages
with automatically generated provenance (confirmed by staged 0.2.3's `dist.attestations.provenance`);
its trusted publisher permits `npm stage publish`, not direct `npm publish`. Passkey approval
on npmjs.com is the final npm gate. JSR adds provenance through CI's OIDC `deno publish`.
Wasmer uses a stored API token from the `release` environment. While a channel is not yet configured and its package
does not exist, the Wasmer job skips publishing with a notice if its token is missing, and
the PyPI job does likewise if its trusted publisher is not configured. For a new version of
an existing package, a missing Wasmer token or invalid PyPI publisher fails the job.
The container job uses its job-scoped GitHub token to publish to ghcr.io and pushes the image
attestation to the registry. The GitHub release job
uses the release App's credentials from the environment to mint a job-scoped token, push the
release commit and tag, publish the release, and dispatch the Homebrew and Scoop updates.

The implementation is in [release.yml](.github/workflows/release.yml),
[fuzz.yml](.github/workflows/fuzz.yml) and [ci.yml](.github/workflows/ci.yml).
