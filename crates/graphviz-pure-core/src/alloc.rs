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
