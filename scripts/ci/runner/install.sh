#!/usr/bin/env bash
# Installs or updates the self-hosted runners on this machine (docs/ci/self-hosted-runners.md):
# creates the service user gvp-ci, builds the image, copies the scripts to the root-owned
# /jobs/gvp-ci/bin, installs the systemd units and the sudoers rule, and (re)starts the
# supervisor. Run from a checkout of master, as an account with sudo, docker and (the first
# time, for the interim token) a gh login.
set -euo pipefail
dir=$(cd "$(dirname "$0")" && pwd)
root=/jobs/gvp-ci

# The service user: no login, no home; in the docker group to start the containers.
if ! id gvp-ci >/dev/null 2>&1; then
  sudo useradd --system --user-group --no-create-home --home-dir /nonexistent \
    --shell /usr/sbin/nologin gvp-ci
fi
sudo usermod -aG docker gvp-ci
sudo visudo -cf "$dir/sudoers" >/dev/null
sudo install -o root -g root -m 0440 "$dir/sudoers" /etc/sudoers.d/gvp-ci

# $root, bin/ and config.env belong to root, so the service user cannot change the scripts the
# service runs; logs/, scratch/ and state/ belong to the service user.
sudo install -d -o root -g root -m 0755 "$root" "$root/bin"
sudo install -d -o gvp-ci -g gvp-ci -m 0755 "$root/logs" "$root/scratch" "$root/state"
sudo chown -R gvp-ci:gvp-ci "$root/logs" "$root/state"
sudo chown gvp-ci:gvp-ci "$root/scratch"
"$dir/build.sh"
sudo install -o root -g root -m 0755 "$dir/supervisor.sh" "$dir/network.sh" "$dir/gvp-ci" "$root/bin/"
if [ ! -f "$root/config.env" ]; then
  sudo tee "$root/config.env" >/dev/null <<'CFG'
# Read by supervisor.sh at start (sudo systemctl restart gvp-ci-runners after a change).
# Keep GVP_CPUS_PER_RUNNER at 4 or more: scripts/parallel-sfdp-bytes.sh refuses fewer, and the
# jobs size their parallelism by it, as on GitHub's four-CPU runners.
GVP_RUNNERS=4
GVP_CPUS_PER_RUNNER=4
GVP_MEMORY_PER_RUNNER=10g
GVP_RUNTIME=runsc
# repo, or org with GVP_RUNNER_GROUP_ID for the narrow token (docs/ci/self-hosted-runners.md).
GVP_RUNNER_SCOPE=repo
CFG
fi
sudo chown root:root "$root/config.env"
sudo chmod 0644 "$root/config.env"

# Earlier installs let gvp-ci mint gh-bot's App token for cancel and re-run; the service's own
# token does that now (Actions, read and write).
sudo rm -f /etc/sudoers.d/gvp-ci-gh-bot

# The token the service uses, root-only; systemd passes it to the units as a credential.
sudo install -d -o root -g root -m 0700 /etc/gvp-ci
if ! sudo test -s /etc/gvp-ci/token; then
  gh auth token | sudo install -o root -g root -m 0600 /dev/stdin /etc/gvp-ci/token
  echo "install.sh: /etc/gvp-ci/token holds the gh login of $(id -un) for now." >&2
  echo "install.sh: replace it with the narrow token (docs/ci/self-hosted-runners.md, \"The token\")." >&2
fi

sudo install -m 0644 "$dir/gvpci.slice" "$dir/gvp-ci-runners.service" "$dir/gvp-ci-off.service" \
  "$dir/gvp-ci-watchdog.service" "$dir/gvp-ci-watchdog.timer" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable gvp-ci-runners.service gvp-ci-watchdog.timer
sudo systemctl restart gvp-ci-runners.service
sudo systemctl start gvp-ci-watchdog.timer
systemctl --no-pager status gvp-ci-runners.service | head -5
