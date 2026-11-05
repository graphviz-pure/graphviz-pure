//! Limits and cooperative cancellation for library calls. Not in C: C has no limits, and
//! layout cost is unbounded there (decision D8; design in `docs/roadmap/limits.md`).
//!
//! A [`Limits`] value says what a call may use. [`Limits::run`] applies it to the calls its
//! closure makes on this thread. Outside a `run`, no limit applies and every function here
//! returns at once, so output and control flow are those of an unlimited call.
//!
//! The long loops of the engines and renderers call [`tick`] (or [`tick_by`]) once per
//! iteration. A tick is one thread-local countdown and a branch. Every
//! [`POLL_INTERVAL`] units of work the slow path reads the cancel tokens, the
//! `cancel_when` closures, the clock and the heap probe. Once a limit is reached, every
//! later tick in the same `run` reports it, so each loop leaves the way it leaves at its
//! iteration limit, and the facade turns the stopped call into the limit's error.
//!
//! The input-size limits ([`max_input_bytes`], [`max_nodes`], [`max_edges`]) are checked
//! where the input is read, and their errors are ordinary errors of that call; they do
//! not stop the rest of the `run`.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::error::GraphvizError;

/// Units of work between two reads of the tokens, the clock and the heap probe.
pub const POLL_INTERVAL: u64 = 1024;

/// Which limit a [`GraphvizErrorKind::LimitExceeded`](crate::GraphvizErrorKind) error
/// reached, from [`GraphvizError::limit`].
///
/// Non-exhaustive, so a new limit can be added in a minor release. A `match` outside this
/// crate needs a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    /// [`Limits::max_input_bytes`]: the DOT source is longer.
    InputBytes,
    /// [`Limits::max_nodes`]: a graph declares more nodes.
    Nodes,
    /// [`Limits::max_edges`]: a graph declares more edges.
    Edges,
    /// [`Limits::deadline`]: the deadline passed.
    Deadline,
    /// [`Limits::max_steps`]: the work budget ran out.
    Steps,
    /// [`Limits::max_allocation`]: an input-sized buffer would be larger.
    Allocation,
    /// [`Limits::heap_limit`]: the heap probe read more.
    Heap,
}

/// A flag that cancels the [`Limits::run`] calls that hold it.
///
/// Clone it and call [`Self::cancel`] from any thread, or from a closure the call runs.
/// The running call sees it at its next checkpoint and returns a
/// [`GraphvizErrorKind::Cancelled`](crate::GraphvizErrorKind) error. A cancelled token stays
/// cancelled.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels every call that holds this token or a clone of it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::cancel`] was called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

type Poll = Arc<dyn Fn() -> bool + Send + Sync>;
type Probe = Arc<dyn Fn() -> usize + Send + Sync>;

/// What a call may use: input size, graph size, time, work, memory, and when to cancel it.
///
/// Every limit is off in [`Limits::new`] (and [`Default`]). Each builder method sets one
/// and returns the value. [`Self::run`] applies the limits to the `graphviz-pure` and
/// `graphviz-pure-raster` calls its closure makes on the calling thread:
///
/// ```
/// use graphviz_pure_core::limits::{CancelToken, Limit, Limits};
/// use graphviz_pure_core::GraphvizErrorKind;
///
/// let cancel = CancelToken::new();
/// let limits = Limits::new().max_nodes(1_000).cancel_token(cancel.clone());
/// let outcome = limits.run(|| {
///     // Calls into graphviz-pure here are limited.
///     Ok::<_, graphviz_pure_core::GraphvizError>(())
/// });
/// assert!(outcome.is_ok());
/// ```
///
/// A limit that is reached ends the call with a
/// [`GraphvizErrorKind::LimitExceeded`](crate::GraphvizErrorKind) error, whose
/// [`GraphvizError::limit`] names it, or a
/// [`GraphvizErrorKind::Cancelled`](crate::GraphvizErrorKind) error. A call either returns
/// its whole output, the same bytes as without limits, or the error.
#[derive(Clone, Default)]
pub struct Limits {
    input_bytes: Option<u64>,
    nodes: Option<u64>,
    edges: Option<u64>,
    deadline: Option<Instant>,
    steps: Option<u64>,
    allocation: Option<u64>,
    heap: Option<(u64, Probe)>,
    tokens: Vec<CancelToken>,
    polls: Vec<Poll>,
}

