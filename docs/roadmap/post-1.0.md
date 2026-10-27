# After 1.0: faster, smaller, idiomatic (a plan for hill-climbing)

Status: **proposal**, not approved. Nothing here is filed as issues yet; the draft list is at the
end. It assumes 1.0 as the 1.0 roadmap defines it (#512, in review; it adds
`docs/roadmap/1.0.md`). The benchmark harness and the measured baseline are a sibling effort
(`docs/perf/baseline.md` and `scripts/bench/`, not on `master` yet). This document reuses them
and doesn't define a second harness.

Up to 1.0 the port is a line-by-line translation of Graphviz 16.1.0, held byte-identical to C by
the fidelity ratchets. After 1.0 the project hill-climbs. Each step makes the code faster, smaller
in memory, or more idiomatic Rust, and each step keeps the bytes.

## Contents

1. [Working assumption](#working-assumption-the-bytes-dont-move)
2. [Goals and success metrics](#goals-and-success-metrics)
3. [Constraints](#constraints)
4. [What the code does today: audit and measurements](#what-the-code-does-today)
5. [Technique catalogue](#technique-catalogue) (table, then a section per high-value item)
6. [The hill-climbing process](#the-hill-climbing-process)
7. [Phased plan](#phased-plan)
8. [Open decisions](#open-decisions)
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
| **float-order** | Changes the order or fusion of floating-point operations: reassociated sums, SIMD reductions, multiple accumulators, fast-math, parallel reductions. | **Not allowed under the current policy.** These are listed separately under [Open decision 1](#1-bit-identity-policy). |

A technique classed *none* or *ordering* still has to pass every ratchet. The class is about how
much reasoning the review needs, not a waiver.

## Goals and success metrics

All metrics are measured with the sibling harness (`scripts/bench/`). The numbers in this
document are one-off measurements from this research, listed under
[measurements](#measurements).

| goal | metric | target for the post-1.0 effort |
| :--- | :--- | :--- |
| Faster | Instruction count (primary) and wall time (secondary) per corpus input, per engine | No engine slower than stock C 16.1.0 on any baseline input. Geometric-mean 2× faster than 1.0 on the benchmark set. |
| Smaller | Peak RSS and bytes allocated per input. WASM `.wasm` size. | Parser and graph model within 2× of C's peak RSS (today 12×). WASM size no larger than 1.0. |
| Idiomatic | Panic-site counts ([the ratchet](../porting/crashes.md#the-panic-site-ratchet)); `clippy::pedantic` warnings; public items of the internal crates | Panic sites ↓ 50% in layout and geom. No C-style sentinels in new code. |
| Safe to change | The three ratchets (fidelity, panic, perf) on every PR | No regressions, enforced by CI. |

Non-goals: new layout algorithms, output that differs from C, or a C-compatible library ABI.

## Constraints

Each constraint is either kept, or proposed for change under [Open decisions](#open-decisions).

| constraint | today | post-1.0 proposal |
| :--- | :--- | :--- |
| Byte-identical output (fidelity ratchets) | Required | **Kept.** Relaxation is [Open decision 1](#1-bit-identity-policy). |
| `#![forbid(unsafe_code)]` workspace-wide (`Cargo.toml` `[workspace.lints.rust]`) | Required in own crates; dependencies (serde, libm, wasm-bindgen, resvg) contain `unsafe` | **Kept** ([Open decision 3](#3-unsafe-policy)). Nothing in the top-ranked techniques needs `unsafe`. |
| Panic-site ratchet (`scripts/panic-sites.sh`) | 7,570 index, 2,120 arithmetic, 256 unwrap and 42 expect sites in layout and geom | **Kept**, and it becomes a goal: iterator rewrites remove index sites. |
| Determinism across Linux, Windows, macOS and WASM, with one math library (`graphviz_pure_core::math`) | Required; checked by `cross-platform.yml` | **Kept.** CPU-specific codegen is allowed only where IEEE-754 gives the same bits (FMA, `roundsd`), never a different algorithm per target. |
| WASM is first-class: size matters, no threads by default | `opt-level=z`, fat LTO, `wasm-opt -Oz` | **Kept.** Parallelism is a native-only feature ([Open decision 5](#5-threads-and-wasm)). |
| MSRV (`rust-version = "1.85"`) | Edition 2024; let-chains used in 33 files (#512's count) | Follow #512's D4. Nothing here needs a newer compiler. |
| Minimal dependencies under `cargo deny` | Small tree; raster pulls in resvg | Each new dependency is justified with a measurement ([Open decision 4](#4-dependency-appetite)). |
| CONTRIBUTING.md: "Follow the C … keep the structure recognisable" | Required | **Conflicts directly** with idiomatic refactoring. A replacement is proposed in [Open decision 2](#2-replacing-follow-the-c). |
| Public API from 1.0 | #512 D1 recommends (b): only the facade and raster are semver-stable | **A prerequisite for this plan.** See [below](#api-stability-is-what-frees-the-internals). |

## What the code does today

### Measurements

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
| rnd2000 | neato `-Tsvg` | **76.99 s** | **15.54 s** (bytes identical) | 14.44 s |
| rnd2000 | sfdp `-Tsvg` | 1.82 s | **0.81 s** (bytes identical) | 0.67 s |
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

1. **Out-of-line FMA.** The port calls `f64::mul_add` and `f32::mul_add` (389 sites, 44 files) to
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
   panic-site counts (7,570 index sites) this is where bounds checks live. Example:
   `right_mult_with_vector_ff` (`neatogen/matrix_ops.rs:39`) indexes `packed`, `vector` and
   `result` in its inner loop.
8. **Small temporaries.** 434 `vec![` and 221 `Vec::new()` in layout. The sfdp run makes 4 M
   allocations, mostly per-node supernode lists in `QuadTree::get_supernodes_internal`.
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
    - 20 files use `thread_local!` for C globals and statics: `drand48` and `rand` state
      (`cutil.rs:130`), `choose_node`'s per-process counter (`neatogen/stuff.rs:320`),
      `fdp_parms`, `PSinputscale`, and others.
    - This state is faithful, and it is **the main obstacle to parallelism**. Components and
      graphs in one file draw from one RNG stream in C's order.
14. **Public API.** Every module of layout is `pub` (113 modules; #512 counts 716 public items in
    layout and 128 public fields on `Graph`). Today any field change is a semver break.

### API stability is what frees the internals

The plan depends on #512's **D1 (b)**:

- Only the facade (`graphviz-pure`) and `graphviz-pure-raster` are stable.
- core, parser, geom, layout and render are implementation crates, pinned with `=`.
- `Graph` becomes opaque, with accessors.

Without that, the data-model refactor (Phase 2) is a major version bump. Before 1.0, three things
have to sit behind the public API:

1. `Graph`'s fields, so the attribute and adjacency representation can change.
2. The layout entry points, which take `&mut Graph` today. Results should come through accessors,
   so `NodeData` and `EdgeData` can split into hot and cold parts.
3. Output. `render_*` returns `String` today. A later `render_to(&mut impl io::Write)` is
   additive, but the `String` functions must not promise how they are implemented.

## Technique catalogue

**Fidelity:** none / ordering / float-order, as [defined above](#working-assumption-the-bytes-dont-move).
**Effort:** S (days), M (1–3 weeks), L (more).

| # | technique | where in this code | expected gain | fidelity | effort | unsafe / deps | WASM |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| T1 | **Inline FMA on x86-64**: release binaries built with `-C target-cpu=x86-64-v3`, with a `v1` compatibility build | every `mul_add` (389 sites) | **Measured: neato 5.0×, sfdp 2.2×** on rnd2000, bytes identical; dot unchanged | none (FMA is exactly rounded, so fused in hardware and in software give the same bits) | S | none | n/a (see T1b) |
| T1b | Faster software FMA for wasm32 | `core::math` | Unknown; WASM pays T1's cost permanently | none if exact (must equal IEEE `fma` bit for bit) | M | none | **the** WASM perf item |
| T2 | **`pow` fast paths for exact results**: `y == 2.0` gives `x*x`, `y == 1.0` gives `x` | `math::portable::pow` | sfdp: `pow` is 38–42% of samples; most of it goes | none (CORE-MATH `pow` is correctly rounded, and so is `x*x`) | S | none | same gain |
| T3 | **Streaming tokens**: lookahead buffer instead of `Vec<Token>`; token text as `&str` or `Cow` into the input | parser `lexer.rs`, `parser.rs` | Large part of the 12× RSS; fewer allocations | none (error messages keep `yytext`) | M | none | less memory |
| T4 | **Attribute interning and typed attributes**: per-graph symbol table; values as `AttrId → Sym` | core `AttrMap`, `AttrDicts`, all `agget`/`late_*` | Memory (strings shared, as cgraph's refstr does); lookups by id; numbers parsed once | ordering (writers that walk attributes by name must keep sorted-by-name order) | L | none, or `rustc-hash` for the interner map | less memory, smaller heap |
| T5 | **Typed index newtypes** (`NodeIx`, `EdgeIx`, `GraphIx`, `LabelIx`), `u32`, with an `IndexVec<I, T>` | dotgen `N`, `E`, `G`, `L` aliases; core ids | Clarity; `Option<NodeIx>` can be 4 B with a `NonMaxU32`-style niche (safe, via `NonZeroU32` of `ix+1`) | none | M, mechanical | none (write our own ~100 lines; `index_vec` or `typed-index-collections` are optional) | smaller structs |
| T6 | **Hot/cold split and struct-of-arrays** for dot's `NodeInfo` and `EdgeInfo` | `dotgen/types.rs`, network simplex, mincross | Cache misses in NS (56% of dot b29) and mincross (20%); 832 B per node today | none | L | none | — |
| T7 | **Compact adjacency**: CSR or `SmallVec`-like inline lists for `in_`/`out`/`flat_*`/`other`/`tree_*` | dotgen, neato `VtxData`, fdp | Fewer allocations (10 `Vec`s per node) | ordering (edge order within a list must stay C's insertion order) | L | none, or `smallvec` (contains unsafe) / `tinyvec` (safe) | smaller |
| T8 | **Bit-identical kernel restructuring**: row-blocking independent reductions; splitting a loop's serial chain from its independent scatter; slices in place of indexing | `right_mult_with_vector_ff` (88% of neato with v3), `vectors_inner_product*`, sfdp force loops | Estimated 2–4× on the neato kernel: the per-row FMA chain is latency-bound, and interleaving k rows gives k independent chains | none if every output's op sequence is unchanged (provable per kernel) | M | none | same gain |
| T9 | **Allocation reuse**: scratch buffers passed in or kept on the engine struct; `clear()` instead of a new `Vec` | sfdp quadtree supernodes (4 M allocations), dot per-phase vectors, pathplan | 5–10% where malloc and free show up (sfdp: 5%) | none | S–M each | none (no `bumpalo` needed) | less memory churn |
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
  2015 AMD), so a `v1` artifact stays available ([Open decision 6](#6-x86-64-baseline-of-release-binaries)).
- `-C target-cpu` is a compiler flag, not a profile setting. `cargo install graphviz-pure-cli`
  users get v1 unless they set `RUSTFLAGS`, so the README should say so.
- Runtime dispatch inside one binary (`#[target_feature(enable = "fma")]` plus
  `is_x86_feature_detected!`) needs an `unsafe` call, so it is excluded by
  [Open decision 3](#3-unsafe-policy) unless that changes.
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
  They share `drand48`, `rand` and per-process counters (`thread_local!` in 20 files) in C's
  order. A component's RNG start state is known only after the previous component finishes.

#### T14: build configuration

The workspace already uses `opt-level=3`, thin LTO and `codegen-units=1` (`.cargo/config.toml`).
Remaining options:

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
3. **Perf ratchet** (new, built on the sibling harness):
   - **Primary metric: instruction counts**, which are deterministic to well under 1% and
     comparable across runs on a noisy shared runner. rustc's own perf CI uses instruction
     counts as its primary metric for the same reason (rustc-perf).
   - On GitHub-hosted runners, hardware counters (`perf stat -e instructions:u`) are generally
     not reliably available, so use **Callgrind** through
     [gungraun](https://docs.rs/gungraun) (formerly iai-callgrind). It runs each benchmark once,
     under simulation.
   - **Secondary metrics:** peak heap and total bytes allocated, through gungraun's DHAT mode or a
     counting allocator in the bench binary, and WASM size.
   - **Wall time** on a quiet self-hosted machine, nightly, for trend only, never as a gate.
   - **Gate:** fail when any benchmark's instruction count rises more than 2%, or its peak memory
     more than 5%, unless the PR carries a `perf-regression-accepted` label and a justification.
     Improvements are reported in the PR comment, and the baseline file is updated on merge
     (like `scripts/update-baselines.sh`).
   - **Benchmark set:** the sibling's set, which should include, per engine, one small input
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

- #512 D1 (b) lands: internal crates carry no semver promise, and `Graph` is opaque.
- The sibling harness lands (`scripts/bench/`, `docs/perf/baseline.md`), and the perf ratchet is
  added to CI.
- The [follow-the-C replacement](#2-replacing-follow-the-c) is decided and written into
  CONTRIBUTING.md.

**Phase 1: quick wins (fidelity: none).**

- T1: x86-64-v3 release artifacts plus a v1 compatibility build; a ratchet job built with v3.
- T2: `pow` fast paths.
- T14: PGO, CLI `strip`, and possibly `panic=abort`.
- T9: allocation reuse in the sfdp quadtree.
- T8: neato mat-vec row-blocking and slice iteration.
- T3: streaming tokens in the parser.

Expected result: neato and sfdp at or below C on x86, and parser memory reduced by a large factor.

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

**Phase 4: only if [Open decision 1](#1-bit-identity-policy) allows.** An opt-in "fast" mode with
reassociated or SIMD reductions, kept out of every default and every ratchet.

**Unlock graph:**

- D1 (b) unlocks T4, T6 and T7.
- T5 makes T6 and T7 mechanical.
- T4 unlocks typed attributes and lower memory.
- T6 unlocks T10 and the mincross work.
- T8 makes T11 straightforward (the column-gather form).
- The perf ratchet gates everything.

## Open decisions

### 1. Bit-identity policy

The ratchet is the project's safety net. Relaxing it gives up the ability to prove a refactor
correct mechanically.

Techniques that are possible **only** with relaxation, with estimated gains:

| technique | where | estimated gain |
| :--- | :--- | :--- |
| Multi-accumulator or SIMD reductions (AVX2: 8 `f32` lanes) | neato `right_mult_with_vector_ff` and inner products | Several× beyond T8 on neato's 88% kernel |
| Reassociated force sums, or `f32` where C uses `f64` | sfdp and fdp force loops | ~2× on those loops |
| Fast-math `pow`/`exp` approximations | sfdp `p ≠ -1` | Large; `pow` is 38% |
| Parallel reductions (the sum of a parallel map) | stress, sfdp energy | ≈ cores × |
| Algorithm substitutions (for example a faster network-simplex pivot rule) | dot | Large on big graphs, but changes layouts |

Options:

- **(a)** Strict forever.
- **(b) Strict by default, with an opt-in fast mode** (a facade option or CLI flag such as
  `-Gfast=true`, outside the ratchets, documented as "not identical to C"). It is tested for
  determinism (the same bytes across platforms *within* fast mode) and for layout sanity, not
  against C.
- **(c)** Relax the default.

**Recommendation: (a) now. Revisit (b) after Phase 1**, when T1, T2 and T8 have shown how much is
left. Measured today, the bit-identical wins already bring neato and sfdp to near C. (c) is not
recommended: identity with C is the product.

### 2. Replacing "follow the C"

CONTRIBUTING.md's rule makes reviews easy and upstream fixes portable. It also forbids most of
this plan.

Options:

- **(a)** Keep the rule. Refactors happen only inside functions.
- **(b) Provenance, not structure.**
  - Every item keeps a provenance doc line naming the C function(s) it implements
    (`/// C: lib/dotgen/position.c:make_lrvn`), even after merging, splitting or renaming.
  - Structure is free whenever the ratchets pass.
  - A script lists C functions with no provenance tag, so coverage of the C stays auditable.
- **(c) Tiered.** Like (b), but some modules are declared **C-mirror**: they stay
  function-for-function because their exact operation order *is* the product, or because
  upstream changes them often. Candidates: `core::math`, `cdt.rs` (the splay-tree order), the
  network-simplex and mincross tie-breaking cores, and `pathplan`. Everything else is free.

**Recommendation: (c).**

- Upstream Graphviz releases are a major bump per [versioning.md](../versioning.md). Porting an
  upstream diff into a freely restructured module costs more, and provenance tags plus the frozen
  1.0 tag as a reference implementation keep that cost manageable.
- Write the C-mirror list into CONTRIBUTING.md.
- Revise the list when an upstream bump shows where the pain is.

### 3. Unsafe policy

Options:

- **(a)** Keep `forbid(unsafe_code)` in every own crate. Dependencies may contain `unsafe` (they
  already do).
- **(b)** Allow `unsafe` in one small audited crate (`graphviz-pure-kernels`) for runtime CPU
  dispatch (`#[target_feature]` + `is_x86_feature_detected!`), unchecked indexing in proven
  kernels, or `std::arch` SIMD. It would need Miri in CI and a SAFETY comment on each block.
- **(c)** Lift the policy.

**Recommendation: (a).** Nothing in Phases 1–3 needs `unsafe`. T1 is achieved with build flags;
bounds checks go with iterator rewrites. Reconsider (b) only for runtime FMA dispatch, if the
maintainer wants one universal x86 binary instead of v1 and v3 artifacts.

### 4. Dependency appetite

| candidate | use | licence | unsafe inside | verdict |
| :--- | :--- | :--- | :--- | :--- |
| `rustc-hash` (FxHash) or `foldhash` | interner and lookup maps | MIT/Apache; Zlib | small | Fine if a profile shows hashing. foldhash documents no stability across versions or platforms, so never let its output order reach bytes. |
| `smallvec` or `tinyvec` | inline adjacency and attribute lists | MIT/Apache; Zlib/MIT/Apache | yes; **no** (tinyvec is 100% safe) | Prefer `tinyvec`, or `Vec` with reuse |
| `bumpalo` | per-phase arenas | MIT/Apache | yes | Not needed: index arenas (`Vec`) plus buffer reuse cover it |
| `rayon` | T11, T12 | MIT/Apache | yes | Behind a `parallel` feature, native CLI only |
| `mimalloc` or `tikv-jemallocator` | CLI allocator | MIT; BSD-2 | C code | **No**: it makes the "pure" binaries depend on C, and malloc isn't the bottleneck |
| `gungraun` | perf ratchet | MIT/Apache | — | dev-dependency or bench-only; needs Valgrind on the runner |

**Recommendation:** small pure-Rust dependencies are allowed when a PR shows a measured gain over
the std-only alternative. No C dependencies. Build and test tools (`cargo-pgo`, `gungraun`) are
fine in CI.

### 5. Threads and WASM

Options:

- **(a)** No threads.
- **(b) A `parallel` cargo feature** (rayon), on for the native CLI and off by default for the
  library and WASM, used only for order-preserving work (T11, T12). CI runs the fidelity ratchet
  with `RAYON_NUM_THREADS=1` and with N threads, and compares bytes.
- **(c)** Also ship a threaded WASM build. That needs nightly, `+atomics,+bulk-memory`, and a
  cross-origin-isolated page (COOP/COEP) for `SharedArrayBuffer` (wasm-bindgen-rayon's
  requirements).

**Recommendation: (b).** (c) is not worth a nightly toolchain and a hosting requirement while the
WASM API is first-class. Revisit when wasm threads reach stable Rust.

### 6. x86-64 baseline of release binaries

This decision is new: it follows from T1.

Options:

- **(a)** Keep v1 (5× slower neato).
- **(b) v3 as the default artifact, plus a `-compat` v1 artifact** for old CPUs. Docker, PyPI
  wheels and Homebrew pick v3.
- **(c)** Runtime dispatch in one binary (needs Open decision 3 (b)).

**Recommendation: (b)**, with a cross-platform job that proves the v1 and v3 bytes are equal.

### 7. MSRV

Defer to #512's D4. Nothing in this plan needs a newer compiler.

## Draft issues

To file once this plan is approved. Each is one PR unless noted.

1. **perf: add a perf ratchet to CI.** Callgrind instruction counts and peak heap on the sibling's
   benchmark set; 2% and 5% gates; a `perf-regression-accepted` label. This depends on
   `scripts/bench/`.
2. **release: build x86-64 artifacts with `-C target-cpu=x86-64-v3`, plus a v1 compat artifact.**
   Add a ratchet job built with v3, and a byte-equality check between v1 and v3 (T1; measured 5×
   neato, 2.2× sfdp).
3. **math: bit-exact fast paths in `pow` for `y == 2` and `y == 1`.** Include a differential test
   against the general path (T2; sfdp `pow` is 38–42% of samples).
4. **neatogen: restructure `right_mult_with_vector_ff` without reordering any sum.** Row-blocking,
   a split scatter loop, slice iteration (T8; 88% of neato).
5. **sfdp: reuse supernode buffers in `QuadTree::get_supernodes_internal`** (T9; 4 M allocations
   per rnd2000 run).
6. **parser: stop retaining every token; borrow token text from the input** (T3; 982 MB against
   C's 82 MB on a 12 MB file).
7. **core: design note and prototype for interned, id-indexed attributes** (T4). Covers how
   writers keep their iteration order and how typed values are cached without moving warnings.
8. **layout: typed index newtypes for dot's `N`, `E`, `G` and `L`** (T5; mechanical, unlocks T6
   and T7).
9. **dotgen: split `NodeInfo` (832 B) into network-simplex, mincross and cold parts** (T6;
   profile-driven, NS first).
10. **docs: replace "Follow the C" in CONTRIBUTING.md with provenance tags and a C-mirror module
    list** (Open decision 2), plus a script that reports C functions without a provenance tag.
11. **release: evaluate PGO (`cargo-pgo`) and CLI `strip`/`panic=abort`** (T14). Report the
    gains and the exit-status change.

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
