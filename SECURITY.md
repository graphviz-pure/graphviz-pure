# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's [private vulnerability
reporting](https://github.com/graphviz-pure/graphviz-pure/security/advisories/new) ("Report a
vulnerability" on the Security tab), not in a public issue. You should get a reply within a week. Once a fix is
released, the advisory is published with credit to the reporter unless you prefer otherwise.

Relevant reports include crashes or hangs on crafted input (the port parses untrusted DOT and
runs in browsers), memory-safety problems (the port forbids `unsafe` outside the test-only
`graphviz-pure-portable-libm` crate), and anything in the build and distribution pipeline.

## Supported versions

Only the latest release receives fixes.

## Supply chain

- Every GitHub Action is pinned to a full commit SHA; only GitHub-owned actions,
  `Swatinem/rust-cache`, `ossf/scorecard-action` and the ClusterFuzzLite actions
  (`google/clusterfuzzlite`, for fuzzing) are used. The ClusterFuzzLite build image is pinned
  by digest. Workflows run with read-only tokens by default and are linted with zizmor in CI.
- Rust dependencies come from crates.io only, through `Cargo.lock`, and are checked in CI by
  `cargo deny` (RustSec advisories, licences, sources). npm dependencies are installed from
  `package-lock.json` without lifecycle scripts until their registry signatures are verified.
- Dependabot proposes updates with a 7-day cooldown.

## Releases

A release starts from a workflow dispatch on `master`. Every artifact is built on GitHub-hosted
runners from the release commit, without caches, and gets a signed build-provenance attestation:

    gh attestation verify FILE --repo graphviz-pure/graphviz-pure

The certificate's source ref is `refs/heads/master` (the workflow run's ref). Releases through
0.2.3 were attested for their tag. crates.io, npm, PyPI and JSR are published through trusted
publishing (OIDC), so no registry token exists for them. Publishing needs one approval of the
`release` environment. The full model is at the top of
[`.github/workflows/release.yml`](.github/workflows/release.yml).
