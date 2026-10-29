//! Diagnostics as `lib/cgraph/agerror.c` reports them: `agerrorf` prints `Error: ...` and
//! raises the error level that `dot`'s `main` returns as its exit status
//! (`graphviz_errors + agerrors()`), while the layout and the output go on. `agwarningf`
//! prints `Warning: ...` and does not change the status.
//!
//! Every diagnostic C prints through `agerr` goes through [`agerr`] here: it applies the
//! level set by [`agseterr`] (`dot -q`), writes `Error: `/`Warning: ` (none for
//! [`AgErrLevel::Prev`]), and writes the message as `default_usererrf` does, up to its first NUL
//! and with control characters escaped as `\ooo`. A message C prints with a bare
//! `fprintf(stderr, ...)` goes through [`fprintf_stderr`] instead, which neither filters nor
//! escapes. `GVDUMP` debugging traces go through [`dump`].
//!
//! The levels are per thread, so concurrent in-process renders do not see each other's errors.
//! A [`capture`] scope collects `agerr` text for that call instead of writing it; the command
//! line does not capture, and prints each message as C does.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::Write;

/// `agerrlevel_t` from `lib/cgraph/cgraph.h`, in C's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgErrLevel {
    /// `AGWARN`: `Warning: ...`.
    Warn,
    /// `AGERR`: `Error: ...`.
    Err,
    /// `AGMAX`: counted as an error but written as `Warning: ...`; as the [`agseterr`] level
    /// it turns every message off.
    Max,
    /// `AGPREV`: continues the previous message, at its level and with no prefix.
    Prev,
}

thread_local! {
    static ERRORS: Cell<usize> = const { Cell::new(0) };
    /// `agerrlevel`: messages below it are not written.
    static LEVEL: Cell<AgErrLevel> = const { Cell::new(AgErrLevel::Warn) };
    /// `agerrno`: the level of the last message, which an [`AgErrLevel::Prev`] message takes.
    static LAST: Cell<AgErrLevel> = const { Cell::new(AgErrLevel::Warn) };
    static EMITTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static QUIET: Cell<u32> = const { Cell::new(0) };
    static SILENT: Cell<u32> = const { Cell::new(0) };
    static VERBOSE: Cell<i32> = const { Cell::new(0) };
}

/// Increments `depth` for the life of the returned guard.
fn enter(depth: &'static std::thread::LocalKey<Cell<u32>>) -> impl Drop {
    struct Guard(&'static std::thread::LocalKey<Cell<u32>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.with(|q| q.set(q.get() - 1));
        }
    }
    depth.with(|q| q.set(q.get() + 1));
    Guard(depth)
}

/// Runs `f` as a repeat of work C does once. The port sizes labels and nodes again after
/// C's `common_init_node`/`common_init_edge`/`do_graph_label` (for ports, splines and
/// rendering), and the diagnostics of those calls (HTML label warnings and errors,
/// `bad label format`, `size too small for label`) must not print a second time.
///
/// Port warnings still print: C resolves ports again in some of these places and warns again.
pub fn quietly<R>(f: impl FnOnce() -> R) -> R {
    let _guard = enter(&QUIET);
    f()
}

/// [`quietly`], and port warnings too, for a repeat that C does not make at all.
pub fn silently<R>(f: impl FnOnce() -> R) -> R {
    let _guard = enter(&SILENT);
    f()
}

/// False inside [`quietly`] and [`silently`]: label and node sizing diagnostics are reported
/// only outside them.
pub fn reporting() -> bool {
    QUIET.with(|q| q.get() == 0) && reporting_ports()
}

/// False inside [`silently`]: port warnings are reported only outside it.
pub fn reporting_ports() -> bool {
    SILENT.with(|q| q.get() == 0)
}

/// `emit_once(str)`: true the first time `key` is seen on this thread.
///
/// A [`capture`] starts from an empty set and restores the previous one when
/// the call ends, so the same warning is reported again on the next library
/// call. The command line does not capture, and a key stays seen for the rest
/// of the thread: C's once-per-process behaviour.
pub fn emit_once(key: &str) -> bool {
    EMITTED.with(|e| e.borrow_mut().insert(key.to_string()))
}

