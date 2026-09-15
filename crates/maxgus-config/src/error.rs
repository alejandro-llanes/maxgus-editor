//! Configuration errors and non-fatal warnings.

/// A problem that prevents the configuration from loading at all.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error reading configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("{}", describe_syntax(.0))]
    Syntax(#[from] Box<kdl::KdlError>),
}

/// A KDL error said so that it can be acted on: the line, and what is wrong
/// there. The error's own text is "Failed to parse KDL document" whatever the
/// failure, and that was all anyone was told.
fn describe_syntax(error: &kdl::KdlError) -> String {
    let Some(first) = error.diagnostics.first() else {
        return error.to_string();
    };
    let line = error
        .input
        .chars()
        .take(first.span.offset())
        .filter(|c| *c == '\n')
        .count()
        + 1;
    let mut said = format!(
        "line {line}: {}",
        first
            .message
            .as_deref()
            .unwrap_or("this cannot be read as KDL")
    );
    if let Some(help) = &first.help {
        said.push_str(&format!(" ({help})"));
    }
    if error.diagnostics.len() > 1 {
        said.push_str(&format!(", and {} more", error.diagnostics.len() - 1));
    }
    said
}

impl From<kdl::KdlError> for ConfigError {
    fn from(e: kdl::KdlError) -> Self {
        // `KdlError` is large; box it so `ConfigError` stays cheap to move.
        ConfigError::Syntax(Box::new(e))
    }
}

/// A recoverable complaint about one node: the rest of the file still loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// One-based line number in the config file.
    pub line: usize,
    pub message: String,
}

impl Warning {
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Converts a byte offset in `source` into a one-based line number.
pub fn line_of(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    source[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_are_one_based() {
        let src = "first\nsecond\nthird";
        assert_eq!(line_of(src, 0), 1);
        assert_eq!(line_of(src, 6), 2);
        assert_eq!(line_of(src, 13), 3);
    }

    #[test]
    fn offsets_past_the_end_clamp_to_the_last_line() {
        let src = "a\nb";
        assert_eq!(line_of(src, 9999), 2);
    }

    #[test]
    fn warnings_render_with_their_line() {
        assert_eq!(
            Warning::new(7, "unknown node `foo`").to_string(),
            "line 7: unknown node `foo`"
        );
    }
}
