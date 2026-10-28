#!/usr/bin/env bash
# Closes the open GitHub milestone whose title is VERSION, if there is one.
# Writes no comment and opens nothing. See docs/releasing.md.
#
# Usage: scripts/release-close-milestone.sh VERSION
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source-path=SCRIPTDIR
# shellcheck source=release-lib.sh
. scripts/release-lib.sh

version=${1:?usage: scripts/release-close-milestone.sh VERSION}
check_version "$version"

number=""
while IFS=$'\t' read -r n title; do
  [ "$title" = "$version" ] || continue
  number=$n
  break
done < <(gh api --paginate "repos/$REPO/milestones?state=open&per_page=100" --jq '.[] | [.number, .title] | @tsv')

if [ -z "$number" ]; then
  echo "No open milestone titled $version."
else
  gh api --method PATCH "repos/$REPO/milestones/$number" -f state=closed > /dev/null
  echo "Closed milestone $version (#$number)."
fi
