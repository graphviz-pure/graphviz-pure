//! Transcendental functions that do not call the platform libm.
//!
//! `f32` and `f64` inherent methods reach the host libm. A same-named trait method
//! never overrides them while `std` is enabled, so these names are different and
//! every call goes to the `libm` crate 0.2.16 (musl). That is the crate
//! `graphviz_pure_core::math` uses for `sin`, `cos`, `atan2` and the rest of its
//! functions other than `pow` and `hypot`.

#![allow(dead_code)]

/// Portable substitutes for the platform transcendental methods.
pub(crate) trait PortableMath: Copy {
    /// Sine.
    fn p_sin(self) -> Self;
    /// Cosine.
    fn p_cos(self) -> Self;
    /// Tangent.
    fn p_tan(self) -> Self;
    /// Arc sine.
    fn p_asin(self) -> Self;
    /// Arc cosine.
    fn p_acos(self) -> Self;
    /// Arc tangent.
    fn p_atan(self) -> Self;
    /// Arc tangent of `self` / `other`, with the quadrant from the signs.
    fn p_atan2(self, other: Self) -> Self;
    /// Sine and cosine together. The pair is `(sin, cos)`.
    fn p_sin_cos(self) -> (Self, Self);
    /// Natural exponential.
    fn p_exp(self) -> Self;
    /// Base-2 exponential.
    fn p_exp2(self) -> Self;
    /// Natural logarithm.
    fn p_ln(self) -> Self;
    /// Base-2 logarithm.
    fn p_log2(self) -> Self;
    /// Base-10 logarithm.
    fn p_log10(self) -> Self;
    /// `self` raised to `other`.
    fn p_powf(self, other: Self) -> Self;
    /// Hypotenuse of the right triangle with sides `self` and `other`.
    fn p_hypot(self, other: Self) -> Self;
    /// Cube root.
    fn p_cbrt(self) -> Self;
}

impl PortableMath for f32 {
    #[inline]
    fn p_sin(self) -> Self {
        libm::sinf(self)
    }
    #[inline]
    fn p_cos(self) -> Self {
        libm::cosf(self)
    }
    #[inline]
    fn p_tan(self) -> Self {
        libm::tanf(self)
    }
    #[inline]
    fn p_asin(self) -> Self {
        libm::asinf(self)
    }
    #[inline]
    fn p_acos(self) -> Self {
        libm::acosf(self)
    }
    #[inline]
    fn p_atan(self) -> Self {
        libm::atanf(self)
    }
    #[inline]
    fn p_atan2(self, other: Self) -> Self {
        libm::atan2f(self, other)
    }
    #[inline]
    fn p_sin_cos(self) -> (Self, Self) {
        libm::sincosf(self)
    }
    #[inline]
    fn p_exp(self) -> Self {
        libm::expf(self)
    }
    #[inline]
    fn p_exp2(self) -> Self {
        libm::exp2f(self)
    }
    #[inline]
    fn p_ln(self) -> Self {
        libm::logf(self)
    }
    #[inline]
    fn p_log2(self) -> Self {
        libm::log2f(self)
    }
    #[inline]
    fn p_log10(self) -> Self {
        libm::log10f(self)
    }
    #[inline]
    fn p_powf(self, other: Self) -> Self {
        libm::powf(self, other)
    }
    #[inline]
    fn p_hypot(self, other: Self) -> Self {
        libm::hypotf(self, other)
    }
    #[inline]
    fn p_cbrt(self) -> Self {
        libm::cbrtf(self)
    }
}

impl PortableMath for f64 {
    #[inline]
    fn p_sin(self) -> Self {
        libm::sin(self)
    }
    #[inline]
    fn p_cos(self) -> Self {
        libm::cos(self)
    }
    #[inline]
    fn p_tan(self) -> Self {
        libm::tan(self)
    }
    #[inline]
    fn p_asin(self) -> Self {
        libm::asin(self)
    }
    #[inline]
    fn p_acos(self) -> Self {
        libm::acos(self)
    }
    #[inline]
    fn p_atan(self) -> Self {
        libm::atan(self)
    }
    #[inline]
    fn p_atan2(self, other: Self) -> Self {
        libm::atan2(self, other)
    }
    #[inline]
    fn p_sin_cos(self) -> (Self, Self) {
        libm::sincos(self)
    }
    #[inline]
    fn p_exp(self) -> Self {
        libm::exp(self)
    }
    #[inline]
    fn p_exp2(self) -> Self {
        libm::exp2(self)
    }
    #[inline]
    fn p_ln(self) -> Self {
        libm::log(self)
    }
    #[inline]
    fn p_log2(self) -> Self {
        libm::log2(self)
    }
    #[inline]
    fn p_log10(self) -> Self {
        libm::log10(self)
    }
    #[inline]
    fn p_powf(self, other: Self) -> Self {
        libm::pow(self, other)
    }
    #[inline]
    fn p_hypot(self, other: Self) -> Self {
        libm::hypot(self, other)
    }
    #[inline]
    fn p_cbrt(self) -> Self {
        libm::cbrt(self)
    }
}