/// `agseterr(lvl)`: messages below `lvl` are no longer written (they still count as errors).
/// `dot -q` sets [`AgErrLevel::Err`], `-q2` and higher [`AgErrLevel::Max`]. Returns the old level.
pub fn agseterr(lvl: AgErrLevel) -> AgErrLevel {
    LEVEL.with(|l| l.replace(lvl))
}

/// `agerr(level, msg)` (`agerr_va`): `msg` is the C format output, normally ending in a newline.
pub fn agerr(level: AgErrLevel, msg: &str) {
    if let Some(bytes) = agerr_va(level, msg) {
        emit_agerr(level, &bytes);
    }
}

/// `agerr_va`: updates the levels and the error count, and gives the bytes `out` writes, if
/// the message is at or above the [`agseterr`] level.
fn agerr_va(level: AgErrLevel, msg: &str) -> Option<Vec<u8>> {
    let lvl = match level {
        AgErrLevel::Prev => LAST.with(Cell::get),
        AgErrLevel::Max => AgErrLevel::Err,
        l => l,
    };
    LAST.with(|l| l.set(lvl));
    // `agmaxerr`: dot's exit status counts an error whether or not it was written.
    if lvl == AgErrLevel::Err && level != AgErrLevel::Prev {
        ERRORS.with(|c| c.set(c.get() + 1));
    }
    if lvl < LEVEL.with(Cell::get) {
        return None;
    }
    // `out`: the prefix and the message each go through `default_usererrf`.
    let text = match level {
        AgErrLevel::Prev => Cow::Borrowed(msg),
        // `level == AGERR ? "Error" : "Warning"`: an AGMAX message counts as an error but
        // is written as a warning.
        AgErrLevel::Err => Cow::Owned(format!("Error: {msg}")),
        AgErrLevel::Warn | AgErrLevel::Max => Cow::Owned(format!("Warning: {msg}")),
    };
    Some(usererr_bytes(&text))
}

/// `agerrorf(msg)`.
pub fn agerrorf(msg: &str) {
    agerr(AgErrLevel::Err, msg);
}

/// `agwarningf(msg)`.
pub fn agwarningf(msg: &str) {
    agerr(AgErrLevel::Warn, msg);
}

/// `agerr(AGPREV, msg)`: continues the previous message.
pub fn agerr_prev(msg: &str) {
    agerr(AgErrLevel::Prev, msg);
}

/// Sets `gvc->common.verbose`, the level `-v[n]` gives (`-v`: 1, `-vN`: N).
pub fn set_verbose(level: i32) {
    VERBOSE.with(|v| v.set(level));
}

/// `gvc->common.verbose`: the run's verbose level.
pub fn verbose() -> i32 {
    VERBOSE.with(Cell::get)
}

/// `Verbose`, the byte `gvParseArgs` keeps of the level (`lib/common/input.c:254`): above
/// `UCHAR_MAX` it is `UCHAR_MAX`, below it its low byte.
pub fn verbose_byte() -> u8 {
    let v = verbose();
    if v > i32::from(u8::MAX) {
        u8::MAX
    } else {
        v as u8
    }
}

/// `if (Verbose)`: whether `-v` (at any level) is on.
pub fn verbose_on() -> bool {
    verbose_byte() != 0
}

/// `if (Verbose > 1)`: whether the level-2 trace (`-v2` and up) is on.
///
/// C guards its level-2 traces with this test and formats their lines inside it; the
/// port's callers do the same, so a trace line costs nothing when it is off.
pub fn verbose_gt1() -> bool {
    verbose_byte() > 1
}

/// `fprintf(stderr, ...)` under `if (Verbose)`: the layouts' progress messages. The arguments
/// are `format!`'s, formatted only when `-v` is on.
#[macro_export]
macro_rules! verbose_msg {
    ($($arg:tt)*) => {
        if $crate::diag::verbose_on() {
            $crate::diag::fprintf_stderr(&format!($($arg)*));
        }
    };
}

/// `GV_INFO(...)` (`lib/util/debug.h`): `gv_info!(file, line, format...)`, under `-v`
/// ([`gv_info_line`]); the message is formatted only when `-v` is on.
#[macro_export]
macro_rules! gv_info {
    ($file:expr, $line:expr, $($arg:tt)*) => {
        if $crate::diag::verbose_on() {
            $crate::diag::gv_info_line($file, $line, &format!($($arg)*));
        }
    };
}

