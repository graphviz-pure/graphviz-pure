use std::fmt;

/// An error from parsing, laying out, or rendering a graph.
///
/// The struct is opaque: it has no public fields, so a later release can attach
/// diagnostics without a breaking change. Read it with [`Self::kind`],
/// [`Self::message`], and [`Self::line`]. [`GraphvizErrorKind`] is
/// non-exhaustive, so a new kind is a minor change. Match it with a wildcard.
///
/// [`Self::message`] is the text [`std::fmt::Display`] writes. The command line
/// prints that text unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphvizError {
    kind: GraphvizErrorKind,
    message: String,
    line: Option<i32>,
    /// The file a `# N "file"` line directive named, for a syntax error.
    file: Option<String>,
}

/// Why a [`GraphvizError`] was returned.
///
/// Non-exhaustive, so a new kind can be added in a minor release.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// A DOT syntax error. `message` is cgraph's wording (`syntax error in line N
    /// near ...`) and `line` is cgraph's `line_num`. `file` is the name a
    /// `# N "file"` line directive set, when there was one.
    pub fn syntax(line: i32, message: impl Into<String>, file: Option<String>) -> Self {
        Self {
            kind: GraphvizErrorKind::Syntax,
            message: message.into(),
            line: Some(line),
            file,
        }
    }

    /// `Unsupported layout engine: '{name}'`.
    pub fn unsupported_engine(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            kind: GraphvizErrorKind::UnsupportedEngine,
            message: format!("Unsupported layout engine: '{name}'"),
            line: None,
            file: None,
        }
    }

    /// `Unsupported output format: '{name}'`.
    pub fn unsupported_format(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            kind: GraphvizErrorKind::UnsupportedFormat,
            message: format!("Unsupported output format: '{name}'"),
            line: None,
            file: None,
        }
    }

    /// `Layout computation error in {stage}: {message}`.
    pub fn layout(stage: &'static str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: GraphvizErrorKind::Layout,
            message: format!("Layout computation error in {stage}: {message}"),
            line: None,
            file: None,
        }
    }

    /// `Invalid attribute '{attr}': {message}`.
    pub fn attribute(attr: impl Into<String>, message: impl Into<String>) -> Self {
        let attr = attr.into();
        let message = message.into();
        Self {
            kind: GraphvizErrorKind::Attribute,
            message: format!("Invalid attribute '{attr}': {message}"),
            line: None,
            file: None,
        }
    }

    /// `Rendering error: {message}`.
    pub fn render(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: GraphvizErrorKind::Render,
            message: format!("Rendering error: {message}"),
            line: None,
            file: None,
        }
    }

    /// The kind of failure.
    pub fn kind(&self) -> GraphvizErrorKind {
        self.kind
    }

    /// The text [`std::fmt::Display`] writes.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The source line, for a syntax error.
    pub fn line(&self) -> Option<i32> {
        self.line
    }

    /// The file a `# N "file"` line directive named. Only a syntax error has one.
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// Applies `f` to the message and to [`Self::file`]. The facade uses this to
    /// convert decoded text at the library boundary.
    pub fn map_text(mut self, mut f: impl FnMut(String) -> String) -> Self {
        self.message = f(self.message);
        if let Some(file) = self.file.take() {
            self.file = Some(f(file));
        }
        self
    }
}

impl fmt::Display for GraphvizError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GraphvizError {}

pub type Result<T> = std::result::Result<T, GraphvizError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_is_the_message_the_cli_prints() {
        let syntax = GraphvizError::syntax(3, "syntax error in line 3 near ';'", None);
        assert_eq!(syntax.kind(), GraphvizErrorKind::Syntax);
        assert_eq!(syntax.line(), Some(3));
        assert_eq!(syntax.file(), None);
        assert_eq!(syntax.to_string(), "syntax error in line 3 near ';'");

        let named = GraphvizError::syntax(
            100,
            "syntax error in line 100 near ';'",
            Some("other.gv".into()),
        );
        assert_eq!(named.file(), Some("other.gv"));
        assert_eq!(named.message(), "syntax error in line 100 near ';'");

        assert_eq!(
            GraphvizError::unsupported_engine("foo").to_string(),
            "Unsupported layout engine: 'foo'"
        );
        assert_eq!(
            GraphvizError::unsupported_format("foo").to_string(),
            "Unsupported output format: 'foo'"
        );
        let layout =
            GraphvizError::layout("dot", "overflow when calculating virtual weight of edge");
        assert_eq!(layout.kind(), GraphvizErrorKind::Layout);
        assert_eq!(layout.line(), None);
        assert_eq!(
            layout.to_string(),
            "Layout computation error in dot: overflow when calculating virtual weight of edge"
        );
        assert_eq!(
            GraphvizError::attribute("rank", "bad").to_string(),
            "Invalid attribute 'rank': bad"
        );
        assert_eq!(
            GraphvizError::render("encoding the PNG: no").to_string(),
            "Rendering error: encoding the PNG: no"
        );
    }
}
