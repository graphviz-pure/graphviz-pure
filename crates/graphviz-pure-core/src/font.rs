//! Built-in font metrics lookup tables and text/label sizing ported from Graphviz 16.1.0
//! (`lib/common/textspan_lut.c`, `lib/common/textspan.c`, `lib/common/ps_font_equiv.h`,
//! and `lib/common/labels.c`).

use crate::charset::{self, Charset};
use serde::{Deserialize, Serialize};

/// Default font family name in Graphviz (`Times-Roman`).
pub const DEFAULT_FONT_NAME: &str = "Times-Roman";

/// Default font size in points (`14.0`).
pub const DEFAULT_FONT_SIZE: f64 = 14.0;

/// Default head/tail label font size in points (`11.0`).
pub const DEFAULT_LABEL_FONT_SIZE: f64 = 11.0;

/// Minimum allowed font size in points (`1.0`).
pub const MIN_FONT_SIZE: f64 = 1.0;

/// Line spacing multiplier (`1.20`) from `lib/common/const.h`.
pub const LINESPACING: f64 = 1.20;

/// Horizontal justification of a single line in a Graphviz label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextJustify {
    /// Centered line (`\n`, default).
    #[default]
    Center,
    /// Left-justified line (`\l`).
    Left,
    /// Right-justified line (`\r`).
    Right,
}

impl TextJustify {
    /// Returns the Graphviz justification character (`'n'`, `'l'`, `'r'`).
    #[inline]
    pub const fn as_char(self) -> char {
        match self {
            Self::Center => 'n',
            Self::Left => 'l',
            Self::Right => 'r',
        }
    }

    /// Parses a Graphviz justification character (`'n'`, `'l'`, `'r'`).
    #[inline]
    pub const fn from_char(c: char) -> Self {
        match c {
            'l' => Self::Left,
            'r' => Self::Right,
            _ => Self::Center,
        }
    }
}

/// Measured layout of a single line within a multiline text label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLineLayout {
    /// Unescaped text content of the line.
    pub text: String,
    /// Width of the line in points.
    pub width: f64,
    /// Height of the line in points (`font_size * LINESPACING`).
    pub height: f64,
    /// Horizontal justification of this line (`\n`, `\l`, `\r`).
    pub justify: TextJustify,
    /// The size of the line's interned font (`span->font->size`): the label's font size,
    /// except where C's font dictionary hands a NaN size another font (see
    /// [`textfont_intern`]). An empty line has no font and keeps the label's size.
    #[serde(default)]
    pub font_size: f64,
}

/// A `textfont_t` key in C's font dictionary (`gvc->textfont_dt`).
#[derive(Debug, Clone)]
struct TextFontKey {
    name: Option<String>,
    color: Option<String>,
    flags: u32,
    size: f64,
}

/// `textfont_comparf` (`lib/common/textspan.c`): names, colours, flags, then sizes with
/// `<` and `>`, so a NaN size compares equal to any size.
fn textfont_cmp(k1: &TextFontKey, k2: &TextFontKey) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Equal, Greater, Less};
    fn opt(a: &Option<String>, b: &Option<String>) -> std::cmp::Ordering {
        match (a, b) {
            (None, None) => Equal,
            (None, Some(_)) => Less,
            (Some(_), None) => Greater,
            (Some(a), Some(b)) => a.as_bytes().cmp(b.as_bytes()),
        }
    }
    let rc = opt(&k1.name, &k2.name);
    if rc != Equal {
        return rc;
    }
    let rc = opt(&k1.color, &k2.color);
    if rc != Equal {
        return rc;
    }
    if k1.flags != k2.flags {
        return k1.flags.cmp(&k2.flags);
    }
    if k1.size < k2.size {
        Less
    } else if k1.size > k2.size {
        Greater
    } else {
        Equal
    }
}

/// A font key with its size's bits, for [`TextFontDict::log`].
type LogKey = (Option<String>, Option<String>, u32, u64);

/// `gvc->textfont_dt`, and what the port needs to measure a label more than once.
#[derive(Default)]
struct TextFontDict {
    dt: crate::cdt::Cdt<TextFontKey>,
    /// The sizes `dtinsert` returned during the current layout, per key (with the size's
    /// bits), in order; with the next one to replay.
    log: std::collections::HashMap<LogKey, (Vec<f64>, usize)>,
    /// Inside [`textfont_remeasuring`].
    remeasuring: u32,
    /// The sizes each object's labels got when layout first measured them, by
    /// [`textfont_recording`]'s tag: what C keeps in each span's `font`.
    objects: std::collections::HashMap<String, Vec<f64>>,
    /// The open [`textfont_recording`] (tag, sizes) and [`textfont_replaying`] (sizes,
    /// next) scopes, innermost last.
    scopes: Vec<Scope>,
}

enum Scope {
    Record(String, Vec<f64>),
    Replay(Vec<f64>, usize),
}

thread_local! {
    /// One dictionary per thread, as C has one per GVC: it lives across the graphs of a run.
    static TEXTFONT_DT: std::cell::RefCell<TextFontDict> =
        std::cell::RefCell::new(TextFontDict::default());
}

/// `dtinsert(gvc->textfont_dt, &tf)`: interns a font and returns the size of the font in
/// the dictionary. That is `size`, except where a NaN is involved: C's comparison finds a
/// NaN size equal to the first font with the same name, colour and flags that the splay
/// tree's search reaches, and a NaN font interned first is found for later sizes too
/// (#274).
///
/// C measures each label once; the port measures some again (the renderer, and a label
/// that layout makes twice). Inside [`textfont_remeasuring`] the tree is left alone and
/// each key gets the sizes the current layout's lookups returned, in order.
pub fn textfont_intern(name: Option<&str>, color: Option<&str>, flags: u32, size: f64) -> f64 {
    let key = TextFontKey {
        name: name.map(str::to_string),
        color: color.map(str::to_string),
        flags,
        size,
    };
    let lkey = (key.name.clone(), key.color.clone(), flags, size.to_bits());
    TEXTFONT_DT.with(|d| {
        let mut d = d.borrow_mut();
        if let Some(Scope::Replay(sizes, next)) = d.scopes.last_mut() {
            let v = sizes.get(*next).copied().unwrap_or(size);
            *next += 1;
            return v;
        }
        if d.remeasuring > 0 {
            return match d.log.get_mut(&lkey) {
                Some((sizes, next)) if !sizes.is_empty() => {
                    let v = sizes[(*next).min(sizes.len() - 1)];
                    *next += 1;
                    v
                }
                _ => size,
            };
        }
        let cmp = |e: &TextFontKey| textfont_cmp(&key, e);
        let (i, _) = d.dt.insert(key.clone(), &cmp);
        let v = d.dt.get(i).size;
        d.log.entry(lkey).or_default().0.push(v);
        if let Some(Scope::Record(_, sizes)) = d.scopes.last_mut() {
            sizes.push(v);
        }
        v
    })
}