/// `GV_INFO(...)` (`lib/util/debug.h`): under `-v`, `[Graphviz] <file>:<line>: <time> <msg>`
/// and a newline, where `file` and `line` are C's (the port names the C site it follows). C
/// stamps local time; the port, which reads no time zone, stamps UTC.
pub fn gv_info_line(file: &str, line: u32, msg: &str) {
    let (y, mo, d, h, mi, sec) = utc_now();
    fprintf_stderr(&format!(
        "[Graphviz] {file}:{line}: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{sec:02} {msg}\n"
    ));
}

/// The time as UTC calendar fields (year, month, day, hour, minute, second).
fn utc_now() -> (i64, i64, i64, i64, i64, i64) {
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // wasm32-unknown-unknown has no clock (the wasm bindings never set a verbose level)
    #[cfg(target_arch = "wasm32")]
    let secs = 0i64;
    civil(secs)
}

/// Seconds since 1970 as UTC calendar fields (year, month, day, hour, minute, second).
fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem / 60 % 60, rem % 60)
}

thread_local! {
    static TIMER: Cell<Option<std::time::Duration>> = const { Cell::new(None) };
}

/// The time since the process (thread) started counting, for [`start_timer`].
fn clock() -> std::time::Duration {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();
        ORIGIN.get_or_init(std::time::Instant::now).elapsed()
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::time::Duration::ZERO
    }
}

/// The majorization solvers' `-v` step: `: <seconds> sec`, a newline and the next phase's
/// name, then the clock restarted for it.
pub fn verbose_phase(next: &str) {
    if verbose_on() {
        fprintf_stderr(&format!(": {} sec\n{next}", fmt_f(elapsed_sec(), 2)));
        start_timer();
    }
}

/// `start_timer()` (`lib/common/timing.c`): starts the clock [`elapsed_sec`] reads. C measures
/// processor time, the port wall time.
pub fn start_timer() {
    TIMER.with(|t| t.set(Some(clock())));
}

/// `elapsed_sec()`: seconds since the last [`start_timer`].
pub fn elapsed_sec() -> f64 {
    let start = TIMER.with(Cell::get).unwrap_or_default();
    clock().saturating_sub(start).as_secs_f64()
}

/// C `printf("%.{prec}f", v)` (macOS libc): `nan`, `inf` and `-inf` for the non-finite values.
pub fn fmt_f(v: f64, prec: usize) -> String {
    if v.is_nan() {
        "nan".to_string()
    } else if v.is_infinite() {
        if v > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        format!("{v:.prec$}")
    }
}

/// `fprintf(stderr, msg)`: a message C writes without `agerr`, so with no level, no count and
/// no escaping. `msg` is written as C's bytes ([`crate::charset::encode`]).
///
/// Not a diagnostic. `-v` progress uses this, and it stays on the command line: a
/// [`capture`] scope drops it instead of returning it.
pub fn fprintf_stderr(msg: &str) {
    write_stderr(&crate::charset::encode(msg));
}

/// One line of a `GVDUMP` debugging trace. These have no C counterpart and are not diagnostics.
pub fn dump(line: &str) {
    write_stderr(format!("{line}\n").as_bytes());
}

/// `default_usererrf`'s bytes for `msg`: C's bytes up to the first NUL, with each control
/// character other than white space (`gv_iscntrl && !gv_isspace`) written as `\ooo`.
fn usererr_bytes(msg: &str) -> Vec<u8> {
    let bytes = crate::charset::encode(msg);
    let bytes = bytes.split(|&b| b == 0).next().unwrap_or_default();
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if (b <= 31 || b == 127) && !matches!(b, b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            out.extend_from_slice(format!("\\{b:03o}").as_bytes());
        } else {
            out.push(b);
        }
    }
    out
}

/// Thread-local state a [`capture`] saves and restores, so one call cannot
/// change what the next call on this thread observes.
struct SavedDiag {
    emitted: HashSet<String>,
    errors: usize,
    last: AgErrLevel,
    /// The caller's quiet mode (`dot -q`). Restored, not cleared: the call
    /// observes the mode it was entered under, and cannot leave a new one behind.
    level: AgErrLevel,
    quiet: u32,
    silent: u32,
}

struct Frame {
    diagnostics: Vec<crate::error::Diagnostic>,
    saved: SavedDiag,
}

