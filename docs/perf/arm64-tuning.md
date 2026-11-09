# CPU tuning for the arm64 release builds (#826)

**No arm64 target should change.** The macOS build already compiles for the M1: `apple-m1` is
rustc's default CPU for `aarch64-apple-darwin`, and the flag changes neither IR nor assembly. On
Linux and Windows arm64, `neoverse-n1` and `neoverse-v2` are **0.2–0.8% slower** than the shipped
generic build (geometric mean of 72 `full.tsv` cases, PGO on every build, on Neoverse N2 runners).
Two independently trained builds of the shipped configuration differ by only 0.02–0.11% on the
same runners, so the slowdown is probably real, but it is tiny either way. A tuned build would
have to win by 5% to justify a second binary and a launcher, and neither CPU runs on every arm64
machine people use. PGO, which every arm64 release already uses, is worth **5.6–7.3%**: builds
without it are that much slower. Every build wrote the same bytes as the x86-64 v1 build on all
2,939 corpus outputs, and rustc contracted no `a * b + c` for any CPU.

| target | candidate | runner CPU | wall vs shipped | CPU time vs shipped | outputs identical to x86-64 v1 |
| :-- | :-- | :-- | --: | --: | --: |
| `aarch64-apple-darwin` | `apple-m1` (= the default; A/A) | Apple M1 (virtual, 3 cores) | 0.9986 | 1.0000 | 2,939 / 2,939 |
| `aarch64-unknown-linux-musl` | `neoverse-n1` | Neoverse N2 (Cobalt 100, 4 cores) | 1.0067 | 1.0067 | 2,939 / 2,939 |
| `aarch64-unknown-linux-musl` | `neoverse-v2` | Neoverse N2 | 1.0024 | 1.0025 | 2,939 / 2,939 |
| `aarch64-pc-windows-msvc` | `neoverse-n1` | Cobalt 100 (4 cores) | 1.0082 | — | 2,939 / 2,939 |
| `aarch64-pc-windows-msvc` | `neoverse-v2` | Cobalt 100 | 1.0028 | — | 2,939 / 2,939 |

Ratios below 1 are faster. The shipped build of each target is the reference; 2,939 outputs are
2,671 SVGs (every corpus input) and 268 PNGs (every tenth).

## 1. What the arm64 builds compile for today

The release (`release.yml`, job `binaries`) passes no `-C target-cpu` on arm64. That does not mean
the same thing on every target, because each target triple carries its own default CPU (rustc
1.99.1, `--print target-spec-json`):

| target | default CPU / features | what that assumes |
| :-- | :-- | :-- |
| `aarch64-apple-darwin` | `cpu: apple-m1` | every Apple Silicon Mac (M1 is the oldest) |
| `aarch64-unknown-linux-musl` | `+v8a,+outline-atomics` | any Armv8.0-A core (Cortex-A53 and up) |
| `aarch64-pc-windows-msvc` | `+v8a,+neon` | any Armv8.0-A core |

So **the macOS arm64 build is already tuned for the M1**. `-C target-cpu=apple-m1` restates the
target's default: it enables the same 28 target features (`rustc --print cfg` lists the same set
with and without it) and the same LLVM scheduling model. Linux and Windows build for generic
Armv8.0-A, with generic scheduling.

All three arm64 builds already use PGO, with the same training list as x86-64
(`scripts/build-pgo.py`, `docs/perf/pgo.md`), and CI's `native-pgo` job checks each against a plain
build on the whole corpus. Every tuned build measured below uses that same PGO; §4.1 measures what
PGO itself buys on arm64.

The `armv8.2-a` the issue also names is not a rustc CPU (`rustc --print target-cpus` has no such
entry). Its nearest equivalent, `-C target-feature=+v8.2a`, keeps generic scheduling and only adds
instructions, a subset of what `neoverse-n1` enables, so it was not measured separately.

## 2. Method

Two runs of a temporary workflow (§6) on GitHub's arm64 runners, with Rust 1.99.1 (release.yml's
`RUST_VERSION`). Both built commits whose product tree is master's `9f940ba0`; they only add the
workflow and its helper script.

- **Run 1** built, per target, the shipped configuration and each candidate CPU.
- **Run 2** built, per target, the shipped configuration twice (two independent PGO trainings, an
  A/A pair that measures the noise of the method on that runner) and once without PGO.