impl fmt::Debug for Limits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Limits")
            .field("input_bytes", &self.input_bytes)
            .field("nodes", &self.nodes)
            .field("edges", &self.edges)
            .field("deadline", &self.deadline)
            .field("steps", &self.steps)
            .field("allocation", &self.allocation)
            .field("heap", &self.heap.as_ref().map(|(max, _)| max))
            .field("tokens", &self.tokens)
            .field("cancel_when", &self.polls.len())
            .finish()
    }
}

/// The smaller of two optional limits.
fn stricter<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl Limits {
    /// No limits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The longest DOT source a call accepts, in bytes of the `&str` passed in.
    #[must_use]
    pub fn max_input_bytes(mut self, bytes: u64) -> Self {
        self.input_bytes = Some(bytes);
        self
    }

    /// The most nodes one graph may declare, counted while it is parsed. Nodes an engine
    /// adds (dot's virtual nodes) are not counted: [`Self::max_steps`],
    /// [`Self::deadline`] and [`Self::heap_limit`] bound those.
    #[must_use]
    pub fn max_nodes(mut self, nodes: u64) -> Self {
        self.nodes = Some(nodes);
        self
    }

    /// The most edges one graph may declare, counted while it is parsed.
    #[must_use]
    pub fn max_edges(mut self, edges: u64) -> Self {
        self.edges = Some(edges);
        self
    }

    /// When to stop. Read at checkpoints, so a call stops shortly after it, not at it.
    ///
    /// `wasm32-unknown-unknown` has no clock, so an [`Instant`] cannot be made there; use
    /// [`Self::cancel_when`] with the host's clock.
    #[must_use]
    pub fn deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// A deterministic work budget. Each checkpoint counts its units of work; the run stops
    /// when the count passes `steps`. The same input, budget and release stop at the same
    /// point on every machine. The count is not stable across releases: one that moves a
    /// checkpoint changes it.
    #[must_use]
    pub fn max_steps(mut self, steps: u64) -> Self {
        self.steps = Some(steps);
        self
    }

    /// The largest single buffer whose size the input chooses (polygon vertices, dense
    /// matrices, the raster pixmap), in bytes. Checked before the buffer is allocated.
    #[must_use]
    pub fn max_allocation(mut self, bytes: u64) -> Self {
        self.allocation = Some(bytes);
        self
    }

    /// A total-heap limit. `probe` returns the bytes in use, read from the caller's own
    /// counting allocator (a library cannot install one); checkpoints compare it with
    /// `bytes`. The probe runs only at checkpoints, so one allocation larger than the room
    /// left is not stopped by it: [`Self::max_allocation`] covers the input-sized buffers,
    /// and an OS limit covers the rest.
    ///
    /// The probe runs inside the call, like a [`Self::cancel_when`] closure, with the same
    /// rules for calling back into the library.
    #[must_use]
    pub fn heap_limit(
        mut self,
        bytes: u64,
        probe: impl Fn() -> usize + Send + Sync + 'static,
    ) -> Self {
        self.heap = Some((bytes, Arc::new(probe)));
        self
    }

    /// Cancels the call when `token` is cancelled. Several tokens may be added.
    #[must_use]
    pub fn cancel_token(mut self, token: CancelToken) -> Self {
        self.tokens.push(token);
        self
    }

    /// Cancels the call when `poll` returns `true`. Checkpoints call it, so a caller with
    /// one thread (an event loop, WebAssembly) can cancel. Several may be added.
    ///
    /// `poll` runs inside the call, on its thread. It may call back into the library: that
    /// call runs under the same limits, counts its work against them, and does not run the
    /// closures again (a nested check reports the state as it is), so it cannot recurse.
    #[must_use]
    pub fn cancel_when(mut self, poll: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.polls.push(Arc::new(poll));
        self
    }

    /// Runs `f` with these limits on the `graphviz-pure` and `graphviz-pure-raster` calls it
    /// makes on this thread, and returns what `f` returns.
    ///
    /// The time, work, heap and cancel limits cover the whole `run`: work done by one call
    /// counts against the next, and once one is reached every later call in the `run`
    /// fails with its error. The input-size and allocation limits apply to each call on its
    /// own.
    ///
    /// A nested `run` adds its limits to the enclosing ones: the stricter value of each
    /// applies, and every token and `cancel_when` closure of both is checked.
    ///
    /// The limits live in thread-local state. A thread that `f` starts does not see them,
    /// and neither do the worker threads of the `parallel` feature; the checkpoints are on
    /// the calling thread. An `async` caller cannot hold a `run` across an `.await`: call
    /// `run` inside the synchronous part, around the library calls.
    ///
    /// The checks are cooperative: they sit in the long loops, so a call stops at its next
    /// checkpoint, which in some phases comes long after the limit.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        push(self);
        let _guard = PopGuard;
        f()
    }

    /// `self` inside `outer`: the stricter value of each limit, and both sets of
    /// tokens and closures.
    fn within(&self, outer: &Limits) -> Limits {
        let heap = match (&outer.heap, &self.heap) {
            (Some(a), Some(b)) => Some(if b.0 < a.0 { b.clone() } else { a.clone() }),
            (a, b) => a.clone().or_else(|| b.clone()),
        };
        Limits {
            input_bytes: stricter(outer.input_bytes, self.input_bytes),
            nodes: stricter(outer.nodes, self.nodes),
            edges: stricter(outer.edges, self.edges),
            deadline: stricter(outer.deadline, self.deadline),
            steps: None,
            allocation: stricter(outer.allocation, self.allocation),
            heap,
            tokens: outer.tokens.iter().chain(&self.tokens).cloned().collect(),
            polls: outer.polls.iter().chain(&self.polls).cloned().collect(),
        }
    }
}