/// The current capture lives in `top`. `below` holds the captures it is nested
/// in, and that `Vec` is allocated only then.
///
/// An empty [`HashSet`] and an empty [`Vec`] allocate nothing, so one capture
/// that emits no diagnostic and no once-only key allocates nothing. The command
/// line captures once, for a raster format, and the allocation ratchet is exact.
struct Frames {
    top: Option<Frame>,
    below: Vec<Frame>,
}

thread_local! {
    static FRAMES: RefCell<Frames> = const { RefCell::new(Frames { top: None, below: Vec::new() }) };
}

fn push_frame() {
    let saved = SavedDiag {
        emitted: EMITTED.with(|cell| cell.replace(HashSet::new())),
        errors: ERRORS.with(|cell| cell.replace(0)),
        last: LAST.with(|cell| cell.replace(AgErrLevel::Warn)),
        level: LEVEL.with(Cell::get),
        quiet: QUIET.with(|cell| cell.replace(0)),
        silent: SILENT.with(|cell| cell.replace(0)),
    };
    FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        if let Some(prev) = frames.top.take() {
            frames.below.push(prev);
        }
        frames.top = Some(Frame {
            diagnostics: Vec::new(),
            saved,
        });
    });
    crate::color::push_capture_colors();
}

fn pop_frame() {
    crate::color::pop_capture_colors();
    let Some(frame) = FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        let top = frames.top.take()?;
        frames.top = frames.below.pop();
        Some(top)
    }) else {
        return;
    };
    EMITTED.with(|cell| {
        cell.replace(frame.saved.emitted);
    });
    ERRORS.with(|cell| cell.set(frame.saved.errors));
    LAST.with(|cell| cell.set(frame.saved.last));
    LEVEL.with(|cell| cell.set(frame.saved.level));
    QUIET.with(|cell| cell.set(frame.saved.quiet));
    SILENT.with(|cell| cell.set(frame.saved.silent));
}

/// Pops the [`capture`] frame, including when `f` panics or returns early, so a
/// call cannot leave its messages or its once-only set on the thread.
struct FrameGuard;

impl Drop for FrameGuard {
    fn drop(&mut self) {
        pop_frame();
    }
}

/// Runs `f` and returns the `agerr` messages it wrote, in order, instead of
/// writing them to stderr.
///
/// The frame is this call's alone. A nested [`capture`] keeps its own messages
/// and its own once-only set, and another thread has its own frame, so two
/// layouts cannot mix diagnostics. `fprintf` to stderr (including `-v`) is
/// dropped for the duration: the command line does not capture, so its progress
/// and its own messages still go to stderr as they are written.
///
/// A warning C prints once per process stays once per process on the command
/// line, which does not capture: the key stays seen for the rest of that
/// thread. A library call captures, so the same warning is reported again on
/// the next call.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, Vec<crate::error::Diagnostic>) {
    push_frame();
    let _guard = FrameGuard;
    let result = f();
    let diagnostics = FRAMES.with(|frames| {
        frames
            .borrow_mut()
            .top
            .as_mut()
            .map(|frame| std::mem::take(&mut frame.diagnostics))
            .unwrap_or_default()
    });
    (result, diagnostics)
}

fn capturing() -> bool {
    FRAMES.with(|frames| frames.borrow().top.is_some())
}

/// The level a collected message reports. `AGPREV` keeps the level of the
/// message it continues (`AGMAX` is stored as an error for that purpose).
fn diagnostic_level(original: AgErrLevel) -> crate::error::DiagnosticLevel {
    use crate::error::DiagnosticLevel;
    match original {
        AgErrLevel::Err => DiagnosticLevel::Error,
        AgErrLevel::Warn => DiagnosticLevel::Warning,
        AgErrLevel::Max => DiagnosticLevel::CountedAsError,
        AgErrLevel::Prev => match LAST.with(Cell::get) {
            AgErrLevel::Warn => DiagnosticLevel::Warning,
            AgErrLevel::Max => DiagnosticLevel::CountedAsError,
            AgErrLevel::Err | AgErrLevel::Prev => DiagnosticLevel::Error,
        },
    }
}

fn emit_agerr(original: AgErrLevel, bytes: &[u8]) {
    if capturing() {
        let message = String::from_utf8_lossy(bytes).into_owned();
        let diagnostic = crate::error::Diagnostic::new(diagnostic_level(original), message);
        FRAMES.with(|frames| {
            if let Some(frame) = frames.borrow_mut().top.as_mut() {
                frame.diagnostics.push(diagnostic);
            }
        });
        return;
    }
    write_stderr(bytes);
}

