#!/usr/bin/env bash
# Run lake for proofs/ with one shared build directory for every worktree on this machine.
#
# Why: Mathlib's compiled oleans are several GB. Worktrees live on the small root disk, so
# proofs/.lake is a symlink into /jobs/work/gvp-lean/dotlake (one copy, built once). Runs are
# serialised with flock, niced, and joined to the delegate cgroup, like cargo-slot.sh.
#
# Usage: scripts/proofs/lake.sh build          # the default target
#        scripts/proofs/lake.sh exe cache get  # fetch Mathlib's prebuilt oleans
#        scripts/proofs/lake.sh env lean GvpProofs/Twopi/LeafDist.lean
set -euo pipefail
root=$(git rev-parse --show-toplevel)
: "${GVP_LEAN_HOME:=/jobs/work/gvp-lean}"
export ELAN_HOME="${ELAN_HOME:-$GVP_LEAN_HOME/elan}"
export PATH="$ELAN_HOME/bin:$PATH"
shared="$GVP_LEAN_HOME/dotlake"
mkdir -p "$shared" "$GVP_LEAN_HOME/locks"
cd "$root/proofs"
if [ -e .lake ] && [ ! -L .lake ]; then
  echo "lake.sh: proofs/.lake is a real directory; remove it so it can be shared" >&2
  exit 65
fi
[ -L .lake ] || ln -s "$shared" .lake
exec {fd}>"$GVP_LEAN_HOME/locks/lake.lock"
flock "$fd"
if [ -d /sys/fs/cgroup/gvp-delegates ]; then
  echo $$ | sudo -n tee /sys/fs/cgroup/gvp-delegates/cgroup.procs >/dev/null 2>&1 || true
fi
renice -n 15 -p $$ >/dev/null 2>&1 || true
exec lake "$@"