/// Runs `f`, a measurement of the labels of the object `tag` names (`node:a`, ...). The
/// first one in a layout interns the fonts, as C's single measurement does, and keeps the
/// sizes they get, as C keeps each span's `font`; a later one (the port measures some
/// objects again, and the renderer measures them all) gets the same sizes and leaves the
/// dictionary alone. The recorded sizes belong to the most recent layout on this thread
/// ([`textfont_layout_begin`]), so a graph is rendered after its own layout.
pub fn textfont_object<R>(tag: String, f: impl FnOnce() -> R) -> R {
    let recorded = TEXTFONT_DT.with(|d| d.borrow().objects.contains_key(&tag));
    if recorded {
        textfont_replaying(&tag, f)
    } else {
        textfont_recording(tag, f)
    }
}

/// While the returned guard lives, [`textfont_object`] tags name the objects of another
/// graph, such as dot's aux graph for flat adjacent edges, whose copies C measures (and
/// interns) again and whose ids restart at 0. The layout's own records come back when the
/// guard is dropped.
pub fn textfont_other_graph() -> impl Drop {
    type Saved = (
        std::collections::HashMap<String, Vec<f64>>,
        std::collections::HashMap<LogKey, (Vec<f64>, usize)>,
    );
    struct Guard(Option<Saved>);
    impl Drop for Guard {
        fn drop(&mut self) {
            let (objects, log) = self.0.take().unwrap_or_default();
            TEXTFONT_DT.with(|d| {
                let mut d = d.borrow_mut();
                d.objects = objects;
                d.log = log;
            });
        }
    }
    Guard(Some(TEXTFONT_DT.with(|d| {
        let mut d = d.borrow_mut();
        (std::mem::take(&mut d.objects), std::mem::take(&mut d.log))
    })))
}

/// Runs `f`, layout's first measurement of the labels of the object `tag` names: fonts
/// are interned, and the sizes they get are kept for [`textfont_replaying`].
fn textfont_recording<R>(tag: impl Into<String>, f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            TEXTFONT_DT.with(|d| {
                let mut d = d.borrow_mut();
                if let Some(Scope::Record(tag, sizes)) = d.scopes.pop() {
                    d.objects.insert(tag, sizes);
                }
            });
        }
    }
    let replaying = TEXTFONT_DT.with(|d| {
        let mut d = d.borrow_mut();
        let replaying = d.remeasuring > 0 || matches!(d.scopes.last(), Some(Scope::Replay(..)));
        if !replaying {
            d.scopes.push(Scope::Record(tag.into(), Vec::new()));
        }
        replaying
    });
    if replaying {
        return f();
    }
    let _guard = Guard;
    f()
}

/// Runs `f`, a measurement of the labels of the object `tag` names that layout has
/// already made: each lookup returns the size the recorded measurement got, in order, and
/// the dictionary does not change. An object never recorded is measured as
/// [`textfont_remeasuring`] does.
fn textfont_replaying<R>(tag: &str, f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            TEXTFONT_DT.with(|d| {
                d.borrow_mut().scopes.pop();
            });
        }
    }
    let sizes = TEXTFONT_DT.with(|d| d.borrow().objects.get(tag).cloned());
    let Some(sizes) = sizes else {
        return textfont_remeasuring(f);
    };
    TEXTFONT_DT.with(|d| d.borrow_mut().scopes.push(Scope::Replay(sizes, 0)));
    let _guard = Guard;
    f()
}

/// Runs `f`, a measurement of labels that the current layout has already measured: fonts
/// are looked up as they were then, and the dictionary does not change.
pub fn textfont_remeasuring<R>(f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            TEXTFONT_DT.with(|d| d.borrow_mut().remeasuring -= 1);
        }
    }
    TEXTFONT_DT.with(|d| {
        let mut d = d.borrow_mut();
        if d.remeasuring == 0 {
            for (_, next) in d.log.values_mut() {
                *next = 0;
            }
        }
        d.remeasuring += 1;
    });
    let _guard = Guard;
    f()
}

/// Starts a graph's layout: later [`textfont_remeasuring`] calls replay this layout's
/// lookups. The dictionary itself is kept, as C keeps it for the next graph.
pub fn textfont_layout_begin() {
    TEXTFONT_DT.with(|d| {
        let mut d = d.borrow_mut();
        d.log.clear();
        d.objects.clear();
    });
}

/// Empties the font dictionary, as a new GVC starts with an empty one.
pub fn textfont_dict_reset() {
    TEXTFONT_DT.with(|d| *d.borrow_mut() = TextFontDict::default());
}

/// Measured layout of a multiline Graphviz label in points.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextLabelLayout {
    /// Measured lines in top-to-bottom order.
    pub lines: Vec<TextLineLayout>,
    /// Individual line widths in points (mirrors `lines.iter().map(|l| l.width)`).
    pub line_widths: Vec<f64>,
    /// Total bounding width in points (maximum line width).
    pub width: f64,
    /// Total bounding height in points (sum of line heights).
    pub height: f64,
    /// Font family name used for measurement.
    pub font_name: String,
    /// Font size in points used for measurement.
    pub font_size: f64,
    /// The label text after object-escape substitution and entity conversion (C's
    /// `lp->text`), still containing the `\\n`/`\\l`/`\\r` line escapes.
    #[serde(default)]
    pub source: String,
}

/// Which names of a [`LabelSubstContext`] are agnameof's static buffer
/// ([`crate::graph::AgName`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NamesInBuffer {
    pub graph: bool,
    pub node: bool,
    pub tail: bool,
    pub head: bool,
}

