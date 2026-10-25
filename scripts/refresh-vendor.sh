#!/usr/bin/env bash
# Re-download the pinned raster crates and re-apply scripts/vendor-patches.
#
#   scripts/refresh-vendor.sh                 # replace vendor/<crate>
#   scripts/refresh-vendor.sh --check         # fail if that result differs
#   scripts/refresh-vendor.sh --save-patches  # write the patches from the current vendor/
#
# The pins are the versions below. A file trim (examples, benches, tests, docs,
# lockfiles) is applied before the patch, so the patch is only the portable-math
# edit. Licences stay. vendor/README.md is not touched.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=${1:-apply}
crates=(
  "tiny-skia 0.12.0"
  "tiny-skia-path 0.12.0"
  "kurbo 0.13.1"
  "svgtypes 0.16.1"
  "skrifa 0.44.0"
)

trim_crate() {
  local dir=$1
  rm -rf "$dir/examples" "$dir/benches" "$dir/tests" "$dir/.github" "$dir/.cargo" "$dir/scripts"
  rm -f "$dir/Cargo.lock" "$dir/Cargo.toml.orig" "$dir/.cargo_vcs_info.json" "$dir/.cargo-ok" \
    "$dir/.gitignore" "$dir/.typos.toml" "$dir/.taplo.toml" "$dir/README.md" \
    "$dir/CHANGELOG.md" "$dir/AUTHORS"
  python3 - "$dir/Cargo.toml" << 'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()

def skip_header(line):
    return (
        line.startswith("[[example]]")
        or line.startswith("[[bench]]")
        or line.startswith("[[test]]")
        or line.startswith("[dev-dependencies")
    )

out = []
skipping = False
for line in text.splitlines(keepends=True):
    if skip_header(line):
        skipping = True
        continue
    if skipping:
        if line.startswith("[") and not skip_header(line):
            skipping = False
        else:
            continue
    out.append(line)
open(path, "w", encoding="utf-8").write("".join(out).rstrip() + "\n")
PY
}

download_trimmed() {
  local name=$1 ver=$2 dest=$3
  local crate="${name}-${ver}.crate"
  curl -fsSL "https://static.crates.io/crates/${name}/${crate}" -o "$dest/${crate}"
  tar -xzf "$dest/${crate}" -C "$dest"
  rm -f "$dest/${crate}"
  # The tarball's top directory is name-ver.
  if [[ -d "$dest/${name}-${ver}" && "$dest/${name}-${ver}" != "$dest/$name" ]]; then
    mv "$dest/${name}-${ver}" "$dest/$name"
  fi
  trim_crate "$dest/$name"
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case "$mode" in
  --trim-worktree)
    for spec in "${crates[@]}"; do
      name=${spec%% *}
      trim_crate "vendor/$name"
      echo "trimmed vendor/$name"
    done
    ;;
  --save-patches)
    mkdir -p scripts/vendor-patches
    for spec in "${crates[@]}"; do
      name=${spec%% *}
      ver=${spec##* }
      base="$work/base-$name"
      mkdir -p "$base"
      download_trimmed "$name" "$ver" "$base"
      ours="$work/ours-$name"
      mkdir -p "$ours"
      # Copy the trimmed worktree crate to a directory named like the extract,
      # so the diff prefixes are both one component.
      mkdir -p "$work/diff-$name"
      cp -R "$base/$name" "$work/diff-$name/a"
      cp -R "vendor/$name" "$work/diff-$name/b"
      # Relative a/ and b/ prefixes so `patch -p1` applies inside the crate.
      # diff exits 1 when files differ.
      (
        cd "$work/diff-$name"
        diff -ruN a b || true
      ) | sed -E $'s/\t.*//' > "scripts/vendor-patches/${name}.patch"
      if [[ ! -s "scripts/vendor-patches/${name}.patch" ]]; then
        echo "no differences for $name; the patch would be empty" >&2
        exit 1
      fi
      echo "wrote scripts/vendor-patches/${name}.patch"
    done
    ;;
  apply|--check)
    for spec in "${crates[@]}"; do
      name=${spec%% *}
      ver=${spec##* }
      stage="$work/$name"
      mkdir -p "$stage"
      download_trimmed "$name" "$ver" "$stage"
      # download_trimmed left the crate at $stage/$name.
      (
        cd "$stage/$name"
        patch -p1 --forward --batch < "$OLDPWD/scripts/vendor-patches/${name}.patch"
      )
      if [[ "$mode" == "--check" ]]; then
        if ! diff -ru "$stage/$name" "vendor/$name" > "$work/${name}.diff"; then
          echo "$name differs from a fresh vendor plus the patch:" >&2
          cat "$work/${name}.diff" >&2
          exit 1
        fi
        echo "$name matches"
      else
        rm -rf "vendor/$name"
        mv "$stage/$name" "vendor/$name"
        echo "refreshed vendor/$name"
      fi
    done
    ;;
  *)
    echo "usage: scripts/refresh-vendor.sh [--check|--save-patches|--trim-worktree]" >&2
    exit 2
    ;;
esac
