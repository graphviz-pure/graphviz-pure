#!/usr/bin/env bash
# Checks that VERSION is out on every channel, and prints pass or fail per channel:
#
#   github  the GitHub release: every asset's build-provenance attestation (from release.yml) and
#           SHA256SUMS
#   crates  every publishable workspace crate at VERSION on crates.io
#   jsr     @graphviz-pure/wasm VERSION in JSR's meta.json, not yanked
#   ghcr    VERSION's attestation verifies, and X.Y and latest point at VERSION's digest where
#           VERSION should hold them: X.Y if it is the newest stable X.Y.*, latest if it is the
#           newest stable version among the image's tags (as the ghcr job moves them; a
#           pre-release or a backport holds neither)
#   pypi    graphviz-pure VERSION on PyPI: one wheel per release platform, none yanked
#           (scripts/check-pypi-wheels.py), each with its build-provenance attestation
#   npm     @graphviz-pure/wasm VERSION is the `latest` dist-tag, and `npm audit signatures`
#           passes for it (only after the owner approved the staged version)
#   taps    the Homebrew tap's formula and the Scoop bucket's manifest carry VERSION, and their
#           newest update.yml run succeeded
#
# --ci checks github, crates, jsr, ghcr and pypi only: release.yml's verify job runs it before npm is
# approved and before the tap and bucket update. --channels LIST checks only those channels
# (comma-separated); --failed LIST reports those as failing at once, without checking (the verify
# job passes the channels whose publishing job did not succeed). --wait SECONDS retries failing
# registry checks (crates.io, JSR, ghcr, PyPI) until they pass or SECONDS have passed in all. Reads only.
#
# Usage: scripts/release-verify.sh VERSION [--ci] [--channels LIST] [--failed LIST] [--wait SECONDS]
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/release-lib.sh

version="" ci=0 wait=0 only="" failed=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ci) ci=1 ;;
    --channels) only=${2?--channels needs a list}; shift ;;
    --failed) failed=${2?--failed needs a list}; shift ;;
    --wait) wait=${2:?--wait needs seconds}; shift ;;
    -*) die "unknown option $1" ;;
    *) version=$1 ;;
  esac
  shift
done
[ -n "$version" ] || die "usage: scripts/release-verify.sh VERSION [--ci] [--wait SECONDS]"
check_version "$version"
tag=v$version
signer=$REPO/.github/workflows/release.yml
image=ghcr.io/$REPO
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
ua="graphviz-pure release check (https://github.com/$REPO)"
fails=0

result() { # result NAME PASS DETAIL
  if [ "$2" = 1 ]; then echo "- [x] $1: $3"; else echo "- [ ] $1: FAIL — $3"; fails=$((fails + 1)); fi
}
# retry CMD...: runs CMD until it succeeds or the deadline ($wait seconds after the start, shared
# by every check) has passed.
deadline=$((SECONDS + wait))
retry() {
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 30
  done
}

echo "## Release $version: channels"
echo

# GitHub release and its attestations.
github() {
  local n=0 bad=() f
  [ "$(gh release view "$tag" --repo "$REPO" --json isDraft -q .isDraft 2> /dev/null)" = false ] ||
    { result github 0 "no published release $tag"; return; }
  gh release download "$tag" --repo "$REPO" --dir "$work/release" > /dev/null
  for f in "$work"/release/*; do
    n=$((n + 1))
    gh attestation verify "$f" --repo "$REPO" --signer-workflow "$signer" --source-ref "refs/tags/$tag" > /dev/null 2>&1 ||
      bad+=("$(basename "$f")")
  done
  if ! (cd "$work/release" && if command -v sha256sum > /dev/null; then sha256sum --check --quiet SHA256SUMS; else shasum -a 256 --check --quiet SHA256SUMS; fi) > /dev/null 2>&1; then
    bad+=(SHA256SUMS)
  fi
  if [ ${#bad[@]} = 0 ]; then result github 1 "$n assets attested, SHA256SUMS matches"
  else result github 0 "not verified: ${bad[*]}"; fi
}

# The workspace crates without publish = false, by package name.
crates() {
  local f
  for f in crates/*/Cargo.toml; do
    grep -qE '^publish *= *false' "$f" && continue
    awk -F'"' '/^\[package\]/{p=1} p && /^name *=/{print $2; exit}' "$f"
  done
}
crates_missing() {
  local c
  : > "$work/crates-missing"
  for c in $(crates); do
    curl -fsS -A "$ua" "https://crates.io/api/v1/crates/$c/$version" 2> /dev/null | jq -e '.version.yanked == false' > /dev/null ||
      echo "$c" >> "$work/crates-missing"
  done
  [ ! -s "$work/crates-missing" ]
}
crates_io() {
  local n
  n=$(crates | wc -l | tr -d ' ')
  [ "$n" -gt 0 ] || { result crates 0 "no publishable crate found under crates/"; return; }
  if retry crates_missing; then result crates 1 "$n crates at $version"
  else result crates 0 "not on crates.io: $(tr '\n' ' ' < "$work/crates-missing")"; fi
}

