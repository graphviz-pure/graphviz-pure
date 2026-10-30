//! Deterministic index ranges for the `parallel` feature.
//!
//! Workers receive contiguous, non-overlapping ranges. The same `n` and thread
//! count always produce the same ranges. A spawn that the OS refuses runs on
//! the caller, so the same ranges still run and the floats stay the same.

use std::ops::Range;

/// Split `0..n` into at most `threads` contiguous ranges and call `f` on each.
///
/// A thread count of 0 or 1, an empty `n`, and every wasm target run `f` on
/// the caller. Extra threads beyond `n` are dropped so no range is empty.
/// The mat-vec uses [`for_each_chunk_mut`]. This one is the range check.
#[cfg(test)]
pub(super) fn for_each_chunk(n: usize, threads: usize, f: impl Fn(Range<usize>) + Sync) {
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

/// Split `items` into at most `threads` contiguous subslices and call `f` on
/// each. The subslices are disjoint. A thread count of 0 or 1 runs `f` on the
/// caller.
pub(super) fn for_each_chunk_mut<T: Send + Sync>(
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

/// The join handle when `work` is running on a new thread.
///
/// `None` when the OS refused the spawn. `work` has been dropped, and the
/// caller runs that chunk itself. The handle must stay alive until this
/// scope ends: dropping it joins the thread immediately.
#[cfg(not(target_family = "wasm"))]
pub(super) fn try_spawn<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> Option<std::thread::ScopedJoinHandle<'scope, T>> {
    #[cfg(test)]
    if spawn_blocked() {
        return None;
    }
    std::thread::Builder::new().spawn_scoped(scope, work).ok()
}

#[cfg(all(test, not(target_family = "wasm")))]
fn spawn_ranges(base: usize, n: usize, threads: usize, f: &(impl Fn(Range<usize>) + Sync)) {
    if threads <= 1 || n == 0 {
        if n > 0 {
            f(base..base + n);
        }
        return;
    }
    let len = first_chunk_len(n, threads);
    let range = base..base + len;
    let spawned = std::thread::scope(|scope| {
        let worker_range = range.clone();
        let handle = try_spawn(scope, move || f(worker_range));
        let started = handle.is_some();
        if started {
            spawn_ranges(base + len, n - len, threads - 1, f);
        }
        // `handle` joins as this scope ends, after the rest of the ranges
        // have been started.
        started
    });
    if !spawned {
        f(range);
        spawn_ranges(base + len, n - len, threads - 1, f);
    }
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
    // The slice cannot be recovered from a closure the OS drops. The mutex
    // only hands it to whichever side actually runs the chunk.
    let head_job = std::sync::Mutex::new(Some((base..base + len, head)));
    let spawned = std::thread::scope(|scope| {
        let handle = try_spawn(scope, || {
            if let Some((range, slot)) = take_job(&head_job) {
                f(range, slot);
            }
        });
        let started = handle.is_some();
        if started {
            spawn_mut(tail, base + len, threads - 1, f);
        }
        started
    });
    if !spawned {
        if let Some((range, slot)) = take_job(&head_job) {
            f(range, slot);
        }
        spawn_mut(tail, base + len, threads - 1, f);
    }
}

#[cfg(not(target_family = "wasm"))]
fn take_job<'slice, T>(
    job: &std::sync::Mutex<Option<(Range<usize>, &'slice mut [T])>>,
) -> Option<(Range<usize>, &'slice mut [T])> {
    job.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Length of the first chunk. The remainder is handed out one element at a
/// time to the earliest chunks, so `n = 10`, `threads = 3` is `4 + 3 + 3`.
#[cfg(not(target_family = "wasm"))]
fn first_chunk_len(n: usize, threads: usize) -> usize {
    n / threads + usize::from(!n.is_multiple_of(threads))
}

#[cfg(all(test, not(target_family = "wasm")))]
thread_local! {
    static SPAWN_LIMIT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// `Some(left)` allows `left` real spawns, then refuses the way `ulimit -u`
/// does. `None` spawns for real.
#[cfg(all(test, not(target_family = "wasm")))]
fn spawn_blocked() -> bool {
    SPAWN_LIMIT.with(|cell| match cell.get() {
        None => false,
        Some(0) => true,
        Some(left) => {
            cell.set(Some(left - 1));
            false
        }
    })
}

/// Restores the spawn limit when dropped, including on a panic.
#[cfg(all(test, not(target_family = "wasm")))]
pub(super) struct SpawnLimit {
    prev: Option<usize>,
}

#[cfg(all(test, not(target_family = "wasm")))]
impl SpawnLimit {
    /// Allow `n` real spawns on this thread, then run chunks inline.
    pub(super) fn at_most(n: usize) -> Self {
        let prev = SPAWN_LIMIT.with(|cell| cell.replace(Some(n)));
        Self { prev }
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
impl Drop for SpawnLimit {
    fn drop(&mut self) {
        let prev = self.prev;
        SPAWN_LIMIT.with(|cell| cell.set(prev));
    }
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

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn refused_spawns_still_cover_every_index() {
        for limit in [0usize, 1, 2] {
            let _limit = SpawnLimit::at_most(limit);
            let mut items = vec![0u32; 37];
            for_each_chunk_mut(&mut items, 4, |range, slot| {
                assert_eq!(slot.len(), range.len());
                for (local, value) in slot.iter_mut().enumerate() {
                    *value = (range.start + local) as u32;
                }
            });
            assert_eq!(items, (0..37).collect::<Vec<_>>(), "limit={limit}");
            let ranges = ranges_for(40, 5);
            assert_eq!(ranges.iter().map(|range| range.len()).sum::<usize>(), 40);
        }
    }
}
