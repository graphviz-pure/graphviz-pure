#!/usr/bin/env bash
# Select live channels for release-verify.sh without hiding failed publishing jobs or
# registry lookup errors. npm is pending only after successful staging and an HTTP 404.
set -euo pipefail
cd "$(dirname "$0")/.."
ok="" failed=""
staged=()
# binstall requires both the crate metadata and GitHub release archives.
if [ "$CRATES" = success ] && [ "$GITHUB" = success ]; then RESULTS="$RESULTS binstall=success"; else RESULTS="$RESULTS binstall=skipped"; fi
for kv in $RESULTS; do
  channel=${kv%%=*}
  case "${kv#*=}" in
    success)
      if [ "$channel" = npm ]; then
        status=""
        if status=$(curl -sS --retry 3 --retry-connrefused --max-time 15 --retry-max-time 60 \
          -o /dev/null -w '%{http_code}' "https://registry.npmjs.org/@graphviz-pure%2fwasm/$VERSION"); then
          case "$status" in
            200) ;;
            404)
              echo "::notice::npm $VERSION is staged: approve it with your passkey on npmjs.com, then re-run verify or run scripts/release-verify.sh $VERSION."
              echo "npm_pending=true" >> "$GITHUB_OUTPUT"
              staged=(--npm-staged)
              continue ;;
            *)
              echo "::error::npm version lookup returned HTTP $status; other channels will still be checked."
              echo "- [ ] npm: FAIL — version lookup returned HTTP $status." >> "$GITHUB_STEP_SUMMARY"
              failed="$failed,npm"
              continue ;;
          esac
        else
          echo "::error::npm version lookup failed (curl transport error, HTTP ${status:-unavailable}); other channels will still be checked."
          echo "- [ ] npm: FAIL — version lookup failed (transport error, HTTP ${status:-unavailable})." >> "$GITHUB_STEP_SUMMARY"
          failed="$failed,npm"
          continue
        fi
      fi
      ok="$ok,$channel" ;;
    skipped) ;;
    *) failed="$failed,$channel" ;;
  esac
done
# Explicit --channels "" means none. --failed still reports every failed channel.
scripts/release-verify.sh "$VERSION" --ci --wait 600 --channels "${ok#,}" --failed "${failed#,}" "${staged[@]}" |
  tee -a "$GITHUB_STEP_SUMMARY"
