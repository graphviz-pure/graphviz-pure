# Self-hosted Linux runners

The heavy Linux x86-64 jobs of `ci.yml`, `cross-platform.yml` and `semver.yml` can run on the
project's own machine instead of GitHub's hosted runners. GitHub's free plan runs at most 20
jobs at once for the whole account, and the merge queue waited behind that limit. Short jobs
stay on GitHub's `ubuntu-24.04`, so both pools work at once. macOS, Windows and Linux arm64
jobs stay on GitHub-hosted runners, as do `release.yml`, `fuzz.yml` (the release calls it),
`perf.yml` and `scorecard.yml`.

## Kill switch

One command sends every new job back to GitHub-hosted runners:

```sh
/jobs/gvp-ci/bin/gvp-ci off      # GVP_SELF_HOSTED=false, then cancel and re-run the waiting runs
/jobs/gvp-ci/bin/gvp-ci on       # GVP_SELF_HOSTED=true
/jobs/gvp-ci/bin/gvp-ci requeue  # only the cancel and re-run
/jobs/gvp-ci/bin/gvp-ci status   # the variable, the supervisor, the containers, GitHub's list
                                 # (the list is read with the service's token; needs sudo)
```

It sets the repository variable `GVP_SELF_HOSTED`, which every `runs-on` that can pick a local
runner reads. Without the variable, or with any value but `true`, every job runs on
`ubuntu-24.04`. The same switch from the web: Settings → Secrets and variables → Actions →
Variables.

`runs-on` is read when a job is queued, so the variable alone does not move a job already
waiting for a local runner. `gvp-ci off` therefore also cancels every run of the repository
that has a job queued for the `gvp-ci` labels, waits for it to stop, and re-runs its cancelled
jobs (`gh run rerun --failed`, through gh-bot or the service token: see "The token"). A re-run
reads the variable again, so those jobs go to `ubuntu-24.04`: tested, a job that had run locally
went to `ubuntu-24.04` on its re-run with the switch off. Jobs of those runs that were running on hosted runners are re-run too.
Two cases need a person:
- **The merge queue.** A merge-queue run that is cancelled drops its pull request from the
  queue. The re-run passes, but the pull request must be enqueued again.
- **Switching off by hand without the script** (the web page, or `gh variable set`). Then run
  `gvp-ci requeue`, or cancel and re-run each waiting run.

The switch also turns itself off:
- `gvp-ci-watchdog.timer` runs `gvp-ci watchdog` every two minutes. It runs `gvp-ci off` when
  the variable is on and, for five minutes, one of these holds: `gvp-ci-runners.service` is not
  active; the supervisor has not touched its heartbeat (`/jobs/gvp-ci/state/heartbeat`, every
  minute) for five minutes; or GitHub lists no online `gvp-ci` runner. That covers the
  supervisor being stopped, hung or crashing, Docker failing, the disk guard below, and a reboot
  that does not bring the runners back.
- `gvp-ci-runners.service` gives up after five failed starts in ten minutes and then starts
  `gvp-ci-off.service`, which runs `gvp-ci off` at once.

Each logs to `/jobs/gvp-ci/logs/watchdog.log`. Neither turns the switch back on: check the
runners, then run `gvp-ci on`.

## Which jobs run where

A job runs here when it takes five minutes or more on a hosted runner. Each heavy job's
`runs-on` is the same expression, written once per workflow file as the YAML anchor
`&heavy-linux-runner`; any further heavy job in the same file reuses it as `*heavy-linux-runner`.
Every short job says `runs-on: ubuntu-24.04`.

```yaml
runs-on: &heavy-linux-runner ${{ vars.GVP_SELF_HOSTED == 'true'
  && (contains(fromJSON('["push", "merge_group", "workflow_dispatch"]'), github.event_name)
      || (github.event_name == 'pull_request'
          && github.event.pull_request.head.repo.full_name == github.repository))
  && fromJSON('["self-hosted", "linux", "x64", "gvp-ci"]') || 'ubuntu-24.04' }}
```

So a local runner takes a heavy job of a push, a merge-queue run, a manual run, or a pull
request from a branch of this repository, and only while the variable is on. A fork's pull
request, a schedule and every other event run on `ubuntu-24.04`. To move a job, change its
`runs-on` between the alias and `ubuntu-24.04`.

