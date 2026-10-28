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
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

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
        self
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const _: fn() = || {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<GraphvizError>();
    };

    #[test]
    fn graphviz_error_is_send_sync_and_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<GraphvizError>();
    }

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
    }
}
