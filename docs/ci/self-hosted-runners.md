# Self-hosted Linux runners

The Linux x86-64 jobs of `ci.yml`, `cross-platform.yml` and `semver.yml` can run on the
project's own machine instead of GitHub's hosted runners. GitHub's free plan runs at most 20
jobs at once for the whole account, and the merge queue waited behind that limit. macOS,
Windows and Linux arm64 jobs stay on GitHub-hosted runners, as do `release.yml`, `fuzz.yml`
(the release calls it), `perf.yml` and `scorecard.yml`.

## Kill switch

One command sends every new job back to GitHub-hosted runners:

```sh
/jobs/gvp-ci/bin/gvp-ci off      # GVP_SELF_HOSTED=false
/jobs/gvp-ci/bin/gvp-ci on       # GVP_SELF_HOSTED=true
/jobs/gvp-ci/bin/gvp-ci status   # the variable, the supervisor, the containers, GitHub's list
```

It sets the repository variable `GVP_SELF_HOSTED`, which every `runs-on` that can pick a local
runner reads. Without the variable, or with any value but `true`, every job runs on
`ubuntu-24.04`. The same switch from the web: Settings → Secrets and variables → Actions →
Variables. Turn the variable off before stopping the supervisor; a job already routed to a
local label waits for a local runner. Jobs that are running finish where they are.

## Which jobs run where

Each Linux job's `runs-on` is the same expression, written once per workflow file as the YAML
anchor `&linux-runner` and reused as `*linux-runner`:

```yaml
runs-on: &linux-runner ${{ vars.GVP_SELF_HOSTED == 'true'
  && (contains(fromJSON('["push", "merge_group", "workflow_dispatch"]'), github.event_name)
      || (github.event_name == 'pull_request'
          && github.event.pull_request.head.repo.full_name == github.repository))
  && fromJSON('["self-hosted", "linux", "x64", "gvp-ci"]') || 'ubuntu-24.04' }}
```

So a local runner takes a push, a merge-queue run, a manual run, or a pull request from a
branch of this repository, and only while the variable is on. A fork's pull request, a
schedule and every other event run on `ubuntu-24.04`. In `cross-platform.yml` the Linux
x86-64 render runs wherever that workflow's lookup job ran (`RUNNER_ENVIRONMENT`).

| Workflow | Local when switched on | Always GitHub-hosted |
| --- | --- | --- |
| `ci.yml` | every Linux x86-64 job | native PGO (Windows, macOS, Linux arm64) |
| `cross-platform.yml` | lookup, Linux x86-64 render, wasm, compare, site, record | other platforms; the failure report (`issues: write`) |
| `semver.yml` | the API check | |
| `perf.yml` | | all: its baseline came from GitHub's machines |
| `release.yml`, `fuzz.yml`, `scorecard.yml` | | all: publish credentials, OIDC, the release's fuzz run |

The perf ratchet stays hosted. Callgrind counts the instructions the program runs, and those
can depend on the CPU: the standard library and crates pick code paths by CPU feature at run time.
The baseline in `scripts/bench/ratchet-baseline.json` was measured on GitHub's CPUs. Moving the
ratchet means first showing that this machine gives the same counts for the same commit.

## Security model

The repository is public, so anyone can open a pull request. Code from a fork's pull request
must never run here. Four layers keep it out:

1. **runs-on.** The expression above sends a fork's pull request to `ubuntu-24.04`.
2. **The job-started hook.** A fork's pull request runs the workflow files of its own branch,
   so it could rewrite `runs-on` to ask for `gvp-ci` directly. The runner image sets
   `ACTIONS_RUNNER_HOOK_JOB_STARTED` to `scripts/ci/runner/job-started.sh`, which runs before
   the job's first step and fails the job unless the repository is graphviz-pure/graphviz-pure
   and the event is `push`, `merge_group`, `workflow_dispatch`, or `pull_request` with a head
   repository of graphviz-pure/graphviz-pure. A workflow's `env:` cannot change the hook: the
   runner reads it from its own environment. A probe job that set the variable to `/bin/true`
   still ran the hook (pull request #807).
3. **Ephemeral containers.** Each runner is registered with a just-in-time configuration
   (`POST /repos/{owner}/{repo}/actions/runners/generate-jitconfig`) for exactly one job, in a
   fresh container that is deleted with its scratch directory when the job ends. Nothing a job
   leaves behind reaches the next one.
4. **Isolation from this machine.** The containers run under gVisor (`--runtime runsc`), a
   user-space kernel, so a job does not make system calls into the host kernel. Each job runs as
   the unprivileged user `runner` (uid 1001) with no capabilities, `no-new-privileges` and no
   sudo (a shim makes the jobs' `sudo apt-get` lines succeed when the package is already in the
   image and fails anything else). The only host mount is the job's scratch directory
   `/jobs/gvp-ci/scratch/<slot>`. The containers are on their own network, `gvp-ci`
   (172.31.240.0/24, bridge `gvpci0`), where `network.sh` drops all traffic to the host itself
   and to private, loopback, link-local and tailnet ranges, so a job reaches the internet and
   nothing local. No credential is in a container: the supervisor's `gh` login stays on the
   host, and the job's only token is the `GITHUB_TOKEN` GitHub issues to it, as on a hosted
   runner.

The supervisor uses the owner's `gh` login, approved for exactly three things: registering
runners, generating JIT configurations and setting `GVP_SELF_HOSTED`. Runner names appear in
public job logs, so they carry a slot number and a time, not the machine's name.

## Resources

| Limit | Value | Where |
| --- | --- | --- |
| All runners together | 14 CPUs (half of 28), 43 GB memory (40 GB soft) | `gvp-ci.slice` |
| CPU and IO weight | 20 (other groups default to 100) | `gvp-ci.slice` |
| Each runner | 4 CPUs, 10 GB memory, no swap, 16,384 processes | `supervisor.sh` |
| Runners | 6 | `/jobs/gvp-ci/config.env` |
| Disk | a slot waits while `/jobs` has under 150 GB free | `supervisor.sh` |

The weight makes the runners yield whenever the CPUs are contended: the delegates' builds
(`gvp-delegates`, capped at 80%) and everything else win over CI. The supervisor itself runs at
nice 15. Docker's data root, the images and the scratch directories are all on `/jobs`; nothing
goes to the root filesystem.

## Setup

From a checkout of master on the machine:

```sh
scripts/ci/runner/install.sh
```

It builds the image (`build.sh`), copies the supervisor, `network.sh` and `gvp-ci` to
`/jobs/gvp-ci/bin`, writes `/jobs/gvp-ci/config.env` the first time, installs
`gvp-ci.slice` and `gvp-ci-runners.service` into `/etc/systemd/system`, enables the service
and restarts it. The service runs the installed copies, so a branch or a pull request never
changes the running supervisor. Run `install.sh` again after a change to `scripts/ci/runner/`
lands on master.

| Path | What |
| --- | --- |
| `scripts/ci/runner/Dockerfile` | the image: Ubuntu 24.04 by digest, the Actions runner and `gh` pinned with SHA-256 |
| `scripts/ci/runner/job-started.sh` | the event and repository check |
| `scripts/ci/runner/entrypoint.sh` | writes public DNS resolvers, drops to `runner`, starts the JIT runner |
| `scripts/ci/runner/supervisor.sh` | keeps the runners registered and replaces each after its job |
| `/jobs/gvp-ci/logs/` | `supervisor.log`, `service.log`, one log per slot's runner containers |

The supervisor recovers from a reboot: the service is enabled and restarts on failure,
`network.sh` reapplies the firewall rules at every start, and at start and every ten minutes the
supervisor deletes registrations that no container serves.

### The image

The image stands in for GitHub's `ubuntu-24.04` for these jobs only. It has the tools they call
without installing: a C toolchain, Python 3.12, `jq`, `git`, `curl`, `unzip`, `zstd`, `gh`,
Google Chrome (the wasm-runtimes job), rustup with stable, and the packages jobs install with
apt (`musl-tools`, `qemu-user`, `valgrind`). Node comes from `actions/setup-node`, as on hosted
runners. It deliberately lacks what the hosted image lacks where a job's output depends on it,
such as `wasm-opt`. A job that fails for a missing tool names it; add it to the Dockerfile.

To update the runner: change `RUNNER_VERSION` and `RUNNER_SHA256` from the release notes of
actions/runner, and run `install.sh`. GitHub refuses runners more than about a month behind the
current release.

### Changing the number of runners

Edit `GVP_RUNNERS` in `/jobs/gvp-ci/config.env` and run
`sudo systemctl restart gvp-ci-runners`. A restart cancels the jobs the runners hold, so do it
while the variable is off or the queue is empty.
