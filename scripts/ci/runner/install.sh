#!/usr/bin/env bash
# Installs or updates the self-hosted runners on this machine (docs/ci/self-hosted-runners.md):
# builds the image, copies the scripts to /jobs/gvp-ci/bin, installs the systemd units and
# (re)starts the supervisor. Run from a checkout of master; needs sudo and docker.
set -euo pipefail
dir=$(cd "$(dirname "$0")" && pwd)
root=/jobs/gvp-ci
sudo mkdir -p "$root"
sudo chown "$(id -u):$(id -g)" "$root"
mkdir -p "$root/bin" "$root/logs" "$root/scratch"
"$dir/build.sh"
install -m 0755 "$dir/supervisor.sh" "$dir/network.sh" "$dir/gvp-ci" "$root/bin/"
[ -f "$root/config.env" ] || cat > "$root/config.env" <<'CFG'
# Read by supervisor.sh at start (systemctl restart gvp-ci-runners after a change).
GVP_RUNNERS=4
GVP_CPUS_PER_RUNNER=4
GVP_MEMORY_PER_RUNNER=10g
GVP_RUNTIME=runsc
CFG
sudo install -m 0644 "$dir/gvp-ci.slice" "$dir/gvp-ci-runners.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable gvp-ci-runners.service
sudo systemctl restart gvp-ci-runners.service
systemctl --no-pager status gvp-ci-runners.service | head -5
