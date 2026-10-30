# After 1.0: faster, smaller, idiomatic (a plan for hill-climbing)

Status: **decided** on 2026-10-27 (see [Decisions](#decisions)). The draft issues are filed in the
[`post-1.0` milestone](https://github.com/graphviz-pure/graphviz-pure/milestone/8) as #530–#540; the `-Tplain` item (#541) was fixed before 1.0 by #566.
It assumes 1.0 as the 1.0 roadmap defines it (#512; it adds `docs/roadmap/1.0.md`), which has no
performance target of its own. The benchmark harness and the measured baseline are a sibling
effort (`docs/perf/baseline.md` and `scripts/bench/`, #518). This document reuses them
and doesn't define a second harness.

Up to 1.0 the port is a line-by-line translation of Graphviz 16.1.0, held byte-identical to C by
the fidelity ratchets. After 1.0 the project hill-climbs. Each step makes the code faster, smaller
in memory, or more idiomatic Rust, and each step keeps the bytes.

Completed since this plan was written: the performance ratchet (#530, #547), `pow` fast paths
(#532, #543), sfdp quadtree reuse (#534, #548), provenance reporting (#539, #542), and the
release-profile evaluation and CLI stripping (#540, #546). Their original measurements below
remain historical; they are not pending work. The declared MSRV is now Rust 1.88 and CI tests it
(#504, #565).

The first 1.x goal, 10× faster than 1.0, has its own metric, baseline and budget:
[`docs/perf/1.x-10x.md`](../perf/1.x-10x.md) (milestone `1.x: 10× faster`).

## Contents

1. [Working assumption](#working-assumption-the-bytes-dont-move)
2. [Goals and success metrics](#goals-and-success-metrics)
3. [Constraints](#constraints)
4. [What the code does today: audit and measurements](#what-the-code-does-today)
5. [Technique catalogue](#technique-catalogue) (table, then a section per high-value item)
6. [The hill-climbing process](#the-hill-climbing-process)
7. [Phased plan](#phased-plan)
8. [Decisions](#decisions)
9. [Draft issues](#draft-issues)
10. [Sources](#sources)

## Working assumption: the bytes don't move

**Post-1.0 output stays byte-identical to C (C computing with the port's math).** The ~2,600-file
fidelity ratchet is what makes aggressive refactoring safe. A rewrite of a data structure, a
loop, or a whole phase is acceptable when every ratchet file still matches. Without that
guarantee, every refactor needs a judgement call about whether a layout is "still fine".

So every technique below is classed by its **fidelity risk**:

| class | meaning | how it is proved |
| :--- | :--- | :--- |
| **none** | No floating-point operation changes order, operands or rounding, and no tie-break or iteration order that reaches output changes. | Ratchets pass, so nothing more is needed. |
| **ordering** | The floats are untouched, but an iteration order, a sort's tie order, a hash-map walk, or the order of RNG draws could change. | Ratchets, plus an argument in the PR that the order is preserved (for example: "the map is never iterated", or "a stable sort over the same key"). |
| **float-order** | Changes the order or fusion of floating-point operations: reassociated sums, SIMD reductions, multiple accumulators, fast-math, parallel reductions. | **Not allowed under the current policy.** These are listed separately under [decision 1](#decisions). |

A technique classed *none* or *ordering* still has to pass every ratchet. The class is about how
much reasoning the review needs, not a waiver.

## Goals and success metrics

All metrics are measured with the harness of #518 (`scripts/bench/`). Its
[performance and memory baseline](../perf/baseline.md) (`docs/perf/baseline.md`, 0.2.2) is the
reference for every number here; its hotspot table ranks the work. The numbers in this document
are one-off measurements from this research, listed under [measurements](#measurements), and
reconciled with the baseline there.

| goal | metric | target for the post-1.0 effort |
| :--- | :--- | :--- |
| Faster | Instruction count (primary) and wall time (secondary) per corpus input, per engine | Headline against stock C 16.1.0: no engine slower than C on any baseline input, and the geometric-mean speed-up over C reported. Also tracked against 1.0: geometric-mean 2× faster on the benchmark set. |
| Smaller | Peak RSS and bytes allocated per input. WASM `.wasm` size. | Against C: parser and graph model within 2× of C's peak RSS (today 12× when parsing a 12 MB graph; 1.0–3.1× end to end on the baseline set). Against 1.0: WASM size no larger. |
| Idiomatic | Panic-site counts ([the ratchet](../porting/crashes.md#the-panic-site-ratchet)); `clippy::pedantic` warnings; public items of the internal crates | Panic sites ↓ 50% in layout and geom. No C-style sentinels in new code. |
| Safe to change | The three ratchets (fidelity, panic, perf) on every PR | No regressions, enforced by CI. |

Non-goals: new layout algorithms, output that differs from C, or a C-compatible library ABI.

## Constraints

Each constraint is either kept, or proposed for change under [Decisions](#decisions).

| constraint | today | post-1.0 proposal |
| :--- | :--- | :--- |
| Byte-identical output (fidelity ratchets) | Required | **Kept.** Relaxation is [decision 1](#decisions). |
| `#![forbid(unsafe_code)]` workspace-wide (`Cargo.toml` `[workspace.lints.rust]`) | Required in own crates; dependencies (serde, libm, wasm-bindgen, resvg) contain `unsafe` | **Kept** ([decision 3](#decisions)). Nothing in the top-ranked techniques needs `unsafe`. |
| Panic-site ratchet (`scripts/panic-sites.sh`) | 7,568 index, 2,119 arithmetic, 256 unwrap and 42 expect sites in layout and geom at 0.2.2 (9,985; `tests/panic-sites.tsv` allows 9,988) | **Kept**, and it becomes a goal: iterator rewrites remove index sites. |
| Determinism across Linux, Windows, macOS and WASM, with one math library (`graphviz_pure_core::math`) | Required; checked by `cross-platform.yml` | **Kept.** CPU-specific codegen is allowed only where IEEE-754 gives the same bits (FMA, `roundsd`), never a different algorithm per target. |
| WASM is first-class: size matters, no threads by default | `opt-level=z`, fat LTO, `wasm-opt -Oz` | **Kept.** Parallelism is a native-only feature ([decision 5](#decisions)). |
| MSRV | Edition 2024; let-chains used in 33 files (#512's count) | #512's D4: at most current stable minus two releases, kept accurate (#504). Nothing here needs a newer compiler. |
| Minimal dependencies under `cargo deny` | Small tree; raster pulls in resvg | Each new dependency is justified with a measurement ([decision 4](#decisions)). |
| CONTRIBUTING.md: "Follow the C … keep the structure recognisable" | Was required | **Replaced** by provenance tags and a C-mirror list ([decision 2](#decisions)). |
| Public API from 1.0 | #512 D1 (decided): only the facade and raster are semver-stable; `Graph` is opaque with no accessors | **A prerequisite for this plan.** See [below](#api-stability-is-what-frees-the-internals). |

## What the code does today

### Measurements

The full measurements are in [docs/perf/baseline.md](../perf/baseline.md) (#518): every corpus
input against C, scaling, allocations, WASM and a ranked hotspot table. Where it and this
section differ:

- **T1.** The baseline measured `-C target-feature=+fma` over its whole set: neato 4.8–5.0×,
  sfdp 2.3×, fdp 2.0–2.1×, twopi spline routing 2.1×, circo with splines 2.8×, dot 1.0–1.1×,
  with the SVG of 2,603 corpus inputs identical (all but the six slow `badvoro` copies). This
  plan's `x86-64-v3` build passes the full fidelity ratchet unchanged, and measured neato 4.5×
  and sfdp 2.3× on a quiet machine. `+fma` alone gives the same gain and also runs on AMD
  Piledriver (2012) and later, where v3 needs Excavator (2015).
- **Allocation counts.** The baseline's counter gives 4,032,213 allocations and 215 MB for sfdp
  on rnd2000, against 4,054,086 and 272 MB here (another counter, which counts reallocations
  differently). Use the baseline's numbers. Its malloc uprobe puts 85% of them in
  `QuadTree::add_internal`, whose cells hold `Vec<f64>` fields, more than in
  `get_supernodes_internal`.
- **`pow`.** The two agree (hotspot 5): the default `p = -1.0001234` is only the `AUTOP`
  sentinel (`sfdpinit.rs:130`), which `multilevel_spring_electrical_embedding` replaces with
  `-1.0` (`spring_electrical.rs:1022`), so the default exponent is exactly 2. An instrumented
  build counted more than 8.4 M `powf` calls with `y == 2.0` and 12 others on rnd2000, so T2
  was implemented by #543 (#532), including one `pow` call per pair rather than per dimension.
- **Memory.** The parse-only 12× below is the worst case. End to end, the baseline's peak RSS is
  1.0–3.1× C's, and the ratio grows with node count.
- **Release profile.** At 0.2.2 the profile lived in `.cargo/config.toml`, so crates.io
  installs got Cargo's defaults. #546 fixed #540 by moving it into the workspace `Cargo.toml`
  and repeating it in the published CLI manifest.
- **`-Tplain` was quadratic** in the 0.2.2 baseline (hotspot 2). #566 fixed #541
  with one per-output set of plain strings; this is complete.

Machine: AMD Ryzen 9 5900XT (28 threads, FMA and AVX2), shared with other agents (load ~5).
Each wall time is from one run, so treat it as ±10%. The Rust build is `cargo build --release
-p graphviz-pure-cli` at 0.2.2 (the workspace profile is `opt-level=3`, thin LTO,
`codegen-units=1`). C is the stock `/opt/graphviz-16.1.0` build with the platform libm, so it is
a speed reference only, not the fidelity reference. Profiles come from `sudo perf record`, because
`perf_event_paranoid` is 3 on this host. `rnd2000` is `tests/sfdp/gen_rnd2000.gv` without its
`layout=` line (2,000 nodes, 2,999 edges). `big` and `chain` are synthetic graphs: 100k nodes
and 200k random edges (12 MB), and a 50k-node chain (4.9 MB), each with 3 to 5 attributes per
object.

| input | engine / phase | Rust 0.2.2 | Rust with `-C target-cpu=x86-64-v3` | stock C 16.1.0 |
| :--- | :--- | ---: | ---: | ---: |
| rnd2000 | neato `-Tsvg` | **76.99 s** (70.19 s quiet) | **15.54 s** (15.51 s quiet; bytes identical) | 14.44 s |
| rnd2000 | sfdp `-Tsvg` | 1.82 s (1.80 s quiet) | **0.81 s** (0.79 s quiet; bytes identical) | 0.67 s |
| rnd2000 | neato `-Tsvg`, `x86_64-unknown-linux-musl` (the shipped Linux target) | 70.16 s | 15.55 s | — |
| rnd2000 | sfdp `-Tsvg`, `x86_64-unknown-linux-musl` | **7.36 s** | 1.19 s | — |
| rnd2000 | dot `-Tsvg` | 380.9 s | — | 791.3 s |
| `tests/wild/gv16_graphs_b29_d1e4db.gv` | dot `-Tsvg` | 0.89–0.98 s | 0.89–1.00 s | 1.63 s |
| big | parse only (C: `gc`) | 2.85 s, **982 MB RSS** | — | 1.14 s, **82 MB** |
| chain | parse only (C: `gc`) | 0.63 s, 366 MB | — | 0.30 s, 33 MB |

Profiles (self time):

- **neato, rnd2000:** `fmaf_with_fma` 33.9% plus `fmaf` 27.7%, and
  `right_mult_with_vector_ff` 32.9%. Out-of-line FMA calls take 61% of the run. With
  `x86-64-v3` the kernel is 88.5% and the calls are gone.
- **sfdp, rnd2000:** `math::portable::pow` 42.2%, `fma` plus `fma_with_fma` 22.5%, `rint` 5.3%
  (inside `pow`), and `QuadTree::get_supernodes_internal` 13.8%. With `x86-64-v3`, `pow` is
  38.4% and the quadtree 27.1%.
- **dot, b29:** network simplex is 56%: `dfs_range_impl` 29.0%, `enter_edge` 15.7%,
  `rerank` 10.8%. mincross (`mincross`, `in_cross`, `out_cross`, `left2right`, `transpose`) is
  20%, and pathplan (`isdiagonal`, `solve3`, `Pshortestpath`) 9%. malloc and free together are
  under 1%.
- **parse, big:** `__rust_alloc`, `malloc` and `cfree` total about 30%. `Parser::node` is 7.8%,
  `ensure_node_html` 3.3%, and `Token::clone` 2.4%.

Allocations, counted with a scratch `#[global_allocator]` outside the repo:

| run | allocations | bytes allocated | peak live |
| :--- | ---: | ---: | ---: |
| parse b29 (62 KB, 110 nodes, 386 edges) | 96,795 | 6.3 MB | — |
| dot `-Tsvg` b29 after parsing | 348,929 | 142 MB | 15 MB |
| sfdp `-Tsvg` rnd2000 after parsing | 4,054,086 | 272 MB | 15 MB |
| parse big | 24.6 M | 2.07 GB | 982 MB RSS |

Struct sizes (`size_of`): dot's `NodeInfo` is 832 B, `EdgeInfo` 408 B and `GraphInfo` 672 B.
core's `NodeData` is 224 B and `EdgeData` 376 B, each with an `AttrMap` header of 48 B plus heap
data.

### Audit findings

Counts come from `grep` over `crates/*/src` (tests included where they live in `src`). The
columns are core / parser / geom / layout / render.

1. **Out-of-line FMA.** The port calls `f64::mul_add` and `f32::mul_add` (389 sites in 61 files, 44 of them in layout) to
   reproduce clang's fused contraction. On the default `x86_64-unknown-linux-gnu` target, LLVM
   lowers `llvm.fma` without the `fma` target feature to a library call. That call is
   compiler-builtins' `fma`, which dispatches at run time to `fma_with_fma`: correct, but one
   call per multiply-add, and it blocks inlining and vectorization. This is **the single largest
   cost in neato and sfdp** (61% and 22% of samples). aarch64 (macOS arm64, Linux arm64) always
   has fused multiply-add, so it doesn't pay this. wasm32 has no scalar FMA instruction and always
   runs a software `fma`.
2. **`pow` on exact cases.** sfdp's default is `p = -1` (`AUTOP` becomes `-1.0`;
   `spring_electrical.rs:1022`), so every repulsive-force term calls
   `math::powf(dist, 1.0 - p)`, which is `pow(dist, 2.0)`, through the full CORE-MATH
   correctly-rounded path (`spring_electrical.rs:471,513,526`, `quad_tree.rs:294,326`).
   `round_ties_even` inside `pow` becomes a `rint` call without SSE4.1.
3. **The graph and attribute model** (`core/src/graph.rs`).
   - `AttrMap` is a `BTreeMap<String, String>` plus a `BTreeSet<String>` of HTML keys, per
     object. Each node and edge owns copies of every key and value, including inherited
     defaults (a chain node holds `color`, `fontsize`, `label`, `shape` and `tooltip` as 10
     separate heap `String`s).
   - Name lookup is `node_lookup: BTreeMap<String, NodeId>`.
   - Attribute reads are string lookups at use sites: 51 `agget(`, 54 `late_double(`,
     39 `late_int(`, 45 `mapbool(` and 23 `late_nnstring(` in layout, geom and render, and they
     re-parse numbers on every call.
   - GV-BUG-010 ([ledger](../c-bugs-ledger.md)) is "not applicable" precisely because there is
     no interner. A per-graph interner keeps that property: no global, no shared refcount, no
     thread-safety hazard.
4. **Parser.**
   - `Parser.tokens: Vec<Token>` keeps **every token of the input** for the whole parse, each
     with an owned `text: String` (`parser.rs:80`, `lexer.rs:75`). The grammar needs one token
     of lookahead plus `yytext` for error messages.
   - `Atom { text: String }`, `attr_items: Vec<(String, String, bool)>`, ports as
     `Option<(String, bool)>`, and 33 `.clone()`s in `parser.rs`.
   - The result is 1.5 allocations per input byte, and 12× C's memory.
5. **dot's working state** (`layout/src/dotgen/types.rs`).
   - `NodeInfo` mirrors `Agnodeinfo_t` as one 832-byte struct. It has 10 `Vec<E>` fields
     (`in_`, `out`, `flat_in`, `flat_out`, `other`, `tree_in`, `tree_out`, `cg_in`, `cg_out`,
     `rec_fields`), two `Option<Vec<E>>`, about 15 `Option<usize>` (16 B each) and a `String`
     name.
   - Hot fields of network simplex (`low`, `lim`, `par`, `tree_in`, `tree_out`) share cache
     lines with cold ones.
   - `GraphInfo` holds `nodes: BTreeSet<N>` and `edges: BTreeSet<E>` per (sub)graph, and
     `AdjMatrix` stores a `Vec<bool>` (1 byte per bit).
   - The ids are `type N = usize; type E = usize; type G = usize; type L = usize`, untyped
     aliases that are easy to mix up. core uses `NodeId(u32)` and `EdgeId(u32)` newtypes.
6. **C sentinels instead of `Option`.** In layout: 48 `usize::MAX`, 39 `i32::MAX`, 24 `f64::MAX`
   and 22 `== -1`. Many are faithful to C's `-1` returns and flags.
7. **Index loops.** 516 `for i in 0..` loops in layout (26 in geom). Together with the
   panic-site counts (7,568 index sites) this is where bounds checks live. Example:
   `right_mult_with_vector_ff` (`neatogen/matrix_ops.rs:39`) indexes `packed`, `vector` and
   `result` in its inner loop.
8. **Small temporaries.** 434 `vec![` and 221 `Vec::new()` in layout. The sfdp run makes 4 M
   allocations, 85% of them the quadtree cells' `Vec<f64>` fields in `QuadTree::add_internal`
   (the baseline's malloc uprobe).
9. **Hash maps.** `HashMap` (std SipHash) appears 22 times in layout, `HashSet` 7, `BTreeMap` 9
   and `BTreeSet` 22. None shows up in a profile. Swapping the hasher is a small, low-value
   change, and an *ordering* risk wherever a map is iterated.
10. **No `Box<dyn>` in hot paths** (0 sites). Generics are used, for example
    `dfs_range_impl<DotNs>`. Nothing to do here.
11. **Recursion.** The deep recursions that crashed already run on explicit stacks
    ([crashes.md](../porting/crashes.md): `locate_endpoint`, `traverse_polygon`, `DFS_visit`,
    the bison depth count). Recursion is not a performance item.
12. **String building in renderers.** Output accumulates in a `String` (`svg_engine.rs:256`,
    `agwrite.rs:17`, `json.rs:242`). Numbers are formatted through `String`-returning helpers
    (`emit/num.rs`: `gvprintnum`, `fmt_g`, `c_fixed`, `trim_zeros`), plus 71 `format!` in render,
    each a temporary allocation. The facade returns `Result<String>`, so output is materialised
    whole before it is written. Render was under 1% of the b29 profile. Measure it on
    render-heavy inputs with the baseline harness before ranking it higher.
13. **Process-wide C state.**
    - 18 files use `thread_local!` for C globals and statics: `drand48` and `rand` state
      (`cutil.rs:130`), `choose_node`'s per-process counter (`neatogen/stuff.rs:320`),
      `fdp_parms`, `PSinputscale`, and others.
    - This state is faithful, and it is **the main obstacle to parallelism**. Components and
      graphs in one file draw from one RNG stream in C's order.
14. **Public API.** Every module of layout is `pub` (113 modules; #512's draft counted 716 public
    items in layout, and `Graph` has 18 public fields of its own, which reach 128 public fields
    of core structs). Today any field change is a semver break.

### API stability is what frees the internals

The plan depends on #512's **D1**:

- Only the facade (`graphviz-pure`) and `graphviz-pure-raster` are stable.
- core, parser, geom, layout and render are implementation crates, pinned with `=`.
- `Graph` becomes opaque, with no public fields and no accessors (#520).

Without that, the data-model refactor (Phase 2) is a major version bump. Before 1.0, three things
have to sit behind the public API:

1. `Graph`'s fields, so the attribute and adjacency representation can change.
2. The layout entry points, which take `&mut Graph` today. Results come out only through rendering
   (`-Tjson` for structure), so `NodeData` and `EdgeData` can split into hot and cold parts.
3. Output. `render_*` returns `String` today. A later `render_to(&mut impl io::Write)` is
   additive, but the `String` functions must not promise how they are implemented.

## Technique catalogue

**Fidelity:** none / ordering / float-order, as [defined above](#working-assumption-the-bytes-dont-move).
**Effort:** S (days), M (1–3 weeks), L (more).

| # | technique | where in this code | expected gain | fidelity | effort | unsafe / deps | WASM |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| T1 | **Inline FMA on x86-64**: release binaries built with `-C target-cpu=x86-64-v3`, with a `v1` compatibility build | every `mul_add` (389 sites) | **Measured: neato 4.5–5.0×, sfdp 2.3×, fdp 2.0–2.1×, spline routing 2.1–2.8×**, bytes identical (full fidelity ratchet; 2,603 SVGs in #518); dot 1.0–1.1× | none (FMA is exactly rounded, so fused in hardware and in software give the same bits) | S | none | n/a (see T1b) |
| T1b | Faster software FMA for wasm32 | `core::math` | Unknown; WASM pays T1's cost permanently (#518: under node, fdp is 4.9× and sfdp 3.5× slower than native) | none if exact (must equal IEEE `fma` bit for bit) | M | none | **the** WASM perf item |
| T2 | **`pow` fast paths for exact results**: `y == 2.0` gives `x*x`, `y == 1.0` gives `x`; and one `pow` per pair instead of per dimension | `math::portable::pow`; `spring_electrical.rs:471,513,526` | sfdp: `pow` is 38–44% of samples; most of it goes | none (CORE-MATH `pow` is correctly rounded, and so is `x*x`) | S | none | same gain |
| T3 | **Streaming tokens**: lookahead buffer instead of `Vec<Token>`; token text as `&str` or `Cow` into the input | parser `lexer.rs`, `parser.rs` | Large part of the 12× RSS; fewer allocations | none (error messages keep `yytext`) | M | none | less memory |
| T4 | **Attribute interning and typed attributes**: per-graph symbol table; values as `AttrId → Sym` | core `AttrMap`, `AttrDicts`, all `agget`/`late_*` | Memory (strings shared, as cgraph's refstr does); lookups by id; numbers parsed once | ordering (writers that walk attributes by name must keep sorted-by-name order) | L | none, or `rustc-hash` for the interner map | less memory, smaller heap |
| T5 | **Typed index newtypes** (`NodeIx`, `EdgeIx`, `GraphIx`, `LabelIx`), `u32`, with an `IndexVec<I, T>` | dotgen `N`, `E`, `G`, `L` aliases; core ids | Clarity; `Option<NodeIx>` can be 4 B with a `NonMaxU32`-style niche (safe, via `NonZeroU32` of `ix+1`) | none | M, mechanical | none (write our own ~100 lines; `index_vec` or `typed-index-collections` are optional) | smaller structs |
| T6 | **Hot/cold split and struct-of-arrays** for dot's `NodeInfo` and `EdgeInfo` | `dotgen/types.rs`, network simplex, mincross | Cache misses in NS (56% of dot b29) and mincross (20%); 832 B per node today | none | L | none | — |
| T7 | **Compact adjacency**: CSR or `SmallVec`-like inline lists for `in_`/`out`/`flat_*`/`other`/`tree_*` | dotgen, neato `VtxData`, fdp | Fewer allocations (10 `Vec`s per node) | ordering (edge order within a list must stay C's insertion order) | L | none, or `smallvec` (contains unsafe) / `tinyvec` (safe) | smaller |
| T8 | **Bit-identical kernel restructuring**: row-blocking independent reductions; splitting a loop's serial chain from its independent scatter; slices in place of indexing | `right_mult_with_vector_ff` (88% of neato with v3), `vectors_inner_product*`, sfdp force loops | Estimated 2–4× on the neato kernel: the per-row FMA chain is latency-bound, and interleaving k rows gives k independent chains | none if every output's op sequence is unchanged (provable per kernel) | M | none | same gain |
| T9 | **Allocation reuse**: scratch buffers passed in or kept on the engine struct; `clear()` instead of a new `Vec` | sfdp quadtree cells (`add_internal`, 85% of its 4 M allocations; `[f64; 2]` inline) and supernodes, circo `count_all_crossings` (98% of circo's), twopi `set_n_steps_to_leaf`, pathplan `pt_vis` | 5–10% where malloc and free show up (sfdp: 5%) | none | S–M each | none (no `bumpalo` needed) | less memory churn |
| T10 | **Network-simplex engineering**: SoA fields, an iterative `dfs_range` that touches only the changed subtree (already C's approach in 16.x; verify), and no bounds checks in `enter_edge` | `common/ns.rs` | dot: NS is 56% of b29 | none if the same entering and leaving edges are chosen (ties!) | M | none | same gain |
| T11 | **Order-preserving parallelism**: per-node force evaluation where every node reads a frozen tree; column-gather mat-vec | sfdp quadtree repulsion (27% with v3), neato stress mat-vec | ≈ cores × on those kernels (native only) | ordering: safe only where each output's op sequence and RNG use are unchanged. Must confirm per kernel (shared counters, in-place updates). | M | `rayon` | none: sequential fallback |
| T12 | **Parallel independent work**: several `-T` outputs of one layout; PNG/JPEG encoding; graphs in one file (blocked: shared RNG and counters) | CLI, raster | Proportional to the number of outputs | ordering: none for renders of one finished layout; blocked for layouts | S–M | `rayon` or `std::thread::scope` (no dependency) | n/a |
| T13 | **Writers into one buffer**: number formatting into `&mut String` (`ryu`-free, keeping C's `%g`/`%.2f` rules), no per-number `String`; `render_to(io::Write)` streaming API | `render/src/emit/num.rs`, `svg_engine.rs`, `agwrite.rs`, `json.rs` | Fewer allocations; memory for large outputs; measure first | none (same digits) | M | none | lower peak memory |
| T14 | **Build profile**: PGO for release binaries; `panic = "abort"` and `strip` for the CLI | release workflow, `[profile.release]` | PGO 10%+ (Perf Book); abort: smaller binary, less unwind code | none (codegen only) | S–M | `cargo-pgo` in CI only | `opt-level=z` stays; PGO not for WASM |
| T15 | **Faster deterministic hasher** (FxHash / `rustc-hash`, or foldhash with a fixed seed) | 29 `HashMap`/`HashSet` in layout, `node_lookup` if it becomes a hash map | Small; not in any profile | ordering if iterated; none if lookup-only | S | `rustc-hash` (MIT/Apache) or `foldhash` (Zlib) | — |
| T16 | **Alternative allocator for the CLI** (mimalloc, jemalloc) | `graphviz-pure-cli` | Unknown here; malloc+free is under 1% in dot and about 5% in sfdp | none | S | **C code**: contradicts "pure Rust". Reject by default. | n/a |
| T17 | **Sentinels to `Option` and enums** (`u8` node types to `enum NodeType`) | dotgen constants, 130+ sentinel sites | Clarity; sometimes smaller with niches | none | M, mechanical | none | — |
| T18 | **Incremental and streaming APIs**: parse to an iterator of graphs (exists: `GraphStream`); render to a writer; re-layout after small edits | facade | Memory for large inputs; editor use cases | none | L | none | high value for the browser |

### High-value items

#### T1: inline FMA on x86-64 (measured; do first)

The port reproduces clang's contraction with explicit `mul_add`. On baseline x86-64, each one is a
call into compiler-builtins' runtime-dispatched `fma`. Building with `-C target-cpu=x86-64-v3`
(AVX2, FMA, BMI1/2; the [x86-64 psABI levels](https://gitlab.com/x86-psABIs/x86-64-ABI)) makes
each one a `vfmadd` instruction. Results:

- neato rnd2000: 76.99 s → 15.54 s.
- sfdp rnd2000: 1.82 s → 0.81 s.
- **Both outputs are byte-identical.** IEEE-754 fused multiply-add has one correct result.

Building with `+fma` alone, or with `x86-64-v2` (which adds SSE4.1 `roundsd`, removing `pow`'s
`rint` calls), gives part of this.

Caveats:

- **Binaries built this way don't run on x86 CPUs without AVX2/FMA** (before about 2013 Intel and
  2015 AMD), so a `v1` artifact stays available ([decision 6](#decisions)).
- `-C target-cpu` is a compiler flag, not a profile setting. `cargo install graphviz-pure-cli`
  users get v1 unless they set `RUSTFLAGS`, so the README should say so.
- Runtime dispatch inside one binary (`#[target_feature(enable = "fma")]` plus
  `is_x86_feature_detected!`) needs an `unsafe` call, so it is excluded by
  [decision 3](#decisions) unless that changes.
- Check the result with the full fidelity ratchet built with the same flags, and add a
  cross-platform job that compares a v3 build's bytes with v1's.

WASM: wasm32 has no deterministic scalar FMA. The relaxed-SIMD `relaxed_madd` is explicitly
allowed to be fused or not, so it is ruled out. Hence T1b: an exact software FMA tuned for the
operand ranges the port sees. The fast path covers products whose exponent sum keeps the error
term representable, and falls back to the full algorithm. It needs a proof or an exhaustive-style
differential test against `libm::fma`.

#### T2: `pow` fast paths

`pow(x, 2.0)` correctly rounded is `round(x·x)`, which is exactly what IEEE `x * x` computes,
including overflow, underflow, NaN and sign. `pow(x, 1.0)` is `x`. The C reference runs this same
`pow` ([libm.md](../porting/libm.md)), so the shortcut matches C by construction. Other
candidates need case analysis before use:

- `y == -1.0` gives `1.0 / x`.
- `y == 0.5` gives `sqrt(x)`, except at `-0.0` and `-inf`.
- `y == -2.0` is **not** exact: `1/(x·x)` rounds twice.

The PR carries a randomized and edge-case differential test asserting bit equality with the
general path. sfdp's power-law case (`p = -1.8`, so `y = 2.8`) gets no shortcut; T8, T9 and T11
apply there.

#### T3 and T4: the parser and the attribute model (the memory goal)

Parsing a 12 MB file takes 982 MB where C takes 82 MB. Two causes compound:

1. **Tokens.** Every token stays alive with an owned `String`. Fix: keep a small lookahead ring
   and the last `yytext`. Lexeme text becomes `&'a str` into the input, or a `Cow` where escapes
   or `+` concatenation build a new string.
2. **Attributes.** Every object owns every key and value. cgraph instead keeps, per object, an
   array of value pointers indexed by the attribute's `sym->id`, into a refcounted string
   dictionary.

The idiomatic equivalent, with no global and no refcount (GV-BUG-010 stays "not applicable"):

- A per-`Graph` `Interner` (`Vec<Box<str>>` plus a map from string to `Sym(u32)`), and per object
  a sparse `SmallVec<[(AttrId, Sym); N]>`, or a dense `Vec<Sym>` by `AttrId`, as cgraph does.
  HTML-ness becomes a bit on the value.
- **Typed attributes parsed once.** `late_double(agget(n, "width"), …)` re-parses on every call.
  Phase 2 adds a per-graph cache of the typed values the layouts read, in the order C reads them.
  Warnings that C prints on parse (for example "Illegal value") must still come once, in C's
  order. That is the risk to manage.
- Writers (`agwrite`, canon, json) iterate attributes in dictionary or name order. That order is
  output, so the new representation must expose the same iteration (an *ordering* item).

#### T6, T7 and T10: dot's working state and network simplex

dot is already faster than stock C on the measured inputs (b29: 0.9 s against 1.6 s; rnd2000:
381 s against 791 s). The work is constant-factor:

- Split `NodeInfo` into the network-simplex hot set (`low`, `lim`, `par`, `rank`, the tree edge
  lists), the mincross hot set (`order`, `mval`, `rank`, the in/out lists) and cold data (labels,
  shapes, names).
- Store each set as its own `IndexVec`.
- Replace the 10 per-node `Vec`s with CSR-like arrays, where the lists are rebuilt per phase, or
  with inline small vectors.
- Edge-list order is observable (C walks `ND_out(n).list` in insertion order and ties break on
  it), so each conversion keeps insertion order and is classed *ordering*.

#### T8: bit-identical kernel restructuring

`right_mult_with_vector_ff` takes 88% of neato with v3. Its inner loop does two things:

- A serial reduction `res = a·v[j] + res`. This is latency-bound: one FMA every ~4 cycles.
- An independent scatter `result[j] = a·v_i + result[j]`.

The order of each scalar's operations can be kept while changing *what runs next to what*:

- Process rows `i` and `i+1` (or 4 rows) together. There are then k independent chains, and each
  `result[j]` still receives row `i`'s term before row `i+1`'s.
- Split the scatter into its own loop, so LLVM can vectorize it. Each lane is a different `j`, so
  no sum is reordered.
- Iterate slices (`zip`) so bounds checks vanish. This also lowers the panic-site count.

The same pattern applies to the sfdp force loops and the inner products. The test for every such
change: each output float is produced by the same sequence of IEEE operations on the same
operands.

#### T11 and T12: parallelism that cannot change bytes

Parallelism is safe only where **each output's computation is unchanged and independent**:

- **sfdp with the quadtree:** the repulsive force on each node is computed against the tree built
  at the start of the iteration. If positions are updated only after all forces are computed
  (check `spring_electrical.rs` for in-place updates and shared statistics counters), each node's
  force can be computed in parallel, each in its own unchanged order.
- **neato mat-vec:** with a column-gather formulation, `result[j]` is the same left-to-right sum
  over `i`, computed by one thread.
- **Several `-T` outputs of one layout, and raster encoding:** independent by construction.
- **Not safe without more work:** laying out pack components or several graphs concurrently.
  They share `drand48`, `rand` and per-process counters (`thread_local!` in 18 files) in C's
  order. A component's RNG start state is known only after the previous component finishes.

#### T14: build configuration

Completed by #546 (#540): the release profile is in the workspace `Cargo.toml` and repeated
in the published CLI manifest, with CLI symbols stripped. The evaluation retained thin LTO and
unwinding. The original options considered were:

- **PGO** for release binaries via `cargo-pgo` with the corpus as the training set. The Perf Book
  reports 10%+; it is unavailable to `cargo install` users.
- **Fat LTO** for the CLI: measure; WASM already uses it.
- **`panic = "abort"` for the CLI.** The CLI uses no `catch_unwind`. It changes the exit status
  on a panic (SIGABRT instead of 101), and the exit status is covered by semver
  ([versioning.md](../versioning.md)). Panics are bugs, but say so in the changelog.
- **`strip = true`.** The 0.2.2 CLI is 13.4 MB unstripped.

`-C target-cpu=native` is never used for published artifacts.

## The hill-climbing process

Every step is one PR that is proved by three ratchets:

1. **Fidelity ratchet** (exists): no file that matched C stops matching. This is the hard gate.
2. **Panic-site ratchet** (exists): no module gains a site. Refactors are expected to lower
   counts, and the PR runs `scripts/panic-sites.sh --update` to lock them in.
3. **Perf ratchet** (implemented by #547, built on #518's harness; see [the current guide](../perf/ratchet.md)):
   - **Allocation counts**, through `dhat` as a dev-only dependency. They do not vary at all
     between runs (#518), so this gate is exact: any increase fails. They are blind to CPU work,
     though: the FMA libcalls, `pow` and network simplex would regress without a single new
     allocation.
   - **Instruction counts** for CPU work, which are deterministic to well under 1% and
     comparable across runs on a noisy shared runner. rustc's own perf CI uses instruction
     counts as its primary metric for the same reason (rustc-perf).
   - On GitHub-hosted runners, hardware counters (`perf stat -e instructions:u`) are generally
     not reliably available, so use **Callgrind** through
     [gungraun](https://docs.rs/gungraun) (formerly iai-callgrind). It runs each benchmark once,
     under simulation.
   - **Also:** peak heap and total bytes allocated, and WASM size.
   - **Wall time** on a quiet self-hosted machine, nightly, for trend only, never as a gate.
   - **Gate:** fail when any allocation count rises, any benchmark's instruction count rises more
     than 2%, or its peak memory more than 5%, unless the PR carries a `perf-regression-accepted` label and a justification.
     Improvements are reported in the PR comment, and the baseline file is updated on merge
     (like `scripts/update-baselines.sh`).
   - **Benchmark set:** #518's `scripts/bench/sets/quick.tsv`, plus the shipped musl artifact, which should include, per engine, one small input
     (start-up cost), one realistic corpus file (b29-like) and one large synthetic input
     (rnd2000, big, chain), plus parse-only and render-only cases.

**Benchmark-driven PR template**, appended to the existing one:

```markdown
## Performance
Technique: T<n> (docs/roadmap/post-1.0.md). Fidelity class: none | ordering (argument: …)
| benchmark | instructions before | after | Δ | peak heap before | after |
Profile evidence (before): <top symbols>
## Changelog entry
Not user-visible. (or: ### Changed – faster/smaller …)
```

**Rules of thumb:**

- One technique per PR.
- A refactor that is neutral for performance is fine if it lowers panic sites or unlocks a later
  step. Say which step.
- Never mix a fidelity fix with a refactor. A ratchet diff must have one explanation.

## Phased plan

Order matters: the API boundary and the perf ratchet come first, because everything later is
proved by them. Quick wins come next, because they are measured, need no structural change, and
fund the effort. The data model comes before algorithmic work, because the hot loops' data layout
depends on it.

**Phase 0: prerequisites (can start before 1.0).**

- #512 D1 lands: internal crates carry no semver promise, and `Graph` is opaque.
- The sibling harness lands (`scripts/bench/`, `docs/perf/baseline.md`), and the perf ratchet is
  added to CI.
- The [follow-the-C replacement](#decisions) is decided and written into
  CONTRIBUTING.md.

**Phase 1: quick wins (fidelity: none).**

- T1: x86-64-v3 release artifacts plus a v1 compatibility build; a ratchet job built with v3.
- T2: `pow` fast paths (completed by #543).
- T14: build-profile evaluation and CLI stripping (completed by #546).
- T9: allocation reuse in the sfdp quadtree (completed by #548).
- T8: neato mat-vec row-blocking and slice iteration.
- T3: streaming tokens in the parser.

Expected result: neato and sfdp at or below C on x86 (T1 alone leaves neato about 7% and sfdp about 18% slower than C), and parser memory reduced by a large factor.

**Phase 2: the data model (fidelity: none or ordering).**

- T5: typed indices.
- T4: interned attributes, with typed attributes parsed once.
- T6 and T7: dot's hot/cold split and compact adjacency.
- T17: sentinels to `Option` and enums.
- Make `Graph`'s adjacency explicit (CSR built once after parsing, shared by the engines).

Each module moves on its own, behind the opaque `Graph`.

**Phase 3: algorithmic and structural (fidelity: none or ordering).**

- T10: network simplex.
- Mincross caching.
- Pathplan triangulation (`isdiagonal`, 4%).
- T11: order-preserving parallel kernels behind a `parallel` feature, native only.
- T12: parallel multi-output.
- T13 and T18: writer streaming and the `render_to` API.
- T1b: WASM FMA.

**Phase 4: only if [decision 1](#decisions) allows.** An opt-in "fast" mode with
reassociated or SIMD reductions, kept out of every default and every ratchet.

**Unlock graph:**

- D1 unlocks T4, T6 and T7.
- T5 makes T6 and T7 mechanical.
- T4 unlocks typed attributes and lower memory.
- T6 unlocks T10 and the mincross work.
- T8 makes T11 straightforward (the column-gather form).
- The perf ratchet gates everything.

## Decisions

Made by the maintainer on 2026-10-27, after the 1.0 decisions in #512.

| # | Decision | Rationale |
| :--- | :--- | :--- |
| 1 | **Strict byte-identity now** (option (a)). An opt-in fast mode (b) is reconsidered after Phase 1, once T1, T2 and T8 are measured. The default output stays C's. | The ratchet is what makes aggressive refactoring provably safe, and bit-identical wins already bring neato and sfdp near C. |
| 2 | **Tiered** (option (c)). Every item carries a provenance tag (`/// C: lib/dotgen/position.c:make_lrvn`); structure is free whenever the ratchets pass; a C-mirror list stays function-for-function: `core::math`, `cdt.rs`, the network-simplex and mincross tie-breaking cores, and `pathplan`. Written into CONTRIBUTING.md; the tag-coverage script is #539. | Freedom where it pays, and cheap upstream ports where order is the product. |
| 3 | **`forbid(unsafe_code)` stays in every own crate** (option (a)). Dependencies may contain `unsafe`. | Speed comes from build flags and safe idioms; nothing planned needs `unsafe`. |
| 4 | **Small pure-Rust dependencies, each with a measured gain over std; no C dependencies; CI-only tools are fine.** svg2pdf is allowed behind the `pdf` feature (#528). Prefer `tinyvec` to `smallvec`. The slow musl allocator of the Linux artifacts is addressed by removing allocations in safe Rust (T9, T3, T7); the allocator is revisited only if a gap remains (#530 tracks the musl artifact). | The allocator is C in every build; fewer allocations help every platform, wasm included. |
| 5 | **rayon behind a `parallel` feature**, on for the native CLI, off for the library and wasm, used only for order-preserving work (T11, T12). The fidelity ratchet runs with 1 and N threads. | Threaded wasm needs nightly and isolated pages; revisit when stable. |
| 6 | **x86-64 artifacts are built with `x86-64-v3`, plus a `-compat` v1 archive; PyPI wheels ship both binaries behind a safe launcher** (`is_x86_feature_detected!` and exec). A CI job proves v1 and v3 bytes equal (#531). | Measured 4.5× on neato with identical bytes; the launcher keeps old CPUs working without `unsafe`. |
| 7 | **MSRV: #512's D4** (at most current stable minus two releases). Consistent: nothing here needs a newer compiler. | — |

Effects of the 1.0 decisions on this plan: `Graph` has no accessors (D1), so even its read API
is free; serde leaves the graph types (D3), which unblocks T4; diagnostics are returned as
values (D14), which removes one process-wide global in the way of T11 and T12; `ps` and `pdf`
come in 1.x (D6, #528).

## Draft issues

Filed on 2026-10-27 in the `post-1.0` milestone, in this order, as #530–#540 (1 is #530, 11 is
#540). Each is one PR unless noted.

1. **Completed (#547): perf ratchet in CI.** Exact allocation counts (`dhat`), Callgrind instruction
   counts and peak heap on #518's quick set; exact, 2% and 5% gates; a `perf-regression-accepted`
   label. This depends on `scripts/bench/`.
2. **release: build x86-64 artifacts with `-C target-cpu=x86-64-v3`, plus a v1 compat artifact.**
   Add a ratchet job built with v3, and a byte-equality check between v1 and v3 (T1; measured
   4.5–5.0× neato, 2.3× sfdp, 2× fdp).
3. **Completed (#543): math: bit-exact fast paths in `pow` for `y == 2` and `y == 1`.** Include a differential test
   against the general path, and call `pow` once per pair (T2; sfdp `pow` is 38–44% of samples).
4. **neatogen: restructure `right_mult_with_vector_ff` without reordering any sum.** Row-blocking,
   a split scatter loop, slice iteration (T8; 88% of neato).
5. **Completed (#548): sfdp: inline the quadtree cells' vectors and reuse supernode buffers** (T9; 4 M allocations
   per rnd2000 run, 85% in `add_internal`).
6. **parser: stop retaining every token; borrow token text from the input** (T3; 982 MB against
   C's 82 MB on a 12 MB file).
7. **core: design note and prototype for interned, id-indexed attributes** (T4). Covers how
   writers keep their iteration order and how typed values are cached without moving warnings.
8. **layout: typed index newtypes for dot's `N`, `E`, `G` and `L`** (T5; mechanical, unlocks T6
   and T7).
9. **dotgen: split `NodeInfo` (832 B) into network-simplex, mincross and cold parts** (T6;
   profile-driven, NS first).
10. **docs: replace "Follow the C" in CONTRIBUTING.md with provenance tags and a C-mirror module
    list** plus a script that reports C functions without a provenance tag
    (completed: the CONTRIBUTING.md text landed with #514, the script with #542 for #539).
11. **Completed (#546): release profile and CLI size evaluation** (T14, #540). The profile
    moved into `Cargo.toml` and the published CLI manifest, and CLI stripping is enabled.
    Thin LTO and unwinding were retained after evaluating PGO, fat LTO and `panic=abort`.
12. **Completed (#566): render `-Tplain` scaling** (#541). At 0.2.2, `root_holds_plain`
    scanned the graph for every label (baseline hotspot 2). The per-output string set fixes
    the quadratic scan without changing floating-point operations. This landed before 1.0.

The baseline's other hotspots without an issue yet are candidates for the next round, each
classed *none*: circo's `count_all_crossings` (hotspot 7), twopi's `set_n_steps_to_leaf` (11),
`fmt_f` writing into the output buffer (13, part of T13), and osage formatting its trace
messages when `-v` is off (14).

## Sources

- The Rust Performance Book, *Build Configuration* (codegen units, LTO, PGO "10% or more",
  alternative allocators, `target-cpu=native` caveats), *Hashing*, *Heap Allocations* and
  *Bounds Checks*: <https://nnethercote.github.io/perf-book/>
- The rustc book, *Codegen options* (`target-cpu`, `target-feature`) and *Profile-guided
  Optimization*: <https://doc.rust-lang.org/rustc/codegen-options/index.html>,
  <https://doc.rust-lang.org/rustc/profile-guided-optimization.html>
- The Cargo book, *Profiles* (`lto`, `codegen-units`, `panic`, `strip`):
  <https://doc.rust-lang.org/cargo/reference/profiles.html>
- LLVM Language Reference, `llvm.fma.*` (a fused multiply-add rounded once) and `llvm.fmuladd.*`:
  <https://llvm.org/docs/LangRef.html#llvm-fma-intrinsic>
- x86-64 psABI, microarchitecture levels (x86-64-v2, v3, v4): <https://gitlab.com/x86-psABIs/x86-64-ABI>
- WebAssembly relaxed SIMD (`relaxed_madd` is implementation-defined fused or unfused):
  <https://github.com/WebAssembly/relaxed-simd>
- foldhash (speed against FxHash and aHash; no stable output across versions or platforms; Zlib):
  <https://github.com/orlp/foldhash>; rustc-hash: <https://github.com/rust-lang/rustc-hash>
- tinyvec (no `unsafe`): <https://docs.rs/tinyvec>; smallvec: <https://docs.rs/smallvec>;
  bumpalo: <https://docs.rs/bumpalo>
- rayon: <https://docs.rs/rayon>; wasm-bindgen-rayon (nightly, `+atomics,+bulk-memory`,
  COOP/COEP): <https://github.com/RReverser/wasm-bindgen-rayon>
- gungraun, formerly iai-callgrind (Callgrind instruction counts, DHAT):
  <https://docs.rs/gungraun>; rustc-perf (instruction counts as the primary metric):
  <https://github.com/rust-lang/rustc-perf>
- cargo-pgo: <https://github.com/Kobzol/cargo-pgo>
- CORE-MATH (correctly rounded `pow`, which T2 relies on): <https://core-math.gitlabpages.inria.fr/>
- In this repository: [libm.md](../porting/libm.md), [crashes.md](../porting/crashes.md),
  [versioning.md](../versioning.md), [c-bugs-ledger.md](../c-bugs-ledger.md) (GV-BUG-010), and
  #512 (the 1.0 roadmap, D1 and D4).

### Reproducing the measurements

```sh
cargo build --release -p graphviz-pure-cli
RUSTFLAGS="-C target-cpu=x86-64-v3" CARGO_TARGET_DIR=/tmp/v3 cargo build --release -p graphviz-pure-cli
sed '/^layout=/d' tests/sfdp/gen_rnd2000.gv > /tmp/rnd2000.gv
/usr/bin/time -f '%es %MKB' target/release/graphviz-pure -Kneato -Tsvg /tmp/rnd2000.gv -o a.svg
/usr/bin/time -f '%es %MKB' /tmp/v3/release/graphviz-pure -Kneato -Tsvg /tmp/rnd2000.gv -o b.svg
cmp a.svg b.svg
perf record -F 2000 target/release/graphviz-pure -Ksfdp -Tsvg /tmp/rnd2000.gv -o /dev/null
perf report --no-children --stdio
```
