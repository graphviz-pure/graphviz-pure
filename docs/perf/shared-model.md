# Shared graph records for osage and patchwork

Design, 2026-11-04; first step of #610. The owner’s next milestone is the
3× per-engine floor defined by [1.x-10x.md](1.x-10x.md), starting from the
reported 2.84× osage and 2.47× patchwork. Those numbers require another
5.63% and 21.46% speedup respectively (5.33% and 17.67% less time).
Instruction reductions alone are not evidence for this milestone.

## Current ownership

The parser owns `core::Graph`: node names, attributes, adjacency and output
positions. `LGraph` borrows that graph but constructs separate `LNode`s and
`NodeShape`s. `node_from_geometry` transfers polygon vertices into the clipping
shape, keeps only label dimensions and discards measured text/HTML. Finishing
layout writes positions back and drops the layout records. Render’s `Model`
then calls `node_geometry` again for every node, constructs separate labels and
shape records, and borrows only names, attributes and spline points.

Osage initializes in node order. Patchwork initializes in `walk_order`, after
setting `shape=box` and resolving its generated fontsize; its clipping size,
measured shape size and final patch size are distinct. Reuse must preserve all
three. A cache keyed only by node id is not a valid production design: ids
repeat across graphs, and attributes and charset can change between layouts.

## Target ownership

Introduce an internal `PreparedGraph` in the geometry crate, owning the parsed
`core::Graph` and a dense arena of measured node records. This direction avoids
a core → geometry dependency cycle (HTML and record geometry depend on core).
The facade’s opaque `Graph` and the decoded CLI job state own this wrapper;
their stable signatures stay unchanged. The parser moves its graph into it.
Names and attributes remain in that one graph. No second parsed graph is built.

Each record holds the shape, measured main/external labels and intrinsic sizes
once; mutable placement fields are separate. Layout borrows immutable records
and mutates position/scratch arrays. `NodeShape` becomes a clipping view over
the polygon/record geometry rather than owning another vertex vector or shape
name. Render borrows the same records with its final placement. HTML emission
currently mutates cloned labels: move that mutation into render scratch or a
placement view rather than retaining both HTML trees. Text spans likewise
borrow their measured strings. Repeated formats borrow without consuming the
record. Graph clone remains independent through ordinary owned cloning.

Do not premeasure all nodes before engine initialization. Fill slots where C
calls `common_init_node`, preserving diagnostics, font alias state, charset,
user-shape registration and patchwork’s walk order. Records are invalidated at
layout start and on any internal graph/attribute edit. On error drop incomplete
records. Rendering a parsed graph without layout uses the existing sizing path.
Fallback remains for other engines until separately measured and proved.

Geometry reuse also needs an equivalence check: render currently measures from
post-layout attributes. In particular patchwork overwrites output width/height;
its reused intrinsic geometry must match the old measurement, not simply reuse
its final patch dimensions. Test explicit dimensions, fixedsize, HTML, records,
xlabels, malformed labels, images, rankdir, repeated render and relayout.

## Memory and fidelity gates

A retained copy alongside `LNode` and `Model` is not the target. Move ownership
or borrow at each boundary. Dense metadata has a cost too: account for arena
capacity and eliminate replaced records before accepting it. Require no peak
heap increase on both 20k cases and the complete deterministic ratchet; RSS is
reported separately. No new dependencies, unsafe code, float reassociation,
changed summation order, changed tie-breaking, or baseline updates.

Use T9 (allocation reuse) and T6 (phase-specific records) from the post-1.0 catalogue. C provenance remains
`common_init_node`, `poly_init`, `make_label`, `gv_nodesize`, `gv_postprocess`
and `emit_node`. Intrinsic measurements keep the same operations and operands;
placement keeps the same operations in the same traversal order.

## First experiment

Before changing production ownership, isolate the cost of duplicate measurement
on the current master. An unshipped diagnostic patch retains a clone of the
initial `NodeGeometry`, and consumes that retained value during render instead
of measuring again. Only osage and patchwork enable capture; other engines keep their existing sizing but pay the diagnostic
TLS checks, and independent PGO can also move their timings. It intentionally pays for duplicate retained storage and cloning,
so it tests whether avoiding measurement can overcome that cost, not whether
the final ownership design meets the memory gate. It cannot establish the gain
from eliminating all stage models, nor can a null result disprove that broader
premise. It must never become the production API or a global node-id cache.

