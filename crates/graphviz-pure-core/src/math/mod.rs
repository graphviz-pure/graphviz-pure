//! Transcendental functions, called as `math::hypot(x, y)` the way the C sources call
//! `hypot(x, y)`.
//!
//! The platform libms disagree in the last ulp on some inputs (Apple's `libsystem_m`, glibc,
//! compiler-builtins' musl port on wasm32), which is enough to change a force-directed layout.
//! So every call in the port goes through this module, and every function here calls the
//! [`portable`] implementation, on every target: a reproduction of Apple's `hypot`, a port of
//! CORE-MATH's correctly rounded `pow`, and the `libm` crate (the musl port Rust itself uses on
//! wasm32) for the rest. Every build computes the same bits, and no build calls a platform libm
//! for a transcendental function (clippy's `disallowed-methods` enforces it). The fidelity
//! checks compare the port with C Graphviz running these same functions; see
//! `docs/porting/libm.md`.
//!
//! `sqrt` and `fma` are correctly rounded everywhere: `sqrt` stays an `f64` method, while
//! [`fma`] dispatches to [`portable::fma`] on targets whose `f64::mul_add` is an out-of-line
//! integer `fma` (`wasm32`/`wasm64`, and `x86_64` musl without `target_feature = "fma"`) and to
//! `f64::mul_add` everywhere else.

/// The pure-Rust functions every `math` function calls, exposed for `graphviz-pure-portable-libm`.
#[doc(hidden)]
pub mod portable;
#[cfg(feature = "math-trace")]
mod trace;
#[cfg(not(feature = "math-trace"))]
mod trace {
    #[inline(always)]
    pub fn log(_name: u8, _x: f64, _y: f64) {}
}

/// C `fma(a, b, c)`: `RN(a * b + c)` at sites where the macOS arm64 reference build contracts
/// `a * b + c` (`docs/porting/libm.md`).
///
/// On `wasm` (`wasm32`/`wasm64`) and on `x86_64` musl without `target_feature = "fma"`,
/// `f64::mul_add` calls an out-of-line integer `fma`; those builds call [`portable::fma`]
/// instead (inlined on `x86_64` musl, out-of-line on `wasm`). Everywhere else (aarch64, any build
/// with `target_feature = "fma"`, and glibc x86-64 whose `fma` uses an IFUNC hardware dispatch)
/// this calls `f64::mul_add`.
#[inline(always)]
#[must_use]
pub fn fma(a: f64, b: f64, c: f64) -> f64 {
    #[cfg(any(
        target_family = "wasm",
        all(
            target_arch = "x86_64",
            target_env = "musl",
            not(target_feature = "fma")
        )
    ))]
    {
        portable::fma(a, b, c)
    }
    #[cfg(not(any(
        target_family = "wasm",
        all(
            target_arch = "x86_64",
            target_env = "musl",
            not(target_feature = "fma")
        )
    )))]
    {
        #[allow(clippy::disallowed_methods)]
        a.mul_add(b, c)
    }
}

/// C `hypot(x, y)`.
#[inline]
#[must_use]
pub fn hypot(x: f64, y: f64) -> f64 {
    trace::log(0, x, y);
    portable::hypot(x, y)
}

/// C `pow(x, y)`.
#[inline]
#[must_use]
pub fn powf(x: f64, y: f64) -> f64 {
    trace::log(1, x, y);
    portable::pow(x, y)
}

/// C `atan2(y, x)` is `math::atan2(y, x)`.
#[inline]
#[must_use]
pub fn atan2(y: f64, x: f64) -> f64 {
    trace::log(2, y, x);
    portable::atan2(y, x)
}

/// C `sin(x)`.
#[inline]
#[must_use]
pub fn sin(x: f64) -> f64 {
    trace::log(3, x, 0.0);
    portable::sin(x)
}

/// C `cos(x)`.
#[inline]
#[must_use]
pub fn cos(x: f64) -> f64 {
    trace::log(4, x, 0.0);
    portable::cos(x)
}

/// C `tan(x)`.
#[inline]
#[must_use]
pub fn tan(x: f64) -> f64 {
    trace::log(5, x, 0.0);
    portable::tan(x)
}

/// C `asin(x)`.
#[inline]
#[must_use]
pub fn asin(x: f64) -> f64 {
    trace::log(6, x, 0.0);
    portable::asin(x)
}

/// C `acos(x)`.
#[inline]
#[must_use]
pub fn acos(x: f64) -> f64 {
    trace::log(7, x, 0.0);
    portable::acos(x)
}

/// C `exp(x)`.
#[inline]
#[must_use]
pub fn exp(x: f64) -> f64 {
    trace::log(8, x, 0.0);
    portable::exp(x)
}

/// C `log(x)`.
#[inline]
#[must_use]
pub fn ln(x: f64) -> f64 {
    trace::log(9, x, 0.0);
    portable::log(x)
}

/// C `log2(x)`.
#[inline]
#[must_use]
pub fn log2(x: f64) -> f64 {
    trace::log(10, x, 0.0);
    portable::log2(x)
}

/// C `cbrt(x)`.
#[inline]
#[must_use]
pub fn cbrt(x: f64) -> f64 {
    trace::log(11, x, 0.0);
    portable::cbrt(x)
}

#[cfg(test)]
mod tests;
