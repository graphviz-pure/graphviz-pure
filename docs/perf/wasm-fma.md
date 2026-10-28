# An exact software FMA for wasm32 (T1b)

wasm32 has no deterministic scalar fused multiply-add, so every `f64::mul_add` and
`f32::mul_add` in the port (389 call sites) is a call into a software `fma`/`fmaf`. This note
measures what that costs under node, surveys the exact algorithms, and tests a prototype of the
best one: Boldo and Melquiond's emulated FMA, which uses error-free transformations and rounding
to odd, behind an exponent-window check. The prototype returns the same bits as `libm::fma` on
every one of 9.4 × 10^10 test triples and on the 717 M real `mul_add` calls it covers. It is
**1.9× faster** than the builtin `fma` in a wasm microbenchmark, and takes **1.3–1.6× off the
user cycles** of fdp, sfdp and spline-routing runs under node, with byte-identical output.
Catalogue row: T1b in [`docs/roadmap/post-1.0.md`](../roadmap/post-1.0.md). Baseline:
[`baseline.md`](baseline.md#wasm).

The end-to-end measurements below predate the merged `pow` fast paths (#543, #532).
Those paths remove much of sfdp's measured FMA work; the reported sfdp gain is historical,
not a measurement of the current port. The software FMA prototype remains research.

## Headlines

- **fma is most of fdp's and sfdp's WASM time.** Under node 24 (`--cpu-prof`, WASI CLI),
  `fma` plus the `scalbn` that musl's `fma` calls at its end take 60% of the samples of fdp on
  `viewport` (53% + 7%) and 76% of sfdp on `gen_rnd2000` (67% + 8%). `fmaf` is 15% of neato.
  That is why fdp is 4.9× and sfdp 3.5× slower than native in the baseline, while dot and
  neato are 1.1–1.5×.
- **The two wasm targets use different `fma`s.** On `wasm32-unknown-unknown` (the npm/web
  build), `mul_add` calls compiler-builtins' Rust port of musl's `fma`: 640 wasm instructions,
  38 branches, plus a `__multi3` call for the 128-bit product. On `wasm32-wasip1` (the WASI
  CLI), it calls wasi-libc's C `fma` from musl: 587 instructions, 31 branches, plus `scalbn`.
  Both take about 14 ns per call under node. They disagree on one thing: when `a·b` underflows
  to zero and `c = +0`, wasi-libc returns `+0` where IEEE 754 (and `libm::fma`) returns `-0`.
  The port never reaches that case (see below), but it is a cross-target difference today.
- **The port's operands always fit a fast path.** On 14 inputs covering all engines (728 M f64
  calls), 98.5% of calls have both factors and the addend in a window where the emulated FMA is
  exact, and the other 1.5% have a zero factor, where `a*b + c` is already exact. Not one call
  had a subnormal, infinite or NaN operand or a subnormal result.
- **Prototype: 7.2 ns per call against 13.7–14.1 ns** (1.9×), 30 floating-point operations and
  6 branches, safe Rust, no dependencies beyond `libm` (already used) for the rare fallback.
- **End to end under node** (user cycles, `perf stat`, median of 3; the machine was at load
  40–86, so wall times are noisy): sfdp 1.4–1.5×, fdp 1.1–1.3×, twopi with splines 1.4×, and
  dot, neato, circo, osage and patchwork unchanged. Instruction counts fall by 1.5–2.0× on fdp,
  sfdp and spline routing. Every output is byte-identical.
- **The f32 side is already cheap.** musl's `fmaf` (one exact f64 product, one f64 add and a
  rare fix-up) costs 1.5 ns against 1.0 ns for an unfused `a*b + c`. A round-to-odd `fmaf` is
  slower (2.9 ns). Inlining `libm::fmaf` cut neato's instructions by 22%, but its cycles did
  not move measurably on this machine.