- **Builds.** One job per target and CPU ran `scripts/build-pgo.py` exactly as `release.yml` does,
  with release.yml's `RUSTFLAGS` (`+crt-static` on Windows) plus the candidate `-C target-cpu`. Each
  build trained its own instrumented binary on the same 276 commands (Windows's checkout of the list
  hashes differently, by its line endings). The "default" build is the shipped configuration.
- **Byte identity.** An `ubuntu-24.04` job built the x86-64 v1 musl binary (the plain build ci.yml's
  `cpu-fidelity` job compares v3 against) and hashed exit status, stdout and stderr of `-Tsvg` on
  every `tests/**/*.gv` input, plus `-Tpng` on every tenth. Each arm64 build was hashed the same way
  on its own runner, and every hash had to match.
- **Timing.** All builds of one target ran on one runner, interleaved: for every `full.tsv` case a
  warm-up of each build (except the slow cases the set marks with a run count), then rounds of one
  run per build in rotating order; at least 3 rounds, 5 for cases without a run count. The table
  reports the geometric mean, over cases, of the ratio of median wall times (and of median CPU time
  where the OS reports it, and of `perf stat` user-mode instructions and cycles where the runner
  exposes the PMU). The runners are shared VMs, so only ratios measured side by side on one runner
  mean anything.
- **Contraction probe** and **math unit tests**: §5.

## 3. macOS: `apple-m1`

rustc 1.99.1 emits **identical LLVM IR and assembly** with and without `-C target-cpu=apple-m1` for
`aarch64-apple-darwin` (checked on a probe crate; both carry `"target-cpu"="apple-m1"`). The two
release binaries still had different hashes (`4eb84bf3…` and `4b5c1bc4…`, same size), because each
build trained its own PGO profile and training is not bit-reproducible: the two profiles differ by
9,697 of 11.6 billion counts (`docs/perf/pgo.md`, "Release builds and reproducibility", already
records this).

So the macOS timing in run 1 is an **A/A test**: two builds of the same configuration. Its geomean
is 0.9986 (wall) and 1.0000 (CPU), and its per-case ratios spread from 0.81 to 1.39 (standard
deviation of the log ratio 0.062) on this 3-core virtual M1. **macOS needs no change**: the shipped
binary is already the `apple-m1` build.

## 4. Linux and Windows: `neoverse-n1`, `neoverse-v2`

Both runners are Azure Cobalt 100 VMs (Arm Neoverse N2 cores). They do not expose every feature the
candidates declare (`/proc/cpuinfo` lists no `mte`, `rng`, `bti`, `ssbs`, `dit` or `spe`), but the
compiler emits none of those on its own, so both binaries ran. Geometric mean of per-case median
ratios against the shipped generic build; wall time, and on Linux user+system CPU time (`wait4`).
The Linux runner refused `perf stat` (`perf_event_paranoid` 4), so there are no instruction or cycle
counts.

| engine | cases | Linux `neoverse-n1` | Linux `neoverse-v2` | Windows `neoverse-n1` | Windows `neoverse-v2` |
| :-- | --: | --: | --: | --: | --: |
| dot | 17 | 1.0096 | 1.0001 | 1.0045 | 1.0037 |
| neato | 11 | 1.0213 | 0.9816 | 1.0180 | 0.9823 |
| fdp | 8 | 0.9955 | 1.0264 | 1.0125 | 1.0243 |
| sfdp | 10 | 0.9938 | 1.0044 | 0.9978 | 0.9964 |
| circo | 7 | 1.0048 | 1.0048 | 1.0126 | 1.0113 |
| twopi | 7 | 1.0065 | 1.0098 | 1.0052 | 1.0115 |
| osage | 6 | 1.0097 | 0.9961 | 1.0099 | 0.9990 |
| patchwork | 6 | 1.0080 | 1.0080 | 1.0087 | 1.0048 |
| **all** | **72** | **1.0067** | **1.0024** | **1.0082** | **1.0028** |

