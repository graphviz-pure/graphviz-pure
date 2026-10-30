//! Checks for `graphviz_pure_core::math::portable`: differential tests of `portable::fma` on
//! every target, and on Apple Silicon a comparison of `portable::hypot` against the system libm.

// The point of these tests is to call `f64::mul_add` and (on Apple Silicon) the system libm,
// which the rest of the workspace may not.
#![allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::unreadable_literal
)]

use super::portable;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Uniform in [0, 1).
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    /// `±2^e`, `e` uniform in `[lo, hi)`.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn log_uniform(&mut self, lo: f64, hi: f64) -> f64 {
        let v = (lo + (hi - lo) * self.unit()).exp2();
        if self.next() & 1 == 1 { -v } else { v }
    }
    /// Any finite or special bit pattern.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn bits(&mut self) -> f64 {
        f64::from_bits(self.next())
    }
    /// A significand with few set bits (top, bottom and a few random), or runs of ones, to hit
    /// ties and sticky bits.
    fn sparse_mant(&mut self) -> u64 {
        let mut m = 0u64;
        let k = self.below(4);
        for _ in 0..k {
            m |= 1 << self.below(52);
        }
        if self.next() & 1 == 1 {
            m |= 1;
        }
        if self.next() & 1 == 1 {
            m |= 1 << 51;
        }
        if self.next() & 3 == 0 {
            m = (1u64 << 52) - 1 - m;
        }
        m & ((1 << 52) - 1)
    }
    fn f(&mut self, exp: i64, sparse: bool) -> f64 {
        let m = if sparse {
            self.sparse_mant()
        } else {
            self.next() >> 12
        };
        let s = self.next() >> 63;
        let e = exp.clamp(0, 2047) as u64;
        f64::from_bits((s << 63) | (e << 52) | m)
    }
    fn fs(&mut self, e: i64) -> f64 {
        let sp = self.next() & 1 == 0;
        self.f(e, sp)
    }
    fn fr(&mut self, base: i64, span: u64) -> f64 {
        let e = base + self.below(span) as i64;
        self.fs(e)
    }
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
const EDGES: [f64; 16] = [
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
    f64::MIN_POSITIVE,
    -f64::MIN_POSITIVE,
    5e-324,
    1e-310,
    f64::MAX,
    f64::MIN,
    1e300,
    1e-300,
];

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
fn check2(
    name: &str,
    got: impl Fn(f64, f64) -> f64,
    want: impl Fn(f64, f64) -> f64,
    xs: &[(f64, f64)],
) {
    let bad: Vec<_> = xs
        .iter()
        .filter(|&&(x, y)| !same(got(x, y), want(x, y)))
        .collect();
    assert!(
        bad.is_empty(),
        "{name}: {} of {} differ, e.g. {:?} ",
        bad.len(),
        xs.len(),
        &bad[..bad.len().min(5)]
    );
}

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
const N: usize = 200_000;

#[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
#[test]
fn hypot_matches_apple() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut xs = Vec::new();
    for &a in &EDGES {
        for &b in &EDGES {
            xs.push((a, b));
        }
    }
    for _ in 0..N {
        let a = r.log_uniform(-40.0, 40.0);
        xs.push((a, a * r.log_uniform(-30.0, 30.0)));
        xs.push((
            r.log_uniform(-1074.0, 1024.0),
            r.log_uniform(-1074.0, 1024.0),
        ));
        let a = r.log_uniform(-1074.0, 1024.0);
        xs.push((a, a * r.log_uniform(-60.0, 60.0)));
        xs.push((r.bits(), r.bits()));
        xs.push((r.unit() * 1000.0, r.unit() * 1000.0));
    }
    check2("hypot", portable::hypot, f64::hypot, &xs);
}

/// Sample generators for the eight f64 classes of `docs/perf/wasm-fma.md` §4:
/// 0: random bit patterns;
/// 1: random in window (`ea, eb` within ±100 of 0, `c` within ±60 of `a * b`);
/// 2: sparse significands near each other (ties, sticky bits, runs of ones);
/// 3: `c = -RN(a * b)` (the error-free product residual);
/// 4: `c` within ±32,768 ulps of `-a * b` (near cancellation);
/// 5: exponent gaps up to ±1,100 between `c` and `a * b`;
/// 6: exponents within ±2 of every window edge and format edge;
/// 7: subnormal and tiny operands and results.
fn gen_fma(class: usize, r: &mut Rng) -> (f64, f64, f64) {
    match class {
        0 => (
            f64::from_bits(r.next()),
            f64::from_bits(r.next()),
            f64::from_bits(r.next()),
        ),
        1 => {
            let ea = 1023 + r.below(200) as i64 - 100;
            let eb = 1023 + r.below(200) as i64 - 100;
            let ec = ea + eb - 1023 + r.below(120) as i64 - 60;
            (r.f(ea, false), r.f(eb, false), r.f(ec, false))
        }
        2 => {
            let ea = 1023 + r.below(64) as i64 - 32;
            let eb = 1023 + r.below(64) as i64 - 32;
            let ec = ea + eb - 1023 + r.below(110) as i64 - 55;
            (r.f(ea, true), r.f(eb, true), r.f(ec, true))
        }
        3 => {
            let a = r.fr(1023 - 200, 400);
            let b = r.fr(1023 - 200, 400);
            (a, b, -(a * b))
        }
        4 => {
            let a = r.fr(1023 - 40, 80);
            let b = r.fr(1023 - 40, 80);
            let p = a * b;
            let d = (r.next() >> r.below(64)) as i64 & 0xffff;
            let c = f64::from_bits(((-p).to_bits() as i64).wrapping_add(d - 0x8000) as u64);
            (a, b, c)
        }
        5 => {
            let ea = 1023 + r.below(40) as i64 - 20;
            let eb = 1023 + r.below(40) as i64 - 20;
            let gap = r.below(2200) as i64 - 1100;
            (r.fs(ea), r.fs(eb), r.fs(ea + eb - 1023 + gap))
        }
        6 => {
            let edges = [
                1i64,
                2,
                123,
                1023 - 900,
                1023 - 450,
                1023 + 995,
                2018,
                2019,
                2044,
                2045,
                2046,
                2047,
                0,
            ];
            let pick =
                |r: &mut Rng| edges[r.below(edges.len() as u64) as usize] + r.below(5) as i64 - 2;
            let ea = pick(r);
            let eb = if r.next() & 1 == 0 {
                pick(r)
            } else {
                let target = [-900i64, -901, 1018, 1019][r.below(4) as usize];
                2046 + target - ea + r.below(3) as i64 - 1
            };
            let ec = if r.next() & 1 == 0 {
                pick(r)
            } else {
                ea + eb - 1023 + r.below(110) as i64 - 55
            };
            (r.fs(ea), r.fs(eb), r.fs(ec))
        }
        7 => {
            let ea = r.below(600) as i64;
            let eb = 1023 - 400 + r.below(400) as i64;
            let ec = r.below(100) as i64;
            (r.fs(ea), r.fs(eb), r.fs(ec))
        }
        _ => unreachable!(),
    }
}

