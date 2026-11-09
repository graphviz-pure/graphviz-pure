# CPU tuning beyond x86-64-v3 (#825)

**Ship nothing. x86-64 stays at `x86-64-v3` + PGO with the v1 compat build.** No candidate comes
within reach of the 5% bar, and the AVX-512 builds that LLVM produces by default are slower:

- **`x86-64-v4`, `znver4` and `znver5` (AVX-512) are 4–7% slower** than the shipped v3 build on
  Zen 4 and Zen 5 (geometric mean of 72 `full.tsv` cases, PGO on every build), and `x86-64-v4` is
  **16% slower on Ice Lake**. The loss is neato's, whose largest inputs run up to 1.9× (Zen 4) and
  3.2× (Ice Lake) slower: with AVX-512, LLVM vectorizes neato's hottest loop with gathers.
- **Without gathers and scatters the AVX-512 builds are at most 1.2% faster** by geomean, and
  their sums of medians are 1.4% faster to 0.8% slower. That is far too little to pay for a third
  binary per x86 target and a CPU check that CI could not exercise.
- **`znver3` is no faster on any core**: geomean 1.0000–1.0009, and its sum of medians is 1.1–1.9%
  higher than v3's.
- Every build, on every runner, wrote **the same bytes as the x86-64 v1 build** on all 2,939
  corpus outputs, and rustc contracted no `a * b + c` for any CPU.

Geometric mean over the 72 cases of the median wall time against the shipped `x86-64-v3` build on
the same runner; when a CPU type got several runners, the geometric mean of their geomeans and
their range:

| build | Zen 3 (EPYC 7763) | Zen 4 (EPYC 9V74) | Zen 5 (EPYC 9V45) | Ice Lake (Xeon 8370C) | Emerald Rapids (Xeon 8573C) |
| :-- | --: | --: | --: | --: | --: |
| A/A: `x86-64-v3` again | 0.9993 (0.997–1.003) | 1.0018 (1.001–1.003) | 0.9980 (0.994–1.002) | 0.9991 (0.998–1.002) | 0.9993 (0.999–0.999) |
| `znver3` | 1.0009 (0.998–1.003) | 1.0003 (0.998–1.002) | 1.0000 (0.995–1.006) | — | — |
| `x86-64-v4` | — | 1.0623 (1.060–1.064) | 1.0382 (1.031–1.043) | 1.1590 (1.155–1.162) | 0.9965 (0.996–0.997) |
| `x86-64-v4-ng` | — | 0.9877 (0.987–0.988) | 0.9936 (0.992–0.995) | 1.0005 (0.997–1.004) | 0.9912 |
| `znver4` | — | 1.0685 (1.066–1.071) | 1.0418 (1.036–1.046) | — | — |
| `znver4-ng` | — | 0.9925 (0.991–0.994) | 0.9917 (0.989–0.998) | — | — |
| `znver5` | — | — | 1.0415 (1.040–1.044) | — | — |
| `znver5-ng` | — | — | 0.9895 (0.984–0.996) | — | — |
| runners | 15 | 5 | 7 | 3 | 2 |

Ratios are against the shipped `x86-64-v3` PGO build timed on the same runner, interleaved; below 1
is faster. `-ng` builds add `-C target-feature=+prefer-no-gather,+prefer-no-scatter`, which keeps
LLVM from emitting AVX-512 gathers and scatters (§4). The A/A row is a second, independently
trained v3 build: the noise floor of the method on that runner type.

## 1. What the x86-64 builds compile for today

`release.yml` (job `binaries`) builds the three x86-64 targets (`x86_64-unknown-linux-musl`,
`x86_64-apple-darwin`, `x86_64-pc-windows-msvc`) twice: with `-C target-cpu=x86-64-v3` and PGO
(the default archive), and as a plain x86-64-v1 build (the `-compat` archive). The default
archive also carries a v1 launcher (`packaging/launcher/main.rs`) that the wheels, the amd64
container, Homebrew and Scoop install in front of `v3/` and `compat/` directories; it runs the v3
binary when the CPU and OS support every v3 feature (`docs/releasing.md`).

The candidates differ from v3 in what they let LLVM emit (rustc 1.99.1, LLVM 23.1.1,
`rustc --print cfg -C target-cpu=…`) and in the scheduling model they tune for:

