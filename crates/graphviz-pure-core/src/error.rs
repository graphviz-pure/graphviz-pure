use std::fmt;

/// An error from parsing, laying out, or rendering a graph.
///
/// The struct is opaque: it has no public fields, so a later release can attach
/// diagnostics without a breaking change. Read it with [`Self::kind`],
/// [`Self::message`], [`Self::line`], [`Self::column`], [`Self::file`] and
/// [`Self::engine`]. [`GraphvizErrorKind`] is non-exhaustive, so a new kind is a
/// minor change. Match it with a wildcard.
///
/// [`Self::message`] is the detail. [`std::fmt::Display`] adds a category prefix
/// for every kind except a syntax error, and the command line prints that
/// `Display` text unchanged.
///
/// It does not wrap another error: [`std::error::Error::source`] returns `None`.
///
/// [`Self::diagnostics`] holds the `agerr` messages emitted before and around
/// the failure. It does not include the failure's own text: that is
/// [`std::fmt::Display`]. A syntax error carries none.
///
/// [`PartialEq`] compares the kind and the text (the detail, the line, the
/// column, the file, the engine and the attribute name). It does not compare
/// [`Self::diagnostics`].
#[derive(Debug, Clone)]
pub struct GraphvizError {
    kind: GraphvizErrorKind,
    /// The detail. [`std::fmt::Display`] adds the category prefix.
    detail: String,
    /// Layout engine name, only for [`GraphvizErrorKind::Layout`].
    engine: Option<&'static str>,
    /// Attribute name, only for [`GraphvizErrorKind::Attribute`].
    attr: Option<String>,
    /// cgraph's `line_num` for a syntax error. An `int`, so it can wrap negative.
    line: Option<i32>,
    /// Scanner column for a syntax error, when it fits in a `u32`.
    column: Option<u32>,
    /// The file a `# N "file"` line directive named, for a syntax error.
    file: Option<String>,
    /// `agerr` messages collected for the failed call. Absent until a call attaches
    /// some, so the error stays small enough to return.
    diagnostics: Option<Box<Diagnostics>>,
}

/// The heap list behind [`GraphvizError::diagnostics`]. A newtype, so the error
/// holds a thin pointer rather than the three-word `Vec`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Diagnostics(Vec<Diagnostic>);

/// Why a [`GraphvizError`] was returned.
///
/// Non-exhaustive, so a new kind can be added in a minor release.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphvizErrorKind {
    /// DOT the parser rejected.
    Syntax,
    /// The engine name is not one this port lays out.
    UnsupportedEngine,
    /// The output format name is not one this port writes.
    UnsupportedFormat,
    /// A layout engine failed.
    Layout,
    /// An attribute value was rejected.
    Attribute,
    /// A renderer failed, including raster encoding.
    Render,
}

/// How serious a [`Diagnostic`] is.
///
/// Non-exhaustive, so a new level can be added in a minor release. A `match`
/// outside this crate needs a wildcard arm.
///
/// `AGPREV` is not a level of its own: a continuation takes the level of the
/// message it extends. `AGMAX` is counted as an error and written as a warning.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticLevel {
    /// `AGERR`. The message text starts with `Error: `.
    Error,
    /// `AGWARN`. The message text starts with `Warning: `.
    Warning,
    /// `AGMAX`. Counted as an error. The message text starts with `Warning: `.
    CountedAsError,
}

/// One `agerr` / `agwarningf` message, in the order C wrote it.
///
/// [`Self::message`] is the exact text of that write, including the `Error: `
/// or `Warning: ` prefix (none for a continuation) and the trailing newline,
/// after C's escaping of control characters. It is not a whole stderr log:
/// `-v` progress is not a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    level: DiagnosticLevel,
    message: String,
}

impl Diagnostic {
    pub(crate) fn new(level: DiagnosticLevel, message: String) -> Self {
        Self { level, message }
    }

    /// `AGERR`, `AGWARN` or `AGMAX`. A continuation uses the level it extends.
    #[must_use]
    pub fn level(&self) -> DiagnosticLevel {
        self.level
    }