/// Context for Graphviz object-based escape sequence substitution (`\N`, `\G`, `\T`, `\H`, `\E`, `\L`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelSubstContext<'a> {
    pub node_name: &'a str,
    pub graph_name: &'a str,
    pub tail_name: &'a str,
    pub head_name: &'a str,
    pub is_edge: bool,
    pub is_directed: bool,
    pub tail_port: Option<&'a str>,
    pub head_port: Option<&'a str>,
    pub label_text: &'a str,
    /// `getPenColor(obj)` (`pencolor`, else `color`), which an HTML label's outermost
    /// table without a `COLOR` takes (`make_html_label`).
    pub pen_color: Option<&'a str>,
    /// `lp->fontcolor`, the colour an HTML label's text takes without a `COLOR` of its own
    /// (`make_html_label`'s `env.finfo.color`); `None` is black.
    pub font_color: Option<&'a str>,
    /// The object is a node (its name may be empty).
    pub is_node: bool,
    /// The object has a main label (`ND_label`, `ED_label`, `GD_label`), whose text is
    /// `label_text` even when empty; without one `\L` stays literal.
    pub has_label: bool,
    /// Which of the names are agnameof's static buffer.
    pub in_buffer: NamesInBuffer,
}

impl<'a> LabelSubstContext<'a> {
    /// Creates a substitution context for a node with the given name.
    #[inline]
    pub const fn for_node(node_name: &'a str) -> Self {
        Self {
            node_name,
            graph_name: "",
            tail_name: "",
            head_name: "",
            is_edge: false,
            is_directed: true,
            tail_port: None,
            head_port: None,
            label_text: "",
            pen_color: None,
            font_color: None,
            is_node: true,
            has_label: false,
            in_buffer: NamesInBuffer {
                graph: false,
                node: false,
                tail: false,
                head: false,
            },
        }
    }
}

/// Font family character width lookup table (`struct FontFamilyMetrics` in
/// `lib/common/textspan_lut.c`).
pub(crate) struct FontFamilyMetrics {
    /// Case-insensitive list of metrically-equivalent family names.
    pub(crate) font_names: &'static [&'static str],
    /// Widths are stored in `(units_per_em * 1)` pt.
    pub(crate) units_per_em: f64,
    pub(crate) widths_regular: [i16; 128],
    pub(crate) widths_bold: [i16; 128],
    pub(crate) widths_italic: [i16; 128],
    pub(crate) widths_bold_italic: [i16; 128],
}

/// PostScript font alias entry (`PostscriptAlias` in `lib/common/textspan.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostscriptFontAlias {
    pub name: &'static str,
    pub family: &'static str,
    pub weight: Option<&'static str>,
    pub stretch: Option<&'static str>,
    pub style: Option<&'static str>,
    pub xfig_code: i32,
    pub svg_font_family: &'static str,
    pub svg_font_weight: Option<&'static str>,
    pub svg_font_style: Option<&'static str>,
}

#[rustfmt::skip]
#[path = "font_tables.rs"]
mod font_tables;

/// Standard PostScript font equivalence table (`postscript_alias[]` from `lib/common/textspan.c`,
/// generated from `ps_font_equiv.h` by `scripts/gen-font-tables.py`).
pub const POSTSCRIPT_FONT_ALIASES: &[PostscriptFontAlias] = &font_tables::POSTSCRIPT_ALIASES;

/// C `strcasecmp` ordering (ASCII case folding, byte comparison).
fn strcasecmp(a: &str, b: &str) -> std::cmp::Ordering {
    let la = a.bytes().map(|c| c.to_ascii_lowercase());
    let lb = b.bytes().map(|c| c.to_ascii_lowercase());
    la.cmp(lb)
}

/// `translate_postscript_fontname` from `lib/common/textspan.c`: `bsearch` of the alias table
/// with `strcasecmp`. The name is used verbatim (no trimming), as in C.
pub fn translate_postscript_fontname(font_name: &str) -> Option<&'static PostscriptFontAlias> {
    POSTSCRIPT_FONT_ALIASES
        .binary_search_by(|alias| strcasecmp(alias.name, font_name))
        .ok()
        .map(|i| &POSTSCRIPT_FONT_ALIASES[i])
}

/// `font_name_equal_permissive` from `lib/common/textspan_lut.c`: compares two font names
/// ignoring case and every character that is not an ASCII letter.
fn font_name_equal_permissive(a: &str, b: &str) -> bool {
    let a_iter = a
        .bytes()
        .filter(u8::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase());
    let b_iter = b
        .bytes()
        .filter(u8::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase());
    a_iter.eq(b_iter)
}

/// `font_in_list_permissive` from `lib/common/textspan_lut.c`.
fn font_in_list_permissive(value: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|entry| font_name_equal_permissive(value, entry))
}

/// `get_metrics_for_font_family` from `lib/common/textspan_lut.c`: the first family whose
/// name list matches, else the `Times` metrics (C emits a once-only warning).
fn get_metrics_for_font_family(font_name: &str) -> &'static FontFamilyMetrics {
    font_tables::FONT_METRICS
        .iter()
        .find(|m| font_in_list_permissive(font_name, m.font_names))
        .unwrap_or_else(|| {
            let warning = format!(
                "Warning: no hard-coded metrics for '{font_name}'.  Falling back to 'Times' metrics\n"
            );
            // A quiet repeat leaves the warning to the measurement C makes.
            if crate::diag::reporting() && crate::diag::emit_once(&warning) {
                crate::diag::agwarningf(&warning);
            }
            &font_tables::FONT_METRICS[0]
        })
}

/// `get_metrics_for_font_variant` from `lib/common/textspan_lut.c`.
fn get_metrics_for_font_variant(
    family_metrics: &'static FontFamilyMetrics,
    bold: bool,
    italic: bool,
) -> &'static [i16; 128] {
    if bold && italic {
        &family_metrics.widths_bold_italic
    } else if bold {
        &family_metrics.widths_bold
    } else if italic {
        &family_metrics.widths_italic
    } else {
        &family_metrics.widths_regular
    }
}

