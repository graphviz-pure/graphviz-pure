#!/usr/bin/env bash
# One-time: publishes 0.0.0 placeholders of the graphviz-pure crate names, so that trusted
# publishing can be configured for them (crates.io only allows it for existing crates). Run
# after `cargo login` with a short-lived "publish-new" token; see docs/releasing.md. Existing
# names are skipped, so it is safe to run again for newly added raster forks.
# crates.io rate-limits new crate names; the script waits and retries when told to.
set -euo pipefail
crates=(graphviz-pure-core graphviz-pure-parser graphviz-pure-geom graphviz-pure-layout
        graphviz-pure-render graphviz-pure-raster graphviz-pure graphviz-pure-cli
        graphviz-pure-tiny-skia-path graphviz-pure-kurbo graphviz-pure-skrifa
        graphviz-pure-svgtypes graphviz-pure-tiny-skia graphviz-pure-usvg graphviz-pure-resvg
        graphviz-pure-svg2pdf)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for c in "${crates[@]}"; do
  if curl -fsS -A "graphviz-pure reserve script" "https://crates.io/api/v1/crates/$c" >/dev/null 2>&1; then
    echo "$c already exists"; continue
  fi
  license="EPL-2.0"
  description="Placeholder for graphviz-pure, a Rust port of Graphviz."
  case "$c" in
    graphviz-pure-tiny-skia)
      license="BSD-3-Clause"; description="Placeholder for the graphviz-pure fork of upstream tiny-skia 0.12.0." ;;
    graphviz-pure-tiny-skia-path)
      license="BSD-3-Clause"; description="Placeholder for the graphviz-pure fork of upstream tiny-skia-path 0.12.0." ;;
    graphviz-pure-kurbo)
      license="Apache-2.0 OR MIT"; description="Placeholder for the graphviz-pure fork of upstream kurbo 0.13.1." ;;
    graphviz-pure-svgtypes)
      license="Apache-2.0 OR MIT"; description="Placeholder for the graphviz-pure fork of upstream svgtypes 0.16.1." ;;
    graphviz-pure-skrifa)
      license="MIT OR Apache-2.0"; description="Placeholder for the graphviz-pure fork of upstream skrifa 0.44.0." ;;
    graphviz-pure-resvg)
      license="Apache-2.0 OR MIT"; description="Placeholder for the graphviz-pure fork of upstream resvg 0.48.1." ;;
    graphviz-pure-usvg)
      license="Apache-2.0 OR MIT"; description="Placeholder for the graphviz-pure fork of upstream usvg 0.48.1." ;;
    graphviz-pure-svg2pdf)
      license="MIT OR Apache-2.0"; description="Placeholder for the graphviz-pure fork of upstream svg2pdf 0.13.0." ;;
  esac
  d="$work/$c"; mkdir -p "$d/src"
  cat > "$d/Cargo.toml" <<TOML
[package]
name = "$c"
version = "0.0.0"
edition = "2021"
license = "$license"
description = "$description"
repository = "https://github.com/graphviz-pure/graphviz-pure"
readme = "README.md"
TOML
  printf '//! Placeholder: see <https://github.com/graphviz-pure/graphviz-pure>.\n' > "$d/src/lib.rs"
  printf '# %s\n\n%s\n' "$c" "$description" > "$d/README.md"
  until (cd "$d" && cargo publish --allow-dirty 2>&1 | tee "$d/log"); do
    grep -q "429\|rate limit" "$d/log" || exit 1
    echo "rate limited; retrying in 10 minutes"; sleep 600
  done
done
echo "Now configure trusted publishing for each crate (docs/releasing.md) and delete the token."