    /// The exact text of this one `agerr` write.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Not part of the stable API. The facade rewrites decoded text at the boundary.
    #[doc(hidden)]
    pub fn __map_message(mut self, mut f: impl FnMut(String) -> String) -> Self {
        self.message = f(self.message);
        self
    }
}

/// A successful call's value plus the diagnostics it collected.
///
/// Text from the facade is `Rendered<String>`, or `Rendered<Vec<String>>` when
/// one call renders every graph. A parsed graph and a layout use the graph type
/// and `()`. Raster bytes are `Rendered<Vec<u8>>`.
///
/// The diagnostics are the `agerr` messages of that call, in order. Dropping
/// the value drops them. [`Self::into_output`] keeps only the value, and
/// [`Self::into_parts`] keeps both.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered<T> {
    output: T,
    diagnostics: Vec<Diagnostic>,
}

impl<T> Rendered<T> {
    /// Not part of the stable API. The facade and the raster crate build the value.
    #[doc(hidden)]
    pub fn __from_parts(output: T, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            output,
            diagnostics,
        }
    }

    /// The call's value.
    #[must_use]
    pub fn output(&self) -> &T {
        &self.output
    }

    /// The call's value, mutably. Laying a parsed graph out borrows it this way.
    #[must_use]
    pub fn output_mut(&mut self) -> &mut T {
        &mut self.output
    }

    /// The value alone. The diagnostics are dropped.
    #[must_use]
    pub fn into_output(self) -> T {
        self.output
    }

    /// The value and the diagnostics, in that order.
    #[must_use]
    pub fn into_parts(self) -> (T, Vec<Diagnostic>) {
        (self.output, self.diagnostics)
    }

    /// `agerr` messages from the call, in order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl GraphvizError {
    fn plain(kind: GraphvizErrorKind, detail: String) -> Self {
        Self {
            kind,
            detail,
            engine: None,
            attr: None,
            line: None,
            column: None,
            file: None,
            diagnostics: None,
        }
    }

    /// A DOT syntax error. `message` is cgraph's wording (`syntax error in line N
    /// near ...`). `line` is cgraph's `line_num` (an `int`, so it can wrap) and
    /// `column` is the scanner column. `file` is the name a `# N "file"` line
    /// directive set, when there was one.
    pub fn syntax(
        line: i32,
        column: usize,
        message: impl Into<String>,
        file: Option<String>,
    ) -> Self {
        Self {
            kind: GraphvizErrorKind::Syntax,
            detail: message.into(),
            engine: None,
            attr: None,
            line: Some(line),
            column: u32::try_from(column).ok(),
            file,
            diagnostics: None,
        }
    }

    /// `Unsupported layout engine: '{name}'`. [`Self::message`] is `name`.
    pub fn unsupported_engine(name: impl Into<String>) -> Self {
        Self::plain(GraphvizErrorKind::UnsupportedEngine, name.into())
    }

    /// `Unsupported output format: '{name}'`. [`Self::message`] is `name`.
    pub fn unsupported_format(name: impl Into<String>) -> Self {
        Self::plain(GraphvizErrorKind::UnsupportedFormat, name.into())
    }

    /// `Layout computation error in {stage}: {message}`. [`Self::message`] is
    /// `message`, and [`Self::engine`] is `stage`.
    pub fn layout(stage: &'static str, message: impl Into<String>) -> Self {
        let mut err = Self::plain(GraphvizErrorKind::Layout, message.into());
        err.engine = Some(stage);
        err
    }

    /// `Invalid attribute '{attr}': {message}`. [`Self::message`] is `message`.
    pub fn attribute(attr: impl Into<String>, message: impl Into<String>) -> Self {
        let mut err = Self::plain(GraphvizErrorKind::Attribute, message.into());
        err.attr = Some(attr.into());
        err
    }

    /// `Rendering error: {message}`. [`Self::message`] is `message`.
    pub fn render(message: impl Into<String>) -> Self {
        Self::plain(GraphvizErrorKind::Render, message.into())
    }

    /// The kind of failure.
    #[must_use]
    pub fn kind(&self) -> GraphvizErrorKind {
        self.kind
    }

    /// The detail, without the category prefix [`std::fmt::Display`] adds.
    ///
    /// For a syntax error this is the whole message. For a layout error it is the
    /// text after `Layout computation error in {engine}: `.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.detail
    }

    /// The source line of a syntax error, when cgraph's `line_num` fits in a
    /// `u32`. A wrapped negative line stays in [`Self::message`] and
    /// [`std::fmt::Display`], and this returns `None`. Any other kind returns
    /// `None`.
    #[must_use]
    pub fn line(&self) -> Option<u32> {
        self.line.and_then(|n| u32::try_from(n).ok())
    }

    /// The source column of a syntax error. Any other kind returns `None`.
    #[must_use]
    pub fn column(&self) -> Option<u32> {
        self.column
    }

    /// The file a `# N "file"` line directive named, for a syntax error.
    ///
    /// `None` when the error is not a syntax error, or when no directive named
    /// a file. The command line prints this name in place of the input's name.
    #[must_use]
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// The layout engine that failed (`"dot"`, `"neato"`, ...), for a
    /// [`GraphvizErrorKind::Layout`] error. Any other kind returns `None`.
    #[must_use]
    pub fn engine(&self) -> Option<&str> {
        self.engine
    }

    /// `agerr` messages emitted before and around this failure, in order.
    ///
    /// Not this failure's own text: that is [`std::fmt::Display`], and
    /// [`Self::message`] is the detail without the category prefix. Empty when
    /// nothing was emitted around the failure. A syntax error is reported only
    /// through this error, so its list is empty. Empty also when the error was
    /// built without a collected call, as the command line's own messages are.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.diagnostics
            .as_deref()
            .map(|list| list.0.as_slice())
            .unwrap_or(&[])
    }

    /// Attaches the `agerr` messages of the call that produced `self`.
    ///
    /// Not part of the stable API. The facade and the raster crate use it.
    #[doc(hidden)]
    pub fn __set_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        self.diagnostics = (!diagnostics.is_empty()).then(|| Box::new(Diagnostics(diagnostics)));
        self
    }

    /// Rewrites the detail, the attribute name and [`Self::file`].
    ///
    /// Not part of the stable API. The facade uses it to convert decoded text at
    /// the library boundary. The category prefix [`std::fmt::Display`] adds is
    /// left alone.
    #[doc(hidden)]
    pub fn __map_text(mut self, mut f: impl FnMut(String) -> String) -> Self {
        self.detail = f(self.detail);
        if let Some(file) = self.file.take() {
            self.file = Some(f(file));
        }
        if let Some(attr) = self.attr.take() {
            self.attr = Some(f(attr));
        }
        let mapped = self
            .diagnostics
            .take()
            .map(|list| {
                list.0
                    .into_iter()
                    .map(|diagnostic| diagnostic.__map_message(&mut f))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.diagnostics = (!mapped.is_empty()).then(|| Box::new(Diagnostics(mapped)));
        self
    }
}

impl PartialEq for GraphvizError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.detail == other.detail
            && self.engine == other.engine
            && self.attr == other.attr
            && self.line == other.line
            && self.column == other.column
            && self.file == other.file
    }
}

