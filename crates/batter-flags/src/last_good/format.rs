//! The `BFSN` v1 last-good snapshot file, as `docs/snapshot-file-v1.md`
//! defines it: a header, length-prefixed entries and a SHA-256 trailer over
//! every byte before it.

use sha2::{Digest, Sha256};

use super::FileProblem;

const MAGIC: &[u8; 4] = b"BFSN";
const FORMAT_VERSION: u8 = 1;
/// The largest file this build reads or writes.
pub(crate) const LIMIT: usize = 1 << 20;
const TRAILER: usize = 32;

/// One file's content, before any catalog has judged it. Formatting shows
/// the version and the keys, never a state.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FileSnapshot {
    /// Which process scope wrote it: the settings prefix and process kind.
    pub(crate) scope: String,
    pub(crate) catalog_digest: [u8; 32],
    pub(crate) manifest_digest: [u8; 32],
    pub(crate) version: u64,
    pub(crate) entries: Vec<FileEntry>,
}

/// One persisted row, with the fingerprint of the declaration it was
/// written for.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FileEntry {
    pub(crate) key: String,
    pub(crate) kind: String,
    pub(crate) value_type: String,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) updated_version: u64,
    pub(crate) value: EntryValue,
}

/// What an entry holds. On disk an absent value is a state of length zero,
/// which no state encoding produces.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum EntryValue {
    /// The row's state in state encoding 1.
    State(String),
    /// The entry was held and its last value did not come from the control
    /// plane, so none is written: a reader serves the declared base for it.
    Absent,
}

impl std::fmt::Debug for FileSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSnapshot")
            .field("version", &self.version)
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for FileEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self.value {
            EntryValue::State(_) => "state",
            EntryValue::Absent => "absent",
        };
        f.debug_struct("FileEntry")
            .field("key", &self.key)
            .field("value", &value)
            .finish_non_exhaustive()
    }
}

pub(crate) fn encode(file: &FileSnapshot) -> Result<Vec<u8>, FileProblem> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    put_text(&mut out, &file.scope, Width::Two)?;
    out.extend_from_slice(&file.catalog_digest);
    out.extend_from_slice(&file.manifest_digest);
    out.extend_from_slice(&file.version.to_be_bytes());
    let count = u32::try_from(file.entries.len()).map_err(|_| FileProblem::TooLarge)?;
    out.extend_from_slice(&count.to_be_bytes());
    for entry in &file.entries {
        put_text(&mut out, &entry.key, Width::One)?;
        put_text(&mut out, &entry.kind, Width::One)?;
        put_text(&mut out, &entry.value_type, Width::One)?;
        out.extend_from_slice(&entry.fingerprint);
        out.extend_from_slice(&entry.updated_version.to_be_bytes());
        match &entry.value {
            EntryValue::State(state) if state.is_empty() => return Err(FileProblem::Malformed),
            EntryValue::State(state) => put_text(&mut out, state, Width::Two)?,
            EntryValue::Absent => put_text(&mut out, "", Width::Two)?,
        }
    }
    let trailer = Sha256::digest(&out);
    out.extend_from_slice(&trailer);
    if out.len() > LIMIT {
        return Err(FileProblem::TooLarge);
    }
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<FileSnapshot, FileProblem> {
    if bytes.len() > LIMIT {
        return Err(FileProblem::TooLarge);
    }
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4)? != MAGIC {
        return Err(FileProblem::Malformed);
    }
    if reader.take(1)?[0] != FORMAT_VERSION {
        return Err(FileProblem::UnsupportedVersion);
    }
    let scope = reader.text(Width::Two)?;
    let catalog_digest = reader.digest()?;
    let manifest_digest = reader.digest()?;
    let version = reader.u64()?;
    let count = reader.u32()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        entries.push(FileEntry {
            key: reader.text(Width::One)?,
            kind: reader.text(Width::One)?,
            value_type: reader.text(Width::One)?,
            fingerprint: reader.digest()?,
            updated_version: reader.u64()?,
            value: match reader.text(Width::Two)? {
                state if state.is_empty() => EntryValue::Absent,
                state => EntryValue::State(state),
            },
        });
    }
    let body = reader.at;
    match bytes.len() - body {
        TRAILER => {}
        short if short < TRAILER => return Err(FileProblem::Truncated),
        _ => return Err(FileProblem::TrailingBytes),
    }
    if Sha256::digest(&bytes[..body]).as_slice() != &bytes[body..] {
        return Err(FileProblem::Tampered);
    }
    Ok(FileSnapshot {
        scope,
        catalog_digest,
        manifest_digest,
        version,
        entries,
    })
}

/// The width of a text field's length prefix.
#[derive(Clone, Copy)]
enum Width {
    One,
    Two,
}

fn put_text(out: &mut Vec<u8>, text: &str, width: Width) -> Result<(), FileProblem> {
    match width {
        Width::One => out.push(u8::try_from(text.len()).map_err(|_| FileProblem::TooLarge)?),
        Width::Two => out.extend_from_slice(
            &u16::try_from(text.len())
                .map_err(|_| FileProblem::TooLarge)?
                .to_be_bytes(),
        ),
    }
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// The next `count` bytes of the body, which ends before the trailer.
    fn take(&mut self, count: usize) -> Result<&'a [u8], FileProblem> {
        let end = self.at.checked_add(count).ok_or(FileProblem::Truncated)?;
        let body_end = self.bytes.len().saturating_sub(TRAILER);
        if end > body_end {
            return Err(FileProblem::Truncated);
        }
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u32(&mut self) -> Result<u32, FileProblem> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, FileProblem> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn digest(&mut self) -> Result<[u8; 32], FileProblem> {
        self.array()
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], FileProblem> {
        self.take(N)?.try_into().map_err(|_| FileProblem::Truncated)
    }

    fn text(&mut self, width: Width) -> Result<String, FileProblem> {
        let length = match width {
            Width::One => usize::from(self.take(1)?[0]),
            Width::Two => usize::from(u16::from_be_bytes(self.array()?)),
        };
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| FileProblem::Malformed)
    }
}
