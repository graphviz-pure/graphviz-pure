# Handling inbound pull requests

For maintainers. Two questions, in this order: is it safe to run, and is it worth merging?
Building a pull request runs its code on your machine (every `build.rs`, proc macro and test),
so answer the first question from the diff alone.

## 1. Triage without running anything

```sh
scripts/triage-pr.sh 61
```

It fetches the pull request into `refs/pr/61` without checking it out and prints:

- **who sent it**: author association (first-time contributor, collaborator), account age, and
  how many pull requests they opened on GitHub in the last 30 days. Dozens a day means an
  automated agent working through `good first issue` labels across many repositories. That is not
  disqualifying, but expect no follow-up and judge the change entirely on its own.
- **CI status.** Each workflow run from an outside contributor waits for approval in the Actions
  tab.
- **action pins**: every new `uses: owner/repo@SHA # vX.Y.Z` is checked against the upstream
  tag. A mismatch means the comment lies about what runs.
- **the risk report** from `scripts/pr-risk.py`, which reads git objects only:
  - *Blocking* (the CI job fails): invisible or bidirectional Unicode controls (Trojan Source)
    and symlinks. Close unless there is a very good explanation.
  - *Sensitive paths*: workflows, `CODEOWNERS`, `build.rs`, manifests and lockfiles,
    `deny.toml`, `scripts/`, npm config, licence and policy files. Read every line.
  - *C reference outputs edited*: `tests/golden/**` comes from the C oracle. A hand edit
    changes the answer key so a wrong port "matches".
  - *Ratchet baselines weakened*: a file that matched C now does not. CONTRIBUTING forbids this.
    Also a count in `tests/panic-sites.tsv` that went up: the pull request adds an `unwrap`,
    index or overflow site to the layout or geom crates and must say why
    ([the panic-site ratchet](porting/crashes.md#the-panic-site-ratchet)).
  - *Dependency changes*: new crates, versions and non-crates.io sources; new npm packages with
    install scripts.
  - *Added lines* that spawn processes, reach the network, read the environment or credentials,
    embed files, use `unsafe` or lint exemptions, evaluate code dynamically, or look obfuscated
    (long base64 or byte blobs), plus *removed lines* that held a guard (`forbid`, `permissions:`,
    `persist-credentials: false`, `#[test]`, assertions).

The same report runs in CI as the "PR risk report" job and appears in the run's summary. CI
takes the scanner from the base branch, so a pull request that edits `scripts/pr-risk.py` is
still checked by the old copy (and is itself flagged as a sensitive path).

The report finds suspicious shapes, not intent: a malicious change can look ordinary. Read the
whole diff (`git diff $(git merge-base origin/master refs/pr/61) refs/pr/61`) before you approve
a CI run or build anything.

**Default answers for outside contributors:** changes to `.github/`, the release pipeline,
`deny.toml`, `CODEOWNERS`, `SECURITY.md` or the licence are declined unless we asked for them.
New dependencies need a reason the existing ones cannot cover.

## 2. Approve CI

CI for pull requests runs with a read-only token and no secrets (`pull_request`, never
`pull_request_target`), on throwaway runners. Approving a run is therefore low-risk, but it
still executes the contributor's code: approve only after step 1.

## 3. Run it locally, if you need to

Only once steps 1 and 2 are clean. Use a separate worktree (not `wtp`, whose hooks copy your
gitignored state into it) and a container with no network and none of your credentials:

```sh
git worktree add --detach ~/.cache/graphviz-pure-prs/pr-61 refs/pr/61
cd ~/.cache/graphviz-pure-prs/pr-61
CARGO_HOME=$PWD/.cargo-pr cargo fetch --locked   # downloads crates; runs nothing
id=$(docker create --network none -w /src -e CARGO_HOME=/src/.cargo-pr rust:1 \
       cargo test --workspace --offline --locked)
docker cp -q . "$id":/src && docker start -a "$id"; docker rm -f "$id"
```

`docker cp` copies the tree in, so the container never writes to your disk and the command works
whatever directories your Docker VM shares. Remove the worktree afterwards with
`git worktree remove --force ~/.cache/graphviz-pure-prs/pr-61`.

## 4. Is it worth merging?

The bar is CONTRIBUTING's:

- **Follows the C.** Names the C function it ports or fixes and keeps its structure. Where C
  itself crashes, the port must not, and the comment says so.
- **Proves it with a test that runs.** A new input under `tests/<engine>/` is only compared when
  `tests/golden/<engine>/` has C's output for it; an input C crashes on has none, so it is never
  rendered. Such fixes need a test that renders the input and checks it does not panic.
- **No regressions.** No baseline weakened; refreshed baselines only where more files now match.
- **Changelog** entry in the body, under `## Changelog entry`, for anything a user would notice
  (`Not user-visible.` otherwise), and the `breaking` label on a breaking change. The release
  collects the entries from the bodies; CHANGELOG.md is not edited in the pull request.
- **Housekeeping.** A fix for a `tests/fuzz-open/NNN-*` case removes that directory, which also
  takes it off the What's left page.

If the change is right but incomplete, either ask once or, when "maintainers can push" is true,
push the missing test to the contributor's branch yourself (a missing changelog entry goes in
the body, which a maintainer can edit). That is often
faster for small changes from automated agents. Then approve it and add it to the merge queue
(`gh pr review N --approve && gh pr merge N`). The queue tests it on top of `master` and the pull
requests ahead of it, and squashes it, so the history keeps one reviewed commit; nobody needs to
update the branch first.

## 5. Dependabot

Dependabot's branches live in this repository, and only Dependabot can push to them. Still run
`scripts/triage-pr.sh N`: it verifies every new action SHA against its tag. Updates wait out a
7-day cooldown (`.github/dependabot.yml`). If a pull request has fallen behind `master`, comment
`@dependabot rebase` rather than editing it by hand.

## What the repository enforces

- Every workflow run from an outside contributor needs a maintainer's approval.
- Actions must be pinned to a full SHA, and only selected actions are allowed. zizmor lints
  every workflow.
- The default workflow token is read-only. Workflows cannot approve pull requests.
- `master` accepts changes only through the merge queue: a pull request needs a code owner's
  approval given after its last push, the required checks, and every review thread resolved.
  Force pushes and deletion are blocked. The release workflow's GitHub App is the one bypass,
  and only so it can push the release commit and create the `v*` tag. Nobody else can bypass
  the queue, and nobody else can create a `v*` tag.
- `cargo deny` checks advisories, licences and sources. Secret scanning with push protection is
  on, and vulnerabilities are reported privately (SECURITY.md).