impl Eq for GraphvizError {}

impl fmt::Display for GraphvizError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            GraphvizErrorKind::Syntax => f.write_str(&self.detail),
            GraphvizErrorKind::UnsupportedEngine => {
                write!(f, "Unsupported layout engine: '{}'", self.detail)
            }
            GraphvizErrorKind::UnsupportedFormat => {
                write!(f, "Unsupported output format: '{}'", self.detail)
            }
            GraphvizErrorKind::Layout => {
                write!(
                    f,
                    "Layout computation error in {}: {}",
                    self.engine.unwrap_or(""),
                    self.detail
                )
            }
            GraphvizErrorKind::Attribute => {
                write!(
                    f,
                    "Invalid attribute '{}': {}",
                    self.attr.as_deref().unwrap_or(""),
                    self.detail
                )
            }
            GraphvizErrorKind::Render => write!(f, "Rendering error: {}", self.detail),
        }
    }
}

impl std::error::Error for GraphvizError {}

pub type Result<T> = std::result::Result<T, GraphvizError>;

/// An error has to stay under clippy's `result_large_err` limit. The diagnostic
/// list is heap-allocated so this holds.
const _: () = assert!(std::mem::size_of::<GraphvizError>() < 128);

#[cfg(test)]
mod tests {
    use super::*;

