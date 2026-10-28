//! Byte-faithful strings and the graph `charset` (`lib/common/input.c`, `utils.c`).
//!
//! C works on the bytes of its input: attribute values, names and the text of
//! `-Tdot`/`-Tsvg`/... output are whatever bytes the file held, and only label text
//! is converted to UTF-8 (`latin1ToUTF8` for `charset=latin1`, `htmlEntityUTF8`
//! otherwise, whose invalid-UTF-8 recovery treats the offending byte as Latin-1).
//!
//! The port keeps strings as `String`. Input is decoded with [`decode`]: valid UTF-8
//! sequences stay as they are and every other byte `b` becomes the private-use code
//! point `U+10FF00 + b` (a "raw byte"). [`encode`] maps them back, so the CLI writes
//! C's exact bytes, and the C conversions run on [`encode`]d bytes, so `charset=latin1`
//! text is converted exactly once, where C converts it. Text measurement (which C does
//! byte by byte) also uses the encoded bytes.
//!
//! A real `U+10FF00..=U+10FFFF` character in valid UTF-8 input is decoded as its four
//! raw bytes, so in a decoded string that range only ever stands for raw bytes and
//! [`encode`] gives back the input's bytes. C sees those four bytes too: it measures
//! each one and writes them unchanged. Every string the port reads is in this decoded
//! form; text from elsewhere (a JS string, or a `&str` given to `graphviz_pure`'s root
//! functions) goes through [`from_text`] first, and [`to_text`] or [`encode`] turns a
//! result back into text or bytes.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};

/// First code point of the raw-byte range (`U+10FF00 + byte`).
const RAW_BASE: u32 = 0x10_FF00;

/// Decodes input bytes: valid UTF-8 is kept, every other byte becomes a raw-byte char,
/// and so does each byte of a real char of the raw-byte range.
pub fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) if !has_raw(&s) => s,
        Ok(s) => escaped(&s, &[]),
        Err(e) => {
            let bytes = e.as_bytes();
            let valid = e.utf8_error().valid_up_to();
            escaped(
                std::str::from_utf8(&bytes[..valid]).unwrap_or_default(),
                &bytes[valid..],
            )
        }
    }
}

/// [`decode`] for a borrowed slice.
pub fn decode_slice(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) if !has_raw(s) => s.to_string(),
        Ok(s) => escaped(s, &[]),
        Err(e) => {
            let valid = e.valid_up_to();
            escaped(
                std::str::from_utf8(&bytes[..valid]).unwrap_or_default(),
                &bytes[valid..],
            )
        }
    }
}

/// The decoded form of `valid` (already-checked UTF-8) followed by `rest`, which starts with
/// a byte that isn't part of valid UTF-8 (or is empty).
fn escaped(valid: &str, rest: &[u8]) -> String {
    let mut out = String::with_capacity(valid.len() + rest.len() + 8);
    push_escaped(&mut out, valid);
    for chunk in rest.utf8_chunks() {
        push_escaped(&mut out, chunk.valid());
        out.extend(chunk.invalid().iter().map(|&b| raw_char(b)));
    }
    out
}

/// Appends valid text, each char of the raw-byte range as its four raw bytes.
fn push_escaped(out: &mut String, s: &str) {
    if !has_raw(s) {
        out.push_str(s);
        return;
    }
    let mut buf = [0u8; 4];
    for c in s.chars() {
        if u32::from(c) >= RAW_BASE {
            out.extend(c.encode_utf8(&mut buf).bytes().map(raw_char));
        } else {
            out.push(c);
        }
    }
}

/// The char that stands for the raw (non-UTF-8) byte `b`; [`encode`] turns it back.
pub fn raw_char(b: u8) -> char {
    char::from_u32(RAW_BASE + u32::from(b)).expect("raw byte code point")
}

/// Whether `s` holds raw-byte chars (non-UTF-8 input bytes).
pub fn has_raw(s: &str) -> bool {
    // U+10FFxx is encoded as F4 8F Bx xx.
    s.as_bytes().contains(&0xF4) && s.chars().any(|c| u32::from(c) >= RAW_BASE)
}