/// `estimate_character_width_canonical` from `lib/common/textspan_lut.c`: width of one byte in
/// `(units_per_em * 1)` points. Bytes `>= 128` (every byte of a UTF-8 multi-byte sequence) use
/// the width of `' '`; unknown ASCII widths (`-1`) count as `0`; each case warns once per process.
#[inline]
fn estimate_character_width_canonical(variant_metrics: &[i16; 128], byte: u8) -> u16 {
    use std::sync::atomic::{AtomicBool, Ordering};
    static NON_ASCII_WARNED: AtomicBool = AtomicBool::new(false);
    static NO_WIDTH_WARNED: AtomicBool = AtomicBool::new(false);
    let character = if byte >= 128 {
        if crate::diag::reporting() && !NON_ASCII_WARNED.swap(true, Ordering::Relaxed) {
            crate::diag::agwarningf(&format!(
                "Warning: no value for width of non-ASCII character {byte}. Falling back to \
                 width of space character\n"
            ));
        }
        b' '
    } else {
        byte
    };
    let width = variant_metrics[usize::from(character)];
    if width == -1 {
        if crate::diag::reporting() && !NO_WIDTH_WARNED.swap(true, Ordering::Relaxed) {
            crate::diag::agwarningf(&format!(
                "Warning: no value for width of ASCII character {character}. Falling back to 0\n"
            ));
        }
        0
    } else {
        width as u16
    }
}

/// `estimate_text_width_1pt` from `lib/common/textspan_lut.c`: width of `text` at 1pt.
///
/// Matches C exactly: the font name is looked up directly in the LUT families (PostScript
/// aliases such as `Times-Bold` are *not* consulted, so they measure as regular Times), and
/// the text is measured byte by byte, so each byte of a UTF-8 multi-byte character counts as
/// one space (see `docs/c-bugs-ledger.md`, GV-BUG-011/013).
pub fn estimate_text_width_1pt(font_name: &str, text: &str, bold: bool, italic: bool) -> f64 {
    let family_metrics = get_metrics_for_font_family(font_name);
    let variant_metrics = get_metrics_for_font_variant(family_metrics, bold, italic);
    let text_width_canonical: u32 = charset::encode(text)
        .iter()
        .map(|&b| u32::from(estimate_character_width_canonical(variant_metrics, b)))
        .sum();
    f64::from(text_width_canonical) / family_metrics.units_per_em
}

/// `htmlEntity` from `lib/common/utils.c`. `s` starts at the byte after `&`. On success returns
/// `(value, consumed)`. Like C, a numeric reference terminated by `;` is consumed even when
/// its value is `0` (so `&#0;` collapses to `&`); unknown names consume nothing.
fn html_entity(s: &[u8]) -> (u32, usize) {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    if at(0) == b'#' {
        let mut n: u32 = 0;
        let mut byte;
        let mut i;
        if at(1) == b'x' || at(1) == b'X' {
            i = 2;
            loop {
                byte = at(i);
                if i >= 8 {
                    break;
                }
                let d = match byte {
                    b'A'..=b'F' => byte - b'A' + 10,
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'0'..=b'9' => byte - b'0',
                    _ => break,
                };
                n = n.wrapping_mul(16).wrapping_add(u32::from(d));
                i += 1;
            }
        } else {
            i = 1;
            loop {
                byte = at(i);
                if i >= 8 {
                    break;
                }
                if byte.is_ascii_digit() {
                    n = n.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
                } else {
                    break;
                }
                i += 1;
            }
        }
        // C checks the last byte it examined; when the loop ran to completion `byte` is
        // the final digit, which can never be ';'.
        if i < 8 && byte == b';' {
            (n, i + 1)
        } else {
            (0, 0)
        }
    } else {
        for i in 0..font_tables::ENTITY_NAME_LENGTH_MAX {
            let byte = at(i);
            if byte == 0 {
                break;
            }
            if byte == b';' {
                let key = &s[..i];
                if let Ok(idx) = font_tables::HTML_ENTITIES
                    .binary_search_by(|(name, _)| name.as_bytes().cmp(key))
                {
                    return (font_tables::HTML_ENTITIES[idx].1, i + 1);
                }
                break;
            }
        }
        (0, 0)
    }
}

/// The value of the HTML entity `name` (without `&` and `;`) in `entities.h`, as
/// `scanEntity`'s `bsearch` finds it.
pub fn html_entity_value(name: &[u8]) -> Option<u32> {
    font_tables::HTML_ENTITIES
        .binary_search_by(|(n, _)| n.as_bytes().cmp(name))
        .ok()
        .map(|i| font_tables::HTML_ENTITIES[i].1)
}

/// Appends the C encoding of entity value `v` (`htmlEntityUTF8`/`latin1ToUTF8`: 1, 2 or 3
/// bytes, never 4).
fn push_entity_bytes(out: &mut Vec<u8>, v: u32) {
    if v < 0x7F {
        out.push(v as u8);
    } else if v < 0x07FF {
        out.push(((v >> 6) | 0xC0) as u8);
        out.push(((v & 0x3F) | 0x80) as u8);
    } else {
        out.push(((v >> 12) | 0xE0) as u8);
        out.push((((v >> 6) & 0x3F) | 0x80) as u8);
        out.push(((v & 0x3F) | 0x80) as u8);
    }
}

/// `htmlEntityUTF8` from `lib/common/utils.c`: replaces HTML entities (`&amp;`, `&#123;`,
/// `&#x7B;`) by their UTF-8 encoding. Works on the input's bytes (see [`crate::charset`]):
/// a lead byte without its continuation bytes, or a byte `>= 0xF8`, is converted as if it
/// were Latin-1, with C's once-per-graph warning.
pub fn html_entity_utf8(text: &str) -> String {
    charset::enter_html_entity_utf8();
    html_entity_utf8_probe(text)
}