| CPU | target features beyond `x86-64-v3` | vector width LLVM prefers |
| :-- | :-- | :-- |
| `x86-64-v4` | `avx512f`, `avx512bw`, `avx512cd`, `avx512dq`, `avx512vl` | 256-bit (`ymm`) |
| `znver3` | `adx`, `aes`, `pclmulqdq`, `rdrand`, `rdseed`, `sha`, `sse4a`, `vaes`, `vpclmulqdq`, `xsavec`, `xsaveopt`, `xsaves` | 256-bit |
| `znver4` | `znver3`'s, plus v4's AVX-512 and `avx512bf16`, `avx512bitalg`, `avx512ifma`, `avx512vbmi`, `avx512vbmi2`, `avx512vnni`, `avx512vpopcntdq`, `gfni` | 512-bit (`zmm`) |
| `znver5` | `znver4`'s, plus `avx512vp2intersect`, `avxvnni` | 512-bit |

The vector width comes from the contraction probe (§6): its `axpy` loop uses `ymm` registers
under `x86-64-v4` and `zmm` under `znver4` and `znver5`.

## 2. Method

Three runs of a temporary workflow (§9) on GitHub's `ubuntu-24.04` runners, Rust 1.99.1
(release.yml's `RUST_VERSION`), target `x86_64-unknown-linux-musl` (the Linux release target).
All three built master's product source at `d4f2d84a`; the runs add only the workflow, its
helper script and this documentation.

- **Shards.** A job cannot choose its machine: a `ubuntu-24.04` job lands on whatever Azure VM is
  free. So each run started 8 or 12 identical jobs ("shards"). Each one read `/proc/cpuinfo`
  and built only what its CPU can run (PGO training runs the instrumented binary, and a build
  with instructions the CPU lacks dies with `SIGILL`): the `znver` builds only on AMD Zen cores,
  the AVX-512 builds only where `avx512f/bw/cd/dq/vl` are present, `znver5` only on a Zen 5.
  Run 2 tried `znver5` on Zen 4 anyway; its instrumented binary died with `SIGILL` (exit −4)
  during training.
- **Builds.** `scripts/build-pgo.py` exactly as `release.yml` runs it, with the candidate's
  `-C target-cpu` (and, for `-ng`, the extra target features) as `RUSTFLAGS`. Every build trained
  its own instrumented binary on the same 276 commands. `x86-64-v3` is the shipped configuration;
  `x86-64-v3-b` is the same flags trained again, the A/A pair.
- **Timing.** All builds of one shard ran on its runner, interleaved: for every `full.tsv` case a
  warm-up of each build (except the slow cases the set marks with a run count), then rounds of
  one run per build in rotating order; at least 3 rounds, 5 for cases without a run count. A
  shard's number is the geometric mean, over the 72 cases, of the ratio of median wall times;
  CPU time (`wait4`) gives the same ratios to within 0.001. Where a CPU type got several shards,
  the tables give the geometric mean of the shards and the range.
- **Byte identity.** One job built the plain x86-64 v1 musl binary (the build ci.yml's
  `cpu-fidelity` job compares v3 against) and hashed exit status, stdout and stderr of `-Tsvg` on
  every `tests/**/*.gv` input and `-Tpng` on every tenth: 2,939 outputs. Every candidate build was
  hashed the same way on its own runner, and a last job required every hash to match.
- **Instruction counts.** The runners expose no hardware counters: `perf stat` reports
  `instructions:u` as not supported, even after `perf_event_paranoid` is lowered. §7 gives
  instruction counts for the Zen 3 builds, measured on a local Zen 3.

### Runners

32 shards in three runs, every one usable: each timed the A/A pair and every candidate its CPU
could run. Most `ubuntu-24.04` jobs land on Zen 3; 17 of 32 had AVX-512.

| runner CPU | run 1 | run 2 | run 3 | builds timed besides `x86-64-v3` |
| :-- | --: | --: | --: | :-- |
| Zen 3: AMD EPYC 7763 | 4 | 7 | 4 | A/A, `znver3` |
| Zen 4: AMD EPYC 9V74 | 2 | 2 | 1 | A/A, `znver3`, `x86-64-v4`, `znver4` (runs 2–3: also `-ng`) |
| Zen 5: AMD EPYC 9V45 | 0 | 2 | 5 | A/A, `znver3`, `x86-64-v4`, `znver4` and `-ng` (run 3: also `znver5`, `znver5-ng`) |
| Ice Lake: Intel Xeon Platinum 8370C | 1 | 0 | 2 | A/A, `x86-64-v4` (run 3: also `-ng`) |
| Emerald Rapids: Intel Xeon Platinum 8573C | 1 | 1 | 0 | A/A, `x86-64-v4` (run 2: also `-ng`) |
| shards | 8 | 12 | 12 | |

All are 4-vCPU Azure VMs. The CPU type is the `model name` in `/proc/cpuinfo` (recorded per shard
in the data file); the Zen generation follows from the CPU family (0x19 for the 7763 and
9V74, 0x1a for the 9V45) and the feature flags (the 9V45 lists `avx512_vp2intersect` and
`avx_vnni`, which only Zen 5 has among AMD cores).

## 3. Results

Per engine, geometric mean over that engine's cases (pooled over the runners of a type as above),
for the builds that answer the question; the data file has every build:

| engine (cases) | Zen 3 `znver3` | Zen 4 `x86-64-v4` | Zen 4 `x86-64-v4-ng` | Zen 5 `znver5` | Zen 5 `znver5-ng` | Ice Lake `x86-64-v4` | Ice Lake `x86-64-v4-ng` |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| dot (17) | 0.997 | 1.046 | 0.999 | 1.059 | 0.989 | 1.062 | 1.000 |
| neato (11) | 0.996 | 1.389 | 0.932 | 1.260 | 0.995 | 2.398 | 1.005 |
| fdp (8) | 1.024 | 1.008 | 0.998 | 1.011 | 1.009 | 0.988 | 0.982 |
| sfdp (10) | 1.005 | 0.992 | 0.996 | 0.995 | 0.996 | 0.996 | 0.995 |
| circo (7) | 0.998 | 0.998 | 1.000 | 0.974 | 0.986 | 0.993 | 0.999 |
| twopi (7) | 0.996 | 0.991 | 0.987 | 0.961 | 0.961 | 0.987 | 0.988 |
| osage (6) | 0.998 | 1.005 | 1.001 | 0.986 | 0.986 | 1.024 | 1.024 |
| patchwork (6) | 0.995 | 1.005 | 1.007 | 0.986 | 0.986 | 1.019 | 1.022 |
| **all (72)** | **1.001** | **1.062** | **0.988** | **1.042** | **0.989** | **1.159** | **1.001** |
| sum of medians | 1.019 | 1.095 | 0.987 | 1.068 | 1.007 | 1.260 | 0.986 |

The A/A pairs give the noise floor: their geomeans stay within 0.994–1.003 on every runner, with a
per-case standard deviation of the log ratio of 0.008–0.012 on average, and their sums of medians within
0.997–1.009. Against that:

- **AVX-512 as LLVM emits it by default loses** on every AMD core and on Ice Lake, by 4–16%, and
  only breaks even on Emerald Rapids (0.9965).
- **The `-ng` builds win by 0.6–1.2% by geomean** on the AMD cores and Emerald Rapids, a few
  times the A/A spread, so probably real but small. Their sums of medians, which the largest
  inputs dominate, range from 1.4% faster (Ice Lake, Zen 4) to 0.8% slower (Zen 5). The gain is
  in small and medium cases (twopi and circo on Zen 5, neato on Zen 4); the slowest inputs gain
  nothing.
- **No case anywhere is more than 12% faster** than v3 with any candidate (the best single ratio
  is 0.888, `x86-64-v4` on Emerald Rapids for a small dot input).

## 4. AVX-512 builds: slower, because of gathers

The plain `x86-64-v4`, `znver4` and `znver5` builds lose on every AVX-512 machine except
Emerald Rapids, and the loss is neato's: neato's geomean is 1.26–1.39 on Zen 4 and Zen 5 and 2.40
on Ice Lake, while dot loses 2–7% and the other engines stay within 4% of 1. The worst cases
are neato's stress-majorization inputs (`synth/neato_sparse_*`, `hw3_vpsc_random200`, `badvoro`),
up to 1.95× slower on Zen 4, 1.56× on Zen 5 and 3.2× on Ice Lake.

The cause is the hottest function in neato. On `synth/neato_sparse_1000.gv`,
`neatogen::matrix_ops::right_mult_with_vector_ff` (a packed symmetric matrix times a vector, the
inner step of the conjugate-gradient solve) takes 61% of the time and
`stress_majorization_kd_mkernel` 28% (`perf record` of a v3 build). That function processes eight
matrix rows at a time, each row its own sequential `fma` chain. With AVX-512's gather
instructions available, LLVM vectorizes *across* the eight rows: one 8-lane chain whose every step
loads `A[r][k]` for the eight rows `r` with a `vgatherqps` (eight addresses, one per row). Every
lane keeps its own row's operation order, so the result is bit-identical, but a gather costs far
more than eight scalar loads feeding eight independent chains: a gather issues as many
micro-ops at a fraction of a plain load's rate on Zen 4 and Zen 5, and on Ice Lake it is slower
still (consistent with the microcode mitigation for
Gather Data Sampling, which Emerald Rapids does not need). Under AVX2 LLVM does not consider
gathers cheap, so the v3 build has none.

The gathers and scatters are in the shipped-style PGO binaries of run 2: 258 gathers and 198
scatters in `x86-64-v4`, 372 and 335 in `znver4`, none in any v3 or `znver3` build. A
symbolized build of the same code (v4, trained with a v3 profile) puts 9 of its 90 gathers in
`right_mult_with_vector_ff`; the rest are in `neatogen::voronoi::adjust`, the spline routers and
font code.

`prefer-no-gather` and `prefer-no-scatter` are LLVM target features that tell the vectorizer not
to use them; rustc passes them through with a warning that they are unstable. The `-ng` builds
(runs 2 and 3) have no gathers or scatters left, and neato is no longer slower: the whole set
comes out 0.988–1.001 (§3) instead of 1.04–1.16. That is what AVX-512 is worth to this code base
today, and it is far from 5%: the port's hot loops are scalar `f64` code with data-dependent branches, and LLVM may not
vectorize a floating-point reduction by reordering it.

## 5. `znver3` and `znver5`

**`znver3`** adds Zen 3's scheduling model and a few instructions the port has no use for (AES,
SHA, carry-less multiply, `sse4a`). On 15 Zen 3 runners its geomean is 1.0009 (0.998–1.003), as on
Zen 4 and Zen 5, and its sum of medians is 1.1–1.9% *higher* than v3's on all three AMD types.
Nearly all of that is fdp (engine geomean 1.018–1.024): its three largest inputs (`badvoro`,
`fdp_sparse_1000`, `fdp_sparse_500`) are about 3% slower. It runs 0.2% fewer instructions (§7), which
buys nothing. Its binary is also 0.7% larger gzipped.