/// One [`Limits::run`] on the thread's stack.
struct Frame {
    /// The limits in force: this `run`'s inside the enclosing ones.
    limits: Rc<Limits>,
    /// The step count at which the work budget runs out, counted from the outermost `run`.
    step_cap: Option<u64>,
    /// The enclosing frame's reached limit, restored when this frame pops.
    saved_trip: Option<GraphvizError>,
}

struct State {
    frames: Vec<Frame>,
    /// Units of work counted since the outermost `run` began.
    steps: u64,
    /// The units the countdown was armed with; 0 when it is not armed.
    armed: u64,
    /// The limit reached in this `run`.
    trip: Option<GraphvizError>,
}

thread_local! {
    /// Units of work before the next slow check. `u64::MAX` outside a `run`, 0 once a limit
    /// is reached.
    static COUNTDOWN: Cell<u64> = const { Cell::new(u64::MAX) };
    /// Whether [`evaluate`] is running the caller's closures.
    static EVALUATING: Cell<bool> = const { Cell::new(false) };
    static MAX_INPUT_BYTES: Cell<u64> = const { Cell::new(u64::MAX) };
    static MAX_NODES: Cell<u64> = const { Cell::new(u64::MAX) };
    static MAX_EDGES: Cell<u64> = const { Cell::new(u64::MAX) };
    static MAX_ALLOCATION: Cell<u64> = const { Cell::new(u64::MAX) };
    static STATE: RefCell<State> = const {
        RefCell::new(State { frames: Vec::new(), steps: 0, armed: 0, trip: None })
    };
}

/// Clears [`EVALUATING`], including when a closure panics.
struct EvaluatingGuard;

impl Drop for EvaluatingGuard {
    fn drop(&mut self) {
        EVALUATING.set(false);
    }
}

/// Pops the [`Limits::run`] frame, including when the closure panics.
struct PopGuard;

impl Drop for PopGuard {
    fn drop(&mut self) {
        pop();
    }
}