Build baseline and candidate separately with the fixed `build-pgo.py` training
set, x86-64-v3 musl and fresh PGO profiles. Copy baseline to two control paths
and verify identical SHA-256; verify candidate differs. Run randomized paired
rounds for osage/patchwork 20k SVG and plain, plus unaffected paths. Compare
bytes before timing. Record wall/user/kernel/RSS, paired ratios and control
noise. Use fresh 1.0 measurements for per-case speedups; the engine floor needs
all three headline cases (5k SVG, 20k SVG, 20k plain), not 20k alone.

## Small PR sequence

1. This design and measured diagnostic experiment, with its limitations.
2. Add graph-owned prepared node records and layout borrowing for these engines;
   preserve the old renderer with a test-mode equality check. Accept only if
   removing replaced layout storage keeps peak heap at or below master.
3. Borrow those records in render, including label/span/HTML placement views;
   remove duplicate sizing and render ownership. Measure all three headline
   cases and unaffected engines/formats with paired controls.
4. Remove remaining node/edge/cluster stage copies only where profiles show a
   wall-time benefit. Each PR passes unchanged fidelity and performance gates.

Stop after each PR for coordinator review; do not merge or edit
`ratchet-baseline.json`. If the experiment cannot distinguish a wall gain from
its identical-binary control, report that before building the full refactor.

### Experiment boundaries

The diagnostic is in `scripts/bench/experiments/shared-model.patch`; it is data,
not compiled workspace code. Apply only in a disposable clean checkout at the
recorded base. Its thread-local slot is deliberately restricted to a single
layout followed by one render. It is not valid for interleaved graphs, retained
facade graphs, or multiple output formats in a job. Candidate training still
uses the complete, unchanged PGO training list. Benchmark commands run one
input and one format per process. The runner checks complete stdout hashes
and diagnostic bytes before each timed case. No fidelity claim extends beyond
those explicit comparisons.

A production implementation must replace the thread-local transfer with an
explicit graph owner and borrowed views. In particular a copied geometry cache
cannot pass simply because instructions fall, and this experiment must not be
used to claim a measured full-model speedup or to close #610.

## Measured result: copied retention is rejected

Base: `36096d9eba0fc3e47bcccfe9da34241092fe6906`. Baseline and patched candidate
were separately built/trained with Rust 1.99.1 / LLVM 23.1.1, musl,
`-C target-cpu=x86-64-v3`, and `scripts/build-pgo.py` (276 identical training
commands, fresh profiles). The candidate is the base plus the diagnostic patch;
the build metadata commit alone does not identify its modified source.

Thirty randomized paired rounds per case, seed 610, serial execution, nice 15,
8 GB virtual-memory ceiling. Times include process startup, the time wrapper,
and output to `/dev/null`. Every before/after stdout hash and stderr matched
for these nine commands. Separate PGO training is part of this experiment;
code placement/profile noise cannot be excluded. This is a single window.

The **control/candidate** column directly compares the candidate with the
identical baseline copy in each round; its interval is a paired percentile
bootstrap (10,000 resamples, seed 610). Values above 1 mean faster. Neither
20k engine/format has an interval excluding 1.0. Base/candidate alone would
misleadingly suggest a 3% patchwork SVG gain while base/control shows 2%.

