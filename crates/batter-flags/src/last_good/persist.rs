//! Writing the last-good file: one writer at a time, under a directory lock,
//! through a fixed temporary file and a rename.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use tokio::time::Instant;

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
    /// This version replaced the file, durably or not: a later write skips a
    /// file newer than it.
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
    phase: Phase,
}

enum Phase {
    Ready,
    Writing,
}

/// Exclusive ownership of one selected write. Destruction releases admission
/// even if encoding, blocking-task delivery or the write itself unwinds.
struct Selected {
    state: Arc<Mutex<State>>,
    file: FileSnapshot,
    written: Written,
}

impl Selected {
    fn finish(
        self,
        outcome: Result<Persisted, PersistFailure>,
    ) -> Result<Persisted, PersistFailure> {
        let mut state = lock(&self.state);
        match outcome {
            Ok(Persisted::Written) => {
                state.written = Written::At(self.file.version);
                self.settle(&mut state);
            }
            Ok(Persisted::NewerOnDisk) => self.settle(&mut state),
            // The rename replaced the file and only its durability is in
            // doubt, so the first-write authority is spent: the retry, which
            // stays queued, must not replace a newer file another process
            // writes meanwhile.
            Err(PersistFailure::NotDurable) => state.written = Written::At(self.file.version),
            Ok(Persisted::Idle) | Err(_) => {}
        }
        drop(state);
        outcome
    }

    /// Clears the queue if it still holds this write's publication.
    fn settle(&self, state: &mut State) {
        if let Queue::Waiting(waiting) = &state.queue
            && waiting.version == self.file.version
        {
            state.queue = Queue::Idle;
        }
    }
}

impl Drop for Selected {
    fn drop(&mut self) {
        lock(&self.state).phase = Phase::Ready;
    }
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
    /// Shared with the blocking writer, which records its own success, so a
    /// write the deadline abandoned still counts once it completes.
    state: Arc<Mutex<State>>,
    /// Test builds only: holds the next blocking write until the test sends,
    /// so a test can abandon a write through [`Persister::attempt`].
    #[cfg(test)]
    hold: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// Test builds only: pauses the next attempt right after it selects, so
    /// a test can show that no in-flight write can change what it selected.
    #[cfg(test)]
    pause: Mutex<Option<Pause>>,
}

/// Signals that an attempt reached the pause, and resumes it.
#[cfg(test)]
type Pause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Persister {
    pub(crate) fn new(path: SnapshotFilePath) -> Self {
        Self {
            path,
            state: Arc::new(Mutex::new(State {
                written: Written::Never,
                queue: Queue::Idle,
                phase: Phase::Ready,
            })),
            #[cfg(test)]
            hold: Mutex::new(None),
            #[cfg(test)]
            pause: Mutex::new(None),
        }
    }

    /// Pauses the next attempt right after its selection: the receiver
    /// completes when it gets there, and sending resumes it.
    #[cfg(test)]
    fn pause_after_selection(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let (resume, resume_rx) = tokio::sync::oneshot::channel();
        *self.pause.lock().unwrap_or_else(PoisonError::into_inner) = Some((reached, resume_rx));
        (reached_rx, resume)
    }

    /// A persister whose next blocking write waits until the returned sender
    /// sends or is dropped.
    #[cfg(test)]
    fn held(path: SnapshotFilePath) -> (Self, std::sync::mpsc::Sender<()>) {
        let (release, hold) = std::sync::mpsc::channel();
        let persister = Self::new(path);
        *persister
            .hold
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(hold);
        (persister, release)
    }

    /// Queues a control-plane publication, unless its version is the one
    /// already written.
    pub(crate) fn offer(&self, file: FileSnapshot) {
        let mut state = lock(&self.state);
        if state.written != Written::At(file.version) {
            state.queue = Queue::Waiting(file);
        }
    }

    /// Selection and exclusive admission happen under one lock. A completing
    /// abandoned write cannot change the first-write rule between these steps.
    fn select(&self) -> Result<Option<Selected>, PersistFailure> {
        let mut state = lock(&self.state);
        match state.phase {
            Phase::Writing => return Err(PersistFailure::Busy),
            Phase::Ready => {}
        }
        let file = match &state.queue {
            Queue::Idle => return Ok(None),
            Queue::Waiting(file) => file.clone(),
        };
        state.phase = Phase::Writing;
        Ok(Some(Selected {
            state: self.state.clone(),
            file,
            written: state.written,
        }))
    }

    /// Writes what is waiting, within `deadline`. A write the deadline
    /// abandons finishes in the background, holds the slot until it does and
    /// records its own outcome.
    pub(crate) async fn attempt(&self, deadline: Instant) -> Result<Persisted, PersistFailure> {
        let selected = match self.select()? {
            Some(selected) => selected,
            None => return Ok(Persisted::Idle),
        };
        #[cfg(test)]
        {
            let pause = self
                .pause
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some((reached, resume)) = pause {
                let _ = reached.send(());
                let _ = resume.await;
            }
        }
        let bytes = format::encode(&selected.file).map_err(|_| PersistFailure::TooLarge)?;
        let path = self.path.clone();
        #[cfg(test)]
        let hold = self
            .hold
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let task = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(hold) = hold {
                // A dropped sender releases the write too.
                let _ = hold.recv();
            }
            let outcome = write(&path, &bytes, selected.written, selected.file.version);
            selected.finish(outcome)
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

#[cfg(test)]
mod tests;