/// Adds the units counted since the countdown was armed (or last settled) to the step
/// count, and counts on from the countdown as it is.
fn settle_count(s: &mut State) {
    if s.armed > 0 {
        let left = COUNTDOWN.get();
        s.steps = s.steps.saturating_add(s.armed.saturating_sub(left));
        s.armed = left;
    }
}

/// Sets the countdown for the top frame: the next slow check comes after
/// [`POLL_INTERVAL`] units, or when the work budget runs out. The units counted since the
/// countdown was last armed are added first, so work done in between (a library call
/// inside a `cancel_when` closure) is not lost.
fn arm(s: &mut State) {
    settle_count(s);
    let Some(frame) = s.frames.last() else {
        s.armed = 0;
        COUNTDOWN.set(u64::MAX);
        return;
    };
    if s.trip.is_some() {
        s.armed = 0;
        COUNTDOWN.set(0);
        return;
    }
    let mut units = POLL_INTERVAL;
    if let Some(cap) = frame.step_cap {
        units = units.min(cap.saturating_add(1).saturating_sub(s.steps));
    }
    s.armed = units;
    COUNTDOWN.set(units);
}

/// Copies the top frame's size limits into the cells the parser and allocators read.
fn publish(s: &State) {
    let limits = s.frames.last().map(|f| Rc::clone(&f.limits));
    let get =
        |pick: fn(&Limits) -> Option<u64>| limits.as_deref().and_then(pick).unwrap_or(u64::MAX);
    MAX_INPUT_BYTES.set(get(|l| l.input_bytes));
    MAX_NODES.set(get(|l| l.nodes));
    MAX_EDGES.set(get(|l| l.edges));
    MAX_ALLOCATION.set(get(|l| l.allocation));
}

fn push(limits: &Limits) {
    STATE.with_borrow_mut(|s| {
        settle_count(s);
        let (merged, outer_cap) = match s.frames.last() {
            Some(outer) => (limits.within(&outer.limits), outer.step_cap),
            None => (limits.clone(), None),
        };
        let own_cap = limits.steps.map(|n| s.steps.saturating_add(n));
        s.frames.push(Frame {
            limits: Rc::new(merged),
            step_cap: stricter(outer_cap, own_cap),
            saved_trip: s.trip.clone(),
        });
        publish(s);
        arm(s);
    });
    // A token cancelled before the call, or a deadline already past, stops the first call.
    evaluate();
}

fn pop() {
    STATE.with_borrow_mut(|s| {
        settle_count(s);
        if let Some(frame) = s.frames.pop() {
            s.trip = frame.saved_trip;
        }
        if s.frames.is_empty() {
            s.steps = 0;
            s.trip = None;
        }
        publish(s);
        arm(s);
    });
}

/// Checks every limit of the top frame now, outside the state's borrow (the closures are
/// the caller's code), and records the first one reached. Returns whether one is.
fn evaluate() -> bool {
    // A `cancel_when` or heap-probe closure that calls into the library comes back here
    // from inside `reached`. The nested evaluation reads nothing and reports the state as
    // it is, so the closure cannot recurse without end.
    if EVALUATING.get() {
        return STATE.with_borrow(|s| s.trip.is_some());
    }
    let Some((limits, steps, cap)) = STATE.with_borrow_mut(|s| {
        if s.trip.is_some() {
            return None;
        }
        settle_count(s);
        let top = s.frames.last()?;
        Some((Rc::clone(&top.limits), s.steps, top.step_cap))
    }) else {
        return STATE.with_borrow(|s| s.trip.is_some());
    };
    EVALUATING.set(true);
    let _guard = EvaluatingGuard;
    let trip = reached(&limits, steps, cap);
    STATE.with_borrow_mut(|s| {
        if s.trip.is_none() {
            s.trip = trip;
        }
        arm(s);
        s.trip.is_some()
    })
}

