//! Where the last-good file lives, and the directory opened for it.

use std::{
    fs::OpenOptions,
    io::{ErrorKind, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{DirectoryError, format::LIMIT};

/// The suffix every last-good file name ends with. The temporary and probe
/// names a writer uses end otherwise, so no valid file name is another's.
const SUFFIX: &str = ".bfsn";

/// The last-good file's location: an absolute directory and a plain file
/// name of 6 to 64 ASCII letters, digits, `.`, `_` or `-`, not starting with
/// `.` and ending with `.bfsn`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotFilePath {
    directory: PathBuf,
    name: String,
}

impl SnapshotFilePath {
    /// Validates the location. Nothing is touched on disk.
    pub fn new(directory: impl Into<PathBuf>, name: &str) -> Result<Self, DirectoryError> {
        let directory = directory.into();
        if !directory.is_absolute() {
            return Err(DirectoryError::NotAbsolute);
        }
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || b"._-".contains(&byte);
        if name.len() <= SUFFIX.len()
            || name.len() > 64
            || name.starts_with('.')
            || !name.ends_with(SUFFIX)
            || !name.bytes().all(allowed)
        {
            return Err(DirectoryError::InvalidName);
        }
        Ok(Self {
            directory,
            name: name.to_owned(),
        })
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn file(&self) -> PathBuf {
        self.directory.join(&self.name)
    }

    /// The fixed temporary name a persist writes before its rename.
    pub(crate) fn temporary(&self) -> PathBuf {
        self.directory.join(format!("{}.tmp", self.name))
    }

    /// A name `open` creates and removes to prove the directory writable,
    /// distinct for each attempt: the file name, `.probe-`, then this
    /// process's id, the wall clock in nanoseconds and a process-wide count.
    /// A probe a crashed process left behind, or one a concurrent opener of
    /// the same path holds, therefore does not fail a later attempt: in the
    /// unlikely event a name already exists, `open` retries under a new one.
    fn probe(&self) -> PathBuf {
        static ATTEMPT: AtomicU64 = AtomicU64::new(0);
        let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        self.directory.join(format!(
            "{}.probe-{}-{nanos}-{attempt}",
            self.name,
            std::process::id()
        ))
    }
}

/// How many distinct probe names `open` tries before calling a directory
/// unwritable; only an existing name is retried.
const PROBE_ATTEMPTS: usize = 4;

/// What the directory held at startup.
#[derive(Clone)]
pub(crate) enum Initial {
    /// No file: nothing to publish.
    Absent,
    /// The file's bytes, at most one byte over the limit.
    Read(Vec<u8>),
    /// The path exists but is not a readable regular file.
    Unreadable,
}

/// A directory opened for the last-good file, with what it held at startup.
///
/// Open it in the root's protected initializer: an unusable directory fails
/// startup there, while a missing, damaged or foreign file never does.
/// Formatting shows the path and how much was read, never the bytes.
///
/// ```no_run
/// use batter_flags::control_plane::{SnapshotDirectory, SnapshotFilePath};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let path = SnapshotFilePath::new("/var/lib/service/flags", "last-good.bfsn")?;
/// let directory = SnapshotDirectory::open(path)?;
/// # let _ = directory;
/// # Ok(())
/// # }
/// ```
pub struct SnapshotDirectory {
    path: SnapshotFilePath,
    initial: Initial,
}

impl SnapshotDirectory {
    /// Checks that the directory exists and that this process can create
    /// and remove a file in it, then reads the last-good file if it is a
    /// regular file. This blocks; call it from a blocking initializer.
    pub fn open(path: SnapshotFilePath) -> Result<Self, DirectoryError> {
        let metadata = std::fs::metadata(path.directory()).map_err(|_| DirectoryError::Missing)?;
        if !metadata.is_dir() {
            return Err(DirectoryError::NotADirectory);
        }
        // Permission bits say nothing about ownership, ACLs or a read-only
        // mount, so prove the operations a persist needs.
        prove_writable(&path)?;
        let initial = read_bounded(&path.file());
        Ok(Self { path, initial })
    }

    pub(crate) fn path(&self) -> &SnapshotFilePath {
        &self.path
    }

    pub(crate) fn initial(&self) -> &Initial {
        &self.initial
    }
}

/// Creates and removes a fresh probe. An existing name, which only a
/// leftover or a racing opener can explain, is retried under a new one.
fn prove_writable(path: &SnapshotFilePath) -> Result<(), DirectoryError> {
    for _ in 0..PROBE_ATTEMPTS {
        let probe = path.probe();
        let created = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&probe);
        match created {
            Ok(file) => {
                drop(file);
                return std::fs::remove_file(&probe).map_err(|_| DirectoryError::NotWritable);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return Err(DirectoryError::NotWritable),
        }
    }
    Err(DirectoryError::NotWritable)
}

impl std::fmt::Debug for SnapshotDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let initial = match &self.initial {
            Initial::Absent => "absent".to_owned(),
            Initial::Read(bytes) => format!("{} bytes", bytes.len()),
            Initial::Unreadable => "unreadable".to_owned(),
        };
        f.debug_struct("SnapshotDirectory")
            .field("path", &self.path)
            .field("initial", &initial)
            .finish()
    }
}

/// Reads at most one byte over the limit from `path`, and only from a
/// regular file, so a FIFO or a device never blocks the reader and an
/// oversized file is never allocated whole.
///
/// The open never waits, since `O_NONBLOCK` returns at once for a FIFO
/// without a writer, and the type is judged on the opened descriptor, so a
/// path replaced after any earlier look cannot slip a FIFO past the check.
/// `O_NONBLOCK` does not change reads of a regular file.
pub(crate) fn read_bounded(path: &Path) -> Initial {
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Initial::Absent,
        Err(_) => return Initial::Unreadable,
    };
    match file.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) | Err(_) => return Initial::Unreadable,
    }
    let mut bytes = Vec::new();
    match file.take(LIMIT as u64 + 1).read_to_end(&mut bytes) {
        Ok(_) => Initial::Read(bytes),
        Err(_) => Initial::Unreadable,
    }
}
