//! C's allocation wrappers (`lib/util/alloc.h`).
//!
//! C: lib/util/alloc.h:gv_calloc, lib/util/alloc.h:gv_alloc
//!
//! `gv_calloc` checks the multiplication for overflow and reports a failed allocation. C
//! writes the message with `fprintf(stderr, ...)` and calls `graphviz_exit(EXIT_FAILURE)`;
//! the port returns the failure where C writes the message and exits, so the run stops with
//! C's message and the caller writes it as C did.

use crate::error::GraphvizError;

/// A failed `gv_calloc` (`lib/util/alloc.h`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocFailure {
    /// `nmemb * size` overflows `size_t`: C's "integer overflow when trying to allocate".
    Overflow {
        /// The number of elements C asked for.
        nmemb: u64,
        /// The size of one element in bytes.
        size: u64,
    },
    /// The allocation failed: C's "out of memory when trying to allocate".
    OutOfMemory {
        /// The number of bytes C asked for.
        bytes: u64,
    },
}

impl AllocFailure {
    /// The message C writes, without the newline.
    pub fn message(&self) -> String {
        match self {
            AllocFailure::Overflow { nmemb, size } => {
                format!("integer overflow when trying to allocate {nmemb} * {size} bytes")
            }
            AllocFailure::OutOfMemory { bytes } => {
                format!("out of memory when trying to allocate {bytes} bytes")
            }
        }
    }
}

impl From<AllocFailure> for GraphvizError {
    fn from(failure: AllocFailure) -> Self {
        GraphvizError::allocation(failure.message())
    }
}

/// `gv_calloc(nmemb, size)` (`lib/util/alloc.h`): `nmemb` zeroed elements of `size` bytes.
///
/// C: lib/util/alloc.h:gv_calloc
///
/// C's overflow check comes first (`nmemb > 0 && SIZE_MAX / nmemb < size`), then C's
/// `calloc`, which returns NULL where the memory is not there. The port's
/// `try_reserve_exact` fails in the same case, and the port reports it where C writes the
/// message and exits.
///
/// The counts are 64-bit, as C's `size_t` is on the 64-bit reference build. C multiplies
/// two `size_t` counts, which wraps on a 32-bit target; the port follows the reference.
pub fn gv_calloc(nmemb: u64, size: u64) -> Result<Vec<u8>, AllocFailure> {
    let bytes = check(nmemb, size)?;
    let count = usize::try_from(nmemb).map_err(|_| AllocFailure::OutOfMemory { bytes })?;
    let mut v: Vec<u8> = Vec::new();
    if v.try_reserve_exact(count).is_err() {
        // C's `calloc` returns NULL here.
        return Err(AllocFailure::OutOfMemory { bytes });
    }
    v.resize(count, 0);
    Ok(v)
}

/// `gv_calloc(nmemb, size_of::<T>())`: a zeroed vector of `nmemb` elements, as C's
/// `gv_calloc` allocates them.
///
/// C: lib/util/alloc.h:gv_calloc
pub fn gv_calloc_vec<T: Clone + Default>(nmemb: u64) -> Result<Vec<T>, AllocFailure> {
    let size = size_of::<T>() as u64;
    let bytes = check(nmemb, size)?;
    let count = usize::try_from(nmemb).map_err(|_| AllocFailure::OutOfMemory { bytes })?;
    let mut v: Vec<T> = Vec::new();
    if v.try_reserve_exact(count).is_err() {
        // C's `calloc` returns NULL here.
        return Err(AllocFailure::OutOfMemory { bytes });
    }
    v.resize(count, T::default());
    Ok(v)
}

/// Grows `v` to `nmemb` elements, as C's `gv_calloc` allocates the whole array. The
/// message counts the whole array, as C's `gv_calloc(nmemb, size)` does.
///
/// C: lib/util/alloc.h:gv_calloc
pub fn gv_calloc_grow<T: Clone + Default>(v: &mut Vec<T>, nmemb: u64) -> Result<(), AllocFailure> {
    let size = size_of::<T>() as u64;
    let bytes = check(nmemb, size)?;
    let count = usize::try_from(nmemb).map_err(|_| AllocFailure::OutOfMemory { bytes })?;
    if v.try_reserve(count.saturating_sub(v.len())).is_err() {
        // C's `calloc` returns NULL here.
        return Err(AllocFailure::OutOfMemory { bytes });
    }
    v.resize(count, T::default());
    Ok(())
}