// The one place that writes to stderr (`clippy.toml`).
#[allow(clippy::disallowed_methods)]
fn write_stderr(bytes: &[u8]) {
    // A library call is collecting. stderr belongs to the command line, which
    // does not capture, so its writes still land here in order.
    if capturing() {
        return;
    }
    let _ = std::io::stderr().write_all(bytes);
}

/// C `printf("%.{prec}g", v)` on the reference platform (macOS libc), as diagnostics and
/// renderers print numbers.
pub fn fmt_g(v: f64, prec: usize) -> String {
    fn trim(mut s: String) -> String {
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        s
    }
    if v.is_nan() {
        // macOS libc prints `nan` for either sign.
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let p = prec.max(1);
    if v == 0.0 {
        return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    // The exponent after rounding to `p` significant digits.
    let e_form = format!("{:.*e}", p - 1, v);
    let exp: i32 = e_form
        .rsplit_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        let (mant, _) = e_form.rsplit_once('e').unwrap_or((&e_form, "0"));
        let mant = trim(mant.to_string());
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mant}e{sign}{:02}", exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        trim(format!("{v:.decimals$}"))
    }
}

/// Number of errors reported on this thread since the last [`reset_errors`].
pub fn error_count() -> usize {
    ERRORS.with(Cell::get)
}

/// `agreseterrors`: clears the error count, which `dot` does after each graph.
pub fn reset_error_count() {
    ERRORS.with(|c| c.set(0));
}

/// Clears the error count and the once-only messages of this thread.
pub fn reset_errors() {
    ERRORS.with(|c| c.set(0));
    EMITTED.with(|e| e.borrow_mut().clear());
}

#[cfg(test)]
mod tests {