fn assert_fma(a: f64, b: f64, c: f64, want: f64) {
    let got = portable::fma(a, b, c);
    let disp = super::fma(a, b, c);
    assert!(
        same(got, want) && same(disp, want),
        "fma({a:e} = {:#018x}, {b:e} = {:#018x}, {c:e} = {:#018x}): portable={got:e} ({:#018x}), math::fma={disp:e} ({:#018x}), want={want:e} ({:#018x}), in_window={}",
        a.to_bits(),
        b.to_bits(),
        c.to_bits(),
        got.to_bits(),
        disp.to_bits(),
        want.to_bits(),
        portable::fma::in_window(a, b, c),
    );
}

fn check_fma(samples_per_class: u64) {
    let pos_edges = [
        0.0,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::INFINITY,
        f64::NAN,
        1.0,
        f64::from_bits(1),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        1.0 + f64::EPSILON,
        1.0 - f64::EPSILON / 2.0,
        f64::from_bits(((1023 - 900) as u64) << 52),
        f64::from_bits(((1023 - 901) as u64) << 52),
        f64::from_bits(((1023 + 995) as u64) << 52),
        f64::from_bits(((1023 + 996) as u64) << 52),
        f64::from_bits(((1023 + 1018) as u64) << 52),
        f64::from_bits(((1023 + 1019) as u64) << 52),
        f64::from_bits(((1023 + 1021) as u64) << 52),
        f64::from_bits(((1023 + 1022) as u64) << 52),
        f64::from_bits(((1023 - 450) as u64) << 52),
        f64::from_bits(((1023 - 451) as u64) << 52),
        3.0,
        0.1,
        1e300,
        1e-300,
        134_217_729.0,
        f64::MIN_POSITIVE * 1.5,
        f64::from_bits((((1023 - 900) as u64) << 52) - 1),
        f64::from_bits((((1023 + 996) as u64) << 52) - 1),
    ];
    let mut special = Vec::with_capacity(pos_edges.len() * 2);
    for &x in &pos_edges {
        special.push(x);
        special.push(-x);
    }
    for &a in &special {
        for &b in &special {
            for &c in &special {
                let want = libm::fma(a, b, c);
                assert_fma(a, b, c, want);
                // On musl / wasi-libc without hardware FMA, C `fma` returns +0 when `a * b`
                // underflows to zero with `c = +0`, where IEEE 754 and `libm::fma` return -0.
                #[cfg(not(any(
                    all(target_arch = "wasm32", target_os = "wasi"),
                    all(
                        target_arch = "x86_64",
                        target_env = "musl",
                        not(target_feature = "fma")
                    )
                )))]
                assert!(same(a.mul_add(b, c), want));
            }
        }
    }

    for class in 0..8 {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15 ^ ((class as u64 + 1) << 40));
        let mut fast = 0u64;
        for i in 0..samples_per_class {
            let (a, b, c) = gen_fma(class, &mut r);
            fast += u64::from(portable::fma::in_window(a, b, c));
            #[cfg(any(
                all(target_arch = "wasm32", target_os = "wasi"),
                all(
                    target_arch = "x86_64",
                    target_env = "musl",
                    not(target_feature = "fma")
                )
            ))]
            let want = {
                let _ = i;
                libm::fma(a, b, c)
            };
            #[cfg(not(any(
                all(target_arch = "wasm32", target_os = "wasi"),
                all(
                    target_arch = "x86_64",
                    target_env = "musl",
                    not(target_feature = "fma")
                )
            )))]
            let want = {
                let hw = a.mul_add(b, c);
                if i & 15 == 0 {
                    let lm = libm::fma(a, b, c);
                    assert!(same(hw, lm));
                }
                hw
            };
            assert_fma(a, b, c, want);
        }
        match class {
            1..=4 => assert_eq!(fast, samples_per_class),
            7 => assert!(fast * 100 < samples_per_class),
            _ => assert!(fast > 0 && fast < samples_per_class),
        }
    }
}

#[test]
fn fma_matches_reference() {
    check_fma(50_000);
}

/// Long differential run (~16M samples per class, 1.3 × 10^8 total):
/// `cargo test --release -p graphviz-pure-core -- --ignored fma_matches_reference_many`.
#[test]
#[ignore = "slow; run in release"]
fn fma_matches_reference_many() {
    check_fma(1 << 24);
}