No case is more than 6.1% faster with either CPU (the best is Linux `neoverse-n1` on
`sfdp_sparse_20000`, 0.939, the only case over 5%). Run 2 measured the method's noise on the same
runners: two independently trained PGO builds of the shipped configuration differ by a geomean of
1.0002 (Linux) and 0.9989 (Windows), with a per-case log standard deviation of 0.010 and 0.009. The
candidates' per-case spread is 0.014–0.016, and their geomeans sit 1.4 (`neoverse-v2`) to 5
(`neoverse-n1` on Windows) standard errors above 1. So `neoverse-n1` is probably a little slower on
N2, and `neoverse-v2` is indistinguishable from the shipped build; neither comes near 5%. The sums
of median times agree: 53.2 s for both the generic and `neoverse-n1` builds on Linux, 54.0 s for
`neoverse-v2`; on Windows 50.9 s generic and 51.4 s for each tuned build.

The likely reason: PGO already gives LLVM the block frequencies and layout that matter, and the
port's hot loops are scalar `f64` code with data-dependent branches that `dotprod`, `lse` or SVE
cannot speed up, and LLVM may not vectorise a floating-point sum by reordering it. These numbers
come from one core design (N2). The tuning could behave differently on an N1, a V2 or a Snapdragon
X, but a 5% gain on those would have to come from scheduling alone, which the N2 numbers give no
sign of.

### 4.1 What PGO buys on arm64

Run 2 timed the shipped PGO build against a second PGO build of the same configuration (A/A) and
against the same flags without PGO, on each target's runner:

| target | A/A (second PGO build) | without PGO | sum of medians, PGO → no PGO |
| :-- | --: | --: | --: |
| `aarch64-apple-darwin` | 1.0023 | 1.0559 | 38.5 s → 42.1 s |
| `aarch64-unknown-linux-musl` | 1.0002 | 1.0734 | 53.2 s → 61.8 s |
| `aarch64-pc-windows-msvc` | 0.9989 | 1.0626 | 51.4 s → 57.8 s |

Without PGO, fdp and sfdp lose the most (1.13–1.20 on Linux and Windows) and dot, neato and twopi
the least (0.99–1.05). On x86-64 neato gains much more from PGO (`docs/perf/pgo.md`). PGO is applied
on arm64, and it is worth an order of magnitude more than either CPU.

**Recommendation: ship neither CPU.** The shipped generic builds stay. A tuned arm64 Linux or
Windows binary would also need a launcher, because neither CPU runs everywhere (next section).

### Who a tuned binary would exclude

A `target-cpu` is a promise about the instruction set, not only a scheduling hint. The two
candidates enable (rustc 1.99.1 `--print cfg`, beyond the generic `neon`):

- `neoverse-n1`: Armv8.2-A with `lse`, `rcpc`, `dotprod`, `fp16`, `rdm`, `crc`, `aes`, `sha2`,
  `ssbs`, `spe`, ...
- `neoverse-v2`: Armv9.0-A with `sve`, `sve2`, `bf16`, `i8mm`, `mte`, `rand` and more; it does not
  declare `aes` or `sha2`, which `neoverse-n1` does.

| CPU people run the binary on | generic (shipped) | `neoverse-n1` | `neoverse-v2` |
| :-- | :-: | :-: | :-: |
| Raspberry Pi 3 / 4 (Cortex-A53 / A72, Armv8.0) | yes | **no** | **no** |
| Raspberry Pi 5 (Cortex-A76, Armv8.2) | yes | yes¹ | **no** (no SVE) |
| AWS Graviton2, Ampere Altra (Neoverse N1) | yes | yes | **no** |
| AWS Graviton3 (Neoverse V1) | yes | yes | **no** (no SVE2) |
| Graviton4, Azure Cobalt 100, Google Axion (V2 / N2) | yes | yes¹ | yes¹ |
| Snapdragon 8cx, SQ1–SQ3 (Windows on Arm) | yes | yes¹ | **no** |
| Snapdragon X (Oryon, Windows on Arm) | yes | yes | **no** (no SVE) |
| Docker / Linux VMs on Apple Silicon | yes | yes¹ | **no** (no SVE) |

¹ These cores, or the VMs that run on them, do not expose every feature the target CPU declares
(`spe`, `ssbs`, `mte`, `rng`, depending on the core), but the compiler emits none of those on its
own, so the binary runs in practice. The Cobalt 100 runners in §4 are an example.

No single tuned arm64 Linux or Windows binary runs on every likely CPU: `neoverse-n1` loses the
Raspberry Pi 3/4 and other Armv8.0 boards, and `neoverse-v2` loses nearly every client machine and
every Graviton before the fourth generation.

### What a launcher would cost

