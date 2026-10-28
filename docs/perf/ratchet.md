# The perf ratchet

A CI check (`.github/workflows/perf.yml`) that fails a pull request which makes the command line
do more work or hold more memory on a fixed benchmark set. It is the third ratchet of the
post-1.0 plan ([post-1.0.md](../roadmap/post-1.0.md#the-hill-climbing-process)), after fidelity
and panic sites, and builds on the harness and the measurements of the
[baseline](baseline.md). Wall time is never checked: it is too noisy on shared runners.

## What it measures

For every case of [`scripts/bench/sets/quick.tsv`](../../scripts/bench/sets/quick.tsv) and
[`sets/ratchet-extra.tsv`](../../scripts/bench/sets/ratchet-extra.tsv), each run once:

| metric | how | gate |
| :-- | :-- | :-- |
| instructions | `valgrind --tool=callgrind` on the `graphviz-pure` binary (Callgrind's `Collected`, the instructions executed) | fails above +2% |
| allocations | `examples/heap_counts.rs`: the command line under [dhat](https://docs.rs/dhat)'s counting allocator, `total_blocks` | fails on any increase |
| peak heap | the same run, dhat's `max_bytes` (the most heap live at once) | fails above +5% |
| bytes allocated | the same run, dhat's `total_bytes` | reported |
| peak RSS | `/usr/bin/time` on a native run | measured, not reported or checked (it depends on the kernel and the allocator's page reuse) |
| `.wasm` size | `graphviz_pure_wasm.wasm`, release build for `wasm32-unknown-unknown`, before wasm-bindgen | reported |

The cases: one or two inputs per engine from the quick set (10–500 ms natively), `-Tpng` on
`unix.gv`, `-Tplain` on a realistic dot graph, and two cases without a layout: b29 laid out by
the build under test (`-Tdot`) and read back with `-Knop2`, written as `-Tcanon` (mostly parsing
and the graph model) and as `-Tsvg` (mostly rendering). The large inputs of the baseline
(rnd2000, dag_5000, the 20,000-node graphs) are not in the set: Callgrind runs about 40× slower
than native and dhat about 15×, so each would add minutes.

Both binaries are built for **x86_64-unknown-linux-musl**, the static build releases ship for
Linux. A static binary carries its own `malloc` and `memcpy`, so the instruction counts do not
depend on the runner's glibc or on which CPU-specific `memcpy` glibc picks; counts from the
same build agree to within a few thousand instructions between runs (the environment and the
process ID). Allocation counts and peak heap do not vary at all, as long as the input paths are
the same, which is why every case runs from the repository root with relative paths. musl's
allocator is slower than glibc's, so the musl instruction counts include more `malloc` work
than a glibc build would; the ratchet compares like with like, so that does not matter for the
gate.

dhat is a dev-dependency of `graphviz-pure-cli`, used only by the example: no shipped binary has
it as its allocator, and `#[global_allocator] static ALLOC: dhat::Alloc` needs no `unsafe`.

The build pins the release profile of the workspace `Cargo.toml` (`ratchet.py`'s `PROFILE`) and
`perf.yml` pins the Rust version (`RUST_VERSION`, as release.yml does). A change to the shipped
profile or a Rust upgrade therefore moves the ratchet only when it is copied there, in a pull
request that also refreshes the baseline.

## Running it

```sh
python3 -I scripts/bench/ratchet.py            # build, measure, check: about 3 minutes, 4 jobs
python3 -I scripts/bench/ratchet.py --update   # ... and rewrite scripts/bench/ratchet-baseline.json
python3 -I scripts/bench/ratchet.py --update --from results.json   # from CI's perf-ratchet artifact
python3 -I scripts/bench/ratchet.py --only b29 # a subset
```

It needs Valgrind and the `x86_64-unknown-linux-musl` target (`rustup target add`); `--wasm`
also builds the wasm module. The measurement goes to `target/ratchet/results.json` and the report
to stdout (and the job summary in CI).

## The baseline and the label

[`scripts/bench/ratchet-baseline.json`](../../scripts/bench/ratchet-baseline.json) holds, per
case, the counts the check compares with, plus two reference columns that `--update` keeps:
`reference`, the instruction counts of release 0.2.2 (the `×0.2.2` column; `--update
--set-reference NAME` replaces it, for 1.0), and `c_instructions`, stock C 16.1.0 under the same
Callgrind (the `×C` column), measured with

```sh
python3 -I scripts/bench/ratchet.py --c-dot /opt/graphviz-16.1.0/bin/dot \
    --c-gvbindir /opt/graphviz-16.1.0/lutplugins
```

C has no `-Tpng` in that setup, and it runs with the machine's glibc while the port's counts are
the musl build's (musl's `malloc` and `memcpy` execute more instructions than glibc's, and
x86-64 has no FMA instruction in its baseline, so every `mul_add` is a software `fma` call:
hotspot 1 of the baseline). The ×C ratios are a headline, not a gate, and they are not wall-time
ratios: dot is faster than C in wall time on the baseline's gnu build but executes more
instructions here.

A case that gets better passes; lock the gain in by running `--update --from` the pull request's
CI artifact (`perf-ratchet`) and committing the baseline in the same pull request, as with
`scripts/panic-sites.sh --update`. Take instruction counts from CI only: allocations and peak
heap are the same on every machine, but a local Valgrind and CPU count up to 3% differently
from the runner's on the dot cases, enough to fail the 2% gate there. A case missing from the
baseline, a baseline case that was not measured, and a case whose command fails are failures
that the label below does not accept. A pull request that has to make a case worse carries the
`perf-regression-accepted` label and says why in its description; the job then reports the
regressions and passes, and the pull request refreshes the baseline with `--update`.

The measurement itself takes about 75 seconds with 4 jobs on a 5900XT; on a 4-core GitHub-hosted
runner, the two release builds (musl CLI and example, and wasm) come first.