/// The first limit reached, in a fixed order: the work budget first, as it is the
/// deterministic one, then cancellation, the deadline and the heap.
fn reached(limits: &Limits, steps: u64, cap: Option<u64>) -> Option<GraphvizError> {
    if cap.is_some_and(|cap| steps > cap) {
        return Some(GraphvizError::limit_exceeded(
            Limit::Steps,
            "the work budget ran out",
        ));
    }
    if limits.tokens.iter().any(CancelToken::is_cancelled) || limits.polls.iter().any(|p| p()) {
        return Some(GraphvizError::cancelled());
    }
    if limits.deadline.is_some_and(|d| Instant::now() >= d) {
        return Some(GraphvizError::limit_exceeded(
            Limit::Deadline,
            "the deadline passed",
        ));
    }
    if let Some((max, probe)) = &limits.heap {
        let used = probe() as u64;
        if used > *max {
            return Some(GraphvizError::limit_exceeded(
                Limit::Heap,
                format!("the heap holds {used} bytes, over the limit of {max}"),
            ));
        }
    }
    None
}

/// A checkpoint for one unit of work. Returns `true` when a limit is reached: the loop
/// stops as it stops at its iteration limit. Outside a [`Limits::run`] it returns `false`.
#[inline]
pub fn tick() -> bool {
    tick_by(1)
}

/// A checkpoint for `units` of work (the elements one iteration of a coarse loop visits),
/// so a long iteration counts as more work than a short one.
#[inline]
pub fn tick_by(units: u64) -> bool {
    let left = COUNTDOWN.get();
    if units < left {
        COUNTDOWN.set(left - units);
        return false;
    }
    slow_tick(units)
}

#[cold]
#[inline(never)]
fn slow_tick(units: u64) -> bool {
    let in_run = STATE.with_borrow_mut(|s| {
        if s.frames.is_empty() {
            COUNTDOWN.set(u64::MAX);
            return false;
        }
        if s.trip.is_none() {
            settle_count(s);
            s.steps = s.steps.saturating_add(units);
            s.armed = 0;
            COUNTDOWN.set(0);
        }
        true
    });
    in_run && evaluate()
}

/// Whether a limit was reached in this `run`, without counting work.
#[must_use]
pub fn stopped() -> bool {
    COUNTDOWN.get() == 0 && STATE.with_borrow(|s| s.trip.is_some())
}

/// The error of the limit reached in this `run`, if one was.
#[must_use]
pub fn error() -> Option<GraphvizError> {
    STATE.with_borrow(|s| s.trip.clone())
}

/// A checkpoint at the start of a library call: reads every limit now, and returns the
/// error of one that is reached.
pub fn check() -> Result<(), GraphvizError> {
    if STATE.with_borrow(|s| s.frames.is_empty()) {
        return Ok(());
    }
    if evaluate() {
        return Err(error().unwrap_or_else(GraphvizError::cancelled));
    }
    Ok(())
}

/// `result`, or the error of the limit reached during the call that produced it. A call that
/// was stopped part way returns the limit's error, whatever its pipeline returned.
pub fn settle<T>(result: Result<T, GraphvizError>) -> Result<T, GraphvizError> {
    match error() {
        Some(e) => Err(e),
        None => result,
    }
}

/// [`Limits::max_input_bytes`] for a DOT source of `len` bytes.
pub fn check_input_bytes(len: usize) -> Result<(), GraphvizError> {
    let max = MAX_INPUT_BYTES.get();
    let len = len as u64;
    if len > max {
        return Err(GraphvizError::limit_exceeded(
            Limit::InputBytes,
            format!("the input has {len} bytes, over the limit of {max}"),
        ));
    }
    Ok(())
}

/// [`Limits::max_nodes`] for a graph that now has `count` nodes.
pub fn check_nodes(count: usize) -> Result<(), GraphvizError> {
    let max = MAX_NODES.get();
    if count as u64 > max {
        return Err(GraphvizError::limit_exceeded(
            Limit::Nodes,
            format!("the graph has more than {max} nodes"),
        ));
    }
    Ok(())
}

/// [`Limits::max_edges`] for a graph that now has `count` edges.
pub fn check_edges(count: usize) -> Result<(), GraphvizError> {
    let max = MAX_EDGES.get();
    if count as u64 > max {
        return Err(GraphvizError::limit_exceeded(
            Limit::Edges,
            format!("the graph has more than {max} edges"),
        ));
    }
    Ok(())
}

