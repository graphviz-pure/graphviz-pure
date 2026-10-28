#!/usr/bin/env bash
# Is origin/master ready to release? Prints a Markdown checklist and the changelog
# section, and exits 0 only when every item passes. Reads only. See docs/releasing.md.
#
#   - no open priority:p1 or priority:p2 issue, and no open "Cross-platform fails on master";
#   - ci.yml and cross-platform.yml succeeded on master's head;
#   - every pull request since the last release has a changelog entry, none is marked
#     breaking without the breaking label, and the labels ask for the version;
#   - stable Rust API breaks against crates.io have a merged breaking PR and a matching bump;
#   - CHANGELOG.md's Unreleased section is empty (entries belong in pull request bodies);
#   - the version is not tagged yet, and no GitHub release (including a draft) exists for it;
#   - the e2e fidelity count (tests/golden/e2e-baseline.libm-matched.portable.json) is at
#     least the last release's, and every input that was identical to C still is.
#
# A version given as an argument or as VERSION_INPUT must be at least what the labels ask for.
# With neither, the version is that one. INTRO_INPUT, when set, is the changelog intro;
# otherwise the intro is a line counted from the entries.
#
#   --summary FILE   also append the report to FILE
#   --meta FILE      on success, write {"version","base_sha","prev"} for release-commit.sh
#   --section FILE   on success, write the rendered changelog section
#
# Usage: scripts/release-check.sh [VERSION] [--summary FILE] [--meta FILE] [--section FILE]
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR
# shellcheck source=release-lib.sh
. scripts/release-lib.sh

version=${VERSION_INPUT:-} summary="" meta="" section=""
while [ $# -gt 0 ]; do
  case "$1" in
    --summary) summary=${2:?--summary needs a file}; shift ;;
    --meta) meta=${2:?--meta needs a file}; shift ;;
    --section) section=${2:?--section needs a file}; shift ;;
    -*) die "unknown option $1" ;;
    *) version=$1 ;;
  esac
  shift
done
[ -z "$version" ] || check_version "$version"
intro=${INTRO_INPUT:-}