    const _: fn() = || {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<GraphvizError>();
        assert_send_sync_static::<Diagnostic>();
        assert_send_sync_static::<DiagnosticLevel>();
        assert_send_sync_static::<Rendered<String>>();
    };

    #[test]
    fn display_text_is_the_message_the_cli_prints() {
        let syntax = GraphvizError::syntax(3, 1, "syntax error in line 3 near ';'", None);
        assert_eq!(syntax.kind(), GraphvizErrorKind::Syntax);
        assert_eq!(syntax.line(), Some(3));
        assert_eq!(syntax.column(), Some(1));
        assert_eq!(syntax.file(), None);
        assert_eq!(syntax.engine(), None);
        assert_eq!(syntax.message(), "syntax error in line 3 near ';'");
        assert_eq!(syntax.to_string(), syntax.message());

        let named = GraphvizError::syntax(
            100,
            4,
            "syntax error in line 100 near ';'",
            Some("other.gv".into()),
        );
        assert_eq!(named.file(), Some("other.gv"));
        assert_eq!(named.message(), "syntax error in line 100 near ';'");

        let wrapped = GraphvizError::syntax(
            i32::MIN,
            1,
            "syntax error in line -2147483648 near ';'",
            None,
        );
        assert_eq!(wrapped.line(), None);
        assert_eq!(
            wrapped.to_string(),
            "syntax error in line -2147483648 near ';'"
        );

        let unknown_engine = GraphvizError::unsupported_engine("foo");
        assert_eq!(unknown_engine.message(), "foo");
        assert_eq!(
            unknown_engine.to_string(),
            "Unsupported layout engine: 'foo'"
        );
        assert_eq!(unknown_engine.engine(), None);

        let unknown_format = GraphvizError::unsupported_format("foo");
        assert_eq!(unknown_format.message(), "foo");
        assert_eq!(
            unknown_format.to_string(),
            "Unsupported output format: 'foo'"
        );

        let layout =
            GraphvizError::layout("dot", "overflow when calculating virtual weight of edge");
        assert_eq!(layout.kind(), GraphvizErrorKind::Layout);
        assert_eq!(layout.line(), None);
        assert_eq!(layout.column(), None);
        assert_eq!(layout.engine(), Some("dot"));
        assert_eq!(
            layout.message(),
            "overflow when calculating virtual weight of edge"
        );
        assert_eq!(
            layout.to_string(),
            format!("Layout computation error in dot: {}", layout.message())
        );

        let mapped = GraphvizError::layout("dot", "overflow").__map_text(|s| format!("[{s}]"));
        assert_eq!(mapped.message(), "[overflow]");
        assert_eq!(
            mapped.to_string(),
            "Layout computation error in dot: [overflow]"
        );

        let attribute = GraphvizError::attribute("rank", "bad");
        assert_eq!(attribute.message(), "bad");
        assert_eq!(attribute.to_string(), "Invalid attribute 'rank': bad");

        let render = GraphvizError::render("encoding the PNG: no");
        assert_eq!(render.message(), "encoding the PNG: no");
        assert_eq!(render.to_string(), "Rendering error: encoding the PNG: no");
        assert!(std::error::Error::source(&render).is_none());

        let plain = GraphvizError::syntax(1, 1, "syntax error in line 1 near ';'", None);
        assert!(plain.diagnostics().is_empty());
        let marked = plain.clone().__set_diagnostics(vec![Diagnostic::new(
            DiagnosticLevel::Warning,
            "Warning: x\n".into(),
        )]);
        assert_eq!(plain, marked);
        assert_ne!(plain.diagnostics(), marked.diagnostics());
    }
}
