# The math library (`graphviz_pure_core::math`)

Every build of graphviz-pure computes its transcendental functions (`pow`, `sin`, `cos`,
`atan2`, `hypot`, ...) with the same pure-Rust implementations, `graphviz_pure_core::math`, on
every target: Linux x86-64 and arm64, Windows, macOS on Apple Silicon and Intel, and wasm32.
No build calls a platform libm for them, and no feature brings one back: clippy's
`disallowed-methods` (`clippy.toml`) rejects the `f64` methods everywhere except inside
`math::portable`. So the same input gives the same bytes everywhere, which
`.github/workflows/cross-platform.yml` checks on every push, including PNG on the native
platforms (the WASM build has no raster output). `-Tpng` and `-Tjpeg` rasterize that SVG with
resvg. The crates on that path (`tiny-skia`, `tiny-skia-path`, `kurbo`, `svgtypes`, `skrifa`)
are patched copies under `vendor/` whose transcendental calls go through the `libm` crate
instead of the platform libm (#339). Release binaries, wasm, npm, PyPI, Docker and builds
from this repository use the portable math for PNG/JPEG; a crates.io source build
(`cargo install`) still gets the platform libm in raster output until the forks or upstream
features land.

Those vendored calls use the `libm` crate for `pow` and `hypot` too. `graphviz_pure_core::math`
uses CORE-MATH for `pow` and its own `hypot`, and the `libm` crate for the other functions.
Both are deterministic. They are two implementations: the raster path does not call
`graphviz_pure_core::math`. Core does not depend on the raster crates, so a dependency the
other way would not cycle. It is not a math-only crate (about 19,000 lines: the graph model,
attributes, fonts and colour tables) and `math` is `f64` only, while the raster crates also
call `f32`. Routing the `f64` calls through core would still leave `f32` on the `libm` crate
and would compile the graph model into kurbo, so the vendored calls stay on `libm` directly.

C Graphviz, on the other hand, calls its platform's libm, so stock C writes different bytes on
macOS and on Linux. The port is therefore compared with C computing with the port's own math
functions (below), and a difference that comes only from a platform's libm is not a bug
(`docs/versioning.md`).

## How the port is compared with C

`crates/graphviz-pure-portable-libm` exports the port's math functions under the libm names C
imports, and `scripts/dot-portable-libm.sh` loads it into the unmodified C `dot`
(`DYLD_INSERT_LIBRARIES` on macOS, `LD_PRELOAD` on Linux, see `scripts/c-oracle-linux/`). The
wrapper refuses to run unless `GVBINDIR` names the pango-free plugin directory of the reference
setup — only the core, dot and neato layout plugins, with the config file `dot -c` writes there,
as `scripts/gen-golden.sh` builds — because with a full installation's directory, or none, a
pango textlayout plugin measures C's text instead of the built-in font metric tables, and every
comparison silently differs by millipoints. The corpus references are made with it:

```sh
cargo build --release -p graphviz-pure-portable-libm
scripts/gen-golden.sh                                                  # layout goldens, 3 runs each
GVBINDIR=~/src/gv-ref/lutplugins cargo run --release -p graphviz-pure-diff -- e2e --generate --runs 5
cargo test --workspace                                                 # every ratchet against them
```

It covers every transcendental symbol that Graphviz 16.1.0's libraries and plugins, and GTS,
import (`nm -u`: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`,
`log10`, `pow`, `hypot`, `cbrt` and clang's `__sincos_stret`); the rest (`sqrt`, `fmod`) are
exact in IEEE arithmetic. Against it the port is byte-identical on all 2,595 stable corpus
inputs (`gv-diff e2e`; 14 more are skipped: C's output changes from run to run on 11, and C
crashes on 3).

`gv-diff e2e --stock` compares with stock C on macOS instead (`tests/golden/e2e.json`, C with
Apple's libm). It is a report and never fails: the port matches 2,474 of 2,594 inputs, and on
every one of the other 120 it equals C with the same math, so they differ only through Apple's
libm (114 sfdp, whose multilevel layout amplifies a last-bit difference in `pow`; 4 fdp; 1
circo; 1 main).

## History: why one math library

Until 0.2.0, Apple targets called Apple's libm by default (the `portable-math` feature opted
into the portable functions, and is now a deprecated no-op), so the port matched stock C on
macOS byte for byte, but the Apple Silicon CLI, crate and Homebrew formula wrote different bytes
from every other platform, and from the browser build on the same Mac. The rest of this page is
how the portable functions were chosen. Measurements written "on macOS with `portable-math`"
were taken with what is now every build, and "Apple profile" means the former default on macOS.

The native aarch64-apple-darwin build with Apple's libm matched C Graphviz byte for byte on the
whole corpus. The wasm32 build did not: 279 of the 2526 corpus files rendered differently (counting
multi-graph files; 197 without them). The cause is libm. On macOS, Rust's `f64::hypot`,
`powf`, `sin`, ... call the system libm (`libsystem_m`), the same one C Graphviz uses. On
wasm32 they come from compiler-builtins' musl port, and on Linux from glibc. These disagree
with Apple in the last bit on some inputs, and one differing bit is enough to change a
force-directed layout.

## Measurements

Arguments came from a native run over every corpus file with the `math-trace` feature
(`GV_MATH_TRACE=dir`, distinct argument tuples, capped at 4,000,000 per function), plus
1,000,000 random inputs per function. Correctly rounded (CR) results were computed with MPFR
4.2.2. Apple means macOS 27 arm64 `libsystem_m`, and wasm means Rust's wasm32 `f64` methods
run under node. Each cell counts calls whose result differs from Apple's.

| function | corpus args | Apple ≠ CR | wasm ≠ CR | wasm ≠ Apple | random: Apple ≠ CR | random: wasm ≠ Apple |
|---|---:|---:|---:|---:|---:|---:|
| hypot | 4,000,000 | 637,069 (15.9%) | 0 | 637,069 | 141,280 | 141,280 |
| pow | 4,000,000 | 5,224 (0.13%) | 106,530 | 110,356 | 1,303 | 88,984 |
| atan2 | 361,216 | 5,771 (1.6%) | 68,614 | 69,391 | 2,900 | 159,839 |
| sin | 17,303 | 495 (2.9%) | 467 | 566 | 33,390 | 36,977 |
| cos | 1,061,293 | 49,777 (4.7%) | 34,160 | 45,709 | 35,761 | 38,432 |
| tan | 205 | 63 | 5 | 62 | 392,982 | 390,921 |
| asin | 6 | 0 | 0 | 0 | 30,984 | 49,874 |
| acos | 47,255 | 5,074 | 2,785 | 6,097 | 206,546 | 220,971 |
| exp | 135 | 0 | 14 | 14 | 1,719 | 66,536 |
| log | 4 | 0 | 0 | 0 | 71 | 11,527 |
| log2 | 113 | 0 | 0 | 0 | 34 | 431 |
| cbrt | 2,273,583 | 177,250 (7.8%) | 0 | 177,250 | 79,233 | 79,233 |

Findings:

- Apple's libm is not correctly rounded. The wasm `hypot` and `cbrt` are (they are now
  CORE-MATH-derived), so a CR port would not have helped. Apple's `pow` is close: its largest
  error is 0.5106 ulp, and it differs from CR only on near-halfway cases. Its `sin` and `cos`
  are off by up to 0.89 ulp.
- `sqrt` is IEEE everywhere. `mul_add` on wasm32 (compiler-builtins' software `fma`) matched
  the hardware `fma` on 3,000,000 random, edge and subnormal-producing inputs.

### Which function moves which corpus

On macOS with `--features graphviz-pure-core/portable-math`, one function at a time was swapped for the
`libm` crate's version, which is the code wasm32 runs. With every function swapped, the native
build failed on the same 135 files the wasm build does. Swapping one function at a time:

| function swapped | files that change |
|---|---|
| `hypot` | fdp 143, main 1 (fixed by the port below) |
| `pow` | sfdp 127, neato 3, main 1 |
| `atan2` | fdp 3 |
| `sin` | fdp 2 |
| `cos` | circo 1, fdp 3 |
| `tan`, `asin`, `acos`, `exp`, `log`, `log2` | none |

## What Apple's functions are

Candidates were compared with the system results on the inputs above:

- **`hypot`**: equals `sqrt(fma(small, small, big * big))` (with `big = max(|x|, |y|)`) on
  every corpus and random input, and on extreme, subnormal and special-value pairs. The
  variants `sqrt(x*x + y*y)`, `sqrt(fma(big, big, small²))`, `x*sqrt(1 + r²)` and musl all
  differ on 4–40% of inputs.
- **`cbrt`**: equals Apple's open-source `Libm-2026/Source/ARM/cbrt.c` compiled with clang's
  default FMA contraction, with 0 differences. Without contraction it differs on 1,038 of the
  corpus arguments. The port does not use that code (see Provenance): the corpus output is the
  same with the `libm` crate's correctly rounded `cbrt`.
- **`pow`, `sin`, `cos`, `tan`, `atan2`, `acos`, `exp`, `log`**: no match. The published
  Libm's `Source/ARM/*.s` files are empty, and the arm64 `libsystem_m` routines are
  hand-written assembly. Tried and rejected:
  - musl/fdlibm, with and without FMA. The fdlibm `__sin`/`__cos` kernels differ even for
    `|x| < π/4`.
  - musl's pow, which is ARM optimized-routines (6,218 corpus mismatches). Apple's `pow`
    indexes a 256-entry log table where ARM's uses 128.
  - Apple's Intel `sincostan.c`, which is close for `tan` (2/205) and differs for `sin`/`cos`.
  - CR implementations. CR `atan2` happens to fix the 3 fdp files.

## Approach

`graphviz_pure_core::math` (`crates/graphviz-pure-core/src/math/`) is the single entry point. Every `.hypot(`,
`.powf(`, `.sin(`, ... call in the workspace became `math::hypot(a, b)` and so on, the way C
writes `hypot(a, b)`. The rewrite was mechanical, done with a `syn`-based rewriter.

- On every target they call `math::portable` (at first only off Apple targets, where the
  `f64` methods were used):
  - `hypot`: the formula above, with exact power-of-two scaling outside `[2^-500, 2^500]`.
  - `pow`: a Rust port of CORE-MATH's correctly rounded `pow` (`portable/pow/`, see below).
    Apple's `pow` is within 0.511 ulp and differs from the correctly rounded result only near
    halfway cases, so a correctly rounded `pow` agrees with it far more often than musl's.
  - everything else, `cbrt` included: the `libm` crate 0.2.16. This is the same musl-derived
    code compiler-builtins uses for wasm32. Linux gets the same bits as wasm (glibc's
    `pow`/`exp` pick FMA or non-FMA paths by CPU), and `Cargo.lock` pins the results rather
    than the toolchain.
- `crates/graphviz-pure-core/src/math/tests.rs` checks `portable::fma` against hardware FMA and
  `libm::fma` on every target, and on macOS arm64 checks the portable `hypot` against the system
  libm on 1,000,000 random and edge inputs.

## Results

`node scripts/wasm-e2e.mjs` (see its header for the build steps) checks the wasm build against
C's hashes, rendering every graph in multi-graph files. The first `graphviz_pure_core::math` change (musl
`pow`, corpus of 2526 files at the time) moved wasm from "before" to "after":

| corpus | files | before | after | native |
|---|---:|---:|---:|---:|
| main | 514 | 512 | 513 | 514 |
| neato | 438 | 435 | 435 | 438 |
| sfdp | 297 | 170 | 170 | 297 |
| circo | 280 | 279 | 279 | 280 |
| twopi | 279 | 279 | 279 | 279 |
| fdp | 379 | 233 | 376 | 379 |
| osage | 178 | 178 | 178 | 178 |
| patchwork | 140 | 140 | 140 | 140 |
| render | 21 | 21 | 21 | 21 |
| **all** | 2526 | 2247 | 2391 | 2526 |

`gv-diff e2e` on macOS with `portable-math` failed on exactly the same 135 files as wasm, so
the remaining gap was entirely `pow`, `sin`, `cos` and `atan2`. Two ways to close it:

1. **Reproduce Apple's `pow`, `sin` and `cos` (and `atan2`) from the binary.** This is the only
   way to get bit-identical output. It means reverse-engineering closed Apple code, which
   raises a licensing question for the project. Not done.
2. **Correctly rounded functions** (CORE-MATH, MIT). Measured on macOS with `portable-math`
   (corpus of 2538 outputs, using the `core-math` crate's C build): CR `pow` alone gains 16
   files (sfdp +13, neato +3) and loses none; CR `atan2` on top gains nothing (and lowers one
   fdp score); CR `sin`/`cos` is worse than musl (circo 274 of 280). So `pow` was ported and
   the others were not.

### The CORE-MATH `pow` port

`crates/graphviz-pure-core/src/math/portable/pow/` ports `src/binary64/pow/pow.c`, `pow.h`, `dint.h` and
`qint.h` from CORE-MATH (as vendored by `core-math-sys` 1.4.0) line by line: the double-double
first phase, the 128-bit second phase with the exact/midpoint test of Lauter and Lefèvre, and
the 256-bit third phase. `tables.rs` was converted from the C initializers by a script.
Differences from the C: only rounding to nearest (Rust has no other mode), no floating-point
flags or `errno`, shifts that are undefined in C by 64 or more use the hardware's modulo-64
behaviour, and the "unexpected worst case" exit returns the 256-bit result instead.
`add_dint_11` keeps the low limb of its destination, as the C does.

Checked bit for bit against the C `cr_pow`, out of tree:

| inputs | count | differ |
|---|---:|---:|
| random bit patterns, ranges typical of the corpus, exact and midpoint cases, x near 1 | 100,000,000 | 0 |
| (of which second phase / exact-midpoint test / third phase) | 358,025 / 93,638 / 131,720 | |
| corpus `pow` arguments (`math-trace`) | 4,000,000 | 0 |
| random inputs with the C built as `POW_ITERATION=8` and the port forced likewise (third phase only) | 10,000,000 | 0 |

The debug build (overflow-checked shifts and arithmetic) ran 22,000,000 of these without a
panic. `pow/tests.rs` keeps 32 C results, 8 per phase, and the special values. The ported
output equals the C build's on all 2538 corpus outputs (`gv-diff e2e --hashes`).

### Portable results

`portable-math` on macOS, Linux and wasm32 give the same output (see Linux below). Identity with
C, by corpus (`gv-diff e2e`):

| corpus | files | Apple libm | portable, musl `pow` | portable, CORE-MATH `pow` |
|---|---:|---:|---:|---:|
| main | 526 | 526 | 525 | 525 |
| neato | 438 | 438 | 435 | 438 |
| sfdp | 297 | 297 | 170 | 183 |
| circo | 280 | 280 | 279 | 279 |
| twopi | 279 | 279 | 279 | 279 |
| fdp | 379 | 379 | 376 | 376 |
| osage | 178 | 178 | 178 | 178 |
| patchwork | 140 | 140 | 140 | 140 |
| render | 21 | 21 | 21 | 21 |
| **all** | 2538 | 2538 | 2403 | 2419 |

## Ratchet baselines per math profile (history)

While there were two profiles, every corpus ratchet kept one baseline per profile: `X.json` for
the Apple profile and `X.portable.json` for the portable one. Since 0.2.0 the `*.portable.json`
files are the only baselines, `scripts/update-baselines.sh` refreshes all of them, and the
layout goldens they compare with are made by C with the port's math functions.

These counts are from when the portable profile was introduced, against stock C on macOS; the
corpora have grown since.
The current figures are on the documentation site's Fidelity page (`scripts/site-data.py`).

| ratchet | baseline | Apple | portable |
|---|---|---:|---:|
| main (dot) | `tests/golden/baseline.json` | exact 526/526 | 525/526 |
| neato | `neato-baseline.json` | 438/438 | 438/438 |
| sfdp | `sfdp-baseline.json` | 297/297 | 208/297 |
| circo | `circo-baseline.json` | 280/280 | 279/280 |
| twopi | `twopi-baseline.json` | 280/287 | 279/287 |
| fdp | `fdp-baseline.json` | 383/383 | 379/383 |
| osage | `osage-baseline.json` | 178/178 | 178/178 |
| patchwork | `patchwork-baseline.json` | 140/140 | 140/140 |
| render | `render/baseline.json` | svg 547/547, xdot 547/547 | same |
| e2e | `e2e-baseline.json` | 2538/2538 | 2419/2538 |

(The layout ratchets count `gv-diff`'s geometric "exact" files and also guard the total score;
twopi's 7 non-exact files are not math-related.)

### Linux and wasm32

Checked in Ubuntu 24.04 containers (colima on an arm64 Mac, Rust 1.99.0), with the worktree
copied in, against the portable baselines generated on macOS. The hash comparison covers every
corpus output through the whole pipeline, matching C or not:

| platform | `cargo test --workspace` | baselines regenerated by `scripts/update-baselines.sh` | `gv-diff e2e --hashes` vs macOS `portable-math` |
|---|---|---|---|
| Linux aarch64 (native VM) | pass (debug) | byte-identical to the committed `*.portable.json` | all 2538 hashes identical |
| Linux x86_64 (QEMU user emulation) | pass (`--release`; about 2 hours) | not run (several more hours under QEMU) | all 2538 hashes identical |
| wasm32 (node, `scripts/wasm-e2e.mjs --hashes`) | n/a | n/a | all 2538 hashes identical |

So Linux and wasm compute the same bits as macOS with `portable-math`: no platform dependence
was found (the math goes through `graphviz_pure_core::math`, `sqrt`/`math::fma` are exact everywhere, and
the `libm` crate has no CPU-dependent paths on these targets). Two harness issues were fixed:

- `scripts/wasm-e2e.mjs` read inputs as JS strings, which replaced the non-UTF-8 bytes of the
  charset corpus files, so 9 of them differed from native. The new `renderAllBytes` binding
  (`graphviz-pure-wasm`) takes and returns raw bytes like the native harness (`graphviz_pure_core::charset`).
- `.cargo/config.toml` used to set `rustc-wrapper = "/opt/homebrew/bin/sccache"`, which does
  not exist on Linux; the containers used `RUSTC_WRAPPER=` (empty). The setting has since moved
  out of the repository.

CI (`.github/workflows/ci.yml`, `ubuntu-latest`): the `verify` job runs fmt, clippy,
`cargo test --workspace` (so every ratchet against its portable baseline) and the wasm32
check. The `wasm-e2e` job installs `wasm-bindgen-cli` at the `wasm-bindgen` version in
`Cargo.lock` (0.2.129, cached), builds the package, runs `scripts/wasm-e2e.mjs` against
`e2e-baseline.portable.json` (now `e2e-baseline.libm-matched.portable.json`), and `cmp`s the wasm output hashes with native Linux's.

## Is the gap only libm? C with the port's math functions

The claim above, that every portable difference comes from libm, can be tested directly: run C
with the port's portable math functions and compare. `crates/graphviz-pure-portable-libm` is a small
library that replaces every transcendental function C Graphviz imports (`sin`, `cos`, `tan`,
`asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`, `log10`, `pow`, `hypot`, `cbrt`, and
clang's combined `__sincos_stret`) with the ones `graphviz_pure_core::math` uses. It is loaded
into the unmodified C `dot` with `DYLD_INSERT_LIBRARIES` on macOS (dyld interposition) and
`LD_PRELOAD` on Linux; `scripts/dot-portable-libm.sh` does this. The remaining libm imports
(`sqrt`, `sqrtf`, `fmod`) are exact in IEEE arithmetic.

### macOS: the gap is libm, all of it

`gv-diff e2e --generate --runs 5 --c-dot scripts/dot-portable-libm.sh --golden
tests/golden/e2e.libm-matched.json` records C's output with the portable functions. Against it:

| build | vs C with Apple's libm (`e2e.json`) | vs C with the portable functions (`e2e.libm-matched.json`) |
|---|---:|---:|
| macOS, Apple profile | 2591 / 2591 | — |
| macOS `portable-math`, Linux | 2471 / 2591 | **2592 / 2592** |
| wasm32 (`scripts/wasm-e2e.mjs --golden tests/golden/e2e.libm-matched.json`) | 2471 / 2591 | **2592 / 2592** |

The denominators differ because each oracle has its own set of stable inputs: an input counts
only if C gave the same output on every run of that oracle, and C with Apple's libm and C with
the portable functions are unstable on slightly different files (12 and 11). The same holds for
the Linux table below.

So with the same math functions on both sides, the portable build and the browser build are
byte-identical to C on every stable input, including every file that differs from stock C.
The ratchet `e2e_svg_identity_with_matched_libm_does_not_regress`
(`crates/graphviz-pure-diff/tests/e2e_fidelity.rs`, portable builds only, so it runs in CI) keeps it so.
11 inputs are unstable in C (its output changed within 5 runs): the twopi `arrows` graphs and a
few fdp/sfdp files whose C layout depends on heap addresses (issues #34, #41). `--runs 5`
replaced the two-run check because two runs of `twopi/fx_arrows` sometimes agree by chance.

### Linux: C does not agree with itself

`scripts/c-oracle-linux/` builds C Graphviz 16.1.0 on Linux (Debian, arm64, in Docker) the way
Homebrew builds it on macOS: clang, LLVM's libc++ for the C++ parts, GTS for triangulation, and
the same three plugins. `run.sh` records C's output three ways and compares the port's Linux
build with each (`gv-diff e2e`, inputs stable over 5 runs):

| C on Linux | vs stock C on macOS | vs the port (portable) |
|---|---:|---:|
| stock (glibc libm, `qsort`, `rand`) | 2005 / 2586 | 2005 / 2590 |
| with the port's math functions | 2150 / 2589 | 2150 / 2591 |
| with the port's math functions and Apple's `qsort`, `rand`, `srand` (feature `apple-libc`) | 2488 / 2589 | 2488 / 2590 |

Stock C on Linux matches stock C on macOS on 77.5% of the corpus; the port's portable build
(the browser build) matches stock C on macOS on 95.4%. The Linux differences come from three
sources:

- **libm** (glibc's functions are not Apple's): 145 files.
- **libc** (338 files): glibc's `qsort` leaves equal elements in a different order than Apple's, and its
  `rand` is a different generator. With Apple's (the port's reproductions in
  `graphviz_pure_layout::cutil`, exported by `graphviz-pure-portable-libm --features apple-libc`) circo, osage,
  patchwork, sfdp and twopi match on every file.
- **the rest**: 102 files, 97 of them neato. The ones checked differ by 0.01 in a spline
  control point, a last-bit rounding difference with every node in the same place. Not
  identified. It is not the fused multiply-adds of Graphviz's own code, which clang contracts
  the same way on both (see "Fused multiply-adds" below).

Two things that were checked and are not the cause: the clang version (14 and 19 give the same
results), and `long double`, which is 128-bit on Linux aarch64 and 64-bit on Apple arm64 (the
only use, neato's `stress_majorization_kD_mkernel` accumulator, makes no difference on the
corpus). The first Linux build lacked GTS, which changes neato's and sfdp's overlap removal; the
Dockerfile now checks that GTS is configured.

### Linux x86-64: the reference corpus reproduces with `-mfma`

On x86-64 the same Dockerfile builds with `CFLAGS='-O2 -mfma'`. clang (14 in the image, 19
natively) defaults to `-ffp-contract=on` on x86-64 as on arm64, but fuses only for a target with
an FMA instruction, so without `-mfma` C on x86-64 fuses nothing (see "Fused multiply-adds"
below). With it, the port's math and Apple's `qsort`, `rand` and `srand` (`apple-libc`), C's
`-Tsvg` (`gv-diff e2e --generate --runs 5`) equals `e2e.libm-matched.json` on 2588 of its 2595
stable inputs, against 2093 without `-mfma`:

| corpus | stable inputs | without `-mfma` | with `-mfma` |
|---|---:|---:|---:|
| main | 566 | 557 | 565 |
| neato | 442 | 284 | **442** |
| sfdp | 296 | 137 | 295 |
| fdp | 386 | 216 | 383 |
| circo | 282 | 279 | 281 |
| twopi | 281 | 281 | **281** |
| osage | 180 | 178 | 179 |
| patchwork | 141 | 141 | **141** |
| render | 21 | 20 | **21** |
| **total** | **2595** | **2093** | **2588** |

The `-Tdot` references (`scripts/gen-golden.sh`) are identical for 2575 of 2585 files, all of
them apart from the 6 inputs unstable on macOS too and 4 of the 7 below. In the render corpus
(`scripts/gen-render-golden.sh`) `laid/`, `dot/` and `json0/` are identical; `svg/`, `xdot/`
and `json/` differ only on the NaN input below, and `plain/` and `plain-ext/` also on 12
Latin-1 or `Symbol` inputs whose label C prints from freed memory (`canon` in
`lib/common/output.c`). The render ratchet already skips 10 of those as unstable; on Linux
`gv16_graphs_Latin1_cd0c3d` and `gv16_graphs_b60_bf5b3a` print garbage too.

The 7 `-Tsvg` inputs that still differ:

- **heap addresses** (3, fdp: `wild_gv16_graphs_b15_8c212e`, `b51_33bcb0`, `badvoro_2e8cb8`):
  fdp orients each edge of its derived graph by comparing node addresses
  (`lib/fdpgen/layout.c:466`, #34), and glibc's `malloc` returns them in another order than
  macOS's, which the port models (`NodeHeap`). On `badvoro` C applies the attraction of
  `washdc80 -- washdc82` from the other end in the first iteration, so `xdelta` changes sign
  and the fused `DISP += xdelta * force` rounds differently. C's result changes with
  `GLIBC_TUNABLES=glibc.malloc.tcache_count=0`.
- **NaN sign** (1, `main/fuzz_cylinder_diagonals_nan`): x86-64's default NaN has the sign bit
  set, arm64's does not, and glibc prints it (`-nan`, Apple's libc `nan`).
- **NaN to `int`** (1, `osage/fuzz_pack_degenerate_box`): `computeStep` (`lib/pack/pack.c`)
  converts a NaN root with `(int)`, which is undefined. x86-64 gives `INT_MIN`, so `putRects`
  returns NULL and osage dereferences it (segfault); arm64, like Rust's `as`, gives 0.
- **C run to run** (1, `sfdp/wild_gv16_linux_x86_neatosplines_neato_c2c6f6`): GTS's edge
  order, 1 run in 10 differs (`docs/porting/sfdp.md`); the other runs equal the reference.
- **time** (1, `circo/wild_gv16_graphs_badvoro_2e8cb8`): C takes 79 s on the x86-64 machine
  measured, over `gv-diff`'s 60 s limit; it gave the reference's output in 10 of 10 runs.

`long double` makes no difference here either: building with `APPLE_LONG_DOUBLE=1` or without
gives the same neato results (x86-64's `long double` is 80-bit).

So C on Linux x86-64 can make the references of a new corpus input, with three exceptions that
still need the macOS set-up: fdp inputs (heap order, 3 of 386 differ), an input whose output
prints a NaN or whose C converts a NaN to an integer, and the stock-C files (`e2e.json`,
`e2e-baseline.portable.json`, `structure.json`), which need Apple's libm.
`scripts/fma-sites.sh` needs Homebrew's dylibs.

## Fused multiply-adds: the reference build decides

The math library is not the only place where two builds of the same C compute different bits.
A compiler may contract `a * b + c` into one fused multiply-add, which rounds once instead of
twice. Clang does this by default (`-ffp-contract=on`) within one expression, and only on
targets with an FMA instruction. So C Graphviz computes different bits on Apple Silicon, where
every `fmadd` is used, and on x86-64 Linux, whose baseline instruction set has none.

**The reference is Homebrew's Graphviz 16.1.0 on macOS arm64**, the build the corpus goldens
come from (`scripts/gen-golden.sh`). The port computes what that build computes, on every
target: where it fuses, the port calls `math::fma` explicitly (`f32::mul_add` for `f32`); where
it does not, the port does not. `math::fma` is correctly rounded on every target: on `wasm`
(`wasm32`/`wasm64`) and on `x86_64` musl without `target_feature = "fma"` (where `f64::mul_add`
calls an out-of-line integer `fma`) it calls `math::portable::fma` (Boldo–Melquiond emulated FMA
in the common exponent window, with a zero-factor shortcut and `libm::fma` outside it, inlined on
`x86_64` musl and out-of-line on `wasm`; see `docs/perf/wasm-fma.md`), and everywhere else it
calls `f64::mul_add` (hardware FMA, or glibc's IFUNC-dispatched `fma`). So the port's bits do not
depend on the platform. Rust never contracts on its own, and clippy's `disallowed-methods`
rejects direct `f64::mul_add` calls outside `math`.

How a site is decided: within one C expression, clang fuses a product that is an operand of an
addition or subtraction, preferring the left operand. So `a*b + c*d` is `fma(a, b, c*d)`,
`c - a*b` is `fma(-a, b, c)`, `x += a*b` is `fma(a, b, x)`, and
`a*b + c*d + e*f` is `fma(e, f, fma(a, b, c*d))`. Macros count (`DIST2`, `SQR`, `ROUND`,
`POINTS`: `ROUND(x * 72)` tests the sign of the rounded product, then fuses `x * 72 ± .5`, which
is `graphviz_pure_core::cnum::c_round_mul`). A product by a power of two is exact, so it is left
unfused. The port's code says `C (clang) fuses …` at each site.

`scripts/fma-sites.sh` lists the sites. Homebrew's dylibs have no debug information, so it
builds the same sources with the same compiler and Homebrew's flags (`-Os -g`; at `-O2` clang
inlines differently), checks that every function has the same fused instructions as the
installed dylibs, and maps each instruction to its source line. For 16.1.0 the reference build has
616 fused instructions (589 scalar `fmadd`/`fmsub`/`fnmadd`/`fnmsub`, 27 vector `fmla`/`fmls`)
and 38 `__sincos_stret` calls, on 466 source lines (18 of them line 0, code the debug
information gives no line for) in `libgvc`, `libpathplan`, and the core, dot
and neato plugins. The audit for C11 (#89, #90, #96) checked every line against the port (rows
of the audit; a row can cover several lines, such as the two coordinates of one point):

| status | rows |
|---|---:|
| the port already fused it | 259 |
| fused by C11 (#96's folder corner, render arrows, ellipses, tapers, gradients, polygon outlines, HTML tables, `recPt`, `Initial_dist`, `medians`, dot's flat-edge boxes, patchwork, `POINTS`) | 77 |
| exact either way (a product by a power of two or ±1, or of integers too small to round) | 35 |
| not ported (paging, z output, `Verbose` output, gvevent, tk, edgepaint and gvmap helpers) | 23 |
| `__sincos_stret` (below) | 33 |

The `Verbose` output was ported later (#416). Its fused lines are `computeStep`'s `r1` and
`r2` (`pack.c:93-94`), `total_e` (`stuff.c:404-408`) and sgd's `calculate_stress`
(`sgd.c:23`), and `compute_stressf`'s sum (`stress.c`, a site the debug information gives
line 0); the port fuses them as C does. patchwork's `-v` rectangles (`patchwork.c:181-182`) are
fused too, by products with 0.5, which are exact either way.

Every `math::fma` / `f32::mul_add` in the port is at a site C fuses, except three
`(-1.0f32).mul_add(x, c)` in neato's `cola.rs`, `digcola.rs` and `stress.rs`, where C subtracts;
they are exact.

`__sincos_stret` is clang merging a `sin(a)` and a `cos(a)` into one call. It needs nothing in
the port: `graphviz-pure-portable-libm` interposes it with the portable `sin` and `cos`, so C
with the port's math computes the same values whether or not clang merged the calls.

**GTS**, which neato uses for triangulation, is contracted by the same compiler (checked by
rebuilding GTS 0.7.6's `predicates.c` with `-Os -g` and comparing the fused instructions per
function with Homebrew's `libgts`; `scripts/fma-sites.sh` does not cover it). Shewchuk's
`incircle` fast stage fuses its lifts, `det` and `permanent`; `neatogen/robust.rs` does too,
with the error-bound constants GTS's generated `predicates_init.h` stores (`%.16g`, one ulp
above the exact value), so the port computes C's `det` bit for bit. No output depends on it:
GTS and the port use only the sign of `orient2d` and `incircle`, and the predicates make the
sign exact however the fast stage rounds.

**Other C builds disagree with the reference.** C on x86-64 Linux has no fused operations at
all unless it is built with `-mfma`, as `scripts/c-oracle-linux/` now does on x86-64 (see "Linux
x86-64" above; for sfdp, `docs/porting/sfdp.md`, "Comparing on Linux x86-64"). On Debian arm64
the same oracle builds Graphviz with clang, which contracts the same expressions as on macOS:
contraction is decided per expression by the compiler's front end, not by the optimisation
level. It links Debian's GTS, which is compiled differently (in `libgts-0.7.so.5` of Debian
bookworm arm64, `orient2d` has no fused instruction, 8 in Homebrew's, and `incircle` has 7, 134
in Homebrew's), but since only the predicates' signs are used, that cannot explain the Linux
differences above. None of these
builds is the reference; the port follows the macOS arm64 build.

## Provenance and licences

Everything under `crates/graphviz-pure-core/src/math/`:

| file | what | origin | licence |
|---|---|---|---|
| `mod.rs` | the `math::*` entry points | written for this port | project licence |
| `portable/mod.rs` | re-exports of the `libm` crate | written for this port | project licence |
| `portable/fma.rs` | correctly rounded emulated `fma` (Boldo–Melquiond fast path + `libm::fma` fallback) | written for this port from the published algorithm (IEEE TC 57(4):462–471, 2008) | project licence |
| `portable/pow/` | correctly rounded `pow` | Rust port of CORE-MATH `src/binary64/pow/` (Tom Hubrecht, Paul Zimmermann; Copyright (c) 2022-2025 CERN and Inria), tables converted mechanically; licence text kept in `pow/mod.rs` | MIT |
| `portable/hypot.rs` | Apple-equivalent `hypot` | an original formula identified by black-box comparison with the system libm; no code copied | project licence |
| `trace.rs` | `math-trace` argument logging | written for this port | project licence |
| `tests.rs`, `portable/pow/tests.rs` | portable `fma` vs hardware/`libm::fma`; portable `hypot` vs system libm; `pow` regression values from the C | written for this port | project licence |
| `libm` crate 0.2.16 (dependency) | `atan2`, `sin`, `cos`, `tan`, `asin`, `acos`, `exp`, `log`, `log2`, `cbrt`, fallback `fma` | rust-lang/compiler-builtins: musl/fdlibm ports; `cbrt` is CORE-MATH's (Alexei Sibidanov) | MIT |

No Apple source is shipped. An earlier version ported Apple Libm's `Source/ARM/cbrt.c`, which
has no licence header and comes from an APSL release; it was replaced by the `libm` crate's
`cbrt`. On macOS with `portable-math`, the swap left every corpus output unchanged: the per-file
scores of all eight layout ratchets, the render ratchet, and the `gv-diff e2e --hashes` hashes
of all 2538 outputs were identical before and after. Only pathplan's cubic solver calls `cbrt`.

Measurement tools: `scripts/libm-probe.c` (Apple vs MPFR) and the `math-trace` feature.
