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
        write_stderr(&bytes);
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

// The one place that writes to stderr (`clippy.toml`).
#[allow(clippy::disallowed_methods)]
fn write_stderr(bytes: &[u8]) {
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
    fn prev_continues_at_the_level_of_the_message_before_it() {
        let old = agseterr(AgErrLevel::Err);
        assert_eq!(out(AgErrLevel::Warn, "w\n"), None);
        assert_eq!(out(AgErrLevel::Prev, "more\n"), None);
        assert_eq!(out(AgErrLevel::Err, "e\n").as_deref(), Some("Error: e\n"));
        assert_eq!(out(AgErrLevel::Prev, "more\n").as_deref(), Some("more\n"));
        agseterr(old);
    }
}
