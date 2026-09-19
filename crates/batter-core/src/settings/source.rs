use super::SettingsError;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt,
    io::Read,
    path::Path,
};

/// An explicit source whose formatting hides both keys and values.
#[derive(Clone, Default)]
pub struct SettingsSource(BTreeMap<OsString, OsString>);
impl fmt::Debug for SettingsSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SettingsSource([REDACTED])")
    }
}
impl fmt::Display for SettingsSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl SettingsSource {
    /// Collect explicit inputs, rejecting duplicates before any merge.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, SettingsError> {
        let mut values = BTreeMap::new();
        for (key, value) in pairs {
            if values.insert(key, value).is_some() {
                return Err(SettingsError::new("source", "duplicate key"));
            }
        }
        Ok(Self(values))
    }
    /// Validate a dedicated source, or select known environment names.
    /// Unknown reserved-prefix names always fail; other unknown environment
    /// names are ignored only when `environment` is true. No input is echoed.
    pub fn select(
        &self,
        allowed: &[&str],
        reserved: &[&str],
        environment: bool,
    ) -> Result<Self, SettingsError> {
        let mut selected = BTreeMap::new();
        for (key, value) in &self.0 {
            if allowed.iter().any(|name| key == OsStr::new(name)) {
                selected.insert(key.clone(), value.clone());
            } else if !environment
                || reserved
                    .iter()
                    .any(|prefix| key.as_encoded_bytes().starts_with(prefix.as_bytes()))
            {
                return Err(SettingsError::new("source", "unknown key"));
            }
        }
        Ok(Self(selected))
    }
    /// Overlay a higher-precedence, already structurally validated source.
    pub fn overlay(&mut self, higher: Self) {
        self.0.extend(higher.0);
    }
    /// Deliberate raw access, retaining missing/empty/non-Unicode distinctions.
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.0.get(OsStr::new(name)).map(OsString::as_os_str)
    }
    /// Read an optional Unicode value, retaining non-Unicode data as a typed cause.
    pub fn text(&self, name: &'static str) -> Result<Option<&str>, SettingsError> {
        self.get(name)
            .map(|value| {
                value.to_str().ok_or_else(|| {
                    SettingsError::new(name, "non-Unicode value")
                        .with_cause(std::env::VarError::NotUnicode(value.to_owned()))
                })
            })
            .transpose()
    }
    /// Read required Unicode text; empty text is present and left for root policy.
    pub fn required(&self, name: &'static str) -> Result<&str, SettingsError> {
        self.text(name)?
            .ok_or_else(|| SettingsError::new(name, "missing value"))
    }
}

/// Read only the selected file. No search, substitution or environment mutation.
/// At most `limit + 1` bytes are read; missing/unreadable files are errors.
pub fn read_file(path: impl AsRef<Path>, limit: usize) -> Result<SettingsSource, SettingsError> {
    let file = std::fs::File::open(path)
        .map_err(|e| SettingsError::new("file", "cannot open").with_cause(e))?;
    read_literal(file, limit)
}

/// Parse bounded UTF-8 literal dotenv text, with LF or CRLF lines.
/// Keys are ASCII identifiers. Outer spaces/tabs are trimmed. Blank lines and
/// full-line comments are ignored. Values are unquoted without whitespace or
/// quotes, or wholly quoted in matching single/double quotes. Interior bytes,
/// dollars and backslashes are literal. No export, interpolation, escaping,
/// multiline values or inline comments are supported. Duplicate keys fail.
pub fn read_literal(reader: impl Read, limit: usize) -> Result<SettingsSource, SettingsError> {
    let bound = limit
        .checked_add(1)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| SettingsError::new("file", "byte limit overflow"))?;
    let mut bytes = Vec::new();
    reader
        .take(bound)
        .read_to_end(&mut bytes)
        .map_err(|e| SettingsError::new("file", "cannot read").with_cause(e))?;
    if bytes.len() > limit {
        return Err(SettingsError::new("file", "byte limit exceeded"));
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| SettingsError::new("file", "non-UTF-8 source").with_cause(e))?;
    let mut values = BTreeMap::new();
    for (index, line) in text.split('\n').enumerate() {
        let fail = |reason| SettingsError::new("file", reason).at_line(index + 1);
        let line = line
            .strip_suffix('\r')
            .unwrap_or(line)
            .trim_matches([' ', '\t']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| fail("expected KEY=VALUE"))?;
        let key = key.trim_matches([' ', '\t']);
        if key.is_empty()
            || !key
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
        {
            return Err(fail("invalid key syntax"));
        }
        let value = value.trim_matches([' ', '\t']);
        if value.contains(['\r', '\0']) {
            return Err(fail("invalid value syntax"));
        }
        let value = if value.starts_with(['\'', '"']) {
            if value.len() < 2 || value.as_bytes().last() != value.as_bytes().first() {
                return Err(fail("unmatched quotes"));
            }
            &value[1..value.len() - 1]
        } else {
            if value
                .chars()
                .any(|c| c.is_whitespace() || c == '\'' || c == '"' || c == '#')
            {
                return Err(fail("invalid unquoted value"));
            }
            value
        };
        if values
            .insert(OsString::from(key), OsString::from(value))
            .is_some()
        {
            return Err(fail("duplicate key"));
        }
    }
    Ok(SettingsSource(values))
}
