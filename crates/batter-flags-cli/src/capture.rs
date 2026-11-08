//! The one read of the process environment: the database URL, captured once
//! and redacted everywhere it travels.

use std::{ffi::OsString, fmt};

/// The variable the operator command reads its endpoint from.
pub(crate) const DATABASE_URL: &str = "BATTER_FLAGS_DATABASE_URL";

/// The captured configuration. Its formatting never shows the URL.
pub(crate) struct Captured {
    url: String,
}

impl Captured {
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for Captured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Captured([redacted])")
    }
}

/// Why the environment was refused. It names keys, never values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureError {
    /// The database URL is not set.
    Missing,
    /// The database URL is not valid Unicode.
    NotUnicode,
    /// A `PG*` variable is set; libpq-style ambient settings are refused, so
    /// only the captured URL decides where the command connects.
    Ambient { key: String },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "{DATABASE_URL} is not set"),
            Self::NotUnicode => write!(f, "{DATABASE_URL} is not valid Unicode"),
            Self::Ambient { key } => write!(f, "ambient PostgreSQL setting {key} is set; unset it"),
        }
    }
}

/// Captures the configuration from `pairs`, the whole environment, without
/// reading anything else.
pub(crate) fn capture(
    pairs: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Captured, CaptureError> {
    let mut url = None;
    for (key, value) in pairs {
        let Some(key) = key.to_str() else {
            continue;
        };
        if key.starts_with("PG") {
            return Err(CaptureError::Ambient {
                key: key.to_owned(),
            });
        }
        if key == DATABASE_URL {
            url = Some(value.into_string().map_err(|_| CaptureError::NotUnicode)?);
        }
    }
    url.map(|url| Captured { url }).ok_or(CaptureError::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(entries: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        entries
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn the_url_is_captured_and_never_formatted() {
        let captured = capture(pairs(&[
            (DATABASE_URL, "postgres://u:SECRET@localhost/db"),
            ("HOME", "/"),
        ]))
        .ok();
        let captured = captured.as_ref().map(Captured::url);
        assert_eq!(captured, Some("postgres://u:SECRET@localhost/db"));
        let debug = format!(
            "{:?}",
            capture(pairs(&[(DATABASE_URL, "postgres://u:SECRET@h/d")]))
        );
        assert!(!debug.contains("SECRET"), "{debug}");
    }

    #[test]
    fn a_missing_url_and_ambient_settings_are_refused_by_key() {
        assert_eq!(capture(pairs(&[])).err(), Some(CaptureError::Missing));
        assert_eq!(
            capture(pairs(&[
                (DATABASE_URL, "postgres://h/d"),
                ("PGPASSWORD", "SECRET")
            ]))
            .err(),
            Some(CaptureError::Ambient {
                key: "PGPASSWORD".into()
            })
        );
        let shown = CaptureError::Ambient {
            key: "PGPASSWORD".into(),
        }
        .to_string();
        assert!(!shown.contains("SECRET"));
    }
}