    // Issue #416: GV_INFO's time stamp, as UTC.
    #[test]
    fn gv_info_stamps_utc_calendar_time() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(civil(1_793_017_384), (2026, 10, 26, 12, 23, 4));
        assert_eq!(fmt_f(f64::NAN, 2), "nan");
        assert_eq!(fmt_f(-f64::INFINITY, 2), "-inf");
        assert_eq!(fmt_f(0.125, 2), "0.12");
    }
    use super::*;

    fn out(level: AgErrLevel, msg: &str) -> Option<String> {
        agerr_va(level, msg).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn control_characters_are_escaped_and_nul_ends_the_message() {
        assert_eq!(
            out(AgErrLevel::Warn, "x\x01y\x1b\x7f is not a known color.\n").as_deref(),
            Some("Warning: x\\001y\\033\\177 is not a known color.\n")
        );
        // White space is written as it is.
        assert_eq!(
            out(AgErrLevel::Warn, "a\tb\x0bc\x0cd\re\n").as_deref(),
            Some("Warning: a\tb\x0bc\x0cd\re\n")
        );
        assert_eq!(out(AgErrLevel::Prev, "ab\0cd\n").as_deref(), Some("ab"));
    }

    #[test]
    fn q_drops_warnings_and_q2_drops_errors_but_both_still_count() {
        reset_errors();
        let old = agseterr(AgErrLevel::Err);
        assert_eq!(out(AgErrLevel::Warn, "w\n"), None);
        assert_eq!(out(AgErrLevel::Err, "e\n").as_deref(), Some("Error: e\n"));
        assert_eq!(out(AgErrLevel::Max, "m\n").as_deref(), Some("Warning: m\n"));
        agseterr(AgErrLevel::Max);
        assert_eq!(out(AgErrLevel::Err, "e\n"), None);
        assert_eq!(error_count(), 3);
        agseterr(old);
    }

    #[test]
    fn capture_returns_each_call_s_messages_and_not_the_last_call_s() {
        let ((), first) = capture(|| {
            agwarningf("alpha\n");
            agerrorf("bad\n");
        });
        let ((), second) = capture(|| agwarningf("beta\n"));
        assert_eq!(first[0].level(), crate::error::DiagnosticLevel::Warning);
        assert_eq!(first[0].message(), "Warning: alpha\n");
        assert_eq!(first[1].level(), crate::error::DiagnosticLevel::Error);
        assert_eq!(first[1].message(), "Error: bad\n");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].message(), "Warning: beta\n");
        assert!(second.iter().all(|d| !d.message().contains("alpha")));
    }

    #[test]
    fn captures_on_two_threads_do_not_mix() {
        let left = std::thread::spawn(|| capture(|| agwarningf("from-a\n")).1);
        let right = std::thread::spawn(|| capture(|| agwarningf("from-b\n")).1);
        let left = left.join().unwrap();
        let right = right.join().unwrap();
        assert!(left.iter().all(|d| d.message().contains("from-a")));
        assert!(right.iter().all(|d| d.message().contains("from-b")));
        assert!(!left.iter().any(|d| d.message().contains("from-b")));
        assert!(!right.iter().any(|d| d.message().contains("from-a")));
    }

    #[test]
    fn prev_continues_at_the_level_of_the_message_before_it() {
        let old = agseterr(AgErrLevel::Err);
        assert_eq!(out(AgErrLevel::Warn, "w\n"), None);
        assert_eq!(out(AgErrLevel::Prev, "more\n"), None);
        assert_eq!(out(AgErrLevel::Err, "e\n").as_deref(), Some("Error: e\n"));
        assert_eq!(out(AgErrLevel::Prev, "more\n").as_deref(), Some("more\n"));
        agseterr(old);
    }

    #[test]
    fn a_panic_inside_capture_restores_the_frame() {
        let old = agseterr(AgErrLevel::Warn);
        reset_errors();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = capture(|| {
                agwarningf("during\n");
                assert!(emit_once("panic-key"));
                panic!("boom");
            });
        }));
        assert!(panicked.is_err());
        assert!(reporting());
        assert_eq!(error_count(), 0);
        assert!(emit_once("panic-key"));
        let ((), after) = capture(|| agwarningf("after\n"));
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].message(), "Warning: after\n");
        agseterr(old);
    }

    #[test]
    fn nested_capture_keeps_its_own_messages_and_once_set() {
        let ((inner_fresh, inner), outer) = capture(|| {
            assert!(emit_once("nest"));
            agwarningf("outer\n");
            let (inner_fresh, inner) = capture(|| {
                let fresh = emit_once("nest");
                agwarningf("inner\n");
                fresh
            });
            agwarningf("outer-after\n");
            (inner_fresh, inner)
        });
        assert!(inner_fresh);
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].message(), "Warning: inner\n");
        assert_eq!(outer.len(), 2);
        assert_eq!(outer[0].message(), "Warning: outer\n");
        assert_eq!(outer[1].message(), "Warning: outer-after\n");
        assert!(outer.iter().all(|d| !d.message().contains("inner")));
    }

    #[test]
    fn an_early_return_returns_the_messages_so_far_and_restores_the_frame() {
        let old = agseterr(AgErrLevel::Warn);
        reset_errors();
        let ((), messages) = capture(|| {
            agerrorf("kept\n");
            if error_count() > 0 {
                return;
            }
            agwarningf("not reached\n");
        });
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message(), "Error: kept\n");
        assert_eq!(error_count(), 0);
        assert!(reporting());
        let ((), next) = capture(|| agwarningf("next\n"));
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].message(), "Warning: next\n");
        agseterr(old);
    }

    #[test]
    fn capture_does_not_keep_counts_quiet_or_the_last_level() {
        let old = agseterr(AgErrLevel::Warn);
        reset_errors();
        let ((), messages) = capture(|| {
            agerrorf("inside\n");
            agseterr(AgErrLevel::Max);
            quietly(|| assert!(!reporting()));
            assert!(reporting());
            assert_eq!(error_count(), 1);
            assert_eq!(agseterr(AgErrLevel::Max), AgErrLevel::Max);
        });
        assert_eq!(messages.len(), 1);
        assert_eq!(error_count(), 0);
        assert!(reporting());
        // The caller's mode is restored. A continuation does not keep the
        // captured error's level.
        assert_eq!(agseterr(AgErrLevel::Warn), AgErrLevel::Warn);
        let ((), continued) = capture(|| agerr_prev("more\n"));
        assert_eq!(continued[0].level(), crate::error::DiagnosticLevel::Warning);
        assert!(emit_once("scoped-once"));
        assert!(!emit_once("scoped-once"));
        let ((), again) = capture(|| assert!(emit_once("scoped-once")));
        assert!(again.is_empty());
        agseterr(old);
    }
}