| Case | Base ms | Candidate ms | Base/candidate paired | Base/control paired | Control/candidate (95%) | 1.0 ms | Candidate ×1.0 |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| osage_clustered_5000.gv / svg | 46.50 | 47.66 | 0.975 | 0.988 | 0.987 (0.966–1.006) | 134.95 | 2.83 |
| osage_clustered_20000.gv / svg | 177.72 | 176.81 | 0.996 | 1.005 | 0.991 (0.979–1.003) | 559.84 | 3.17 |
| osage_clustered_20000.gv / plain | 174.70 | 171.93 | 1.021 | 1.012 | 1.008 (0.999–1.017) | 545.79 | 3.17 |
| patchwork_clustered_5000.gv / svg | 56.50 | 57.04 | 0.980 | 0.981 | 0.999 (0.982–1.015) | 134.09 | 2.35 |
| patchwork_clustered_20000.gv / svg | 204.58 | 201.25 | 1.030 | 1.020 | 1.009 (0.995–1.023) | 559.62 | 2.78 |
| patchwork_clustered_20000.gv / plain | 193.85 | 191.30 | 1.005 | 0.999 | 1.006 (0.987–1.020) | 531.92 | 2.78 |
| dot_dag_1000.gv / svg | 90.36 | 92.52 | 0.983 | 0.995 | 0.988 (0.976–1.001) | 292.94 | 3.17 |
| twopi_tree_5000.gv / svg | 72.16 | 71.49 | 1.013 | 1.010 | 1.003 (0.980–1.026) | 160.50 | 2.25 |
| neato_sparse_250.gv / svg | 63.20 | 62.85 | 0.996 | 1.002 | 0.994 (0.980–1.010) | 768.08 | 12.22 |

The unoptimized dot path is 1.7% slower against base (1.2% against control);
twopi and neato also remain in the table, rather than hiding incidental
movement. They retain diagnostic TLS checks, and PGO also changes code layout.
These observations are not a production regression because the patch is not
compiled by this PR.

| 20k case | User ms base → candidate | Kernel ms base → candidate | RSS KiB base → candidate | Minor faults base → candidate |
| :-- | --: | --: | --: | --: |
| osage_clustered_20000.gv / svg | 110 → 110 | 50 → 50 | 61542 → 78538 | 13271 → 13935 |
| osage_clustered_20000.gv / plain | 120 → 115 | 45 → 50 | 50504 → 67380 | 11978 → 12615 |
| patchwork_clustered_20000.gv / svg | 140 → 130 | 60 → 60 | 64656 → 83432 | 14404 → 14831 |
| patchwork_clustered_20000.gv / plain | 130 → 125 | 50 → 50 | 51916 → 70372 | 12970 → 13730 |

User/kernel times are medians from GNU time's centisecond resolution; do not
interpret sub-10-ms differences in those columns. The added geometry clone
and dense retained arena inflate RSS and erase the hoped-for wall benefit.
The experiment does **not** falsify a model that actually removes duplicate
storage. It does falsify treating retained copies as a sufficient first
optimization. A production change must borrow/transfer the records and meet
the heap gate before claiming any gain.

### Progress toward the floor

This PR ships no runtime change: credited progress is **osage 2.84× → 2.84×;
patchwork 2.47× → 2.47×**, using the owner's starting figures. No floor is
claimed. The fresh reference medians above are informational, not replacements
for those figures; the diagnostic candidate is not releasable. The structural
implementation and all production fidelity/performance gates remain work for
subsequent PRs. The owner/coordinator can review this negative experiment
before committing to that larger implementation.

Using this window’s three headline cases and fresh reference medians gives
osage **3.057× base / 3.053× diagnostic**, and patchwork **2.612× base /
2.629× diagnostic**. The quiet-window base already differs from the owner's
starting figures. These are single-window observations, and the paired
comparison establishes no useful improvement; do not credit the apparent
osage crossing to this experiment or claim that the milestone is achieved.

### Reproduction and evidence

[Raw paired samples and sanitized build metadata](data/shared-model-2026-11-04.json)
include hashes for every binary and case, timings and RSS/faults for every run.
[Diagnostic patch](../../scripts/bench/experiments/shared-model.patch),
[runner](../../scripts/bench/experiments/shared-model.py).

Build each revision using `scripts/build-pgo.py --target
x86_64-unknown-linux-musl --target-dir JOBS_TARGET --profiles FRESH_PROFILES
--cargo-wrapper /usr/local/bin/cargo-slot.sh`, with `RUSTFLAGS='-C
target-cpu=x86-64-v3'`. Copy the base executable to a separate control path.
Run the Python runner with `--base`, `--candidate`, `--control`, `--reference`
(the shipped-style 1.0 musl binary), `--synth`, and `--out`, under nice 15 and
`ulimit -v 8000000`. Keep builds and profiles on the jobs volume; do not run
builds concurrently with the paired timings. Apply the patch only after the
baseline build has completed.

