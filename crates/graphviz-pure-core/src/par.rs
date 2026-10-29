//! Deterministic index ranges for the `parallel` feature.
//!
//! Workers receive contiguous, non-overlapping ranges. The same `n` and thread
//! count always produce the same ranges. On wasm there are no worker threads:
//! the whole range runs on the caller.

use std::ops::Range;

/// Split `0..n` into at most `threads` contiguous ranges and call `f` on each.
///
/// A thread count of 0 or 1, an empty `n`, and every wasm target run `f` on
/// the caller. Extra threads beyond `n` are dropped so no range is empty.
pub fn for_each_chunk(n: usize, threads: usize, f: impl Fn(Range<usize>) + Sync) {
    if n == 0 {
        return;
    }
    let threads = threads.max(1).min(n);
    #[cfg(target_family = "wasm")]
    {
        let _ = threads;
        f(0..n);
    }
    #[cfg(not(target_family = "wasm"))]
    {
        if threads == 1 {
            f(0..n);
            return;
        }
        spawn_ranges(0, n, threads, &f);
    }
}

/// Like [`for_each_chunk`], handing each worker the subslice of `items` its
/// range covers. The subslices are disjoint.
pub fn for_each_chunk_mut<T: Send + Sync>(
    items: &mut [T],
    threads: usize,
    f: impl Fn(Range<usize>, &mut [T]) + Sync,
) {
    let n = items.len();
    if n == 0 {
        return;
    }
    let threads = threads.max(1).min(n);
    #[cfg(target_family = "wasm")]
    {
        let _ = threads;
        f(0..n, items);
    }
    #[cfg(not(target_family = "wasm"))]
    {
        if threads == 1 {
            f(0..n, items);
            return;
        }
        spawn_mut(items, 0, threads, &f);
    }
}

#[cfg(not(target_family = "wasm"))]
fn spawn_ranges(base: usize, n: usize, threads: usize, f: &(impl Fn(Range<usize>) + Sync)) {
    if threads <= 1 || n == 0 {
        if n > 0 {
            f(base..base + n);
        }
        return;
    }
    let len = first_chunk_len(n, threads);
    std::thread::scope(|scope| {
        scope.spawn(|| f(base..base + len));
        spawn_ranges(base + len, n - len, threads - 1, f);
    });
}

#[cfg(not(target_family = "wasm"))]
fn spawn_mut<T: Send + Sync>(
    items: &mut [T],
    base: usize,
    threads: usize,
    f: &(impl Fn(Range<usize>, &mut [T]) + Sync),
) {
    let n = items.len();
    if threads <= 1 || n == 0 {
        if n > 0 {
            f(base..base + n, items);
        }
        return;
    }
    let len = first_chunk_len(n, threads);
    let (head, tail) = items.split_at_mut(len);
    std::thread::scope(|scope| {
        scope.spawn(|| f(base..base + len, head));
        spawn_mut(tail, base + len, threads - 1, f);
    });
}

/// Length of the first chunk. The remainder is handed out one element at a
/// time to the earliest chunks, so `n = 10`, `threads = 3` is `4 + 3 + 3`.
#[cfg(not(target_family = "wasm"))]
fn first_chunk_len(n: usize, threads: usize) -> usize {
    n / threads + usize::from(!n.is_multiple_of(threads))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn ranges_for(n: usize, threads: usize) -> Vec<Range<usize>> {
        let got = Mutex::new(Vec::new());
        for_each_chunk(n, threads, |range| got.lock().expect("lock").push(range));
        let mut ranges = got.into_inner().expect("ranges");
        ranges.sort_by_key(|range| range.start);
        ranges
    }

    #[test]
    fn chunks_are_deterministic_and_cover_every_index() {
        for n in [0, 1, 2, 3, 5, 7, 8, 17, 37, 64, 65] {
            for threads in [0, 1, 2, 3, 8, 28] {
                let once = ranges_for(n, threads);
                let twice = ranges_for(n, threads);
                assert_eq!(once, twice, "n={n} threads={threads}");
                let mut at = 0;
                for range in &once {
                    assert_eq!(range.start, at, "n={n} threads={threads}");
                    assert!(range.end > range.start);
                    at = range.end;
                }
                assert_eq!(at, n, "n={n} threads={threads}");
                let cap = threads.max(1).min(n.max(1));
                if n == 0 {
                    assert!(once.is_empty());
                } else {
                    assert!(once.len() <= cap);
                }
            }
        }
    }

    #[test]
    fn mut_chunks_write_disjoint_slices() {
        let mut items = vec![0u32; 37];
        for_each_chunk_mut(&mut items, 3, |range, slot| {
            assert_eq!(slot.len(), range.len());
            for (local, value) in slot.iter_mut().enumerate() {
                *value = (range.start + local) as u32;
            }
        });
        assert_eq!(items, (0..37).collect::<Vec<_>>());
    }
}