| Workflow | Heavy: local when switched on (hosted median, minutes) | Always GitHub-hosted |
| --- | --- | --- |
| `ci.yml` | x86-64-v3 fidelity (18.7), WASM end-to-end (7.4), Format, Clippy, Test & WASM (7.2), mincross check (7.1), network simplex check (6.9), routing check (6.8), parallel neato bytes (3.6, named heavy by the owner) | lookup, MSRV, WASM package, supply chain, PR risk, Lean proofs, record; native PGO (Windows, macOS, Linux arm64) |
| `cross-platform.yml` | WebAssembly render (5.5) | lookup, Linux x86-64 render (2.3), the other platforms, compare, site (1.2), record, the failure report |
| `semver.yml` | Stable Rust API (5.7) | |
| `perf.yml` | | all: its baseline came from GitHub's machines |
| `release.yml`, `fuzz.yml`, `scorecard.yml` | | all: publish credentials, OIDC, the release's fuzz run |

The medians are over 13 or more recent hosted runs. In the merge queue, Stable Rust API only
writes a line to the summary, and still takes a local runner for under a minute.

The perf ratchet stays hosted. Callgrind counts the instructions the program runs, and those
can depend on the CPU: the standard library and crates pick code paths by CPU feature at run
time. This machine is a Zen 3 (AVX2, no AVX-512); GitHub's runners have AVX-512. The baseline
in `scripts/bench/ratchet-baseline.json` was measured on GitHub's CPUs. Moving the ratchet
means first showing that this machine gives the same counts for the same commit. Output bytes
do not depend on those code paths, and the cross-platform `compare` job checks that.

## Security model

The repository is public, so anyone can open a pull request. Code from a fork's pull request
must not run here before someone has read it. Five layers:

1. **Approval.** The repository requires approval before a workflow runs for any outside
   contributor's pull request (Settings → Actions → General: "Require approval for all
   external contributors"). Do not loosen it.
2. **runs-on.** The expression above sends a fork's pull request to `ubuntu-24.04`.
3. **The job-started hook.** A fork's pull request runs the workflow files of its own branch,
   so it could rewrite `runs-on` to ask for `gvp-ci` directly. The image sets
   `ACTIONS_RUNNER_HOOK_JOB_STARTED` to `/opt/gvp-ci/job-started.sh`. That is a root-owned
   copy baked into the image from `scripts/ci/runner/job-started.sh`, so a pull request cannot
   change the copy that runs. Before the job's first step it fails the job unless the
   repository is graphviz-pure/graphviz-pure and the event is `push`, `merge_group`,
   `workflow_dispatch`, or `pull_request` with a head repository of
   graphviz-pure/graphviz-pure. A workflow cannot change the hook or what it reads. The runner
   (v2.338.2, `JobHookProvider`) builds the hook's environment from an empty set plus the
   server's `github` and `runner` context, never from the job's `env:`. `event.json` is the
   server's payload. The runner reads the hook's path from its own environment, which a probe
   job in pull request #807 confirmed by setting the variable to `/bin/true`.
4. **Ephemeral containers.** Each runner is registered with a just-in-time configuration
   (`POST /repos/{owner}/{repo}/actions/runners/generate-jitconfig`) for exactly one job, in a
   fresh container that is deleted with its scratch directory when the job ends. Nothing a job
   leaves behind reaches the next one.
5. **Isolation from this machine.**
   - The containers run under gVisor (`--runtime runsc`), a user-space kernel, so a job does
     not make system calls into the host kernel.
   - Each job runs as the unprivileged user `runner` (uid and gid 64001, unused on the host)
     with no capabilities and `no-new-privileges`.
   - There is no sudo: a shim lets the jobs' `sudo apt-get` lines succeed when the package is
     already in the image, and fails anything else.
   - The entrypoint keeps CAP_SETUID and CAP_SETGID only to write `/etc/resolv.conf` and
     switch to `runner`. That switch clears them.
   - The only host mount is the job's scratch directory `/jobs/gvp-ci/scratch/<slot>`.
   - The containers are on their own network, `gvp-ci` (172.31.240.0/24, bridge `gvpci0`).
     There, `network.sh` drops all traffic to the host itself and to private, loopback,
     link-local (the cloud metadata address included), tailnet, benchmarking and reserved
     ranges. A job reaches the internet and nothing local. IPv6 is off on the bridge.