/// The bytes C would hold for `s`: raw-byte chars become their byte again.
pub fn encode(s: &str) -> Cow<'_, [u8]> {
    if !has_raw(s) {
        return Cow::Borrowed(s.as_bytes());
    }
    let mut out = Vec::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for c in s.chars() {
        let v = u32::from(c);
        if v >= RAW_BASE {
            out.push((v - RAW_BASE) as u8);
        } else {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    Cow::Owned(out)
}

/// Text from outside the port (a JS string, say) in the decoded form the port reads:
/// [`decode_slice`] of its UTF-8 bytes, so a real char of the raw-byte range is not taken
/// for a raw byte.
pub fn from_text(text: &str) -> Cow<'_, str> {
    if has_raw(text) {
        Cow::Owned(decode_slice(text.as_bytes()))
    } else {
        Cow::Borrowed(text)
    }
}

/// A decoded string as text for outside the port: its [`encode`]d bytes, with any that are
/// not UTF-8 replaced by U+FFFD. Undoes [`from_text`].
pub fn to_text(s: &str) -> Cow<'_, str> {
    if has_raw(s) {
        Cow::Owned(String::from_utf8_lossy(&encode(s)).into_owned())
    } else {
        Cow::Borrowed(s)
    }
}

/// A string in the decoded form, for callers that take file bytes rather than text.
/// It is made by decoding bytes or converting text, so text and decoded strings can't be
/// passed for each other by mistake.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decoded(String);

impl Decoded {
    /// [`decode`] of input bytes.
    pub fn decode(bytes: Vec<u8>) -> Self {
        Decoded(decode(bytes))
    }

    /// [`decode_slice`] of input bytes.
    pub fn decode_slice(bytes: &[u8]) -> Self {
        Decoded(decode_slice(bytes))
    }

    /// [`from_text`] of ordinary text.
    pub fn from_text(text: &str) -> Self {
        Decoded(from_text(text).into_owned())
    }

