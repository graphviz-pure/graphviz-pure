#!/usr/bin/env bash
# Keeps GVP_RUNNERS idle ephemeral runners registered for graphviz-pure/graphviz-pure, each in
# its own disposable container, and replaces each one after its job. Run by
# gvp-ci-runners.service from its installed copy under /jobs/gvp-ci (install.sh), never from a
# checkout. See docs/ci/self-hosted-runners.md.
#
# Each slot loops: generate a just-in-time runner configuration, start a container with it,
# wait for the runner to take one job and exit, keep its log, delete the container and its
# scratch directory, repeat. The service runs as the system user gvp-ci; its GitHub token comes
# from systemd (LoadCredential=gh-token, from /etc/gvp-ci/token) and never enters a container.
set -uo pipefail

ROOT=${GVP_CI_ROOT:-/jobs/gvp-ci}
# shellcheck source=/dev/null
[ -f "$ROOT/config.env" ] && . "$ROOT/config.env"
REPO=${GVP_REPO:-graphviz-pure/graphviz-pure}
# repo: runners registered on the repository (needs the token's Administration permission).
# org: runners registered in the organization's runner group GVP_RUNNER_GROUP_ID, restricted to
# this repository (needs only the "Self-hosted runners" organization permission).
SCOPE=${GVP_RUNNER_SCOPE:-repo}
GROUP_ID=${GVP_RUNNER_GROUP_ID:-1}
case "$SCOPE" in
  repo) API=repos/$REPO/actions/runners ;;
  org) API=orgs/${REPO%%/*}/actions/runners ;;
  *) echo "GVP_RUNNER_SCOPE must be repo or org" >&2; exit 1 ;;
esac
if [ -n "${CREDENTIALS_DIRECTORY:-}" ] && [ -r "$CREDENTIALS_DIRECTORY/gh-token" ]; then
  GH_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/gh-token")
  export GH_TOKEN
fi
export GH_CONFIG_DIR=${GH_CONFIG_DIR:-${GVP_CI_ROOT:-/jobs/gvp-ci}/state/gh}
RUNNERS=${GVP_RUNNERS:-4}
IMAGE=${GVP_IMAGE:-gvp-ci-runner:current}
RUNTIME=${GVP_RUNTIME:-runsc}
LABELS=${GVP_LABELS:-gvp-ci}
CPUS=${GVP_CPUS_PER_RUNNER:-4}
MEMORY=${GVP_MEMORY_PER_RUNNER:-10g}
MIN_FREE_GB=${GVP_MIN_FREE_GB:-150}
# Below this, the jobs running are killed: a container's writable layer has no size limit
# (/jobs is XFS without project quotas), so this is what stops one job filling the disk.
KILL_FREE_GB=${GVP_KILL_FREE_GB:-50}
PREFIX=${GVP_PREFIX:-gvp-ci}
# Runner names appear in public job logs: no hostname.
HOST=${GVP_HOST_TAG:-h1}
# install.sh makes $ROOT, bin/ and config.env root-owned; these three belong to the service user.
LOGS=$ROOT/logs
SCRATCH=$ROOT/scratch
STATE=$ROOT/state

mkdir -p "$LOGS" "$SCRATCH" "$STATE"
log() { echo "$(date -u +%FT%TZ) $*" >> "$LOGS/supervisor.log"; echo "$*"; }

# Runners of this machine that GitHub still lists but no container serves (left by a crash or a
# reboot) are removed, so the repository's runner list stays the live set.
remove_stale_runners() {
  local live ids
  live=$(docker ps --filter "label=gvp-ci.runner" --format '{{.Label "gvp-ci.runner"}}')
  ids=$(gh api --paginate "$API" \
    --jq ".runners[] | select(.name | startswith(\"$PREFIX-$HOST-\")) | \"\(.id) \(.name) \(.status) \(.busy)\"") || return 0
  while read -r id name status busy; do
    [ -n "$id" ] || continue
    grep -qxF "$name" <<< "$live" && continue
    [ "$busy" = true ] && continue
    gh api -X DELETE "$API/$id" >/dev/null 2>&1 && log "removed stale runner $name ($status)"
  done <<< "$ids"
}

clean_slot() {
  local slot=$1 c="$PREFIX-$1"
  if docker container inspect "$c" >/dev/null 2>&1; then
    docker logs "$c" >> "$LOGS/$c.log" 2>&1 || true
    docker rm -f "$c" >/dev/null 2>&1 || true
  fi
  # Files in the scratch directory belong to the container's uid; root removes them.
  if [ -d "$SCRATCH/$slot" ]; then
    sudo -n rm -rf --one-file-system -- "${SCRATCH:?}/$slot"
  fi
}

free_gb() { df -BG --output=avail /jobs | awk 'NR==2 { gsub(/G/, ""); print $1 }'; }

run_slot() {
  local slot=$1 c="$PREFIX-$1" name jit started rc
  while :; do
    clean_slot "$slot"
    if [ -f "$STATE/paused" ]; then sleep 30; continue; fi
    if [ "$(free_gb)" -lt "$MIN_FREE_GB" ]; then
      log "slot $slot: /jobs has $(free_gb) GB free (< $MIN_FREE_GB), waiting"
      sleep 60; continue
    fi
    name="$PREFIX-$HOST-$slot-$(date -u +%Y%m%d%H%M%S)"
    labels_json=$(jq -cn --arg l "$LABELS" '["self-hosted","linux","x64"] + ($l | split(","))')
    jit=$(jq -n --arg n "$name" --argjson l "$labels_json" \
        --argjson g "$GROUP_ID" '{name: $n, runner_group_id: $g, labels: $l, work_folder: "/home/runner/work"}' \
      | gh api -X POST "$API/generate-jitconfig" --input - --jq .encoded_jit_config 2>> "$LOGS/supervisor.log")
    if [ -z "$jit" ]; then
      log "slot $slot: generate-jitconfig failed, retrying in 60 s"
      sleep 60; continue
    fi
    mkdir -p "$SCRATCH/$slot"
    sudo -n chown 64001:64001 "$SCRATCH/$slot"
    started=$(date +%s)
    if ! GVP_JIT_CONFIG=$jit docker run -d --name "$c" \
        --label "gvp-ci.runner=$name" \
        --runtime "$RUNTIME" \
        --cgroup-parent gvpci.slice \
        --cpus "$CPUS" --memory "$MEMORY" --memory-swap "$MEMORY" --pids-limit 16384 \
        --network gvp-ci --dns 1.1.1.1 --dns 9.9.9.9 \
        --security-opt no-new-privileges --cap-drop ALL --cap-add SETUID --cap-add SETGID \
        --shm-size 2g \
        --mount "type=bind,src=$SCRATCH/$slot,dst=/home/runner/work" \
        -e GVP_JIT_CONFIG \
        "$IMAGE" >/dev/null 2>> "$LOGS/supervisor.log"; then
      log "slot $slot: docker run failed, retrying in 60 s"
      sleep 60; continue
    fi
    log "slot $slot: started $name"
    rc=$(docker wait "$c" 2>/dev/null || echo "?")
    log "slot $slot: $name exited ($rc) after $(( $(date +%s) - started )) s"
    # A runner that fails within a minute is a registration or start-up failure; don't spin.
    if [ "$rc" != 0 ] && [ $(( $(date +%s) - started )) -lt 60 ]; then sleep 30; fi
  done
}

stop_all() {
  log "stopping"
  trap - TERM INT
  # The slot loops first, so none starts a replacement.
  for p in "${pids[@]}"; do pkill -TERM -P "$p" 2>/dev/null; kill -TERM "$p" 2>/dev/null; done
  # The runners deregister when they stop (ephemeral); a job in progress is cancelled.
  [ -n "${sleeper:-}" ] && kill "$sleeper" 2>/dev/null
  # One docker stop for all containers: it stops them in parallel.
  mapfile -t names < <(docker ps -a --filter "label=gvp-ci.runner" --format '{{.Names}}')
  [ "${#names[@]}" -gt 0 ] && docker stop -t 20 "${names[@]}" >/dev/null 2>&1
  for s in $(seq 1 "$RUNNERS"); do clean_slot "$s"; done
  remove_stale_runners
  exit 0
}

trap stop_all TERM INT

log "starting $RUNNERS runners ($IMAGE, runtime $RUNTIME, $CPUS CPUs and $MEMORY each, $SCOPE scope)"
docker image inspect "$IMAGE" >/dev/null || { log "image $IMAGE missing: run build.sh"; exit 1; }
for s in $(seq 1 "$RUNNERS"); do clean_slot "$s"; done
remove_stale_runners
pids=()
for s in $(seq 1 "$RUNNERS"); do run_slot "$s" & pids+=($!); done
# Every minute, check the disk and touch the heartbeat the watchdog reads; every 10 minutes,
# drop registrations no container serves.
tick=0
touch "$STATE/heartbeat"
while :; do
  sleep 60 &
  sleeper=$!
  wait "$sleeper" || true
  if [ "$(free_gb)" -lt "$KILL_FREE_GB" ]; then
    mapfile -t running < <(docker ps --filter "label=gvp-ci.runner" --format '{{.Names}}')
    if [ "${#running[@]}" -gt 0 ]; then
      log "/jobs has $(free_gb) GB free (< $KILL_FREE_GB): killing ${running[*]}"
      docker kill "${running[@]}" >/dev/null 2>&1
    fi
  fi
  touch "$STATE/heartbeat"
  tick=$(( (tick + 1) % 10 ))
  [ "$tick" = 0 ] && remove_stale_runners
done