What a job can see: the `GITHUB_TOKEN` GitHub issues to it, as on a hosted runner, and its own
runner's JIT configuration. That configuration is on the runner's command line and in
`/opt/actions-runner`, both readable by the job's own user. It registers one runner for one
job, which is already running, so it is worth nothing to the job.

What stays on the host:
- **The service user.** The supervisor, the watchdog and `gvp-ci-off.service` run as the system
  user `gvp-ci`, which has no login shell and no home. It is in the docker group, which is
  equivalent to root on this machine, since that is how it starts containers. Its sudo rights
  (`scripts/ci/runner/sudoers`) are two commands: removing a scratch slot and handing it to the
  containers' uid (`rm`, and `chown --no-dereference`, so a symlink planted as a slot cannot hand
  over its target). While `GVP_ACTIONS_TOKEN=gh-bot`, a third rule
  (`scripts/ci/runner/sudoers-gh-bot`) lets it mint gh-bot's short-lived App token for cancelling
  and re-running runs. That App token can do far more, admin merges included, which is why the
  rule goes once the service token can do the job (below).
- **Root-owned files.** `/jobs/gvp-ci`, `bin/` and `config.env` are root-owned, so `gvp-ci`
  cannot change what its own service runs.
- **The token.** Its GitHub token is `/etc/gvp-ci/token`, root-only (0600 in a 0700
  directory). systemd hands it to the three units alone (`LoadCredential=`), and it never
  enters a container.

`/etc/gvp-ci/token` is the narrow fine-grained token below (since #808): the organization
permission "Self-hosted runners" and the repository permission "Variables", nothing else. An
install that has not done those steps yet holds a copy of the installing account's `gh` login
instead, which for the owner is the full account: replace it.

### The token

The runner calls need these permissions:
- registering runners and generating JIT configurations;
- listing runners and deleting idle registrations;
- setting `GVP_SELF_HOSTED`.

For runners registered on the repository, a fine-grained token needs the repository's
"Administration" permission, which also covers every repository setting. Registered in an
organization runner group restricted to this repository, the same calls need only the
organization permission "Self-hosted runners". The supervisor supports both
(`GVP_RUNNER_SCOPE=repo|org`; this machine uses `org`, runner group 12). The narrow setup
needs the owner's clicks:

1. **Runner group.** Organization settings → Actions → Runner groups → New runner group.
   - Name: `gvp-ci`.
   - Repository access: "Selected repositories", graphviz-pure only.
   - Tick "Allow public repositories".
   - Workflow access: all workflows.
   - Create it, and note the number at the end of its URL (the group id).
2. **Token.** Your settings → Developer settings → Personal access tokens → Fine-grained
   tokens → Generate new token.
   - Name: `gvp-ci runners`.
   - Resource owner: graphviz-pure.
   - Expiration: the longest the organization allows; put the date in your calendar.
   - Repository access: "Only select repositories", graphviz-pure.
   - Repository permissions: Variables → Read and write (Metadata → Read is added by itself).
   - Organization permissions: Self-hosted runners → Read and write.
   - Generate, and copy the token.
3. **Approval.** If the organization requires approval of fine-grained tokens, approve it under
   Organization settings → Personal access tokens → Pending requests.
4. **Install.** On the machine:

   ```sh
   sudo install -o root -g root -m 0600 /dev/stdin /etc/gvp-ci/token   # paste, then Ctrl-D
   sudoedit /jobs/gvp-ci/config.env   # GVP_RUNNER_SCOPE=org and GVP_RUNNER_GROUP_ID=<group id>
   sudo systemctl restart gvp-ci-runners
   /jobs/gvp-ci/bin/gvp-ci status
   ```

   `status` must list four online runners. Then delete the repository-level runners that
   GitHub still lists, if any: Settings → Actions → Runners.

`gvp-ci status` reads the runner list with this token, not the caller's login, which cannot
list organization runners: it runs `gvp-ci runners` as `gvp-ci` in a transient unit
(`systemd-run`, with the same `LoadCredential=`), so the token never reaches the shell.

Cancelling and re-running runs (the watchdog, `gvp-ci off`, `gvp-ci requeue`) uses gh-bot, the
project's GitHub App, while `GVP_ACTIONS_TOKEN=gh-bot` (the default). To move it to this token
and drop gh-bot's mint rule:

1. The token's settings (Your settings → Developer settings → Personal access tokens →
   Fine-grained tokens → `gvp-ci runners` → Edit) → Repository permissions → Actions → Read and
   write → Update. Approve the change if the organization asks.