/// [`html_entity_utf8`] as a lookup of the port's own, not a call of C's: it leaves
/// `htmlEntityUTF8`'s `lastg` alone.
pub fn html_entity_utf8_probe(text: &str) -> String {
    if !text.contains('&') && !charset::has_raw(text) {
        return text.to_string();
    }
    let s = charset::encode(text);
    let s: &[u8] = &s;
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let mut c = s[i];
        i += 1;
        let uc: i32 = if c < 0xC0 {
            0
        } else if c < 0xE0 {
            1
        } else if c < 0xF0 {
            2
        } else if c < 0xF8 {
            3
        } else {
            charset::warn_invalid_utf8(|g| {
                format!(
                    "UTF8 codes > 4 bytes are not currently supported (graph {g}) - \
                     treated as Latin-1. Perhaps \"-Gcharset=latin1\" is needed?\n"
                )
            });
            c = cvt_and_append(c, &mut out);
            -1
        };
        if uc == 0 && c == b'&' {
            let (v, used) = html_entity(&s[i..]);
            i += used;
            if v != 0 {
                if v < 0x7F {
                    c = v as u8;
                } else if v < 0x07FF {
                    out.push(((v >> 6) | 0xC0) as u8);
                    c = ((v & 0x3F) | 0x80) as u8;
                } else {
                    out.push(((v >> 12) | 0xE0) as u8);
                    out.push((((v >> 6) & 0x3F) | 0x80) as u8);
                    c = ((v & 0x3F) | 0x80) as u8;
                }
            }
        } else {
            for _ in 0..uc.max(0) {
                if at(i) & 0xC0 == 0x80 {
                    out.push(c);
                    c = at(i);
                    i += 1;
                } else {
                    charset::warn_invalid_utf8(|g| {
                        format!(
                            "Invalid {}-byte UTF8 found in input of graph {g} - \
                             treated as Latin-1. Perhaps \"-Gcharset=latin1\" is needed?\n",
                            uc + 1
                        )
                    });
                    c = cvt_and_append(c, &mut out);
                    break;
                }
            }
        }
        out.push(c);
    }
    charset::decode(out)
}

/// `cvtAndAppend`: appends all but the last byte of the Latin-1-to-UTF-8 conversion of `c`
/// and returns the last one.
fn cvt_and_append(c: u8, out: &mut Vec<u8>) -> u8 {
    let mut conv = Vec::with_capacity(3);
    push_entity_bytes(&mut conv, u32::from(c));
    let last = conv.pop().unwrap_or(c);
    out.extend_from_slice(&conv);
    last
}

/// `latin1ToUTF8` from `lib/common/utils.c` (used when `charset=latin1`): translates HTML
/// entities and re-encodes every byte `>= 0x7F` of the input (see [`crate::charset`]) as
/// if it were a Latin-1 code point.
pub fn latin1_to_utf8(text: &str) -> String {
    let s = charset::encode(text);
    let s: &[u8] = &s;
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let mut v = u32::from(s[i]);
        i += 1;
        if v == u32::from(b'&') {
            let (ev, used) = html_entity(&s[i..]);
            i += used;
            v = if ev == 0 { u32::from(b'&') } else { ev };
        }
        push_entity_bytes(&mut out, v);
    }
    charset::decode(out)
}

/// The label text conversion of `make_label` for a graph of charset `cs`
/// (`latin1ToUTF8` for Latin-1, `htmlEntityUTF8` otherwise).
pub fn convert_label_text(text: &str, cs: Charset) -> String {
    match cs {
        Charset::Latin1 => latin1_to_utf8(text),
        Charset::Utf8 | Charset::Big5 => html_entity_utf8(text),
    }
}

/// Decodes HTML character entity references, matching `htmlEntityUTF8` in
/// `lib/common/utils.c` (kept under its historical name for existing callers).
pub fn decode_html_entities(text: &str) -> String {
    html_entity_utf8(text)
}

/// Estimates the `(width, height)` in points of a single-line text span for a given
/// `font_name` and `font_size`, matching `estimate_textspan_size` in `lib/common/textspan.c`.
pub fn estimate_text_size(text: &str, font_name: &str, font_size: f64) -> (f64, f64) {
    estimate_text_size_styled(text, font_name, font_size, false, false)
}

/// `estimate_textspan_size` from `lib/common/textspan.c` with explicit `bold`/`italic` flags
/// (`HTML_BF`/`HTML_IF`): `(fontsize * width_1pt, fontsize * LINESPACING)`.
pub fn estimate_text_size_styled(
    text: &str,
    font_name: &str,
    font_size: f64,
    bold: bool,
    italic: bool,
) -> (f64, f64) {
    let width = font_size * estimate_text_width_1pt(font_name, text, bold, italic);
    let height = font_size * LINESPACING;
    // textspan_size: with no text layout plugin, estimate_textspan_size resolves every font
    if crate::diag::verbose_on() && crate::diag::reporting() && crate::diag::emit_once(font_name) {
        crate::diag::fprintf_stderr(&format!(
            "fontname: \"{font_name}\" resolved to: [internal hard-coded]\n"
        ));
    }
    (width, height)
}

/// Kind of graph object a label belongs to, deciding which `strdup_and_subst_obj0`
/// substitutions are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubstObjKind {
    Graph,
    Node,
    Edge,
}

impl LabelSubstContext<'_> {
    fn obj_kind(&self) -> SubstObjKind {
        if self.is_edge {
            SubstObjKind::Edge
        } else if self.is_node || !self.node_name.is_empty() {
            SubstObjKind::Node
        } else {
            SubstObjKind::Graph
        }
    }
}

