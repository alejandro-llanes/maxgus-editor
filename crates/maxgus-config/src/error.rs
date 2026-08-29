//! Configuration errors and non-fatal warnings.

/// A problem that prevents the configuration from loading at all.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error reading configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Syntax(#[from] Box<kdl::KdlError>),
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
        Self { line, message: message.into() }
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
        assert_eq!(Warning::new(7, "unknown node `foo`").to_string(), "line 7: unknown node `foo`");
    }
}