2. On the machine: `sudoedit /jobs/gvp-ci/config.env`, set `GVP_ACTIONS_TOKEN=service`, then run
   `scripts/ci/runner/install.sh` from a checkout of master. It removes
   `/etc/sudoers.d/gvp-ci-gh-bot`.
3. Check: `/jobs/gvp-ci/bin/gvp-ci requeue` logs "no run is waiting for a local runner" (it
   lists runs with the token), and `sudo -l -U gvp-ci` lists no `gvp-app-token-mint`.

If the token expires, the supervisor cannot register runners. The watchdog then sees no runner
online, but it cannot set the variable with an expired token either: renew the token before it
expires, or switch off from the web page.

Residual risk: code that did not start in this repository still runs here, inside gVisor, in
three ways:
- **The merge queue.** A fork's pull request, once someone enqueues it, runs here as a
  `merge_group` run.
- **Pushes to master.** The merged commit runs here as a `push` run.
- **Dependabot.** Its pull requests are branches of this repository, so their `pull_request`
  runs are local from the first push. A dependency update can bring in a new crate's build
  script or proc macro, which runs at build time.

The rule, for every fork pull request before it is enqueued and every Dependabot pull request
before it is merged: read every change to
- `build.rs` files, proc-macro crates and `Cargo.toml`/`Cargo.lock` (new or changed crates and
  their sources);
- `.github/`, `scripts/` and anything a workflow runs;
- tests and test data that run programs or open the network;
- `package.json`/`package-lock.json` under `web/`, `site/` and `scripts/` (npm install scripts).

A change there that you cannot account for blocks the merge. Turning the switch off
(`gvp-ci off`) is the fallback for running a pull request you have not read fully: it then runs
on GitHub's runners instead of here.

Runner names appear in public job logs, so they carry a slot number and a time, not the
machine's name.

`zizmor --persona=pedantic` (the supply-chain job) has no findings. `--persona=auditor` reports
its `self-hosted-runner` warning once for each heavy job's `runs-on`: expected, and the reason
for this section.

## Resources

| Limit | Value | Where |
| --- | --- | --- |
| All runners together | 14 CPUs (half of 28), 43 GB memory (40 GB soft) | `gvpci.slice` |
| CPU and IO weight | 20 (other groups default to 100) | `gvpci.slice` |
| Each runner | 4 CPUs, 10 GB memory, no swap, 16,384 processes | `/jobs/gvp-ci/config.env` |
| Runners | 4 | `/jobs/gvp-ci/config.env` |
| Disk | a slot waits while `/jobs` has under 150 GB free; below 50 GB the running jobs are killed | `supervisor.sh` |

The disk guard exists because a container's writable layer (`~/.cargo` and `~/.rustup` are in
it, outside the scratch mount) has no size limit: `/jobs` is XFS without project quotas, so
Docker cannot cap it. The supervisor checks every minute.

Each runner has four CPUs because the hosted runner has four and the jobs assume it.
`scripts/parallel-sfdp-bytes.sh` refuses to run with fewer: `nproc` below 4 would compare the
sequential path with itself. Cargo and the test harness also size their parallelism by
`nproc`, which under gVisor is the container's CPU limit. Keep `GVP_CPUS_PER_RUNNER` at 4 or
more. Four runners of four ask for 16 CPUs from a 14-CPU slice, so the quota binds only when
all four are saturated at once. The trials:

| Runners × CPUs | Demand on 14 CPUs | Result |
| --- | --- | --- |
| 6 × 4 | 24 (1.7×) | every job passed but the two that needed a tool the image then lacked; the slice was throttled for 690 s |
| 6 × 4, plus one more 4-CPU container | 28 (2×) | `ipsep_vpsc_cycle_error_ends_the_layout`, which must finish in 20 s (5 s on an idle slice), timed out |
| 7 × 2 | 14 (1×) | no throttling, but the sfdp byte check refused two CPUs |
| 4 × 4 | 16 (1.1×) | all green; 76 throttling events, 22 s in all |

That test's 20-second deadline is the one most likely to flake if the machine is busy, since
the runners yield to everything else. A timeout there is contention, not a port bug; re-run
it. The peak memory of the whole slice in the trials was 14.9 GB.

