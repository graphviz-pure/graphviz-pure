#!/usr/bin/env bash
# Installs or updates the self-hosted runners on this machine (docs/ci/self-hosted-runners.md):
# builds the image, copies the scripts to /jobs/gvp-ci/bin, installs the systemd units and
# (re)starts the supervisor. Run from a checkout of master; needs sudo and docker.
set -euo pipefail
dir=$(cd "$(dirname "$0")" && pwd)
root=/jobs/gvp-ci
user=$(id -un)
# $root, bin/ and config.env belong to root, so the service user cannot change the scripts the
# service runs; logs/, scratch/ and state/ belong to the service user.
sudo install -d -o root -g root -m 0755 "$root" "$root/bin"
sudo install -d -o "$user" -g "$user" -m 0755 "$root/logs" "$root/scratch" "$root/state"
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
CFG
fi
sudo chown root:root "$root/config.env"
sudo chmod 0644 "$root/config.env"
sudo install -m 0644 "$dir/gvpci.slice" "$dir/gvp-ci-runners.service" "$dir/gvp-ci-off.service" \
  "$dir/gvp-ci-watchdog.service" "$dir/gvp-ci-watchdog.timer" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable gvp-ci-runners.service gvp-ci-watchdog.timer
sudo systemctl restart gvp-ci-runners.service
sudo systemctl start gvp-ci-watchdog.timer
systemctl --no-pager status gvp-ci-runners.service | head -5