Shipping either CPU would mean doing for arm64 what the x86-64 release does for v3
(`docs/releasing.md`, `packaging/launcher/main.rs`):

- **Build.** A second PGO build (instrumented build, 276 training commands, final build) per target:
  about 2.5 minutes on the Linux arm runner and 4 on Windows arm, plus a corpus comparison against
  the generic build (under 1.5 minutes per binary).
- **Launcher.** `packaging/launcher/main.rs` is x86-only (`is_x86_feature_detected!`). An arm64 path
  in the launcher needs `is_aarch64_feature_detected!` for every feature the tuned build may emit
  (`neoverse-n1`: `lse`, `rcpc`, `dotprod`, `fp16`, `rdm`, `crc`, `aes`, `sha2`, ...), an Armv8.0
  build of the launcher, and its own test, like ci.yml's qemu run of the x86 one under older CPU
  models.
- **Packaging.** Two binaries in the Linux and Windows arm64 archives, wheels, the arm64 container
  and Scoop's arm64 entry, with a tuned directory beside `compat/`, as the x86 packages carry `v3/`
  and `compat/`; a second arm64 archive for binstall and AUR, which pick one file by target triple.
  About 2× the arm64 download.

## 5. Fused multiply-add on aarch64

aarch64 always has `fmadd`. Two things decide whether the port's floating-point bits stay C's:

1. **rustc never contracts `a * b + c` on its own.** It emits plain `fmul`/`fadd` with no `contract`
   fast-math flag, never emits `llvm.fmuladd`, and leaves LLVM's floating-point fusion at its
   default (`FPOpFusion::Standard`), which fuses only `llvm.fmuladd` and `contract`-flagged
   operations. Only `f64::mul_add` becomes `llvm.fma`. None of this depends on the CPU. The
   measurement workflow compiled `a * b + c`, `c - a * b`, a four-term dot product and one `mul_add`
   for every candidate CPU and counted the fused scalar and NEON instructions in the assembly (run 2
   also matched SVE's forms): exactly one, the `mul_add`, for every CPU on all three targets.
2. **`graphviz_pure_core::math` takes the same path for every arm64 CPU.**
   - `math::fma` calls `f64::mul_add` on every `aarch64` target (its `portable::fma` branch is only
     for `wasm` and x86-64 musl without `target_feature = "fma"`). That is the `fmadd` instruction,
     correctly rounded, so it equals C's contracted `fmadd` on the macOS arm64 reference build.
   - `cbrt` uses the `libm` crate's version: the CORE-MATH copy in `portable/cbrt.rs` is selected by
     `target_feature = "fma"`, a feature name that exists only on x86. Inside `libm` 0.2.16 every
     `f64` path that has an aarch64 variant (`fma`, `sqrt`, `rint`) is gated on `neon`, which every
     arm64 target has; only the `f16` functions look at `fp16`, which `neoverse-n1` and `apple-m1`
     turn on, and the port calls no `f16` function.
   - Among the other dependencies, only `zlib-rs` and `crc32fast` look at an aarch64 feature beyond
     `neon` (`crc`, for the PNG checksums), and they already detect it at run time. A CRC is the
     same number whichever instruction computes it.

   The workflow also ran `math`'s unit tests (`portable::fma` against hardware FMA and `libm::fma`,
   `hypot` against Apple's libm on macOS) with each candidate's flags, and hashed every output
   against the x86-64 v1 build: all 2,939 outputs identical for each of the 8 arm64 builds of run 1
   and the 9 builds of run 2.

## 6. Reproducing

The measurement workflow lived on the PR branch only (#843), as `.github/workflows/arm64-tuning.yml`
with its helper `scripts/bench/arm64_tuning.py`: run 1 at commit `0040b493`, run 2 at `d1919a67`.
After the squash merge those commits stay reachable with `git fetch origin pull/843/head`. Runs
[42579158206](https://github.com/graphviz-pure/graphviz-pure/actions/runs/42579158206) and
[42582816610](https://github.com/graphviz-pure/graphviz-pure/actions/runs/42582816610). Run 1's
`results-*` artifacts (raw samples, per-build manifests) expire on 2026-12-09 and its binaries and
x86-64 reference hashes on 2026-11-16; run 2 keeps every artifact until 2027-02-07. The per-case
medians of both runs are in [data/arm64-tuning.tsv](data/arm64-tuning.tsv).