- **Recommendation:** do it, for f64 only, as `math::fma` in `graphviz-pure-core` with the
  fast path compiled on wasm32 only. One design decision is open (inline the fast path at all
  389 sites, +139 KB, about +1.3% of the WASI module, or call it out of line, −29 KB with
  40–60% of the gain on fdp and sfdp); see [Recommendation](#recommendation).

## 1. What `mul_add` compiles to on wasm32

A scratch `cdylib` exporting `a.mul_add(b, c)` for f64 and f32, built `--release` (opt-level 3,
fat LTO) with rustc 1.99.1 and disassembled with the toolchain's `llvm-objdump -d`:

| target | f64 `mul_add` calls | size (wasm instructions) | branches (`br_if`/`if`/`select`) | calls inside |
| :-- | :-- | --: | --: | :-- |
| `wasm32-unknown-unknown` | `compiler_builtins::math::libm_math::fma::fma` (libm's `generic::fma_round`, a port of musl `fma.c`) | 640 | 38 | `__multi3` (61 instructions: the 64×64→128 product as four 32-bit multiplies) |
| `wasm32-wasip1` | `fma` from wasi-libc's `libc.a` (musl `src/math/fma.c`, C) | 587 | 31 | `scalbn` (79 instructions) on every call |

| target | f32 `mul_add` calls | size | branches |
| :-- | :-- | --: | --: |
| `wasm32-unknown-unknown` | `fmaf` (libm `generic::fma_wide_round`) | 75 | 6 |
| `wasm32-wasip1` | `fmaf` (musl `fmaf.c`) | 77 | 7, and `fegetround` on the rare path |

The same holds in the real WASI CLI (`graphviz-pure.wasm`): `mul_add` resolves to wasi-libc's
`fma`/`fmaf`. Both are integer algorithms: unpack, multiply the significands into 128 bits,
align and add the addend with a sticky bit, normalise with `clz`, and round by an integer→float
conversion plus `scalbn`. wasm's `i64.mul` only gives the low 64 bits of a product, so the
128-bit product costs four multiplies and the carries. The `wide-arithmetic` proposal
(`i64.mul_wide_u`) would fix that, but it is enabled by default only in Firefox (from version
155) and not in V8, so node and Chromium can't use it ([Mozilla intent to ship](https://groups.google.com/a/mozilla.org/g/dev-platform/c/JgW2B9k7HzI),
[proposals list](https://github.com/webassembly/proposals)).

### How much of the time is fma

WASI CLI built with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` and `STRIP=none` (for the
names), run under node 24.22.1's `node:wasi` with `--cpu-prof --cpu-prof-interval 200`.
Self time, as a share of samples:

| input | `fma` | `scalbn` | `fmaf` | top other function |
| :-- | --: | --: | --: | :-- |
| fdp/`wild_gv16_graphs_viewport` | 52.8% | 7.0% | – | `fdp_x_layout` 33.8% |
| fdp/`wild_gv16_graphs_b69` | 43.1% | 5.3% | – | `fdp_x_layout` 30.4% |
| sfdp/`gen_rnd2000` | 67.4% | 8.2% | – | `math::portable::pow` 10.3% |
| sfdp/`gen_grid20x20` | 61.3% | 7.7% | – | `pow` 9.2% |
| neato/`hw3_vpsc_random200` | – | – | 15.3% | `right_mult_with_vector_ff` 47.2% |
| neato/`hw2_random_120` | – | 1.2% | 13.1% | `right_mult_with_vector_ff` 41.5% |

wasmtime is not installed on this machine, so there are no wasmtime numbers.

## 2. Exact algorithms

"Exact" here means the result is `RN(a·b + c)`, the correctly rounded value IEEE 754 defines,
so any exact algorithm returns the same bits as hardware FMA and as `libm::fma`. NaN payloads
are the one exception: the port never prints a NaN's bits.

### 2.1 Integer significand arithmetic (musl, compiler-builtins)

The algorithm of musl's [`src/math/fma.c`](https://git.musl-libc.org/cgit/musl/tree/src/math/fma.c)
and its Rust port in libm ([`libm/src/math/generic/fma.rs`](https://github.com/rust-lang/compiler-builtins/blob/master/libm/src/math/generic/fma.rs)):
exact for all inputs, including subnormals, infinities and NaN, by construction. It is a
128-bit integer computation with one final rounding. Cost on wasm32: the 600-instruction bodies
above, with a 128-bit multiply emulated from 32-bit halves and many data-dependent branches
(alignment shift ranges, sign of the sum, normalisation, subnormal result). It needs no
preconditions, which makes it a good fallback but a slow common case.

### 2.2 f32 via f64 (musl `fmaf.c`, libm `fma_wide_round`)

For binary32 operands, `a·b` is exact in binary64 (24 + 24 = 48 ≤ 53 bits, and the exponent
range fits), so the only rounding problem is the double rounding of `RN64(xy + z)` followed by
`RN32`. musl's [`fmaf.c`](https://git.musl-libc.org/cgit/musl/tree/src/math/fmaf.c) returns the
binary64 sum directly unless its low 29 bits are exactly the halfway pattern `0x10000000`. In
that case it decides the direction with an exactness test and nudges the binary64 result one
ulp. It is exact for all inputs. The common case is one multiply, one add, one test. The rare
path ran on 2.4–3.0% of neato's f32 calls.

An alternative is to round the binary64 sum to odd (TwoSum, then fix the last bit) and convert
once. Boldo and Melquiond prove that `RN_p(RO_q(x)) = RN_p(x)` when `q ≥ p + 2`, and 53 ≥ 24 + 2.
It is exact, but it was **slower** here (2.9 ns against 1.5 ns), because it always pays for
TwoSum.

### 2.3 Emulated FMA with error-free transformations and rounding to odd (Boldo–Melquiond)

S. Boldo and G. Melquiond, *Emulation of a FMA and correctly-rounded sums: proved algorithms
using rounding to odd*, IEEE Transactions on Computers 57(4):462–471, 2008,
[doi:10.1109/TC.2007.70819](https://doi.org/10.1109/TC.2007.70819),
[HAL inria-00080427](https://hal-ens-lyon.archives-ouvertes.fr/inria-00080427). The properties
are proved in Coq. In radix 2, with `RN` the working precision and `RO` rounding to odd:

```
(uh, ul) = ExactMult(a, b)    // uh + ul = a·b exactly       (Dekker's TwoProduct, Veltkamp's split)
(th, tl) = TwoSum(c, uh)      // th + tl = c + uh exactly    (Knuth/Møller)
v        = RO(tl + ul)
result   = RN(th + v)         // = RN(a·b + c)
```

The theorem assumes that nothing overflows or underflows. Rounding to odd is not an IEEE
rounding mode, but it is cheap to emulate: compute `(s, e) = TwoSum(x, y)`. If `e ≠ 0` and `s`
has an even significand, step `s` one ulp towards `e`. Otherwise keep `s`. Kornerup, Lefèvre,
Louvet and Muller show a test-free emulation with round-to-nearest additions and a
multiplication by 0.5 (*On the computation of correctly rounded sums*, IEEE Transactions on
Computers 61(3):289–298, 2012,
[HAL ensl-00331519](https://hal-ens-lyon.archives-ouvertes.fr/ensl-00331519)). Graillat and
Muller (*Emulation of the FMA and the correctly-rounded sum of three numbers in
rounding-to-nearest floating-point arithmetic*, Numerische Mathematik 157:1615–1641, 2025,
[doi:10.1007/s00211-025-01487-2](https://doi.org/10.1007/s00211-025-01487-2)) emulate the FMA
with round-to-nearest additions, multiplications and comparisons only. I did not prototype
either: the bit-twiddling version below is already cheap, and a branch-free variant measured
slower (see §4).

Building blocks (all exact, and all pure IEEE `+ − ×`, which the wasm spec requires to round to
nearest on every engine):

- **Veltkamp split** of `a` into two 26-bit halves (`c = (2^27 + 1)·a; h = c − (c − a); l = a − h`):
  4 flops. It overflows if `|a| ≥ 2^996` or so.
- **Dekker's TwoProduct** (T. J. Dekker, *A floating-point technique for extending the available
  precision*, Numerische Mathematik 18:224–242, 1971): `p = a·b`, `e = ((ah·bh − p) + ah·bl + al·bh) + al·bl`,
  so 2 splits + 7 flops = 15 flops. `e` is exact as long as it does not underflow, which needs
  `ulp(a)·ulp(b) ≥ 2^−1074`.
- **TwoSum** (Knuth, TAOCP vol. 2, §4.2.2): 6 flops, exact unless it overflows (even with
  subnormals).

Fast-path window used by the prototype, checked on the biased exponents with integer compares:

| operand | condition | why |
| :-- | :-- | :-- |
| `a`, `b` | normal, `|a|, |b| < 2^996` | the split must not overflow |
| `a·b` | `ea + eb ∈ [−900, 1018]` (unbiased) | the product and its error must be normal; the sum must stay below `2^1023` |
| `c` | `0`, or `2^−900 ≤ |c| < 2^1022` | TwoSum must not overflow; no subnormal grid |

Inside the window every nonzero intermediate is an integer multiple of `2^−1005` (the error grid
of the product is `2^(ea+eb−104)`, that of `c` is `ulp(c)`) and below `2^1023` in magnitude. So
every nonzero intermediate is a normal number, nothing underflows or overflows, and the theorem
applies as stated. Outside the window the prototype calls `libm::fma`. In the fallback, a zero
factor returns `a*b + c`, which is exact because the product is exactly ±0, and IEEE's
signed-zero rules for `+` are the same as for FMA.

Cost on wasm32 (prototype, disassembled): 169 instructions in the exported function, of which
30 are flops (7 `f64.mul`, 23 `f64.add`/`f64.sub`), 6 are branches, and most of the rest are
`local.get`/`local.tee`, which the JIT turns into registers. That compares with 600+
instructions, 31–38 branches and one or two calls in the builtins.

### 2.4 Variants measured and rejected

| variant | ns/call (node, wasm32-unknown-unknown) |
| :-- | --: |
| builtin `f64::mul_add` (compiler-builtins) | 13.7 |
| `libm::fma` | 14.1–17.6 |
| **prototype: integer window check + Dekker + branchy round-to-odd** | **7.2** |
| same, branch-free round to odd (`(bits − toward_zero) | inexact`) | 8.3 |
| TwoProduct error term from the low 64 bits of `i64.mul` (`E = ma·mb − (mp << k)`) | 8.1 |
| window checked with float compares on `p = a·b`, NaN result as the split-overflow detector | 7.6 |
| no window check at all (lower bound; not exact for all inputs) | 5.4 |
| unfused `a*b + c` | 0.93 |

The window check costs about 1.8 ns of the 7.2. The data-dependent branch in round-to-odd is
cheaper under V8 than the branch-free version.

## 3. The operand ranges the port sees

An instrumented build, which was **not committed** (a `sed` rewrite of every `.mul_add(` into a
`#[track_caller]` trait method that classifies the operands, counts call sites, and checks the
prototype's fast path against hardware FMA), was run natively on the quick set
(`scripts/bench/sets/quick.tsv`) plus the inputs profiled above. Every SVG was byte-identical
to the uninstrumented build.

| input | f64 calls | in window | zero factor | other | f32 calls | f32 calls on musl's rare path | hottest f64 site |
| :-- | --: | --: | --: | --: | --: | --: | :-- |
| dot/unix | 87,555 | 73.5% | 26.5% | 0 | 0 |  | `geom/src/pathplan/triang.rs:11` |
| dot/gv16_graphs_b143 | 7,063,063 | 78.4% | 21.6% | 0 | 0 |  | `geom/src/pathplan/triang.rs:11` |
| dot/dot_dag_1000 | 4,515,187 | 75.3% | 24.7% | 0 | 0 |  | `geom/src/pathplan/triang.rs:11` |
| neato/hw2_random_120 | 32,842 | 68.8% | 31.2% | 0 | 66,755,400 | 3.0% | `layout/src/neatogen/matrix_ops.rs:51` |
| neato/hw3_vpsc_random200 | 90,841 | 65.8% | 34.2% | 0 | 121,008,400 | 2.4% | `layout/src/neatogen/matrix_ops.rs:51` |
| fdp/wild_gv16_graphs_b69 | 25,769,823 | 99.9% | 0.1% | 0 | 0 |  | `layout/src/fdpgen/xlayout.rs:190` |
| fdp/wild_gv16_graphs_viewport | 239,770,342 | 100.0% | 0.0% | 0 | 0 |  | `layout/src/fdpgen/xlayout.rs:191` |
| sfdp/gen_grid20x20 | 43,303,316 | 98.0% | 2.0% | 0 | 0 |  | `core/src/math/portable/pow/mod.rs:75` |
| sfdp/gen_rnd2000 | 384,148,775 | 98.2% | 1.8% | 0 | 0 |  | `core/src/math/portable/pow/mod.rs:75` |
| circo/wild_gv16_directed_sdh | 52,243 | 72.6% | 27.4% | 0 | 0 |  | `layout/src/common/splines.rs:30` |
| twopi/fx_arrows | 22,612,240 | 99.4% | 0.6% | 0 | 0 |  | `geom/src/pathplan/visibility.rs:10` |
| twopi/twopi_tree_5000 | 844,967 | 67.4% | 32.6% | 0 | 0 |  | `layout/src/common/splines.rs:30` |
| osage/wild_gv16_graphs_b51 | 34,568 | 73.3% | 26.7% | 0 | 0 |  | `layout/src/common/splines.rs:29` |
| patchwork/wild_gv16_graphs_b51 | 1,949 | 75.3% | 24.7% | 0 | 0 |  | `geom/src/node_size.rs:1201` |
| **total** | **728,327,711** | **98.5%** | **1.5%** | **0** | 187,763,800 | | |

- The fast path plus the zero-factor shortcut covers **every** f64 call. No operand was
  subnormal, infinite or NaN, and no result was subnormal. The window is far wider than needed:
  the product exponents seen lie in `[−200, 100)`, and `c`'s exponent is almost always within
  ±60 of the product's (a few calls, in dot and twopi routing, exceed ±120).
- The fast-path result equalled hardware FMA on all **717,435,142** in-window real calls.
- On sfdp, 90% of the fma calls are inside CORE-MATH `pow` (`pow/mod.rs:75`, the double-double
  product). The `pow(x, 2)` fast path of #543 (T2) removes nearly all of those `pow` calls, so
  **#543 has since reduced sfdp's fma share, so T1b's current sfdp gain needs re-measuring**. fdp
  (`xlayout.rs` and `hypot`) and spline routing (`visibility.rs`, `route.rs`, `triang.rs`) are
  not affected by #543.

## 4. Prototype and tests

The prototype is a scratch crate outside the repository (`#![forbid(unsafe_code)]`, depends on
`libm = "=0.2.16"`, the version in `Cargo.lock`). The function under test:

```rust
const SPLIT: f64 = 134_217_729.0; // 2^27 + 1

fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

fn split(a: f64) -> (f64, f64) {
    let c = SPLIT * a;
    let h = c - (c - a);
    (h, a - h)
}

/// Round to odd of the exact sum `s + e` (a TwoSum pair).
fn odd(s: f64, e: f64) -> f64 {
    let b = s.to_bits();
    if (e > 0.0 || e < 0.0) && b & 1 == 0 {
        // `s != 0` (an inexact sum is never zero); growing the bits grows the magnitude.
        f64::from_bits(if (e > 0.0) == (s > 0.0) { b + 1 } else { b - 1 })
    } else {
        s
    }
}

fn in_window(a: f64, b: f64, c: f64) -> bool {
    let ea = ((a.to_bits() >> 52) & 0x7ff) as u32;
    let eb = ((b.to_bits() >> 52) & 0x7ff) as u32;
    let cb = c.to_bits() & !(1 << 63);
    let ec = (cb >> 52) as u32;
    let ab_ok = ea.wrapping_sub(1) < 2018 && eb.wrapping_sub(1) < 2018; // normal, < 2^996
    let p_ok = (ea + eb).wrapping_sub(2046 - 900) <= 1918; // ea + eb - 2046 in [-900, 1018]
    let c_ok = cb == 0 || ec.wrapping_sub(123) <= 2044 - 123; // 0 or 2^-900 <= |c| < 2^1022
    ab_ok & p_ok & c_ok
}

pub fn fma(a: f64, b: f64, c: f64) -> f64 {
    if in_window(a, b, c) {
        let p = a * b;
        let (ah, al) = split(a);
        let (bh, bl) = split(b);
        let ul = ((ah * bh - p) + ah * bl + al * bh) + al * bl; // a·b = p + ul exactly
        let (th, tl) = two_sum(c, p);
        let (vs, ve) = two_sum(tl, ul);
        th + odd(vs, ve)
    } else {
        fma_slow(a, b, c)
    }
}

#[cold]
#[inline(never)]
fn fma_slow(a: f64, b: f64, c: f64) -> f64 {
    // A zero factor makes the product exact, so the unfused expression is the FMA.
    if (a == 0.0 || b == 0.0) && a.is_finite() && b.is_finite() { a * b + c } else { libm::fma(a, b, c) }
}
```

### Differential test

Native x86-64 build with `-C target-cpu=native`, so `f64::mul_add` is a `vfmadd` instruction.
The reference is that hardware FMA on every sample, plus `libm::fma` on every 16th sample.
6 threads, 15.5 minutes. Then the same test was built for `wasm32-wasip1` and run under node
against `libm::fma` alone (the generic integer algorithm, the one compiler-builtins uses on
wasm32). The edge-value cross product is 56 values (±0, ±min subnormal, ±max subnormal, ±min
normal, ±1, 1 ± ulp, ±max, ±inf, NaN, every window edge and its neighbour, `SPLIT`, …) taken
three at a time: 175,616 triples, for f64 and for f32 (narrowed).

| class (f64 unless marked) | native samples | fast path taken | mismatches | wasm samples (vs `libm::fma`) | mismatches |
| :-- | --: | --: | --: | --: | --: |
| edge cross product | 175,616 | – | 0 | 175,616 | 0 |
| random bit patterns | 8,589,934,590 | 65.9% | 0 | 134,217,728 | 0 |
| random in window (exponents ±100, `c` within ±60 of `a·b`) | 8,589,934,590 | 100% | 0 | 134,217,728 | 0 |
| sparse significands near each other (ties, sticky bits, runs of ones) | 8,589,934,590 | 100% | 0 | 134,217,728 | 0 |
| `c = −RN(a·b)` (the EFT use in `pow`) | 8,589,934,590 | 100% | 0 | 134,217,728 | 0 |
| `c` within ±32,768 ulps of `−a·b` (cancellation) | 8,589,934,590 | 100% | 0 | 134,217,728 | 0 |
| exponent gaps of ±1,100 between `c` and `a·b` | 8,589,934,590 | 87.4% | 0 | 134,217,728 | 0 |
| exponents at every window and format edge ±2 | 8,589,934,590 | 6.6% | 0 | 134,217,728 | 0 |
| subnormal and tiny operands and results | 8,589,934,590 | 0% | 0 | 134,217,728 | 0 |
| f32 random bit patterns (round-to-odd `fmaf`) | 8,589,934,590 | – | 0 | 134,217,728 | 0 |
| f32 sparse significands | 8,589,934,590 | – | 0 | 134,217,728 | 0 |
| f32 cancellation | 8,589,934,590 | – | 0 | 134,217,728 | 0 |
| **total** | **9.4 × 10^10** | | **0** | **1.5 × 10^9** | **0** |

The zero-factor shortcut in `fma_slow` was added after that run. A second native run of the
code exactly as listed above (2^30 samples per class, 1.2 × 10^10 in all) and a wasm run of
2^24 per class also found 0 mismatches.

The test can fail. With the round-to-odd step removed (`th + vs`), it reports 13,142
mismatches in 4.6 × 10^7 samples (sparse, exponent-gap and window-edge classes). With the
f32 round to odd removed, it reports 38,632 in the f32 cancellation class. Widening the window
to products down to `2^−1060` produces subnormal-result mismatches at once.

The wasm run also compared wasi-libc's `fma` (what `f64::mul_add` is on wasip1) with
`libm::fma`. They differ only on the sign of a zero produced by total underflow with `c = +0`
(for example `fma(2.2e−308, −2.2e−308, +0)`: wasi-libc `+0`, IEEE and libm `−0`). The prototype
agrees with libm.

### Microbenchmark (wasm32, node 24.22.1)

`fastfma-bench`, a `cdylib` over the prototype: 4,096 triples with mixed signs and magnitudes
`2^±20`, 2,000 passes, median of 7 after a warm-up, opt-level 3, fat LTO:

| | wasm32-unknown-unknown | wasm32-wasip1 |
| :-- | --: | --: |
| `f64::mul_add` (builtin) | 13.7 ns | 14.1 ns |
| `libm::fma` | 14.1 ns | 14.4 ns |
| **prototype `fma`** | **7.2 ns (1.9×)** | **7.3 ns (1.9×)** |
| unfused `a*b + c` | 0.93 ns | 0.95 ns |
| `f32::mul_add` (builtin) | 1.51 ns | 1.58 ns |
| `libm::fmaf` | 1.56 ns | 1.63 ns |
| round-to-odd `fmaf` | 2.87 ns | 2.97 ns |
| unfused f32 `a*b + c` | 1.02 ns | 1.04 ns |

### End to end under node

A second throwaway worktree rewrote every `.mul_add(` into a trait method backed by the
prototype and built the WASI CLI (`cargo build --release --target wasm32-wasip1`), in four
variants:

- `fast`: the prototype inlined at every site, f32 unchanged (wasi-libc `fmaf`);
- `fast-fmaf`: the same, plus f32 through the inlinable `libm::fmaf`;
- `fast-ool-fmaf`: the prototype `#[inline(never)]`, plus `libm::fmaf`;
- `fast-ool`: the prototype `#[inline(never)]`, f32 unchanged.

All variants wrote byte-identical SVGs to the base build on all 14 inputs. Module sizes:
base 11,184,858 B; `fast` 11,324,153 B (+139 KB, +1.25%); `fast-fmaf` 11,326,855 B;
`fast-ool` 11,155,240 B (−29.6 KB, because wasi-libc's `fma` and `scalbn` are no longer
linked); `fast-ool-fmaf` 11,157,955 B.

User cycles and instructions of the whole node process (`sudo perf stat -r 3 -e
cycles:u,instructions:u`, including node start-up and JIT compilation), shown as base ÷
variant (above 1 is faster):

| input | `fast` cycles | `fast` instructions | `fast-fmaf` cycles | `fast-fmaf` instructions | `fast-ool-fmaf` cycles | `fast-ool-fmaf` instructions |
| :-- | --: | --: | --: | --: | --: | --: |
| sfdp/gen_rnd2000 | **1.50** | 1.96 | 1.54 | 1.96 | 1.31 | 1.58 |
| sfdp/gen_grid20x20 | **1.39** | 1.71 | 1.37 | 1.71 | 1.25 | 1.46 |
| fdp/wild_gv16_graphs_viewport | **1.31** | 1.70 | 1.30 | 1.69 | 1.12 | 1.48 |
| fdp/wild_gv16_graphs_b69 | 1.08 | 1.51 | 1.04 | 1.51 | 0.92 | 1.37 |
| twopi/fx_arrows (splines) | **1.38** | 1.51 | 1.60 | 1.51 | 1.50 | 1.50 |
| neato/hw3_vpsc_random200 | 0.97 | 0.99 | 0.98 | 1.28 | 1.05 | 1.31 |
| dot/gv16_graphs_b143 | 0.98 | 1.01 | 0.99 | 1.01 | 0.99 | 1.03 |

Raw data: [`data/wasm-fma-cycles.tsv`](data/wasm-fma-cycles.tsv), and
[`data/wasm-fma-e2e.tsv`](data/wasm-fma-e2e.tsv) (all 14 inputs, five variants, min and median
wall time of 5 interleaved runs, instructions of one run). **The machine was heavily loaded**
(load average 40–86 on 28 cores) while these ran, so cycles carry shared-cache noise and wall
times more still: the base fdp `viewport` run took 5.0–8.9 s across runs. The instruction
ratios are stable. The cycle ratios are smaller than the instruction ratios because the
emulated FMA is a dependent chain of flops (lower IPC than musl's integer code). A production
PR should re-measure on a quiet machine. The Amdahl estimate from the profiles agrees with the
cycle ratios: halving a 60–76% share predicts 1.4–1.6×.

## Recommendation

**Worth doing, for f64.** It is the only WASM-specific item in the catalogue, it cannot change
a byte (the result is the IEEE value), and it takes up to a third off the user cycles of fdp, sfdp
and spline routing under node. Expected end-to-end gain under node, before #543: sfdp 1.4–1.5×,
fdp 1.1–1.3×, spline routing (twopi/neato/fdp with `splines=true`) about 1.4×, and no change
for dot, circo, osage, patchwork and neato's stress majorization. After #543, sfdp's gain
shrinks to whatever fma share remains outside `pow`. The fdp `xlayout`/`hypot` and routing
gains stay.

What a production PR would touch:

- `crates/graphviz-pure-core/src/math/`: a new `math::fma(a, b, c)` (and maybe
  `math::fmaf`). On `cfg(target_arch = "wasm32")` it calls a new `portable::fma` (the code
  above, with the zero-factor shortcut and `libm::fma` as the fallback). Elsewhere it calls
  `a.mul_add(b, c)`, so native builds keep hardware FMA (T1) and nothing changes there.
  `core::math` is the C-mirror module for `<math.h>`, so `math::fma` is also C's name for it.
  The `math-trace` hook does not need to log it.
- The 389 `x.mul_add(y, z)` sites become `math::fma(x, y, z)`: a mechanical rewrite with the
  operand order unchanged, in 60 files (4 core, 6 geom, 44 layout, 6 render).
- `clippy.toml`: add `f64::mul_add` (and `f32::mul_add` if `math::fmaf` is added) to
  `disallowed-methods`, allowed only inside `math`, the same way the transcendental functions
  are fenced. Update the comment that says `mul_add` stays allowed, and `docs/porting/libm.md`
  ("Fused multiply-adds"), which says `mul_add` is a software `fma` on wasm32.
- Tests in `math/tests.rs`: run natively against hardware/`libm::fma` with a few million
  samples per class from §4, plus an `#[ignore]`d long run. `cross-platform.yml` already runs the
  fidelity ratchet on wasm (add the wasm32 differential test there if it is cheap enough).
- Perf evidence: #518's harness on the quick set for the native binary (should be unchanged),
  plus a node/WASI before-and-after on fdp, sfdp and twopi on a quiet machine.

Open decisions for the maintainer:

1. **Inline or out of line.** Inlining the fast path at every site gives the full gain and adds
   139 KB (+1.25%) to the WASI module. The post-1.0 target says WASM size no larger than 1.0.
   Out of line gives 40–60% of the gain on fdp and sfdp (sfdp 1.31× against 1.50×, fdp
   `viewport` 1.12× against 1.31×, fdp `b69` none) and all of it on twopi routing,
   and makes the module 29.6 KB *smaller*. A middle way: `#[inline]` on the
   function and let LLVM decide per site. The npm build uses `opt-level=z`, which inlines less,
   so its size and speed must be measured separately.
2. **f32.** The round-to-odd `fmaf` is slower than musl's. Calling `libm::fmaf` (inlinable
   Rust, same algorithm as musl) instead of wasi-libc's cut neato's instructions by 22% but
   not its cycles here. Leave f32 alone unless a quiet-machine measurement shows a gain.

Risks:

- **Correctness rests on the Boldo–Melquiond theorem plus the window argument above**, and on
  testing (9.4 × 10^10 native and 1.5 × 10^9 wasm triples, 7.2 × 10^8 real calls, mutation
  checks). There is no machine-checked proof of the window bounds. The production PR should
  keep the differential test in CI.
- Rust never contracts or reassociates float operations, and wasm requires IEEE
  round-to-nearest for `f64.add`/`sub`/`mul` on every engine, so the error-free transformations
  compute the same bits everywhere. The algorithm must not be built with fast-math-style flags,
  which Rust doesn't offer anyway.
- NaN payloads may differ from libm's on the fallback path. The port never prints a NaN's bits.
- After the change both wasm targets compute every `mul_add` with the same Rust code, which
  removes today's wasi-libc vs compiler-builtins signed-zero difference.
- `relaxed_madd` (relaxed SIMD) stays ruled out: the spec lets it fuse or not.

## Reproducing

All scratch code lived in `/tmp` and is not committed. The steps:

1. **Profile.** `CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
   cargo build --release --target wasm32-wasip1 -p graphviz-pure-cli`, then run the module with
   a 10-line `node:wasi` runner (`preopens: {'/': '/'}`, `returnOnExit: true`) under
   `node --cpu-prof --cpu-prof-interval 200`, and sum `samples` by `callFrame.functionName`.
2. **Codegen.** A `cdylib` exporting `a.mul_add(b, c)`, built for both targets, then
   `llvm-objdump -d` (from `rustup component add llvm-tools`).
3. **Histogram.** `sed -i 's/\.mul_add(/.mul_add_t(/g'` over `crates/*/src`, a trait
   `MulAddT` in core with `#[track_caller]` impls for f64 and f32 that classify the operands into
   a `Mutex<HashMap>`, and a dump at the end of `dot_main`.
4. **Differential test and microbenchmark.** The prototype crate above, with a `diff` binary
   (classes as in the table; `RUSTFLAGS="-C target-cpu=native"` natively, `wasm32-wasip1` under
   node) and a `cdylib` of `#[no_mangle]` loops. The bench crate cannot be `forbid(unsafe_code)`
   (`#[no_mangle]` counts as unsafe), so the prototype lives in a separate crate that is.
5. **End to end.** The same `sed` rewrite onto a trait backed by the prototype, four WASI builds,
   `perf stat -r 3 -e cycles:u,instructions:u` around `node run-wasi.mjs`, and `cmp` of every SVG
   against the base build.