jsr_ok() {
  curl -fsS "https://jsr.io/@graphviz-pure/wasm/meta.json" -o "$work/jsr.json" 2> /dev/null &&
    jq -e --arg v "$version" '.versions[$v] != null and (.versions[$v].yanked // false) == false' "$work/jsr.json" > /dev/null
}
jsr() {
  if retry jsr_ok; then result jsr 1 "@graphviz-pure/wasm $version (latest: $(jq -r .latest "$work/jsr.json"))"
  else result jsr 0 "@graphviz-pure/wasm $version is not in meta.json"; fi
}

ghcr_token() { curl -fsS "https://ghcr.io/token?scope=repository:$REPO:pull" | jq -r .token; }
# The digest a tag points to, from the registry's API with an anonymous pull token.
digest() {
  curl -fsSI -H "Authorization: Bearer $(ghcr_token)" \
    -H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json" \
    "https://ghcr.io/v2/$REPO/manifests/$1" 2> /dev/null | tr -d '\r' | awk -F': ' 'tolower($1) == "docker-content-digest" {print $2}'
}
# The image's strict SemVer stable tags, oldest first, as the ghcr job lists them.
stable_tags() {
  curl -fsS -H "Authorization: Bearer $(ghcr_token)" "https://ghcr.io/v2/$REPO/tags/list?n=10000" |
    jq -r '.tags[]' | { grep -E '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' || true; } | sort -V
}
# The tags VERSION must hold: itself, X.Y if it is the newest X.Y.*, latest if it is the newest.
ghcr_expected() {
  echo "$version"
  case "$version" in *-*) return ;; esac
  local stable minor=${version%.*}
  stable=$(stable_tags)
  [ "$(printf '%s\n' "$stable" | awk -F. -v m="$minor" '$1 "." $2 == m' | tail -n 1)" = "$version" ] && echo "$minor"
  [ "$(printf '%s\n' "$stable" | tail -n 1)" = "$version" ] && echo latest
  return 0
}
ghcr_tags() {
  local t
  : > "$work/ghcr"
  for t in $(ghcr_expected); do echo "$t $(digest "$t")" >> "$work/ghcr"; done
  [ "$(awk '{print $2}' "$work/ghcr" | sort -u | wc -l | tr -d ' ')" = 1 ] && [ -n "$(awk 'NR == 1 {print $2}' "$work/ghcr")" ]
}
ghcr() {
  if ! retry ghcr_tags; then
    result ghcr 0 "tags differ: $(awk '{printf "%s=%s ", $1, ($2 == "" ? "missing" : substr($2, 1, 19))}' "$work/ghcr")"
  elif ! gh attestation verify "oci://$image:$version" --repo "$REPO" --signer-workflow "$signer" --source-ref "refs/tags/$tag" > /dev/null 2>&1; then
    result ghcr 0 "$image:$version's attestation does not verify"
  else
    local held newer
    held=$(awk '{printf "%s ", $1}' "$work/ghcr")
    case "$held" in *latest*) newer="" ;; *) newer=" (newer releases hold the others)" ;; esac
    result ghcr 1 "${held% } → $(awk 'NR == 1 {print substr($2, 1, 19)}' "$work/ghcr")…, attested$newer"
  fi
}