Load at start/end:

```text
14:41:51 up 1 day, 10:42,  0 users,  load average: 4.08, 3.28, 1.67
14:45:23 up 1 day, 10:46,  0 users,  load average: 1.70, 2.58, 1.73
```

### Concrete migration boundary

`PreparedGraph { source: core::Graph, nodes: Vec<Option<NodeGeometry>> }`
is an ownership sketch, not a public API. Borrow its disjoint fields for each
phase. Pass `&NodeGeometry` to a clipping view together with mutable `LNode`
placement/scratch; do not add a complete geometry field to every existing
`LNode`. Build records in the engine's initialization order. At postprocess,
apply placement directly to the one source graph or move the detached placement
result as today; keep measured records alive. Render's node view borrows both
the source node and its geometry, with final position and label placement in
separate small values. No self-referential structs are required.

The CLI's decoded `LayoutJobs`/graph-job boundary and the facade's private
`Graph::inner` both need migration; changing only `render_parsed` would miss
streaming jobs, explicit layout/render calls, PNG and multiple output formats.
Internal callers that still accept `core::Graph` may use an explicit uncached
fallback. The migration must not silently break graph cloning, error cleanup,
relayout into a different engine, or rendering two graphs in interleaved order.

## Heap check of the rejected diagnostic

Same source revisions, `heap_counts`, musl x86-64-v3 release without PGO;
equal-length executable paths. Dhat measures live requested bytes, not RSS.
Both builds used cargo-slot; runs were serial, nice 15, with the 8 GB ceiling.
No new counting allocator or dependency was introduced.

| 20k case | Allocations base → diagnostic | Allocated bytes base → diagnostic | Peak bytes base → diagnostic | Peak increase |
| :-- | --: | --: | --: | --: |
| osage svg | 775,747 → 775,748 | 116,095,289 → 132,686,399 | 57,223,363 → 72,903,363 | 27.4% |
| osage plain | 882,432 → 882,433 | 91,830,678 → 108,421,788 | 44,804,503 → 60,484,503 | 35.0% |
| patchwork svg | 788,130 → 788,131 | 125,286,804 → 141,877,914 | 59,703,473 → 75,383,473 | 26.3% |
| patchwork plain | 894,815 → 894,816 | 98,785,167 → 115,376,277 | 46,166,100 → 61,846,100 | 34.0% |

All four cases add exactly 15,680,000 peak bytes and one net allocation.
Remeasurement has been replaced by a clone, not by eliminated storage. The
15.68 MB extra peak is a rejection under the owner's no-increase requirement.
This PR does not enable the diagnostic or claim that this memory increase is
acceptable. No instruction-count reduction is offered as a substitute for its
null wall-time result.

## Informational C reference

Five separate runs after the paired window, stock C Graphviz 16.1.0 with the
pango-free plugins, no preload; nice 15, 8 GB ceiling, output discarded. These
are speed references, not a byte-fidelity oracle or paired performance claims.
Do not compare these absolute C times as if they shared the paired window.

| Case | C median ms | C / diagnostic |
| :-- | --: | --: |
| osage_clustered_5000.gv / svg | 57.07 | 1.20× |
| osage_clustered_20000.gv / svg | 220.13 | 1.25× |
| osage_clustered_20000.gv / plain | 198.16 | 1.15× |
| patchwork_clustered_5000.gv / svg | 67.29 | 1.18× |
| patchwork_clustered_20000.gv / svg | 271.21 | 1.35× |
| patchwork_clustered_20000.gv / plain | 195.96 | 1.02× |

## Verification of this PR

- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `scripts/panic-sites.sh`: pass; baseline unchanged.
- `cargo test --workspace`: pass, including the existing fidelity ratchets.
- All cargo invocations used cargo-slot, four jobs/test threads and the 8 GB
  virtual-memory ceiling; all heavy runs were serial at nice 15.
- Python syntax and diagnostic patch application checks: pass.
- No production Rust source, public API, dependency or baseline changed.

- Full local performance ratchet: pass, no baseline refresh. This checks the
  restored production source, not the rejected diagnostic. Temporary ratchet
  and profiling build directories and raw profiles were removed afterward.
