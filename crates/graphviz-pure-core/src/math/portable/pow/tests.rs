//! Regression values for the CORE-MATH `pow` port: `(x, y, x^y)` as bit patterns, computed by
//! CORE-MATH's C `cr_pow` (core-math-sys 1.4.0) and grouped by the phase that returns them.
//! The port was compared with the C on far more inputs; see `docs/porting/libm.md`.

use super::pow;

const CASES: &[(u64, u64, u64)] = &[
    // phase 1
    (0x40a46b8800000000, 0x4025000000000000, 0x4762511b90746aa3),
    (0x4049567f5d0ff8ea, 0xbffbf8285e98a08a, 0x3f5126988684f4fb),
    (0x4330000000000000, 0x3ff4000000000000, 0x4400000000000000),
    (0x428c988000000000, 0x3fe0000000000000, 0x413e400000000000),
    (0x408301407a55304a, 0x3ffc0d8965283e4c, 0x40f29287d0bdb496),
    (0x4491148400000000, 0x3ff8000000000000, 0x46e1a5b262000000),
    (0x40d4400000000000, 0x3ff8000000000000, 0x4146c80000000000),
    (0x3fefffffddd3f3cd, 0x410c162a6cefbeaa, 0x3fef88e6f3dbeb90),
    // phase 2
    (0x8a997651ef1d218b, 0x7f51b2511daa1908, 0x0000000000000000),
    (0x25e60d839d886700, 0x40033a5b9bd6b7ce, 0x015cf149f8aeba29),
    (0x5e40e49ce4954110, 0xffaa7ea20f0a2f92, 0x0000000000000000),
    (0xd7f9f2e59f7cf214, 0xff6621f008dc0ef0, 0x0000000000000000),
    (0x9899c96667cbecd0, 0x7fde275eba774f09, 0x0000000000000000),
    (0x5cc867ca92085297, 0xff504934808e07f2, 0x0000000000000000),
    (0x08bdaa25cf68605f, 0x7f5c4425dc5b475f, 0x0000000000000000),
    (0x3ff00000315b821e, 0x410fbc71494edd8f, 0x3ff0c88e9cd0d011),
    // exact or midpoint
    (0x43ee0be020000000, 0x4002000000000000, 0x48ebc56f81a6e120),
    (0x4106a64c80000000, 0x4004000000000000, 0x42aaf985d7c89cc6),
    (0x452a68aa20000000, 0x4002000000000000, 0x4bb4c5e1c7e84eae),
    (0x436a68aa20000000, 0x4002000000000000, 0x47c4c5e1c7e84eae),
    (0x42aa68aa20000000, 0x4002000000000000, 0x4614c5e1c7e84eae),
    (0x442a68aa20000000, 0x4002000000000000, 0x4974c5e1c7e84eae),
    (0x418771f880000000, 0x4004000000000000, 0x43ed6800a7ee065c),
    (0x40a54d9880000000, 0x4004000000000000, 0x41b7248286c34914),
    // phase 3
    (0x4972d0b08ed39571, 0x02925be2b5b34e1b, 0x3ff0000000000000),
    (0x5b72d56d580a20ce, 0x80fd48c74a35c70f, 0x3ff0000000000000),
    (0x1db02ac5b0c49d32, 0x83107faca8341888, 0x3ff0000000000000),
    (0x48374c8c819c8614, 0x82e94fff1b4cba18, 0x3ff0000000000000),
    (0x3a941b35173e9d4f, 0x01c39e9dc67453b5, 0x3ff0000000000000),
    (0x52a877a50a81f71c, 0x8255d34db9eeb3f2, 0x3ff0000000000000),
    (0x1a8efcc794d11794, 0x801e38deac39d0bb, 0x3ff0000000000000),
    (0x0b575046f95118d4, 0x8079378c1c350fe5, 0x3ff0000000000000),
];

#[test]
fn matches_core_math() {
    for &(x, y, want) in CASES {
        let (x, y) = (f64::from_bits(x), f64::from_bits(y));
        let got = pow(x, y);
        assert_eq!(got.to_bits(), want, "pow({x:e}, {y:e}) = {got:e}");
    }
}

