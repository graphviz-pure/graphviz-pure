#!/usr/bin/env bash
# Prints a key for everything a fuzzing run's outcome depends on, so fuzz.yml can skip a run whose
# inputs an earlier run already fuzzed:
#
# - the fuzz workspace (fuzz/: targets, Cargo.toml, Cargo.lock, dictionary, options, seed script)
#   and the ClusterFuzzLite build (.clusterfuzzlite/);
# - the code the targets link: every crate's src/ and fonts/ (embedded), and the manifests;
# - the seeds: every tests/**/*.gv (fuzz/seed-corpus.py).
#
# The crates' own version is left out (the root's [workspace.package] version, its requirements on
# the workspace's crates, and their versions in fuzz/Cargo.lock): a release's version bump changes
# no code. release.yml calls fuzz.yml on the base commit, so the key is that commit's. Tests
# other than the seeds, documentation,
# scripts and the site are not part of the key.
#
# Usage: scripts/fuzz-inputs-key.sh   (in a checkout; reads the index, so commit first)
set -euo pipefail
cd "$(dirname "$0")/.."

{
  git ls-files --stage -- fuzz .clusterfuzzlite ':(glob)crates/*/src/**' ':(glob)crates/*/fonts/**' \
    ':(glob)crates/*/Cargo.toml' ':(glob)tests/**/*.gv' ':(exclude)fuzz/Cargo.lock'
  # The root manifest without the workspace version, in [workspace.package] and in the
  # requirements on the workspace's own crates.
  awk '/^\[/{sec=$0} !(sec == "[workspace.package]" && /^version = /)' Cargo.toml |
    sed -E 's/^(graphviz-pure[a-z-]* = \{ path = "[^"]+"), version = "[^"]+"/\1/'
  # fuzz/Cargo.lock without the version of the workspace's own packages.
  awk '/^name = "graphviz-pure/{own=1; print; next} own && /^version = /{own=0; next} {own=0; print}' fuzz/Cargo.lock
} | git hash-object --stdin
