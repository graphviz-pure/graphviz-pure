#!/usr/bin/env bash
# Fills in pkgver and the checksums of both AUR packages for a published release, after
# verifying what it downloads, and regenerates their .SRCINFO files.
#
# Usage: [AUR_MAINTAINER='Name <email>'] packaging/aur/update.sh VERSION    (e.g. 0.1.0)
#
# - VERSION must be a plain release, X.Y.Z. Prereleases (0.2.0-rc.1) are refused: pacman's
#   pkgver cannot contain "-", and the AUR packages are meant to follow stable releases only.
# - The two Linux musl archives (graphviz-pure-bin) must have a build-provenance attestation
#   from this repository's release workflow, built on a GitHub-hosted runner, and match the
#   release's SHA256SUMS. Releases through 0.2.3 were attested for the tag; a later release is
#   attested for refs/heads/master (the workflow_dispatch ref), so either source ref is accepted.
# - The GitHub source tarball (graphviz-pure) is not a release asset and has no attestation, so
#   the tag must be an annotated tag and the tarball's contents must equal the tagged tree.
# - AUR_MAINTAINER, if set, replaces the "# Maintainer:" line. The script refuses to finish
#   while any TODO or placeholder checksum remains, including the maintainer's email.
#
# Nothing is written until every check has passed: the new PKGBUILDs and .SRCINFO files are
# built in a temporary directory and moved into place at the end.
#
# Needs gh (logged in), git, curl, tar, and sha256sum or shasum. .SRCINFO is written with
# `makepkg --printsrcinfo`, or in an archlinux container when makepkg is missing; with neither
# makepkg nor a running docker, the script fails.
set -euo pipefail

version=${1:?usage: $0 VERSION}
version=${version#v}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: VERSION must be X.Y.Z (got '$version'); prereleases are not packaged for the AUR" >&2
  exit 1
fi
tag=v$version
repo=graphviz-pure/graphviz-pure
here=$(cd "$(dirname "$0")" && pwd)
packages=(graphviz-pure-bin graphviz-pure)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
# sed -i differs between GNU and BSD; write through a temporary file instead.
subst() {
  local file=$1; shift
  sed "$@" "$file" > "$file.tmp" && mv "$file.tmp" "$file"
}

# Decide how to write .SRCINFO before downloading anything.
if command -v makepkg >/dev/null; then
  srcinfo=makepkg
elif command -v docker >/dev/null && docker info >/dev/null 2>&1; then
  srcinfo=docker
else
  echo "error: writing .SRCINFO needs makepkg, or docker with a running daemon" >&2
  exit 1
fi

echo "== release assets of $tag" >&2
mkdir "$work/dist"
targets=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
patterns=(--pattern SHA256SUMS)
for t in "${targets[@]}"; do patterns+=(--pattern "graphviz-pure-$version-$t.tar.gz"); done
gh release download "$tag" --repo "$repo" --dir "$work/dist" "${patterns[@]}"
# Releases through 0.2.3 were attested for the tag; later releases for refs/heads/master.
attest() {
  gh attestation verify "$1" --repo "$repo" \
    --signer-workflow "$repo/.github/workflows/release.yml" --source-ref "refs/tags/$tag" \
    --deny-self-hosted-runners >&2 && return 0
  gh attestation verify "$1" --repo "$repo" \
    --signer-workflow "$repo/.github/workflows/release.yml" --source-ref "refs/heads/master" \
    --deny-self-hosted-runners >&2
}
for t in "${targets[@]}"; do
  f=graphviz-pure-$version-$t.tar.gz
  attest "$work/dist/$f"
  sum=$(sha256 "$work/dist/$f")
  listed=$(awk -v f="$f" '$2 == f || $2 == "*" f {print $1}' "$work/dist/SHA256SUMS")
  [ "$sum" = "$listed" ] || { echo "error: $f does not match SHA256SUMS" >&2; exit 1; }
  case $t in x86_64*) sum_x86_64=$sum ;; aarch64*) sum_aarch64=$sum ;; esac
done

echo "== source tarball of $tag" >&2
curl -fsSL -o "$work/src.tar.gz" "https://github.com/$repo/archive/refs/tags/$tag.tar.gz"
git clone --quiet --depth 1 --branch "$tag" "https://github.com/$repo.git" "$work/repo"
[ "$(git -C "$work/repo" cat-file -t "refs/tags/$tag")" = tag ] \
  || { echo "error: $tag is not an annotated tag" >&2; exit 1; }
mkdir "$work/a" "$work/b"
tar -xzf "$work/src.tar.gz" -C "$work/a"
git -C "$work/repo" archive --prefix="graphviz-pure-$version/" "$tag" | tar -x -C "$work/b"
diff -r "$work/a" "$work/b" >&2 || { echo "error: the source tarball differs from $tag" >&2; exit 1; }
src_sum=$(sha256 "$work/src.tar.gz")

echo "== PKGBUILDs (in $work/out)" >&2
mkdir "$work/out"
for p in "${packages[@]}"; do
  mkdir "$work/out/$p"
  cp "$here/$p/PKGBUILD" "$work/out/$p/PKGBUILD"
  # The placeholder header block runs from its TODO line to "removes these TODO lines."
  subst "$work/out/$p/PKGBUILD" \
    -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=1/" \
    -e '/^# TODO(release): the checksum/,/removes these TODO lines\.$/d' \
    -e '/^# TODO(release): placeholder/d'
  if [ -n "${AUR_MAINTAINER:-}" ]; then
    m=$(printf '%s' "$AUR_MAINTAINER" | sed 's/[&|\\]/\\&/g')
    subst "$work/out/$p/PKGBUILD" -e "s|^# Maintainer: .*|# Maintainer: $m|"
  fi
done
subst "$work/out/graphviz-pure-bin/PKGBUILD" \
  -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$sum_x86_64')/" \
  -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$sum_aarch64')/"
subst "$work/out/graphviz-pure/PKGBUILD" -e "s/^sha256sums=.*/sha256sums=('$src_sum')/"

echo "== .SRCINFO ($srcinfo)" >&2
for p in "${packages[@]}"; do
  if [ "$srcinfo" = makepkg ]; then
    (cd "$work/out/$p" && makepkg --printsrcinfo > .SRCINFO)
  else
    # makepkg refuses to run as root, even for --printsrcinfo.
    docker run --rm -i --platform linux/amd64 archlinux:latest bash -c \
      'useradd -m b && mkdir /p && cat > /p/PKGBUILD && chown -R b /p && cd /p && su b -c "makepkg --printsrcinfo"' \
      < "$work/out/$p/PKGBUILD" > "$work/out/$p/.SRCINFO"
  fi
  [ -s "$work/out/$p/.SRCINFO" ] || { echo "error: empty .SRCINFO for $p" >&2; exit 1; }
done

# Refuse to finish while anything is still a placeholder.
if grep -n -E 'TODO|0{64}' "$work"/out/*/PKGBUILD "$work"/out/*/.SRCINFO >&2; then
  echo "error: placeholders remain (set AUR_MAINTAINER='Name <email>' for the Maintainer line)" >&2
  exit 1
fi

for p in "${packages[@]}"; do
  mv "$work/out/$p/PKGBUILD" "$here/$p/PKGBUILD"
  mv "$work/out/$p/.SRCINFO" "$here/$p/.SRCINFO"
done

cat >&2 <<EOF

Updated packaging/aur for $version (attestations, SHA256SUMS and the annotated tag verified).
source tarball sha256: $src_sum
  (the same file conda-forge's recipe.yaml uses: put this value in its source.sha256)
EOF
