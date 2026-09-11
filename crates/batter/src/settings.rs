//! Explicit settings inputs, bounded parsing and value-hiding diagnostics.
//!
//! Applications own names, defaults, source order and required-secret policy.
//! Nothing here reads the process environment or searches for a file implicitly.
//! Explicit exposure and error-source traversal are trusted operations; these
//! wrappers do not erase memory or sanitize third-party logging.
//!
//! ```
//! use batter::settings::{SettingsSource, SecretString, bounded_u64};
//! let source = SettingsSource::from_pairs([("LIMIT".into(), "4".into())])?;
//! let limit = bounded_u64(source.required("LIMIT")?, "LIMIT", 1, 16)?;
//! assert_eq!(limit, 4);
//! assert_eq!(SecretString::new("credential").to_string(), "[REDACTED]");
//! # Ok::<(), batter::settings::SettingsError>(())
//! ```

mod source;
pub use source::{SettingsSource, read_file, read_literal};

use std::{error::Error, fmt, time::Duration};

/// Owned text with deliberate access and fixed redacted formatting.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);
impl SecretString {
    /// Wrap text without changing or erasing its bytes.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Expose the original text to a trusted native consumer.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A static diagnostic projection retaining a concrete cause for trusted inspection.
/// Only use application-authored static labels, never leaked input strings.
pub struct RedactedError<E> {
    field: &'static str,
    reason: &'static str,
    cause: E,
}
impl<E> RedactedError<E> {
    /// Retain a cause without formatting it.
    pub fn new(field: &'static str, reason: &'static str, cause: E) -> Self {
        Self {
            field,
            reason,
            cause,
        }
    }
    /// Deliberately inspect the original typed cause.
    pub fn cause(&self) -> &E {
        &self.cause
    }
}
impl<E> fmt::Display for RedactedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}
impl<E> fmt::Debug for RedactedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<E: Error + 'static> Error for RedactedError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

/// Value-hiding source or parsing failure. Causes remain available intentionally.
pub struct SettingsError {
    field: &'static str,
    reason: &'static str,
    line: Option<usize>,
    cause: Option<crate::BoxError>,
}
impl SettingsError {
    /// Construct a failure using only trusted static diagnostic metadata.
    pub fn new(field: &'static str, reason: &'static str) -> Self {
        Self {
            field,
            reason,
            line: None,
            cause: None,
        }
    }
    /// Retain an original error without including it in Debug or Display.
    pub fn with_cause(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }
    pub(super) fn at_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }
    /// Stable application field or source category.
    pub fn field(&self) -> &'static str {
        self.field
    }
    /// Static rejection reason, never raw input.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
    /// One-based line when failure arose in the literal reader.
    pub fn line(&self) -> Option<usize> {
        self.line
    }
}
impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)?;
        if let Some(line) = self.line {
            write!(f, " (line {line})")?;
        }
        Ok(())
    }
}
impl fmt::Debug for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for SettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause.as_deref().map(|e| e as &(dyn Error + 'static))
    }
}

/// Parse an unsigned decimal integer and reject values outside inclusive bounds.
/// Signs and whitespace are rejected; overflow retains `ParseIntError`.
pub fn bounded_u64(
    value: &str,
    field: &'static str,
    min: u64,
    max: u64,
) -> Result<u64, SettingsError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SettingsError::new(
            field,
            "expected unsigned decimal integer",
        ));
    }
    let value = value
        .parse::<u64>()
        .map_err(|e| SettingsError::new(field, "integer overflow").with_cause(e))?;
    if value < min || value > max {
        return Err(SettingsError::new(field, "outside permitted bounds"));
    }
    Ok(value)
}

/// Parse positive milliseconds with an explicit inclusive application maximum.
pub fn milliseconds(
    value: &str,
    field: &'static str,
    max: Duration,
) -> Result<Duration, SettingsError> {
    let limit = u64::try_from(max.as_millis())
        .map_err(|e| SettingsError::new(field, "duration bound overflow").with_cause(e))?;
    Ok(Duration::from_millis(bounded_u64(value, field, 1, limit)?))
}