/// Not in C. The most `pointf` vertices `poly_init` and `point_init` may ask `gv_calloc` for
/// (`outp * sides`): `peripheries` and `sides` are unbounded `int`s, so
/// `peripheries=2147483647` or `sides=2147483647` asks for 64 GiB, which C allocates (#591).
/// The reference machine has the memory for that, so C does not report it: it writes vertices
/// until the machine runs out of memory (the OOM killer, or minutes of swapping).
///
/// Measured on the reference build under an 8 GB limit (`peripheries=n` on the default
/// ellipse, two 16-byte vertices per periphery; `-Tdot` is the layout alone): C takes 0.07 s
/// and 66 MB at n = 1e6 and 0.3 s and 0.63 GB at 1e7; at n = 67108863 (a 2 GiB array per
/// node) 2.7 s and 2.1 GB for one node, and the port the same memory. `-Tsvg` writes 182
/// bytes per periphery and takes 3 s at 1e6 and 29 s at 1e7 (1.8 GB). `shape=polygon
/// sides=n`: C takes 42 s and 7.8 GB at 1e8. The cost is linear in the vertices, and C's
/// resident memory is about the array's size per node, so the budget of 2^31 vertices, a
/// 32 GiB array, needs about 64 GB and, for the layout alone, minutes in C (extrapolated: the
/// 8 GB limit forbids measuring it). That is the point where C stops finishing on a normal
/// machine, and C does everything below it that fits in memory. Past it the port reports
/// C's out-of-memory message for C's request and exits 1, as C does under a memory limit.
///
/// A 32-bit target (wasm32) has at most 4 GiB; there the budget is 2^24 vertices (256 MiB).
#[cfg(not(target_pointer_width = "32"))]
pub const MAX_VERTICES: u64 = 1 << 31;
/// See the 64-bit constant.
#[cfg(target_pointer_width = "32")]
pub const MAX_VERTICES: u64 = 1 << 24;

/// The check that comes before `gv_calloc(nmemb, size_of::<T>())` for an array whose size the
/// input chooses: C's overflow check first, then the port's budget of `max` elements, which
/// reports the allocation C asked for as failed. Not in C (see [`MAX_VERTICES`]).
pub fn gv_calloc_budget<T>(nmemb: u64, max: u64) -> Result<(), AllocFailure> {
    let bytes = check(nmemb, size_of::<T>() as u64)?;
    if nmemb > max {
        return Err(AllocFailure::OutOfMemory { bytes });
    }
    Ok(())
}

/// The number of elements of C's `gv_calloc(nmemb, size)`, from the two counts C
/// multiplies (`outp * sides` in `poly_init`).
///
/// C multiplies two `size_t` counts, which wraps on a 32-bit target, where C's
/// `gv_calloc` then counts the wrapped number and the code writes past the array it
/// allocated. The port counts in 64 bits, as the 64-bit reference does.
pub fn element_count(a: usize, b: usize) -> u64 {
    (a as u64).saturating_mul(b as u64)
}

/// C's overflow check, and the number of bytes the allocation asks for.
fn check(nmemb: u64, size: u64) -> Result<u64, AllocFailure> {
    if nmemb > 0 && u64::MAX / nmemb < size {
        return Err(AllocFailure::Overflow { nmemb, size });
    }
    Ok(nmemb.saturating_mul(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_are_c_s() {
        assert_eq!(
            AllocFailure::Overflow {
                nmemb: 4611686016279904256,
                size: 16
            }
            .message(),
            "integer overflow when trying to allocate 4611686016279904256 * 16 bytes"
        );
        assert_eq!(
            AllocFailure::OutOfMemory {
                bytes: 137438953472
            }
            .message(),
            "out of memory when trying to allocate 137438953472 bytes"
        );
    }

    #[test]
    fn a_small_allocation_is_zeroed() {
        let v: Vec<u32> = gv_calloc_vec(4).unwrap();
        assert_eq!(v, vec![0; 4]);
        let v: Vec<u8> = gv_calloc(3, 1).unwrap();
        assert_eq!(v, vec![0; 3]);
    }

    #[test]
    fn a_count_past_the_budget_reports_the_allocation_c_asked_for() {
        // `peripheries=2147483647` on an ellipse: 2^31 peripheries of 2 vertices of 16 bytes.
        let err =
            gv_calloc_budget::<[f64; 2]>(element_count(1 << 31, 2), MAX_VERTICES).unwrap_err();
        assert_eq!(
            err.message(),
            "out of memory when trying to allocate 68719476736 bytes"
        );
        // 2 peripheries of 2^31 sides: the same 64 GiB.
        let err =
            gv_calloc_budget::<[f64; 2]>(element_count(2, 1 << 31), MAX_VERTICES).unwrap_err();
        assert_eq!(
            err.message(),
            "out of memory when trying to allocate 68719476736 bytes"
        );
        // The product overflows `size_t` once multiplied by 16: C's overflow message.
        let err = gv_calloc_budget::<[f64; 2]>(element_count(1 << 31, 1 << 31), MAX_VERTICES)
            .unwrap_err();
        assert!(matches!(err, AllocFailure::Overflow { .. }), "{err:?}");
        // One vertex past the budget is refused, the budget itself is not.
        assert!(gv_calloc_budget::<[f64; 2]>(MAX_VERTICES, MAX_VERTICES).is_ok());
        let err = gv_calloc_budget::<[f64; 2]>(MAX_VERTICES + 1, MAX_VERTICES).unwrap_err();
        assert_eq!(
            err.message(),
            "out of memory when trying to allocate 34359738384 bytes"
        );
    }

    #[test]
    fn a_count_that_overflows_the_size_is_reported() {
        let err = gv_calloc_vec::<u64>(u64::MAX).unwrap_err();
        assert_eq!(
            err,
            AllocFailure::Overflow {
                nmemb: u64::MAX,
                size: 8
            }
        );
    }

    #[test]
    fn an_allocation_the_memory_is_not_there_for_fails() {
        // 2^60 elements of 8 bytes: the count passes C's overflow check, and the memory
        // is not there.
        let err = gv_calloc_vec::<u64>(u64::MAX / 16).unwrap_err();
        assert!(matches!(err, AllocFailure::OutOfMemory { .. }), "{err:?}");
    }
}
