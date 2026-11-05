#!/usr/bin/env bash
# sudo in the self-hosted runner image. The job user has no root. The jobs written for GitHub's
# image run `sudo apt-get update` and `sudo apt-get install -y <packages>`; the packages they
# install are in the image already (Dockerfile), so those succeed without doing anything. Any
# other command, or a package the image lacks, fails and names what to add to the Dockerfile.
set -euo pipefail
[ "${1:-}" = -n ] && shift
[ "${1:-}" = -E ] && shift
if [ "${1:-}" = apt-get ] || [ "${1:-}" = apt ]; then
  shift
  sub=""
  pkgs=()
  for a in "$@"; do
    case "$a" in
      -*) ;;
      update | install) [ -z "$sub" ] && sub=$a || pkgs+=("$a") ;;
      *) if [ -n "$sub" ]; then pkgs+=("$a"); else sub=$a; fi ;;
    esac
  done
  case "$sub" in
    update) echo "gvp-ci sudo shim: apt-get update skipped (packages are in the image)"; exit 0 ;;
    install)
      missing=()
      for p in "${pkgs[@]}"; do
        dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed' || missing+=("$p")
      done
      if [ "${#missing[@]}" -eq 0 ]; then
        echo "gvp-ci sudo shim: already installed: ${pkgs[*]}"
        exit 0
      fi
      echo "::error::gvp-ci runner image lacks ${missing[*]}; add it to scripts/ci/runner/Dockerfile" >&2
      exit 1
      ;;
  esac
fi
echo "::error::sudo is not available on the gvp-ci self-hosted runner (asked: $*)" >&2
exit 1