The weight makes the runners yield whenever the CPUs are contended. The slice sits at the top
of the cgroup tree, beside `user.slice`, `system.slice` and `gvp-delegates`, so the weight
applies against them: the delegates' builds (`gvp-delegates`, capped at 80%) and everything
else win over CI. The supervisor itself runs at nice 15. Docker's data root, the images and
the scratch directories are all on `/jobs`; nothing goes to the root filesystem.

## Setup

From a checkout of master on the machine:

```sh
scripts/ci/runner/install.sh
```

It:
- creates the system user `gvp-ci` and installs its sudoers rule;
- the first time, writes the installing account's `gh` login to `/etc/gvp-ci/token`, until the
  narrow token replaces it;
- builds the image (`build.sh`);
- installs the supervisor, `network.sh` and `gvp-ci` into the root-owned `/jobs/gvp-ci/bin`;
- writes the root-owned `/jobs/gvp-ci/config.env` the first time;
- installs `gvpci.slice`, `gvp-ci-runners.service`, `gvp-ci-off.service`,
  `gvp-ci-watchdog.service` and `gvp-ci-watchdog.timer` into `/etc/systemd/system`;
- enables the service and the timer, and restarts the service.

The service runs the installed copies, so a branch or a pull request never changes the running
supervisor. Run `install.sh` again after a change to `scripts/ci/runner/` lands on master.

| Path | What |
| --- | --- |
| `scripts/ci/runner/Dockerfile` | the image: Ubuntu 24.04 by digest; the Actions runner, `gh` and Node pinned with SHA-256 |
| `scripts/ci/runner/job-started.sh` | the event and repository check (installed in the image at `/opt/gvp-ci/job-started.sh`) |
| `scripts/ci/runner/entrypoint.sh` | writes public DNS resolvers, drops to `runner`, starts the JIT runner |
| `scripts/ci/runner/supervisor.sh` | keeps the runners registered, replaces each after its job, guards the disk |
| `scripts/ci/runner/gvp-ci` | the switch, the cancel and re-run, the status and the watchdog |
| `scripts/ci/runner/sudoers` | the service user's sudo rights, installed as `/etc/sudoers.d/gvp-ci` |
| `scripts/ci/runner/sudoers-gh-bot` | gh-bot's mint rule, installed as `/etc/sudoers.d/gvp-ci-gh-bot` only while `GVP_ACTIONS_TOKEN=gh-bot` |
| `/etc/gvp-ci/token` | the service's GitHub token, root-only |
| `/jobs/gvp-ci/logs/` | `supervisor.log`, `service.log`, `watchdog.log`, one log per slot's runner containers |
| `/jobs/gvp-ci/state/` | the supervisor's heartbeat; the watchdog's timer; `paused`, which makes idle slots wait |

The supervisor recovers from a reboot: the service is enabled and restarts on failure,
`network.sh` reapplies the firewall rules at every start, and at start and every ten minutes the
supervisor deletes registrations that no container serves. A stop takes a few seconds and
deletes the idle runners' registrations; a job still running on one is cancelled.

### The image

The image stands in for GitHub's `ubuntu-24.04` for these jobs only. It has the tools they call
without installing:
- a C toolchain, Python 3.12, `jq`, `git`, `curl`, `unzip`, `zstd`, `column`, `bc` and `gh`;
- Node.js 20 on `PATH`, as on the hosted image, for the Python tests that run `node` directly
  (jobs that pin a Node version still use `actions/setup-node`);
- Google Chrome, for the wasm-runtimes job (short, so hosted now; kept so it can move);
- rustup with stable;
- the packages jobs install with apt (`musl-tools`, `qemu-user`, `valgrind`).

`CARGO_HOME` and `RUSTUP_HOME` stay unset, as on the hosted image, because Swatinem/rust-cache
hashes them into its key; set, they would stop these runners sharing the hosted caches. The
image lacks what the hosted image lacks wherever a job's output depends on it, such as
`wasm-opt`. A job that fails for a missing tool names it; add it to the Dockerfile.

To update the runner: change `RUNNER_VERSION` and `RUNNER_SHA256` from the release notes of
actions/runner, and run `install.sh`. GitHub refuses runners more than about a month behind the
current release.

### Changing the number of runners

Edit `GVP_RUNNERS` in `/jobs/gvp-ci/config.env` (it is root-owned: `sudoedit`) and run
`sudo systemctl restart gvp-ci-runners`. A restart cancels the jobs the runners hold, so do it
while the variable is off or the queue is empty.