/// [`Limits::max_allocation`] for a buffer of `bytes`.
pub fn check_allocation(bytes: u64) -> Result<(), GraphvizError> {
    let max = MAX_ALLOCATION.get();
    if bytes > max {
        return Err(GraphvizError::limit_exceeded(
            Limit::Allocation,
            format!("an allocation of {bytes} bytes is over the limit of {max}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphvizErrorKind;
    use std::sync::atomic::AtomicUsize;

    const _: fn() = || {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<Limits>();
        assert_send_sync_static::<CancelToken>();
        assert_send_sync_static::<Limit>();
    };

    /// Ticks until a limit is reached, at most `max` times; the number of ticks taken.
    fn ticks_until_stop(max: u64) -> Option<u64> {
        (1..=max).find(|_| tick())
    }

    #[test]
    fn outside_a_run_nothing_stops() {
        assert_eq!(ticks_until_stop(10_000), None);
        assert!(!stopped());
        assert!(check().is_ok());
        assert!(check_nodes(usize::MAX).is_ok());
        assert!(check_allocation(u64::MAX).is_ok());
        assert_eq!(COUNTDOWN.get(), u64::MAX - 10_000);
    }

    #[test]
    fn the_work_budget_stops_at_the_same_tick_every_time() {
        for budget in [0, 1, 5, 1023, 1024, 1025, 5000] {
            let first = Limits::new()
                .max_steps(budget)
                .run(|| ticks_until_stop(10_000));
            let second = Limits::new()
                .max_steps(budget)
                .run(|| ticks_until_stop(10_000));
            assert_eq!(first, Some(budget + 1), "budget {budget}");
            assert_eq!(first, second);
        }
        let err = Limits::new().max_steps(3).run(|| {
            ticks_until_stop(10);
            assert!(stopped());
            error().unwrap()
        });
        assert_eq!(err.kind(), GraphvizErrorKind::LimitExceeded);
        assert_eq!(err.limit(), Some(Limit::Steps));
        assert!(!stopped(), "the limit ends with its run");
    }

    #[test]
    fn weighted_ticks_count_their_units() {
        let stop = Limits::new()
            .max_steps(1000)
            .run(|| (1..=100).find(|_| tick_by(100)));
        assert_eq!(stop, Some(11));
    }

    #[test]
    fn a_cancelled_token_stops_the_run() {
        let token = CancelToken::new();
        let limits = Limits::new().cancel_token(token.clone());
        let (before, after) = limits.run(|| {
            let before = ticks_until_stop(5000);
            token.cancel();
            (before, ticks_until_stop(5000))
        });
        assert_eq!(before, None);
        assert!(after.is_some_and(|n| n <= POLL_INTERVAL));
        // Already cancelled: the first check fails.
        let err = limits.run(check).unwrap_err();
        assert_eq!(err.kind(), GraphvizErrorKind::Cancelled);
        assert_eq!(err.limit(), None);
    }

    #[test]
    fn cancel_when_is_polled_on_the_calling_thread() {
        let polls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&polls);
        let stop = Limits::new()
            .cancel_when(move || seen.fetch_add(1, Ordering::Relaxed) >= 3)
            .run(|| ticks_until_stop(100_000));
        // Polled when the run starts, then every POLL_INTERVAL ticks.
        assert_eq!(stop, Some(3 * POLL_INTERVAL));
        assert_eq!(polls.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn a_past_deadline_stops_the_first_call() {
        let err = Limits::new()
            .deadline(Instant::now())
            .run(check)
            .unwrap_err();
        assert_eq!(err.limit(), Some(Limit::Deadline));
        let far = Instant::now() + std::time::Duration::from_secs(3600);
        assert!(Limits::new().deadline(far).run(check).is_ok());
    }

    #[test]
    fn the_heap_probe_is_read_at_checkpoints() {
        let used = Arc::new(AtomicUsize::new(100));
        let probe = Arc::clone(&used);
        let limits = Limits::new().heap_limit(1000, move || probe.load(Ordering::Relaxed));
        let stop = limits.run(|| {
            assert_eq!(ticks_until_stop(3000), None);
            used.store(1001, Ordering::Relaxed);
            ticks_until_stop(3000)
        });
        assert!(stop.is_some());
        let err = limits.run(check).unwrap_err();
        assert_eq!(err.limit(), Some(Limit::Heap));
        assert_eq!(
            err.message(),
            "the heap holds 1001 bytes, over the limit of 1000"
        );
    }

    #[test]
    fn size_limits_apply_inside_their_run_only() {
        let limits = Limits::new()
            .max_input_bytes(10)
            .max_nodes(2)
            .max_edges(1)
            .max_allocation(64);
        limits.run(|| {
            assert!(check_input_bytes(10).is_ok());
            let e = check_input_bytes(11).unwrap_err();
            assert_eq!(e.limit(), Some(Limit::InputBytes));
            assert_eq!(
                e.to_string(),
                "Limit exceeded: the input has 11 bytes, over the limit of 10"
            );
            assert!(check_nodes(2).is_ok());
            assert_eq!(check_nodes(3).unwrap_err().limit(), Some(Limit::Nodes));
            assert_eq!(check_edges(2).unwrap_err().limit(), Some(Limit::Edges));
            assert_eq!(
                check_allocation(65).unwrap_err().limit(),
                Some(Limit::Allocation)
            );
            // Not a run-wide stop.
            assert!(!stopped());
            assert!(check().is_ok());
        });
        assert!(check_nodes(3).is_ok());
    }

    #[test]
    fn nested_runs_take_the_stricter_limit() {
        let outer = Limits::new().max_steps(100).max_nodes(10);
        let inner = Limits::new().max_steps(1000).max_nodes(5);
        outer.run(|| {
            assert_eq!(ticks_until_stop(50), None);
            inner.run(|| {
                assert!(check_nodes(6).is_err());
                // 50 of the outer budget's 100 are left.
                assert_eq!(ticks_until_stop(1000), Some(51));
            });
            // The outer budget ran out inside the inner run.
            assert!(tick());
            assert!(check_nodes(6).is_ok());
        });
        // An inner budget that runs out does not stop the outer run.
        Limits::new().max_steps(10_000).run(|| {
            Limits::new()
                .max_steps(10)
                .run(|| assert_eq!(ticks_until_stop(100), Some(11)));
            assert!(!stopped());
            assert_eq!(ticks_until_stop(100), None);
        });
    }

    #[test]
    fn a_closure_that_checks_again_does_not_recurse() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let limits = Limits::new().cancel_when(move || {
            seen.fetch_add(1, Ordering::Relaxed);
            // What a library call inside the closure does first.
            check().is_err()
        });
        assert_eq!(limits.run(|| ticks_until_stop(3000)), None);
        // Once when the run starts, then at 1,024 and 2,048 ticks.
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn work_inside_a_closure_counts() {
        // The closure's ticks stand for a library call it makes: 500 units each time it is
        // polled (when the run starts, then every 1,024 ticks), counted against the budget.
        let stop = Limits::new()
            .max_steps(3000)
            .cancel_when(|| {
                for _ in 0..500 {
                    tick();
                }
                false
            })
            .run(|| ticks_until_stop(10_000));
        // 500 + 1,024 + 500 + 977 = 3,001.
        assert_eq!(stop, Some(2001));
    }

    #[test]
    fn a_panic_inside_a_run_pops_it() {
        let caught = std::panic::catch_unwind(|| {
            Limits::new()
                .max_steps(0)
                .run(|| -> () { panic!("inside") })
        });
        assert!(caught.is_err());
        assert_eq!(ticks_until_stop(10), None);
        assert!(STATE.with_borrow(|s| s.frames.is_empty()));
    }

    #[test]
    fn settle_replaces_the_result_of_a_stopped_call() {
        let r = Limits::new().max_steps(0).run(|| {
            tick();
            settle(Ok::<_, GraphvizError>(1))
        });
        assert_eq!(r.unwrap_err().limit(), Some(Limit::Steps));
        assert_eq!(settle(Ok::<_, GraphvizError>(1)).unwrap(), 1);
    }
}