**`znver5`** ran on 5 Zen 5 runners in run 3. On Zen 4 it can't run at all: its instrumented
build dies with `SIGILL` in PGO training (run 2), so a `znver5` binary would need a Zen 5 check in
the launcher. On Zen 5 it behaves like `znver4`: 1.0415 with gathers and 0.9895 without
(`znver4-ng` on the same runners: 0.9917), and the sum of medians of `znver5-ng` is 0.7% slower
than v3's. Zen 5's full 512-bit data paths do not change the picture: the hot code is scalar.

## 6. Fused multiply-add and floating point

Two things decide whether a CPU flag can move the port's floating-point bits, and neither does:

1. **rustc never contracts `a * b + c` on its own, for any CPU.** It emits plain `fmul`/`fadd`
   with no `contract` flag and never `llvm.fmuladd`, so LLVM's default fusion
   (`FPOpFusion::Standard`) fuses nothing; only `f64::mul_add` becomes `llvm.fma`, and nothing
   carries `reassoc` or other fast-math flags, so no sum is reordered. Every shard compiled
   `a * b + c`, `c - a * b`, a four-term dot product, an `axpy` loop, a slice sum and one
   `mul_add` for `x86-64-v3`, `znver3`, `x86-64-v4`, `znver4` and `znver5` (runs 2 and 3: the
   `-ng` variants too) and counted the fused instructions in the assembly: exactly one, the `mul_add`,
   every time. The IR of the same probe (checked locally) has no `contract`, `reassoc` or `fast`
   flag for any CPU. Wider vectors (the `axpy` loop in `zmm`) compute each element with the same
   operations, and the slice sum stays a scalar chain under every CPU.