pypi_check() { # one attempt, from an empty directory
  rm -rf "$work/pypi"
  PYPI_REQUIRED=$ci python3 scripts/check-pypi-wheels.py "$version" "$work/pypi" > "$work/pypi.log" 2>&1
}
pypi() {
  local wheel bad=() line
  if ! retry pypi_check; then
    result pypi 0 "$(tail -n 1 "$work/pypi.log")"
    return
  fi
  line=$(tail -n 1 "$work/pypi.log")
  case "$line" in skipped:*) result pypi 1 "$line"; return ;; esac
  for wheel in "$work"/pypi/*.whl; do
    gh attestation verify "$wheel" --repo "$REPO" --signer-workflow "$signer" --source-ref "refs/tags/$tag" > /dev/null 2>&1 ||
      bad+=("$(basename "$wheel")")
  done
  if [ ${#bad[@]} = 0 ]; then result pypi 1 "$line, each attested"
  else result pypi 0 "not attested: ${bad[*]}"; fi
}

npm_channel() {
  local latest
  latest=$(npm view @graphviz-pure/wasm dist-tags.latest 2> /dev/null || true)
  if [ "$latest" != "$version" ]; then
    result npm 0 "latest is ${latest:-unknown}, not $version (approve the staged version first)"
    return
  fi
  if (cd "$work" && mkdir npm && cd npm && npm init -y > /dev/null &&
    npm install --ignore-scripts --no-audit --no-fund "@graphviz-pure/wasm@$version" > /dev/null 2>&1 &&
    npm audit signatures > "$work/npm-audit" 2>&1); then
    result npm 1 "latest is $version; $(grep -m1 -i 'verified registry signature' "$work/npm-audit" || echo 'npm audit signatures passed')"
  else
    result npm 0 "npm audit signatures failed: $(tail -n 3 "$work/npm-audit" 2> /dev/null | tr '\n' ' ')"
  fi
}

# The tap's formula and the bucket's manifest, and their newest update.yml run.
taps() {
  local repo path run content
  for spec in homebrew-tap:Formula/graphviz-pure.rb scoop-bucket:bucket/graphviz-pure.json; do
    repo=graphviz-pure/${spec%%:*} path=${spec#*:}
    run=$(gh run list --repo "$repo" --workflow update.yml --limit 1 --json status,conclusion,url \
      -q '.[0] | "\(.status) \(if (.conclusion // "") == "" then "-" else .conclusion end) \(.url)"')
    content=$(gh api "repos/$repo/contents/$path" -q .content | base64 --decode)
    # The formula's `version "X.Y.Z"` line, the manifest's "version" field.
    if ! grep -qE "^[[:space:]]*\"?version\"?:?[[:space:]]+\"${version//./\\.}\"" <<< "$content"; then
      result "${spec%%:*}" 0 "$path does not carry $version (newest update.yml run: $run)"
    elif [ "${run%% *}" != completed ] || [ "$(cut -d' ' -f2 <<< "$run")" != success ]; then
      result "${spec%%:*}" 0 "$path carries $version but the newest update.yml run is $run"
    else
      result "${spec%%:*}" 1 "$path carries $version; update.yml ${run##* }"
    fi
  done
}

want() { # want CHANNEL: is it to be checked?
  case ",$failed," in *",$1,"*) result "$1" 0 "its publishing job did not succeed"; return 1 ;; esac
  [ -z "$only" ] || case ",$only," in *",$1,"*) ;; *) return 1 ;; esac
}
want github && github
want crates && crates_io
want jsr && jsr
want ghcr && ghcr
want pypi && pypi
if [ "$ci" = 0 ]; then
  want npm && npm_channel
  want taps && taps
elif case ",$failed," in *,npm,*) true ;; *) false ;; esac; then
  result npm 0 "its publishing job did not succeed"
fi
echo
if [ "$fails" = 0 ]; then echo "**All channels pass.**"; else echo "**$fails channel(s) fail.**"; fi
[ "$fails" = 0 ]