fetch_master
target=$(git rev-parse origin/master)
prev=$(prev_tag "$target" "${version:-none}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fails=0
item() { # item PASS TEXT [DETAIL]
  if [ "$1" = 1 ]; then echo "- [x] $2"; else echo "- [ ] $2${3:+ — $3}"; fails=$((fails + 1)); fi
}

{
  echo "## Release readiness of ${target:0:12} (master)"
  echo

  for p in p1 p2; do
    open=$(gh issue list --repo "$REPO" --state open --label "priority:$p" --limit 200 --json number -q 'map("#\(.number)") | join(", ")')
    item "$([ -z "$open" ] && echo 1)" "No open \`priority:$p\` issue" "$open"
  done
  xp=$(gh issue list --repo "$REPO" --state open --search 'in:title "Cross-platform fails on master"' --json number,title \
    -q 'map(select(.title == "Cross-platform fails on master") | "#\(.number)") | join(", ")')
  item "$([ -z "$xp" ] && echo 1)" "No open \"Cross-platform fails on master\" issue" "$xp"

  # ci.yml and cross-platform.yml run on every master push. The newest run on this commit
  # must have succeeded. Fuzzing runs in this same release workflow, so it is not a precondition.
  for wf in ci.yml cross-platform.yml; do
    run=$(latest_run "$wf" "$target")
    if [ -z "$run" ]; then
      item 0 "\`$wf\` succeeded" "no run on ${target:0:12}"
    else
      read -r status conclusion url <<< "$run"
      if [ "$status" = completed ] && [ "$conclusion" = success ]; then item 1 "\`$wf\` succeeded: $url"
      else item 0 "\`$wf\` succeeded" "$status/$conclusion: $url"; fi
    fi
  done

  python3 scripts/release-changelog.py collect "$prev" "$target" > "$work/entries.json"
  missing=$(jq -r '.missing | map("#\(.pr)") | join(", ")' "$work/entries.json")
  count=$(jq '[.entries[].pr] + [.none[].pr] | unique | length' "$work/entries.json")
  item "$([ -z "$missing" ] && echo 1)" "Every pull request since $prev has a changelog entry ($count with one)" "missing: $missing"
  unlabelled=$(jq -r '.unlabelled_breaking | map("#\(.pr)") | join(", ")' "$work/entries.json")
  computed=$(python3 scripts/release-changelog.py next-version "$work/entries.json" "${prev#v}")
  [ -n "$version" ] || version=$computed
  check_version "$version"
  breaking=$(jq -r '.breaking | map("#\(.)") | join(", ")' "$work/entries.json")
  if [ -n "$unlabelled" ]; then
    item 0 "The \`breaking\` labels ask for $version" "breaking entries without the label: $unlabelled"
  else
    item "$(python3 scripts/release-changelog.py version-at-least "$version" "$computed" && echo 1)" \
      "The selected $version meets the label-selected minimum $computed (${breaking:-no breaking pull request})" \
      "the minimum is $computed"
  fi

  # Check precisely the master revision being released, without changing this checkout.
  mkdir "$work/api"
  git archive "$target" | tar -x -C "$work/api"
  if CARGO_TERM_COLOR=never SEMVER_MANIFEST="$work/api/Cargo.toml" RELEASE_VERSION="$version" \
    RELEASE_TARGET="$target" \
    scripts/check-semver.sh release > "$work/semver.txt" 2>&1; then
    item 1 "Stable Rust API changes against crates.io agree with the merged labels and $version"
    sed -n '/ API requires a /p' "$work/semver.txt"
  else
    item 0 "Stable Rust API changes against crates.io agree with the merged labels and $version"
    echo '```text'
    tail -n 30 "$work/semver.txt"
    echo '```'
  fi

  unreleased=$(git show "$target:CHANGELOG.md" | python3 -c '
import re, sys
log = sys.stdin.read()
m = re.search(r"^## \[Unreleased\]\n(.*?)(?=^## \[)", log, re.M | re.S)
print("" if m and not m.group(1).strip() else "not empty" if m else "missing")')
  item "$([ -z "$unreleased" ] && echo 1)" "CHANGELOG.md's Unreleased section is empty" "$unreleased"

  tagged=$(git rev-parse -q --verify "refs/tags/v$version" || true)
  item "$([ -z "$tagged" ] && echo 1)" "v$version is not tagged yet" "it is, at ${tagged:0:12}"
  # A 404 from the tags endpoint exits non-zero and still writes an error body. A draft is
  # visible when this token can see drafts.
  if gh api "repos/$REPO/releases/tags/v$version" > "$work/rel.json" 2> /dev/null; then
    draft=$(jq -r '.draft' "$work/rel.json")
    item 0 "No GitHub release for v$version" "draft=$draft"
  else
    listed=$(gh release list --repo "$REPO" --limit 100 --json tagName,isDraft |
      jq -r --arg t "v$version" '.[] | select(.tagName == $t) | "draft=\(.isDraft)"')
    item "$([ -z "$listed" ] && echo 1)" "No GitHub release for v$version" "$listed"
  fi

  # Fidelity: every e2e input byte-identical to C at the last release still is, and the count
  # is not lower.
  e2e=tests/golden/e2e-baseline.libm-matched.portable.json
  git show "$target:$e2e" | jq -r '.identical[]' | sort > "$work/now"
  git show "$prev:$e2e" 2> /dev/null | jq -r '.identical[]' | sort > "$work/then" || : > "$work/then"
  lost=$(comm -13 "$work/now" "$work/then" | head -n 5 | paste -sd ' ' -)
  now_n=$(wc -l < "$work/now" | tr -d ' ')
  then_n=$(wc -l < "$work/then" | tr -d ' ')
  if [ -z "$lost" ] && [ "$now_n" -ge "$then_n" ]; then
    item 1 "e2e fidelity: $now_n inputs identical to C, every one of the $then_n at $prev included"
  else
    item 0 "e2e fidelity: count not lower than $prev ($then_n) and none lost" "now $now_n; no longer identical: ${lost:-none}"
  fi

  render=(python3 scripts/release-changelog.py render "$work/entries.json" "$version")
  [ -n "$intro" ] && render+=(--intro "$intro")
  "${render[@]}" > "$work/section.md"

  echo
  if [ "$fails" = 0 ]; then echo "**Ready:** $version from ${target:0:12}."
  else echo "**Not ready:** $fails item(s) open."; fi
  echo
  echo "### Changelog section"
  echo
  cat "$work/section.md"

  # Shown, not failed: a commit with no pull request, prose beside a bullet, or a breaking
  # label whose entry has no Breaking bullet.
  jq -r '
    (if .breaking_without_entry != [] then "Labelled breaking with no Breaking bullet (check the entry says what breaks):",
      (.breaking_without_entry[] | "- #\(.pr) \(.title)"), "" else empty end),
    (if .notes != [] then "Prose beside the bullets, not copied (check it):",
      (.notes[] | "- #\(.pr): \(.notes | join(" / "))"), "" else empty end),
    (if .without_pr != [] then "Commits without a pull request:",
      (.without_pr[] | "- \(.sha[:12]) \(.subject) (\(.reason))"), "" else empty end)
  ' "$work/entries.json"
} > "$work/report.md"
cat "$work/report.md"
[ -z "$summary" ] || cat "$work/report.md" >> "$summary"

if [ "$fails" = 0 ]; then
  [ -z "$section" ] || cp "$work/section.md" "$section"
  if [ -n "$meta" ]; then
    jq -n --arg version "$version" --arg base "$target" --arg prev "$prev" \
      '{version: $version, base_sha: $base, prev: $prev}' > "$meta"
  fi
fi
[ "$fails" = 0 ]