2. **`graphviz_pure_core::math` takes the same path for every candidate.** Its only CPU switch is
   `target_feature = "fma"` (`math::fma` uses `f64::mul_add`, `cbrt` the CORE-MATH copy), and
   every candidate has `fma`, as v3 does. The `libm` crate's x86 code (`sqrt`, `fma`) is exact
   whatever the CPU. Among the other dependencies, the crates with x86 feature paths
   (`crc32fast`, `simd-adler32`, `memchr`, `base64`, `jpeg-encoder`, `bytemuck`) compute integers
   (checksums, byte searches) or detect the CPU at run time, so their results don't depend on the
   flag.

Each shard also ran `math`'s unit tests with each build's flags (all passed), and the corpus
hashes matched for every one of the 125 builds of runs 2 and 3: 2,939 identical outputs each.
(Run 1's hash check first reported three PNG differences for every PGO build, v3 included. Those were the
binaries' file names: the PNG warning `… graph is too large for cairo-renderer bitmaps` starts
with the basename of `argv[0]`, and run 1 named each binary after its CPU. Runs 2 and 3 name
every binary `graphviz-pure`, and everything matched.)

## 7. Instruction counts

The GitHub runners have no PMU. On a local Zen 3 (Ryzen 9 5900XT), `perf stat` counted user-mode
instructions for run 1's three Zen 3 binaries (shard 1) on the 72 `full.tsv` cases, one run each:

| build vs `x86-64-v3` | instructions (geomean) | cycles (geomean, loaded machine) |
| :-- | --: | --: |
| `x86-64-v3-b` (A/A) | 1.0000 | 1.0018 |
| `znver3` | 0.9982 | 1.0157 |

`znver3` runs 0.2% fewer instructions and is not faster for it (sfdp's large graphs run 0.5–1.2%
*more* instructions). The machine was loaded (load average 42–66 on 22 threads), so the cycle
column is indicative only; the interleaved runner timings in §3 are the measurement. AVX-512
builds cannot run on this machine.

## 8. What a v4 tier would cost

No build qualifies, so nothing changes in `release.yml`, `ci.yml` or the launcher. If a later
compiler or code change made an AVX-512 build pay, a `v4` tier would need:

- **Detection.** `has_v4()` = `has_v3()` plus `is_x86_feature_detected!` for `avx512f`,
  `avx512bw`, `avx512cd`, `avx512dq` and `avx512vl` (std's detection also checks that the OS
  saves the ZMM and mask registers). The launcher picks `v4/`, then `v3/`, then `compat/`; a
  `znver4` binary would need its whole feature list checked, and a vendor check to be worth it.
- **One more binary per x86 target.** A third PGO build (2–3.5 minutes on these runners) and a
  third whole-corpus comparison per target, on three targets. The Linux binary is 13.6 MB, 6.7 MB
  gzipped; each x86 release would grow by about 20 MB (a third archive per target, or a bigger
  default archive), and so would the x86-64 wheels, the amd64 container, Homebrew and Scoop.
- **Tests that CI cannot run.** `ci.yml`'s `cpu-fidelity` job runs v3's fidelity ratchets
  natively and tests the launcher's choices under `qemu-x86_64` CPU models. QEMU's emulator has
  no AVX-512 (QEMU 10.0's `-cpu max` stops a v4 binary with `SIGILL`), and most `ubuntu-24.04`
  runners lack it too (15 of the 32 shards here were Zen 3), as does the self-hosted CI runner, so a v4 fidelity job could run only on whichever runners happen to have
  AVX-512, and the launcher's v4 branch could not be tested at all.
- **macOS Intel** would gain little: among Intel Macs only the iMac Pro, the 2019 Mac Pro and the
  2020 Ice Lake MacBook Air and 13-inch MacBook Pro have AVX-512.

## 9. Reproducing

The workflow lived on the PR branch only, as `.github/workflows/x86-tuning.yml` with its helper
`scripts/bench/x86_tuning.py`, and is removed in the PR's last commit. The runs:
[42588114596](https://github.com/graphviz-pure/graphviz-pure/actions/runs/42588114596) (run 1),
[42593209759](https://github.com/graphviz-pure/graphviz-pure/actions/runs/42593209759) (run 2) and
[42599180616](https://github.com/graphviz-pure/graphviz-pure/actions/runs/42599180616) (run 3)
ran at `4bfd47cc`, `d5dee29b` and `11f5cb18`, on top of `d4f2d84a`. The branch was then rebased
onto a newer master; the rebased commits `60adc315`, `21d5f53e` and `f901d74c` carry the same
workflow and helper files, byte for byte, and after the squash merge they stay reachable with
`git fetch origin pull/<PR>/head`. The
`results-*` artifacts (raw samples, CPU info, manifests, probe assembly) expire on 2027-02-07; the
binaries (`bins-*`) on 2026-12-09. The per-case medians of all three runs are in
[data/x86-tuning.tsv](data/x86-tuning.tsv): one row per run, shard and case, with the runner's
CPU, v3's median in milliseconds and each other build's ratio to it.
