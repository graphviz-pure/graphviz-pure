#!/usr/bin/env bash
# Starts one ephemeral runner from the just-in-time configuration the supervisor generated
# (POST /repos/{owner}/{repo}/actions/runners/generate-jitconfig). The runner takes one job and
# exits; the supervisor then removes this container and its scratch directory.
#
# The container starts as root with only CAP_SETUID and CAP_SETGID, for two things: point
# /etc/resolv.conf at public resolvers (Docker's embedded resolver at 127.0.0.11 does not answer
# under gVisor), then become the runner user. That switch clears every capability, and
# no-new-privileges keeps them cleared, so no step of the job runs as root.
set -euo pipefail
if [ "$(id -u)" = 0 ]; then
  printf 'nameserver 1.1.1.1\nnameserver 9.9.9.9\noptions edns0\n' > /etc/resolv.conf
  exec env HOME=/home/runner USER=runner LOGNAME=runner \
    setpriv --reuid=runner --regid=runner --init-groups "$0" "$@"
fi
jit=${GVP_JIT_CONFIG:?GVP_JIT_CONFIG is not set}
unset GVP_JIT_CONFIG
mkdir -p /home/runner/work/_tool
cd /opt/actions-runner
exec ./run.sh --jitconfig "$jit"