#[test]
fn special_values() {
    let nan = f64::NAN;
    let inf = f64::INFINITY;
    let cases: &[(f64, f64, f64)] = &[
        (2.0, 10.0, 1024.0),
        (2.0, -1074.0, 5e-324),
        (2.0, 1024.0, inf),
        (-2.0, 3.0, -8.0),
        (-8.0, 1.0 / 3.0, nan),
        (0.0, -1.0, inf),
        (-0.0, -1.0, -inf),
        (-0.0, 3.0, -0.0),
        (-0.0, 2.0, 0.0),
        (1.0, nan, 1.0),
        (nan, 0.0, 1.0),
        (-1.0, inf, 1.0),
        (0.5, inf, 0.0),
        (0.5, -inf, inf),
        (-inf, 3.0, -inf),
        (-inf, -3.0, -0.0),
        (inf, -0.5, 0.0),
        (4.0, 0.5, 2.0),
        (9.0, 1.5, 27.0),
        (10.0, 2.0, 100.0),
    ];
    for &(x, y, want) in cases {
        let got = pow(x, y);
        assert!(
            got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
            "pow({x:e}, {y:e}) = {got:e}, want {want:e}"
        );
    }
}

/// SplitMix64, a fixed-seed generator for the differential samples.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn assert_fast_path(x: f64, y: f64) {
    let got = pow(x, y);
    let want = super::pow_general(x, y);
    assert_eq!(
        got.to_bits(),
        want.to_bits(),
        "pow({x:e} = {:#018x}, {y:e}) = {got:e}, general path {want:e}",
        x.to_bits()
    );
}

/// `x` for the differential samples: uniformly random bits, and the two regions where `x * x`
/// overflows or underflows (`|x|` near `2^512` and `2^-537`), where the rounding is subtle.
fn sample(rng: &mut SplitMix64) -> f64 {
    let r = rng.next();
    match r % 4 {
        0 | 1 => f64::from_bits(rng.next()),
        2 => f64::from_bits((rng.next() & 0x801f_ffff_ffff_ffff) | (0x5f0u64 << 52)),
        _ => f64::from_bits((rng.next() & 0x801f_ffff_ffff_ffff) | (0x1d0u64 << 52)),
    }
}

fn check_fast_paths(n: u64) {
    let mut edge = vec![
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1.0,
        -1.0,
        2.0,
        -2.0,
        0.5,
        f64::MAX,
        -f64::MAX,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::EPSILON,
        1.0 + f64::EPSILON,
        1.0 - f64::EPSILON / 2.0,
        // |x| = 2^512 and its neighbours: x * x overflows from 2^512 on.
        f64::from_bits(0x5ff0_0000_0000_0000),
        f64::from_bits(0x5fef_ffff_ffff_ffff),
        f64::from_bits(0x5fe6_a09e_667f_3bcc),
        f64::from_bits(0x5fe6_a09e_667f_3bcd),
        // 2^-537 squares to 2^-1074, the smallest subnormal; 2^-538 squares to its half.
        f64::from_bits(0x1e60_0000_0000_0000),
        f64::from_bits(0x1e50_0000_0000_0000),
        f64::from_bits(0x1e5f_ffff_ffff_ffff),
        f64::from_bits(0x1e56_a09e_667f_3bcd),
        f64::from_bits(0x1ff0_0000_0000_0000),
        f64::from_bits(0x1fef_ffff_ffff_ffff),
    ];
    edge.extend(edge.clone().iter().map(|x| -x));
    for &x in &edge {
        for y in [1.0, 2.0] {
            assert_fast_path(x, y);
        }
    }
    let mut rng = SplitMix64(0x0532_0532_0532_0532);
    for _ in 0..n {
        let x = sample(&mut rng);
        if x.is_nan() {
            continue;
        }
        assert_fast_path(x, 2.0);
        assert_fast_path(x, 1.0);
    }
}

#[test]
fn fast_paths_match_general() {
    check_fast_paths(20_000);
}

/// The same with 100 million samples per exponent:
/// `cargo test --release -p graphviz-pure-core -- --ignored fast_paths_match_general`.
#[test]
#[ignore = "slow; run in release"]
fn fast_paths_match_general_many() {
    check_fast_paths(100_000_000);
}

/// NaN `x` keeps the general path; the bits of a NaN result are left to the platform, so only
/// NaN-ness is compared.
#[test]
fn nan_takes_general_path() {
    for bits in [
        0x7ff8_0000_0000_0000u64,
        0xfff8_0000_0000_0001,
        0x7ff0_0000_0000_0001,
    ] {
        let x = f64::from_bits(bits);
        for y in [1.0, 2.0] {
            assert!(pow(x, y).is_nan());
            assert!(super::pow_general(x, y).is_nan());
        }
    }
}