    /// A string that is already in decoded form: a [`decode`] result, or a value a parsed
    /// graph holds.
    pub fn from_decoded(s: String) -> Self {
        Decoded(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// `GD_charset(g)`: the input character set of a graph (`CHAR_UTF8`, `CHAR_LATIN1`,
/// `CHAR_BIG5` in `lib/common/const.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Charset {
    /// `CHAR_UTF8` (the default, and the fallback for unknown names).
    #[default]
    Utf8,
    /// `CHAR_LATIN1`.
    Latin1,
    /// `CHAR_BIG5`: labels are converted like UTF-8, but `make_simple_label` keeps the
    /// byte after a Big5 lead byte (`0xA1..=0xFE`) with it, and HTML labels fail
    /// (expat has no `BIG-5` decoder).
    Big5,
}

impl Charset {
    /// `findCharset` from `lib/common/input.c`. The second value is `false` when the name
    /// is not recognised (C warns and assumes UTF-8).
    pub fn parse(charset: Option<&str>) -> (Self, bool) {
        let p = charset.unwrap_or("utf-8");
        let is = |names: &[&str]| names.iter().any(|n| n.eq_ignore_ascii_case(p));
        if is(&[
            "latin-1",
            "latin1",
            "l1",
            "ISO-8859-1",
            "ISO_8859-1",
            "ISO8859-1",
            "ISO-IR-100",
        ]) {
            (Self::Latin1, true)
        } else if is(&["big-5", "big5"]) {
            (Self::Big5, true)
        } else if is(&["utf-8", "utf8"]) {
            (Self::Utf8, true)
        } else {
            (Self::Utf8, false)
        }
    }

    /// [`Self::parse`] without the validity flag.
    pub fn from_attr(charset: Option<&str>) -> Self {
        Self::parse(charset).0
    }
}

thread_local! {
    static CURRENT: Cell<Charset> = const { Cell::new(Charset::Utf8) };
    /// The graph labels are converted for: an id standing for its `Agraph_t*`, and its name.
    static GRAPH: RefCell<(u64, String)> = const { RefCell::new((0, String::new())) };
    /// `htmlEntityUTF8`'s `lastg`: the id of the graph of its last call.
    static LAST: Cell<u64> = const { Cell::new(0) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

/// Makes `graph_name`, with a new id, the graph labels are converted for; returns the
/// previous one.
fn set_graph(graph_name: &str) -> (u64, String) {
    let id = NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    GRAPH.with(|g| g.replace((id, graph_name.to_string())))
}

/// `graph_init` for charset purposes: records the root graph's charset and name for the
/// label conversions that follow, clears `htmlEntityUTF8`'s once-per-graph warning flag
/// and warns about an unsupported charset name.
pub fn begin_layout(graph_name: &str, charset: Option<&str>) {
    let (cs, ok) = Charset::parse(charset);
    if !ok {
        crate::diag::agwarningf(&format!(
            "Unsupported charset \"{}\" - assuming utf-8\n",
            charset.unwrap_or("")
        ));
    }
    CURRENT.with(|c| c.set(cs));
    WARNED.with(|w| w.set(false));
    set_graph(graph_name);
    LAST.with(|l| l.set(GRAPH.with(|g| g.borrow().0)));
}

/// Sets the charset of the graph being rendered (no warnings; the layout already gave them).
pub fn begin_render(graph_name: &str, charset: Option<&str>) {
    let cs = Charset::from_attr(charset);
    CURRENT.with(|c| c.set(cs));
    if GRAPH.with(|g| g.borrow().1 != graph_name) {
        set_graph(graph_name);
    }
}

/// Charset of the graph being laid out or rendered on this thread.
pub fn current() -> Charset {
    CURRENT.with(Cell::get)
}

/// Converts labels for another graph, `graph_name` (`htmlEntityUTF8(s, g)` with another `g`),
/// until the returned guard puts the previous graph back.
pub fn enter_graph(graph_name: &str) -> impl Drop + use<> {
    struct Guard((u64, String));
    impl Drop for Guard {
        fn drop(&mut self) {
            GRAPH.with(|g| std::mem::swap(&mut *g.borrow_mut(), &mut self.0));
        }
    }
    Guard(set_graph(graph_name))
}

/// The start of `htmlEntityUTF8`: a call for another graph than the last one (`lastg`)
/// clears the once-per-graph warning flag. A repeat inside [`crate::diag::quietly`] is not a
/// C call and leaves `lastg` alone.
pub(crate) fn enter_html_entity_utf8() {
    if !crate::diag::reporting() {
        return;
    }
    let g = GRAPH.with(|g| g.borrow().0);
    if LAST.with(|l| l.replace(g)) != g {
        WARNED.with(|w| w.set(false));
    }
}

/// `htmlEntityUTF8`'s warning: printed at most once per graph. A repeat inside
/// [`crate::diag::quietly`] neither prints it nor uses up the graph's warning.
pub(crate) fn warn_invalid_utf8(msg: impl FnOnce(&str) -> String) {
    if !crate::diag::reporting() || WARNED.with(|w| w.replace(true)) {
        return;
    }
    let text = GRAPH.with(|g| msg(&g.borrow().1));
    crate::diag::agwarningf(&text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let bytes = b"caf\xe9 \xc3\xa9 \xe4\xbcz \xff".to_vec();
        let s = decode(bytes.clone());
        assert!(s.contains('\u{e9}'));
        assert!(has_raw(&s));
        assert_eq!(&*encode(&s), &bytes[..]);
        assert_eq!(decode(b"plain".to_vec()), "plain");
        assert!(matches!(encode("plain \u{10ffff}x"), Cow::Owned(_)));
    }

    // Issue #151: a real char of the raw-byte range is decoded as its four raw bytes, so
    // `encode` gives them back and measurement sees C's bytes, not one ASCII byte.
    #[test]
    fn real_raw_range_char_keeps_its_bytes() {
        for bytes in [
            &b"x\xf4\x8f\xbd\x81y"[..],
            b"\xf4\x8f\xbc\x80\xf4\x8f\xbf\xbf",
            b"caf\xe9 \xf4\x8f\xbd\x81",
        ] {
            let s = decode(bytes.to_vec());
            assert_eq!(&*encode(&s), bytes);
            assert_eq!(decode_slice(bytes), s);
        }
        let s = decode(b"x\xf4\x8f\xbd\x81y".to_vec());
        let raw: String = [0xf4, 0x8f, 0xbd, 0x81].map(raw_char).iter().collect();
        assert_eq!(s, format!("x{raw}y"));
        // The char just below the range is ordinary text.
        assert_eq!(decode("\u{10feff}".into()), "\u{10feff}");
        let text = "x\u{10ff41}y";
        assert_eq!(from_text(text), s);
        assert_eq!(to_text(&from_text(text)), text);
        assert_eq!(to_text(&decode(b"caf\xe9".to_vec())), "caf\u{fffd}");
    }

    #[test]
    fn find_charset() {
        assert_eq!(Charset::parse(Some("ISO-8859-1")), (Charset::Latin1, true));
        assert_eq!(Charset::parse(Some("BIG5")), (Charset::Big5, true));
        assert_eq!(Charset::parse(None), (Charset::Utf8, true));
        assert_eq!(Charset::parse(Some("koi8-r")), (Charset::Utf8, false));
    }
}
