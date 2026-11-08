//! Writing the last-good file: one writer at a time, under a directory lock,
//! through a fixed temporary file and a rename.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use tokio::{sync::Semaphore, time::Instant};

use super::{
    PersistFailure, SnapshotFilePath,
    format::{self, FileSnapshot},
    path::{Initial, read_bounded},
};

/// The version this process last wrote, which decides whether a newer file
/// on disk is replaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Written {
    /// Nothing yet: the first write replaces even an equal or newer file,
    /// which may hold pre-restore state.
    Never,
    /// This version: a later write skips a file newer than it.
    At(u64),
}

/// What is waiting to be written.
enum Queue {
    Idle,
    /// The latest publication not yet written. It is retried every run,
    /// whatever that run read, until it is written or a newer one replaces
    /// it.
    Waiting(FileSnapshot),
}

struct State {
    written: Written,
    queue: Queue,
}

/// What an attempt did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Persisted {
    /// Nothing was waiting.
    Idle,
    Written,
    /// The file on disk is newer than the publication, which is dropped.
    NewerOnDisk,
}

pub(crate) struct Persister {
    path: SnapshotFilePath,
    /// The library-owned bulkhead: one write at a time, the rest refused.
    slot: Arc<Semaphore>,
    /// Shared with the blocking writer, which records its own success, so a
    /// write the deadline abandoned still counts once it completes.
    state: Arc<Mutex<State>>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Persister {
    pub(crate) fn new(path: SnapshotFilePath) -> Self {
        Self {
            path,
            slot: Arc::new(Semaphore::new(1)),
            state: Arc::new(Mutex::new(State {
                written: Written::Never,
                queue: Queue::Idle,
            })),
        }
    }

    /// Queues a control-plane publication, unless its version is the one
    /// already written.
    pub(crate) fn offer(&self, file: FileSnapshot) {
        let mut state = lock(&self.state);
        if state.written != Written::At(file.version) {
            state.queue = Queue::Waiting(file);
        }
    }

    /// Writes what is waiting, within `deadline`. A write the deadline
    /// abandons finishes in the background, holds the slot until it does and
    /// records its own outcome.
    pub(crate) async fn attempt(&self, deadline: Instant) -> Result<Persisted, PersistFailure> {
        let (file, written) = {
            let state = lock(&self.state);
            match &state.queue {
                Queue::Idle => return Ok(Persisted::Idle),
                Queue::Waiting(file) => (file.clone(), state.written),
            }
        };
        let bytes = format::encode(&file).map_err(|_| PersistFailure::TooLarge)?;
        let permit = self
            .slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| PersistFailure::Busy)?;
        let path = self.path.clone();
        let state = self.state.clone();
        let version = file.version;
        let task = tokio::task::spawn_blocking(move || {
            let outcome = write(&path, &bytes, written, version);
            if let Ok(done @ (Persisted::Written | Persisted::NewerOnDisk)) = outcome {
                let mut state = lock(&state);
                if done == Persisted::Written {
                    state.written = Written::At(version);
                }
                if let Queue::Waiting(waiting) = &state.queue
                    && waiting.version == version
                {
                    state.queue = Queue::Idle;
                }
            }
            drop(permit);
            outcome
        });
        match tokio::time::timeout_at(deadline, task).await {
            Err(_) => Err(PersistFailure::Deadline),
            Ok(Err(_)) => Err(PersistFailure::Io),
            Ok(Ok(outcome)) => outcome,
        }
    }
}

/// The blocking write: lock the directory, compare with the file on disk,
/// then write the temporary file, sync it, rename it and sync the directory.
fn write(
    path: &SnapshotFilePath,
    bytes: &[u8],
    written: Written,
    version: u64,
) -> Result<Persisted, PersistFailure> {
    let directory = File::open(path.directory()).map_err(|_| PersistFailure::Io)?;
    match directory.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(PersistFailure::Busy),
        Err(std::fs::TryLockError::Error(_)) => return Err(PersistFailure::Io),
    }
    if let Written::At(_) = written
        && let Initial::Read(existing) = read_bounded(&path.file())
        && let Ok(existing) = format::decode(&existing)
        && existing.version > version
    {
        return Ok(Persisted::NewerOnDisk);
    }
    // Residue of an abandoned write is ours to remove: only a holder of the
    // directory lock writes the temporary file, and no valid file name is
    // another file's temporary name.
    match std::fs::remove_file(path.temporary()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(PersistFailure::Io),
    }
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path.temporary())
        .map_err(|_| PersistFailure::Io)?;
    temporary.write_all(bytes).map_err(|_| PersistFailure::Io)?;
    temporary.sync_all().map_err(|_| PersistFailure::Io)?;
    drop(temporary);
    std::fs::rename(path.temporary(), path.file()).map_err(|_| PersistFailure::Io)?;
    // The file is replaced from here on; only its durability is in doubt.
    // Directory fsync durability on macOS is unverified; see the format doc.
    directory
        .sync_all()
        .map_err(|_| PersistFailure::NotDurable)?;
    Ok(Persisted::Written)
}
