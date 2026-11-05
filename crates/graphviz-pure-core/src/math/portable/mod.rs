//! Pure-Rust implementations behind every `graphviz_pure_core::math` function, on every target.
//!
//! `hypot` reproduces macOS arm64 libm bit for bit (an original formula found by black-box
//! comparison). `pow` is a port of CORE-MATH's correctly rounded `pow` (MIT): Apple's `pow` is
//! within 0.511 ulp, so a correctly rounded one agrees with it far more often than musl's does.
//! `fma` is Boldo and Melquiond's emulated fused multiply-add, used where `f64::mul_add` would
//! call an out-of-line software `fma` (wasm32 and x86-64 musl without `fma`). Where `fma` is a
//! target feature (the x86-64-v3 builds), `cbrt` is a copy of the `libm` crate's port of
//! CORE-MATH's correctly rounded `cbrt` (MIT) that calls `math::fma`, the `vfmadd` instruction,
//! instead of the crate's `fma` behind its run-time CPU dispatch. Elsewhere the dispatch reaches
//! hardware `fma` where the emulated one would be slower, so `cbrt` stays the crate's. Both
//! give the same bits.
//! Everything else comes from the `libm` crate (MIT, a port of musl):
//! the same code Rust's own wasm32 `f64` methods use, pinned by `Cargo.lock` rather than by the
//! toolchain, and identical on every target (unlike glibc, whose results depend on the CPU).
//! Apple's `sin`, `cos`, `atan2`, ... are closed-source assembly that no published algorithm
//! reproduces. See `docs/porting/libm.md`.

pub(crate) mod cbrt;
pub(crate) mod fma;
mod hypot;
mod pow;

#[cfg(target_feature = "fma")]
pub use cbrt::cbrt;
pub use fma::fma;
pub use hypot::hypot;
#[cfg(not(target_feature = "fma"))]
pub use libm::cbrt;
pub use libm::{acos, asin, atan2, cos, exp, log, log2, sin, tan};
pub use pow::pow;
