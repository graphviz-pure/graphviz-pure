#!/usr/bin/env bash
# Checks that VERSION is out on every channel, and prints pass or fail per channel:
#
#   github  the GitHub release: every asset's build-provenance attestation (from release.yml) and
#           SHA256SUMS
#   crates  every publishable workspace crate at VERSION on crates.io
#   binstall  graphviz-pure-cli VERSION's cargo-binstall metadata, as published on crates.io,
#           names every release archive and the binary in it (scripts/check-binstall.py), and,
#           where cargo-binstall is on PATH, `cargo-binstall --dry-run` fetches and unpacks the
#           archive for every release target from the published crate
#   jsr     @graphviz-pure/wasm VERSION in JSR's meta.json, not yanked
#   ghcr    VERSION's attestation verifies, and X.Y and latest point at VERSION's digest where
#           VERSION should hold them: X.Y if it is the newest stable X.Y.*, latest if it is the
#           newest stable version among the image's tags (as the ghcr job moves them; a
#           pre-release or a backport holds neither)
#   wasmer  graphviz-pure/graphviz-pure VERSION in the Wasmer registry, and its package holds the
#           module of the release's attested wasm32-wasip1 archive byte for byte
#   pypi    graphviz-pure VERSION on PyPI: one wheel per release platform, none yanked
#           (scripts/check-pypi-wheels.py), each with its build-provenance attestation
#   npm     @graphviz-pure/wasm VERSION is the `latest` dist-tag, an install pinned to the
#           attested release tarball's integrity, and `npm audit signatures` passes
#   taps    the Homebrew tap's formula and the Scoop bucket's manifest carry VERSION, and their
#           newest update.yml run succeeded
#
# --ci checks every channel but the tap and bucket: release.yml's verify job runs it once the
# publishing jobs finish, and the tap and bucket updates are dispatched rather than waited on.
# --channels LIST checks only those channels
# (comma-separated); --failed LIST reports those as failing at once, without checking (the verify
# job passes the channels whose publishing job did not succeed). --wait SECONDS retries failing
# registry checks (crates.io, JSR, ghcr, Wasmer, PyPI) until they pass or SECONDS have passed in all. Reads only.
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
# Releases through 0.2.3 were attested for the tag. A release cut by workflow_dispatch is
# attested for refs/heads/master: the certificate's source ref is the workflow run's ref.
attest() { # attest SUBJECT
  gh attestation verify "$1" --repo "$REPO" --signer-workflow "$signer" --source-ref "refs/tags/$tag" > /dev/null 2>&1 && return 0
  gh attestation verify "$1" --repo "$REPO" --signer-workflow "$signer" --source-ref "refs/heads/master" > /dev/null 2>&1
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
    attest "$f" || bad+=("$(basename "$f")")
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

# A Python with tomllib (3.11 or later), for scripts/check-binstall.py.
python_tomllib() {
  local p
  for p in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
    command -v "$p" > /dev/null && "$p" -c 'import tomllib' 2> /dev/null && { echo "$p"; return; }
  done
  return 1
}
binstall() {
  local py crate=graphviz-pure-cli-$version.crate
  py=$(python_tomllib) || { result binstall 0 "no Python 3.11 or later to run scripts/check-binstall.py"; return; }
  mkdir -p "$work/binstall/dist"
  if ! retry curl -fsS -A "$ua" -o "$work/binstall/$crate" "https://static.crates.io/crates/graphviz-pure-cli/$crate" 2> /dev/null; then
    result binstall 0 "graphviz-pure-cli $version is not on crates.io"
  elif ! tar -xzf "$work/binstall/$crate" -C "$work/binstall" "graphviz-pure-cli-$version/Cargo.toml"; then
    result binstall 0 "$crate has no graphviz-pure-cli-$version/Cargo.toml"
  elif ! gh release download "$tag" --repo "$REPO" --pattern "graphviz-pure-$version-*" --dir "$work/binstall/dist" > /dev/null 2>&1; then
    result binstall 0 "could not download the release's archives"
  elif ! (cd "$work/binstall" && "$py" "$OLDPWD/scripts/check-binstall.py" "graphviz-pure-cli-$version/Cargo.toml" dist "$version") > "$work/binstall/log" 2>&1; then
    # Relative paths only: this text is posted to the release issue.
    result binstall 0 "$(grep -v ' → ' "$work/binstall/log" | tr '\n' ' ')"
  elif ! command -v cargo-binstall > /dev/null; then
    # release.yml's verify job installs it, so there its absence is a failure.
    result binstall "$((1 - ci))" "$(tail -n 1 "$work/binstall/log") (templates only: no cargo-binstall on PATH)"
  else
    binstall_dry_runs
  fi
}
# cargo-binstall --dry-run for each release target: resolves graphviz-pure-cli VERSION on
# crates.io, downloads the target's archive from the GitHub release, unpacks it and finds the
# binary, then stops before installing. Compiling and third-party builds are disabled, so only
# the release's archives can satisfy it.
binstall_dry_run() { # binstall_dry_run TARGET: one attempt
  CARGO_HOME="$work/binstall/home" cargo-binstall --dry-run --no-confirm --no-track \
    --disable-strategies compile,quick-install --targets "$1" \
    --install-path "$work/binstall/bin" "graphviz-pure-cli@$version" > "$work/binstall/dry-$1.log" 2>&1 < /dev/null
}
binstall_dry_runs() {
  local archive target compat_archive bad=() n=0
  # The archives check-binstall.py checks: .tar.gz and .zip, not the WASI one.
  for archive in "$work"/binstall/dist/graphviz-pure-"$version"-*.tar.gz "$work"/binstall/dist/graphviz-pure-"$version"-*.zip; do
    [ -f "$archive" ] || continue
    case "$archive" in *-wasm32-*) continue ;; esac
    target=${archive##*/graphviz-pure-"$version"-}
    target=${target%.tar.gz}
    target=${target%.zip}
    compat_archive=${archive%.tar.gz}
    compat_archive=${compat_archive%.zip}
    if [[ "$target" == x86_64* && "$target" != *-compat ]] &&
      { [ -f "$compat_archive-compat.tar.gz" ] || [ -f "$compat_archive-compat.zip" ]; }; then
      continue
    fi
    target=${target%-compat}
    n=$((n + 1))
    if ! retry binstall_dry_run "$target"; then
      bad+=("$target")
      # The log goes to stderr (the job's log), not to the summary, with the temporary paths masked.
      { echo "cargo-binstall --dry-run --targets $target:"; sed "s|$work|…|g" "$work/binstall/dry-$target.log"; } >&2
    fi
  done
  if [ ${#bad[@]} = 0 ]; then
    result binstall 1 "$(tail -n 1 "$work/binstall/log"); cargo-binstall $(cargo-binstall -V 2> /dev/null) dry-runs $n targets"
  else
    result binstall 0 "cargo-binstall --dry-run fails for: ${bad[*]}"
  fi
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
  elif ! attest "oci://$image:$version"; then
    result ghcr 0 "$image:$version's attestation does not verify"
  else
    local held newer
    held=$(awk '{printf "%s ", $1}' "$work/ghcr")
    case "$held" in *latest*) newer="" ;; *) newer=" (newer releases hold the others)" ;; esac
    result ghcr 1 "${held% } → $(awk 'NR == 1 {print substr($2, 1, 19)}' "$work/ghcr")…, attested$newer"
  fi
}

# The registry's package (its webc v3 file, as Wasmer 7 publishes it), downloaded and checked
# against the registry's hash, must contain the attested archive's module: a webc stores its
# modules as they are.
wasmer_query() { # wasmer_query QUERY [VERSION]: the registry's GraphQL answer, or failure
  jq -nc --arg q "$1" --arg v "${2:-}" '{query: $q, variables: {v: $v}}' |
    curl -fsS -A "$ua" -H 'content-type: application/json' --data @- https://registry.wasmer.io/graphql 2> /dev/null |
    jq -ec 'select(.data != null and (.errors | length // 0) == 0)'
}
wasmer_ok() {
  local answer url hash
  # shellcheck disable=SC2016 # $v is a GraphQL variable
  answer=$(wasmer_query 'query($v: String) { getPackageVersion(name: "graphviz-pure/graphviz-pure", version: $v) { distribution(version: V3) { webcDownloadUrl webcSha256Hash } } }' "$version") || return 1
  url=$(jq -r '.data.getPackageVersion.distribution.webcDownloadUrl // empty' <<< "$answer")
  hash=$(jq -r '.data.getPackageVersion.distribution.webcSha256Hash // empty' <<< "$answer")
  [ -n "$url" ] && [ -n "$hash" ] && curl -fsS -A "$ua" -o "$work/wasmer.webc" "$url" 2> /dev/null &&
    [ "$(if command -v sha256sum > /dev/null; then sha256sum "$work/wasmer.webc"; else shasum -a 256 "$work/wasmer.webc"; fi | cut -d' ' -f1)" = "$hash" ]
}
wasmer_channel() {
  local asset=graphviz-pure-$version-wasm32-wasip1.tar.gz
  # release.yml's wasmer job publishes no pre-release, and nothing until the channel is set up.
  case "$version" in *-*) result wasmer 1 "skipped: $version is a pre-release, not published to Wasmer"; return ;; esac
  # Not set up: only outside the verify job, which checks only a channel that published.
  if [ "$ci" = 0 ] && wasmer_query '{ getPackage(name: "graphviz-pure/graphviz-pure") { name } }' | jq -e '.data.getPackage == null' > /dev/null; then
    result wasmer 1 "skipped: not set up yet (no graphviz-pure/graphviz-pure in the registry; docs/releasing.md)"
    return
  fi
  if ! retry wasmer_ok; then
    result wasmer 0 "graphviz-pure/graphviz-pure $version is not in the Wasmer registry (or its webc does not match the registry's hash)"
    return
  fi
  mkdir -p "$work/wasi"
  if ! gh release download "$tag" --repo "$REPO" --pattern "$asset" --dir "$work/wasi" > /dev/null 2>&1; then
    result wasmer 0 "$version is in the registry, but the release has no $asset to compare it with"
  elif ! attest "$work/wasi/$asset"; then
    result wasmer 0 "$asset's attestation does not verify"
  elif ! python3 -I -c '
import sys, tarfile
name = sys.argv[1].rsplit("/", 1)[-1][: -len(".tar.gz")]
module = tarfile.open(sys.argv[1]).extractfile(name + "/graphviz-pure.wasm").read()
sys.exit(module not in open(sys.argv[2], "rb").read())' "$work/wasi/$asset" "$work/wasmer.webc"; then
    result wasmer 0 "graphviz-pure/graphviz-pure $version does not hold the module of $asset"
  else
    result wasmer 1 "graphviz-pure/graphviz-pure $version holds the attested module of $asset"
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
    attest "$wheel" || bad+=("$(basename "$wheel")")
  done
  if [ ${#bad[@]} = 0 ]; then result pypi 1 "$line, each attested"
  else result pypi 0 "not attested: ${bad[*]}"; fi
}

npm_channel() {
  local latest tarball dir integrity
  latest=$(npm view @graphviz-pure/wasm dist-tags.latest 2> /dev/null || true)
  if [ "$latest" != "$version" ]; then
    result npm 0 "latest is ${latest:-unknown}, not $version"
    return
  fi
  # The install is pinned to the attested GitHub-release tarball's sha512, not to anything the
  # registry reports: npm ci must fetch registry bytes that hash to the released bytes.
  tarball=graphviz-pure-wasm-$version.tgz
  dir=$work/npm-release
  if ! gh release download "$tag" --repo "$REPO" --dir "$dir" -p "$tarball" -p SHA256SUMS > /dev/null 2>&1 ||
    ! (cd "$dir" && if command -v sha256sum > /dev/null; then sha256sum --check --quiet --ignore-missing SHA256SUMS; else shasum -a 256 --check --quiet --ignore-missing SHA256SUMS; fi) > /dev/null 2>&1 ||
    ! attest "$dir/$tarball"; then
    result npm 0 "the release's $tarball is missing, fails SHA256SUMS, or its attestation does not verify"
    return
  fi
  integrity=sha512-$(openssl dgst -sha512 -binary "$dir/$tarball" | openssl base64 -A)
  if (cd "$work" && mkdir npm && cd npm &&
    printf '{"name":"release-verify","version":"0.0.0","dependencies":{"@graphviz-pure/wasm":"%s"}}' "$version" > package.json &&
    printf '{"name":"release-verify","version":"0.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"release-verify","version":"0.0.0","dependencies":{"@graphviz-pure/wasm":"%s"}},"node_modules/@graphviz-pure/wasm":{"version":"%s","resolved":"https://registry.npmjs.org/@graphviz-pure/wasm/-/wasm-%s.tgz","integrity":"%s"}}}' \
      "$version" "$version" "$version" "$integrity" > package-lock.json &&
    npm ci --ignore-scripts --no-audit --no-fund > "$work/npm-ci" 2>&1 &&
    npm audit signatures > "$work/npm-audit" 2>&1); then
    result npm 1 "latest is $version; registry bytes pinned to the attested release tarball (${integrity:0:19}…); $(grep -m1 -i 'verified registry signature' "$work/npm-audit" || echo 'npm audit signatures passed')"
  else
    result npm 0 "npm ci against the attested integrity, or the signature check, failed: $(tail -n 3 "$work/npm-ci" 2> /dev/null | tr '\n' ' ')$(tail -n 3 "$work/npm-audit" 2> /dev/null | tr '\n' ' ')"
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
want binstall && binstall
want jsr && jsr
want ghcr && ghcr
want wasmer && wasmer_channel
want pypi && pypi
want npm && npm_channel
if [ "$ci" = 0 ]; then
  want taps && taps
fi
echo
if [ "$fails" = 0 ]; then echo "**All channels pass.**"; else echo "**$fails channel(s) fail.**"; fi
[ "$fails" = 0 ]