/// `strdup_and_subst_obj0` from `lib/common/labels.c`.
///
/// Replaces `\G`, `\N`, `\E`, `\H`, `\T`, `\L` with the names of the associated objects.
/// Escapes that do not apply to the object kind stay literal (e.g. `\T` in a node label stays
/// `\T`, which `make_simple_label` later reduces to `T`), and `\L` stays literal when
/// `label_text` is empty (no label exists yet). When `esc_backslash` is set `\\` becomes `\`;
/// every other `\c` dyad is passed through unchanged.
fn strdup_and_subst_obj0(s: &str, ctx: &LabelSubstContext<'_>, esc_backslash: bool) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let kind = ctx.obj_kind();
    // The names are `agnameof`'s, taken before any is used: those in its static buffer all
    // show the last one taken (labels.c:297-318).
    let agname = |name, buffer| crate::graph::AgName {
        name: std::borrow::Cow::Borrowed(name),
        buffer,
    };
    let b = ctx.in_buffer;
    let (g_str, n_str, t_str, h_str) = match kind {
        SubstObjKind::Graph => (
            ctx.graph_name.into(),
            "\\N".into(),
            "\\T".into(),
            "\\H".into(),
        ),
        SubstObjKind::Node => {
            let [g, n] = crate::graph::agnameof_buffer([
                agname(ctx.graph_name, b.graph),
                agname(ctx.node_name, b.node),
            ]);
            (g, n, "\\T".into(), "\\H".into())
        }
        SubstObjKind::Edge => {
            let [g, t, h] = crate::graph::agnameof_buffer([
                agname(ctx.graph_name, b.graph),
                agname(ctx.tail_name, b.tail),
                agname(ctx.head_name, b.head),
            ]);
            (g, "\\N".into(), t, h)
        }
    };
    let (g_str, n_str, t_str, h_str) = (&*g_str, &*n_str, &*t_str, &*h_str);
    let l_str = if !ctx.has_label && ctx.label_text.is_empty() {
        "\\L"
    } else {
        ctx.label_text
    };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(next) = chars.clone().next() else {
            out.push('\\');
            continue;
        };
        chars.next();
        match next {
            'G' => out.push_str(g_str),
            'N' => out.push_str(n_str),
            'E' => {
                if kind == SubstObjKind::Edge {
                    out.push_str(t_str);
                    if let Some(tp) = ctx.tail_port.filter(|p| !p.is_empty()) {
                        out.push(':');
                        out.push_str(tp);
                    }
                    out.push_str(if ctx.is_directed { "->" } else { "--" });
                    out.push_str(h_str);
                    if let Some(hp) = ctx.head_port.filter(|p| !p.is_empty()) {
                        out.push(':');
                        out.push_str(hp);
                    }
                }
            }
            'T' => out.push_str(t_str),
            'H' => out.push_str(h_str),
            'L' => out.push_str(l_str),
            '\\' if esc_backslash => out.push('\\'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// Substitutes Graphviz object escapes (`\N`, `\G`, `\T`, `\H`, `\E`, `\L`) while preserving
/// line-formatting escapes (`\n`, `\l`, `\r`, `\\`) for `make_simple_label`; this is
/// `strdup_and_subst_obj0(str, obj, 0)` from `lib/common/labels.c`.
pub fn substitute_object_escapes(label: &str, ctx: &LabelSubstContext<'_>) -> String {
    strdup_and_subst_obj0(label, ctx, false)
}

/// `strdup_and_subst_obj` from `lib/common/labels.c`: object escapes plus `\\` -> `\`
/// (used for `URL`, `tooltip`, `id`, ... attributes).
pub fn strdup_and_subst_obj(s: &str, ctx: &LabelSubstContext<'_>) -> String {
    strdup_and_subst_obj0(s, ctx, true)
}

/// `storeline` from `lib/common/labels.c`: measures one line and accumulates it into the
/// label. Empty lines get height `(int)(fontsize * LINESPACING)` (C truncates; non-empty
/// lines do not), see GV-BUG-012 in `docs/c-bugs-ledger.md`.
fn storeline(layout: &mut TextLabelLayout, line: String, terminator: TextJustify) {
    let (width, height, font_size) = if line.is_empty() {
        let size = layout.font_size;
        (0.0, (size * LINESPACING).trunc(), size)
    } else {
        // span->font = dtinsert(gvc->textfont_dt, &tf); textspan_size measures that font
        let size = textfont_intern(Some(&layout.font_name), None, 0, layout.font_size);
        let (w, h) = estimate_text_size(&line, &layout.font_name, size);
        (w, h, size)
    };
    layout.width = crate::cnum::c_max(layout.width, width);
    layout.height += height;
    layout.line_widths.push(width);
    layout.lines.push(TextLineLayout {
        text: line,
        width,
        height,
        justify: terminator,
        font_size,
    });
}

/// `make_simple_label` from `lib/common/labels.c`: splits `text` (already object- and
/// entity-substituted) into lines on `\n`, `\l`, `\r` and literal newlines, reducing every
/// other `\c` to `c`. `width`/`height` are C's `lp->dimen`.
pub fn make_simple_label(text: &str, font_name: &str, font_size: f64) -> TextLabelLayout {
    make_simple_label_cs(text, font_name, font_size, Charset::Utf8)
}

/// [`make_simple_label`] for a graph of charset `cs`. C scans bytes; with `charset=big5` a
/// byte in `0xA1..=0xFE` takes the next byte with it, so that byte is never an escape.
pub fn make_simple_label_cs(
    text: &str,
    font_name: &str,
    font_size: f64,
    cs: Charset,
) -> TextLabelLayout {
    let mut layout = TextLabelLayout {
        font_name: font_name.to_string(),
        font_size,
        ..TextLabelLayout::default()
    };
    if text.is_empty() {
        return layout;
    }
    let bytes = charset::encode(text);
    let s: &[u8] = &bytes;
    let big5 = cs == Charset::Big5;
    let mut line: Vec<u8> = Vec::new();
    // C appends the NUL after a trailing backslash, which makes the buffer non-empty without
    // adding visible text.
    let mut pending_nul = false;
    let take = |line: &mut Vec<u8>| charset::decode(std::mem::take(line));
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if big5 && (0xA1..=0xFE).contains(&c) {
            line.push(c);
            match s.get(i) {
                Some(&n) => {
                    line.push(n);
                    i += 1;
                }
                // NB. C protects against an unexpected end here.
                None => break,
            }
        } else if c == b'\\' {
            match s.get(i) {
                Some(&j @ (b'n' | b'l' | b'r')) => {
                    i += 1;
                    storeline(
                        &mut layout,
                        take(&mut line),
                        TextJustify::from_char(char::from(j)),
                    );
                    pending_nul = false;
                }
                Some(&other) => {
                    i += 1;
                    if !pending_nul {
                        line.push(other);
                    }
                }
                None => pending_nul = true,
            }
        } else if c == b'\n' {
            storeline(&mut layout, take(&mut line), TextJustify::Center);
            pending_nul = false;
        } else if !pending_nul {
            line.push(c);
        }
    }
    if !line.is_empty() || pending_nul {
        storeline(&mut layout, take(&mut line), TextJustify::Center);
    }
    layout
}

/// Plain-text branch of `make_label` from `lib/common/labels.c`: object escape substitution,
/// charset conversion (`latin1ToUTF8` for `charset=latin1`, else `htmlEntityUTF8`) and
/// `make_simple_label`.
pub fn make_text_label(
    label: &str,
    ctx: &LabelSubstContext<'_>,
    font_name: &str,
    font_size: f64,
    cs: Charset,
) -> TextLabelLayout {
    // `\L` is the object's main label (`ND_label(n)->text`, ...): callers pass an empty
    // `label_text` while that label itself is being made, which leaves `\L` literal.
    let substituted = strdup_and_subst_obj0(label, ctx, false);
    let converted = convert_label_text(&substituted, cs);
    let mut layout = make_simple_label_cs(&converted, font_name, font_size, cs);
    layout.source = converted;
    layout
}

/// Returns `true` if a graph's `charset` attribute selects Latin-1 (`findCharset` in
/// `lib/common/input.c`: `latin-1`, `latin1`, `l1`, `ISO-8859-1`, `ISO_8859-1`, `ISO8859-1`,
/// `ISO-IR-100`, case-insensitively).
pub fn charset_is_latin1(charset: Option<&str>) -> bool {
    Charset::from_attr(charset) == Charset::Latin1
}

/// Measures a multiline Graphviz label for a node named `node_name`.
pub fn measure_multiline_label(
    label: &str,
    node_name: &str,
    font_name: &str,
    font_size: f64,
) -> TextLabelLayout {
    let ctx = LabelSubstContext::for_node(node_name);
    measure_multiline_label_with_context(label, &ctx, font_name, font_size)
}

/// Measures a multiline plain-text Graphviz label in the charset of the graph being laid
/// out ([`charset::current`]); see [`make_text_label`].
pub fn measure_multiline_label_with_context(
    label: &str,
    ctx: &LabelSubstContext<'_>,
    font_name: &str,
    font_size: f64,
) -> TextLabelLayout {
    make_text_label(label, ctx, font_name, font_size, charset::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_font_size_is_found_like_textfont_comparf() {
        textfont_dict_reset();
        textfont_layout_begin();
        let t = Some("Times-Roman");
        assert_eq!(textfont_intern(t, None, 0, 30.0), 30.0);
        assert_eq!(textfont_intern(t, None, 0, 20.0), 20.0);
        // the search meets the last font used first
        assert_eq!(textfont_intern(t, None, 0, f64::NAN), 20.0);
        assert!(textfont_intern(Some("Courier"), None, 0, f64::NAN).is_nan());
        // measuring again replays the layout's answers and leaves the tree alone
        textfont_remeasuring(|| {
            assert_eq!(textfont_intern(t, None, 0, 30.0), 30.0);
            assert_eq!(textfont_intern(t, None, 0, f64::NAN), 20.0);
            assert_eq!(textfont_intern(t, None, 0, 10.0), 10.0);
        });
        assert_eq!(textfont_intern(t, None, 0, f64::NAN), 20.0);
        textfont_dict_reset();
    }

    #[test]
    fn test_exact_character_widths_times_helvetica_courier() {
        // Times-Roman: 'A' is index 65 -> width 1479 / 2048
        let w_times_a = estimate_text_width_1pt("Times-Roman", "A", false, false);
        assert!((w_times_a - 1479.0 / 2048.0).abs() < 1e-12);

        // C looks the name up directly in the LUT families: "Times-Bold" and "Times-Italic"
        // are not family names, so they fall back to regular Times (verified against
        // `dot -Txdot` 16.1.0: `AAAA` in Times-Bold 14pt is 40.44pt wide, as in Times-Roman).
        let w_times_bold_a = estimate_text_width_1pt("Times-Bold", "A", false, false);
        assert!((w_times_bold_a - w_times_a).abs() < 1e-12);
        let w_times_italic_a = estimate_text_width_1pt("Times", "A", false, true);
        assert!((w_times_italic_a - 1251.0 / 2048.0).abs() < 1e-12);

        // Helvetica: 'A' is index 65 -> width 1366 / 2048
        let w_helv_a = estimate_text_width_1pt("Helvetica", "A", false, false);
        assert!((w_helv_a - 1366.0 / 2048.0).abs() < 1e-12);

        // "Helvetica-Bold" is not a family name either: Times fallback.
        let w_helv_bold_a = estimate_text_width_1pt("Helvetica-Bold", "A", false, false);
        assert!((w_helv_bold_a - w_times_a).abs() < 1e-12);
        let w_helv_bold_a = estimate_text_width_1pt("Helvetica", "A", true, false);
        assert!((w_helv_bold_a - 1479.0 / 2048.0).abs() < 1e-12);

        // Courier and Courier-Bold: monospace 1229 / 2048 per printable ASCII char
        let w_cour = estimate_text_width_1pt("Courier", "Hello", false, false);
        assert!((w_cour - (5.0 * 1229.0 / 2048.0)).abs() < 1e-12);
        let w_cour_bold = estimate_text_width_1pt("Courier New", "Hello", true, false);
        assert!((w_cour_bold - (5.0 * 1229.0 / 2048.0)).abs() < 1e-12);

        // Symbol falls back to Times metrics
        let w_symbol_a = estimate_text_width_1pt("Symbol", "A", false, false);
        assert!((w_symbol_a - w_times_a).abs() < 1e-12);
    }

    #[test]
    fn test_estimate_text_size_points() {
        let (w, h) = estimate_text_size("abc", "Courier", 14.0);
        let expected_w = 14.0 * (3.0 * 1229.0 / 2048.0);
        let expected_h = 14.0 * 1.20;
        assert!((w - expected_w).abs() < 1e-9);
        assert!((h - expected_h).abs() < 1e-9);
    }

    #[test]
    fn test_measure_multiline_label_escapes_and_justifications() {
        let layout =
            measure_multiline_label("\\N\\nline two\\lright\\r", "node_1", "Courier", 10.0);
        assert_eq!(layout.lines.len(), 3);
        assert_eq!(layout.line_widths.len(), 3);

        assert_eq!(layout.lines[0].text, "node_1");
        assert_eq!(layout.lines[0].justify, TextJustify::Center);

        assert_eq!(layout.lines[1].text, "line two");
        assert_eq!(layout.lines[1].justify, TextJustify::Left);

        assert_eq!(layout.lines[2].text, "right");
        assert_eq!(layout.lines[2].justify, TextJustify::Right);

        // Max width is "line two" (8 chars)
        let char_w = 10.0 * (1229.0 / 2048.0);
        assert!((layout.line_widths[0] - 6.0 * char_w).abs() < 1e-9);
        assert!((layout.line_widths[1] - 8.0 * char_w).abs() < 1e-9);
        assert!((layout.line_widths[2] - 5.0 * char_w).abs() < 1e-9);
        assert!((layout.width - 8.0 * char_w).abs() < 1e-9);
        assert!((layout.height - 3.0 * 10.0 * LINESPACING).abs() < 1e-9);
    }

    // labels.c:297-318 takes every agnameof before substituting: anonymous names share
    // its static buffer, so they all show the last one taken (issues #267, #325).
    #[test]
    fn substituted_anonymous_names_share_agnameofs_buffer() {
        let node = LabelSubstContext {
            graph_name: "%1",
            node_name: "%3",
            is_node: true,
            in_buffer: NamesInBuffer {
                graph: true,
                node: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(strdup_and_subst_obj("\\G \\N", &node), "%3 %3");
        let named = LabelSubstContext {
            node_name: "b",
            in_buffer: NamesInBuffer {
                graph: true,
                ..Default::default()
            },
            ..node.clone()
        };
        assert_eq!(strdup_and_subst_obj("\\G \\N", &named), "%1 b");
        let edge = LabelSubstContext {
            graph_name: "G",
            tail_name: "%3",
            head_name: "%7",
            is_edge: true,
            is_directed: true,
            in_buffer: NamesInBuffer {
                tail: true,
                head: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            strdup_and_subst_obj("\\E \\T \\H \\G", &edge),
            "%7->%7 %7 %7 G"
        );
        // An edge of dot's auxg, whose `%` names are strings of its own.
        let kept = LabelSubstContext {
            in_buffer: NamesInBuffer::default(),
            ..edge
        };
        assert_eq!(strdup_and_subst_obj("\\E", &kept), "%3->%7");
    }

    #[test]
    fn test_measure_multiline_label_context_escapes() {
        let ctx = LabelSubstContext {
            node_name: "n1",
            graph_name: "G_main",
            tail_name: "src",
            head_name: "dst",
            is_edge: true,
            is_directed: true,
            tail_port: None,
            head_port: None,
            label_text: "",
            pen_color: None,
            font_color: None,
            is_node: false,
            has_label: false,
            in_buffer: NamesInBuffer::default(),
        };
        let layout = measure_multiline_label_with_context(
            "\\G: \\T -> \\H (\\E)",
            &ctx,
            "Times-Roman",
            14.0,
        );
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.lines[0].text, "G_main: src -> dst (src->dst)");
    }

    #[test]
    fn utf8_is_measured_per_byte_like_c() {
        // `dot -Txdot`: "é€x" in Times-Roman 14pt is 24.5pt (5 non-ASCII bytes as spaces + x).
        let (w, _) = estimate_text_size("é€x", "Times-Roman", 14.0);
        assert!((w - 24.5).abs() < 1e-9);
    }

    #[test]
    fn empty_line_height_is_truncated_like_c() {
        let l = make_simple_label("a\\n\\nb", "Times-Roman", 14.0);
        assert_eq!(l.lines.len(), 3);
        assert!((l.lines[1].height - 16.0).abs() < 1e-12);
        assert!((l.height - (16.8 + 16.0 + 16.8)).abs() < 1e-9);
    }

    #[test]
    fn html_entities_follow_c_rules() {
        assert_eq!(html_entity_utf8("a&amp;b"), "a&b");
        assert_eq!(html_entity_utf8("&#65;&#x42;"), "AB");
        assert_eq!(html_entity_utf8("&bogus;"), "&bogus;");
        assert_eq!(html_entity_utf8("&#0;x"), "&x");
        assert_eq!(html_entity_utf8("&alpha;"), "\u{3b1}");
    }

    #[test]
    fn charset_conversions_run_on_input_bytes() {
        let raw = |b: &[u8]| charset::decode(b.to_vec());
        // Latin-1 bytes are converted once; UTF-8 bytes under charset=latin1 twice, as in C.
        assert_eq!(
            latin1_to_utf8(&raw(b"caf\xe9 &eacute;")),
            "caf\u{e9} \u{e9}"
        );
        assert_eq!(latin1_to_utf8("\u{e9}"), "\u{c3}\u{a9}");
        // htmlEntityUTF8 treats a lead byte without continuation bytes as Latin-1.
        assert_eq!(html_entity_utf8(&raw(b"caf\xe9 x")), "caf\u{e9} x");
        assert_eq!(html_entity_utf8(&raw(b"\xff")), "\u{ff}");
        // A 3-byte lead with one continuation byte keeps both, then converts the next one:
        // the result is not valid UTF-8, like C's.
        let out = html_entity_utf8(&raw(b"\xe4\xbcz"));
        assert_eq!(&*charset::encode(&out), b"\xe4\xc2\xbcz");
        // Width is measured per input byte: one raw byte is one space wide.
        let w = |t: &str| estimate_text_width_1pt("Times-Roman", t, false, false);
        assert_eq!(w(&raw(b"\xa4")), w(" "));
    }

    #[test]
    fn big5_pairs_bytes_in_make_simple_label() {
        // E4 B8 AD: E4 takes B8, AD takes the backslash, so "\n" is not a line break.
        let l = make_simple_label_cs("\u{4e2d}\\nX\\lY", "Times-Roman", 14.0, Charset::Big5);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].text, "\u{4e2d}\\nX");
        let u = make_simple_label_cs("\u{4e2d}\\nX\\lY", "Times-Roman", 14.0, Charset::Utf8);
        assert_eq!(u.lines.len(), 3);
    }

    #[test]
    fn subst_leaves_inapplicable_escapes() {
        let ctx = LabelSubstContext::for_node("n");
        let l = make_text_label("\\N \\T \\L", &ctx, "Times-Roman", 14.0, Charset::Utf8);
        assert_eq!(l.lines[0].text, "n T L");
    }
}
