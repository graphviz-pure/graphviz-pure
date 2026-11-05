# OSS-Fuzz application for graphviz-pure

The three files [google/oss-fuzz](https://github.com/google/oss-fuzz) needs to
fuzz this port, in the layout it expects:

```
oss-fuzz/
  projects/
    graphviz-pure/
      Dockerfile    the builder image: the Rust builder, the repository cloned in
      build.sh      builds the cargo-fuzz targets and installs them in $OUT
      project.yaml  the project metadata
```

They are a copy for `google/oss-fuzz`, not part of this repository's own build.
The repository keeps fuzzing on
[ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) (`.clusterfuzzlite/`,
`.github/workflows/fuzz.yml`); `oss-fuzz/` is what goes to Google, so that
Google's infrastructure fuzzes the port as well.

Nothing here changes the port, its tests or its CI. The files build the ten
cargo-fuzz targets that already exist in `fuzz/`.

## What each file does

`Dockerfile`
: The OSS-Fuzz Rust builder (`gcr.io/oss-fuzz-base/base-builder-rust`), which
  has a nightly toolchain and `cargo-fuzz` in it, and the repository cloned at
  its default branch. The tag is `latest` and not a digest, because OSS-Fuzz's
  base OS consistency check (`infra/ci/check_base_os.py`) parses the tag of the
  `FROM` line.

`build.sh`
: `cargo fuzz build -O --debug-assertions` in `fuzz/`, then each target is
  copied into `$OUT` with its dictionary (`dot.dict`), its libFuzzer options
  (`fuzz.options`) and its seed corpus (the repository's test graphs, through
  `fuzz/seed-corpus.py`). It is the same build as `.clusterfuzzlite/build.sh`,
  which builds the same targets for ClusterFuzzLite; keep the two in step.

`project.yaml`
: The metadata OSS-Fuzz reads: the language (`rust`), the repository to fuzz
  (`main_repo`), the contact address for the notifications and the crash
  reports (`primary_contact`), and the engine and the sanitizer.

## Before submitting: one thing to change

`primary_contact` is a placeholder:

```yaml
primary_contact: "TODO-REPLACE-ME@example.com"
```

Replace it with the address of the maintainer who should receive the OSS-Fuzz
notifications (build failures) and be copied on the crash reports. It has to be
a real address on a Google account, because OSS-Fuzz gives the contact access
to the project's crash reports and statistics. `example.com` is reserved for
documentation (RFC 2606), so it can never reach a mailbox; OSS-Fuzz only checks
that the address has the shape of one, so the placeholder passes its check and
the pull request can be prepared before the address is known.

## Opening the pull request

The pull request to `google/oss-fuzz` has to come from the owner's account (a
pull request opened by the repository's GitHub App could not be approved
there). Everything else is prepared in this repository.

```sh
# 1. Fork google/oss-fuzz (once): https://github.com/google/oss-fuzz/fork
gh repo fork google/oss-fuzz --clone=false --remote=false

# 2. Clone the fork and add google/oss-fuzz as the upstream.
git clone git@github.com:<owner>/oss-fuzz.git
cd oss-fuzz
git remote add upstream https://github.com/google/oss-fuzz.git

# 3. Branch off upstream/master.
git fetch upstream master
git checkout -b projects/graphviz-pure upstream/master

# 4. Copy the three files in (GVP is a checkout of this repository).
GVP=/path/to/graphviz-pure
mkdir -p projects/graphviz-pure
cp "$GVP"/oss-fuzz/projects/graphviz-pure/{Dockerfile,build.sh,project.yaml} \
   projects/graphviz-pure/

# 5. Replace the primary_contact placeholder in projects/graphviz-pure/project.yaml.

# 6. Commit and push.
git add projects/graphviz-pure
git commit -m "Add graphviz-pure project"
git push origin projects/graphviz-pure

# 7. Open the pull request (the text is below).
gh pr create --repo google/oss-fuzz \
  --base master --head <owner>:projects/graphviz-pure \
  --title "Add graphviz-pure project" \
  --body-file pr-body.md
```

Those three files are all `google/oss-fuzz` needs. Nothing else goes into
`projects/graphviz-pure/`: the seed corpora are built during the build (a
committed corpus zip under `projects/` is refused by OSS-Fuzz's presubmit
check), and the dictionary and the options are copied into `$OUT` by
`build.sh`.

`gh pr create` asks for the body interactively if `--body-file` is left out;
the text below is the one to paste either way.

### The pull request text

Title: `Add graphviz-pure project`

Body:

```markdown
This adds graphviz-pure to OSS-Fuzz.

graphviz-pure is a safe-Rust port of Graphviz 16.1.0 that renders
byte-identical output. It carries coverage-guided fuzz targets of its own
(cargo-fuzz, in `fuzz/`), which its CI fuzzes weekly with ClusterFuzzLite.
This project brings the same targets to OSS-Fuzz, so Google's infrastructure
fuzzes the port too.

The Dockerfile clones the repository and `build.sh` builds the cargo-fuzz
targets with `cargo fuzz build -O --debug-assertions`, then installs each one
in `$OUT` with its dictionary (`dot.dict`), its libFuzzer options
(`fuzz.options`) and its seed corpus (the repository's test graphs, through
`fuzz/seed-corpus.py`). The build script mirrors the repository's own
`.clusterfuzzlite/build.sh`, which builds the same targets for
ClusterFuzzLite.

The targets are:

| Target | What it runs |
|---|---|
| `parse` | the DOT parser and the canonical writer on every graph read before an error |
| `parse_html` | the HTML-label parser (`label=<...>`) |
| `render_dot` | layout with `dot` and an SVG rendering |
| `render_neato` | layout with `neato` and an SVG rendering |
| `render_fdp` | layout with `fdp` and an SVG rendering |
| `render_sfdp` | layout with `sfdp` and an SVG rendering |
| `render_twopi` | layout with `twopi` and an SVG rendering |
| `render_circo` | layout with `circo` and an SVG rendering |
| `render_osage` | layout with `osage` and an SVG rendering |
| `render_patchwork` | layout with `patchwork` and an SVG rendering |

The inputs are at most 8 KiB, and the render targets skip graphs with more
than 50 nodes, 100 edges or 25 subgraphs, so layout's superlinear cost is not
reported as a hang.
```

## Testing it locally

The build was tested with OSS-Fuzz's own tooling, from a checkout of
`google/oss-fuzz`:

```sh
git clone --depth 1 https://github.com/google/oss-fuzz.git
cd oss-fuzz
GVP=/path/to/graphviz-pure
mkdir -p projects/graphviz-pure
cp "$GVP"/oss-fuzz/projects/graphviz-pure/* projects/graphviz-pure/

# Builds the image and the fuzzers; the output lands in build/out/graphviz-pure.
python3 infra/helper.py build_fuzzers graphviz-pure

# Runs every fuzzer in the base-runner image; fails if one of them does not run.
python3 infra/helper.py check_build graphviz-pure
```

`build_fuzzers` and `check_build` both need Docker. The build writes into
`oss-fuzz/build/` (`out/` and `work/`), which is where `infra/helper.py` keeps
its artifacts.

OSS-Fuzz's own checks on the pull request pass on these files:

```sh
python3 infra/ci/check_base_os.py projects/graphviz-pure
python3 -c 'import sys; sys.path.insert(0, "infra"); import presubmit; \
  print(presubmit.do_checks(["projects/graphviz-pure/Dockerfile", \
  "projects/graphviz-pure/build.sh", "projects/graphviz-pure/project.yaml"]))'
```

The first is the base OS consistency check that runs on every pull request that
touches `projects/`; the second is the presubmit check (the Apache licence
headers, the `project.yaml` sections and the project name).

### The result

Both passed on the files in this directory, against the `master` branch of this
repository:

```
python3 infra/helper.py build_fuzzers graphviz-pure   # exit 0
python3 infra/helper.py check_build graphviz-pure     # exit 0, "Check build passed."
```

`build/out/graphviz-pure` then held the ten fuzzers, each with its dictionary,
its options and its seed corpus:

```
parse  parse_html  render_dot  render_neato  render_fdp
render_sfdp  render_twopi  render_circo  render_osage  render_patchwork
```

The seed corpora had the 2,421 test graphs of the repository for the DOT
targets and the 90 HTML labels for `parse_html`. The fuzzers carry the
AddressSanitizer runtime (`-Zsanitizer=address`, which OSS-Fuzz puts in
`RUSTFLAGS`).

## Notes

- The `FROM` line carries the `latest` tag rather than a digest, because the
  base OS consistency check parses the tag. `.clusterfuzzlite/Dockerfile` pins
  the same image by digest, which that check would reject.
- The build does not pass `--locked` to cargo. `fuzz/Cargo.lock` records the
  workspace's own crates at an older version than the manifests have, so cargo
  updates it during the build. That is the repository's current state, not
  something this project changes.
- The fuzzers are built with AddressSanitizer: OSS-Fuzz exports
  `RUSTFLAGS` with `-Zsanitizer=address`, and cargo-fuzz appends the ambient
  `RUSTFLAGS` to its own flags.
- Ten targets are built one after another with `CARGO_BUILD_JOBS=2`. Each one
  links the whole port with thin LTO, and ten links at once run out of memory
  in the builder.
