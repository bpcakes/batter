#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "failure-contract assertions"
)]

use super::*;
use std::time::Duration;

struct Directory(std::path::PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("batter-persist-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> SnapshotFilePath {
        SnapshotFilePath::new(&self.0, "snapshot.bfsn").unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn snapshot(version: u64) -> FileSnapshot {
    FileSnapshot {
        scope: "FLAG_:serving".into(),
        catalog_digest: [0; 32],
        manifest_digest: [0; 32],
        version,
        entries: Vec::new(),
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn write_selected(selected: Selected, path: &SnapshotFilePath) {
    let bytes = format::encode(&selected.file).unwrap();
    let outcome = write(path, &bytes, selected.written, selected.file.version);
    assert_eq!(selected.finish(outcome), Ok(Persisted::Written));
}

#[tokio::test]
async fn abandoned_write_keeps_admission_until_completion_and_next_selection_sees_its_history() {
    let directory = Directory::new();
    let path = directory.path();
    let persister = Persister::new(path.clone());
    persister.offer(snapshot(1));
    let selected = persister.select().unwrap().unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    let (done, completed) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        write_selected(selected, &path);
        // Another process publishes a newer file before the next attempt.
        std::fs::write(path.file(), format::encode(&snapshot(99)).unwrap()).unwrap();
        done.send(()).unwrap();
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), task)
            .await
            .is_err()
    );
    persister.offer(snapshot(2));
    assert_eq!(
        persister.attempt(deadline()).await,
        Err(PersistFailure::Busy)
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), completed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        persister.attempt(deadline()).await,
        Ok(Persisted::NewerOnDisk)
    );
    assert_eq!(
        format::decode(&std::fs::read(directory.path().file()).unwrap())
            .unwrap()
            .version,
        99
    );
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Idle));
}

#[tokio::test]
async fn dropped_selection_retains_the_latest_offer_for_retry() {
    let directory = Directory::new();
    let persister = Persister::new(directory.path());
    persister.offer(snapshot(1));
    let selected = persister.select().unwrap().unwrap();
    persister.offer(snapshot(2));
    drop(selected);
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Written));
    assert_eq!(
        format::decode(&std::fs::read(directory.path().file()).unwrap())
            .unwrap()
            .version,
        2
    );
}

#[tokio::test]
async fn failed_selection_is_retryable_and_the_first_write_may_replace_pre_restore_state() {
    let directory = Directory::new();
    let path = directory.path();
    std::fs::write(path.file(), format::encode(&snapshot(99)).unwrap()).unwrap();
    let persister = Persister::new(path);
    persister.offer(snapshot(1));
    let selected = persister.select().unwrap().unwrap();
    assert_eq!(
        selected.finish(Err(PersistFailure::Io)),
        Err(PersistFailure::Io)
    );
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Written));
    assert_eq!(
        format::decode(&std::fs::read(directory.path().file()).unwrap())
            .unwrap()
            .version,
        1
    );
}

fn on_disk(directory: &Directory) -> u64 {
    format::decode(&std::fs::read(directory.path().file()).unwrap())
        .unwrap()
        .version
}

fn lock_pause(persister: &Persister) -> MutexGuard<'_, Option<Pause>> {
    persister
        .pause
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Waits until no write holds admission.
async fn settled(persister: &Persister) {
    let until = Instant::now() + Duration::from_secs(5);
    while !matches!(lock(&persister.state).phase, Phase::Ready) {
        assert!(Instant::now() < until, "the abandoned write did not finish");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn an_unconfirmed_replacement_spends_the_first_write_so_its_retry_keeps_a_newer_file() {
    let directory = Directory::new();
    let path = directory.path();
    let persister = Persister::new(path.clone());
    persister.offer(snapshot(1));
    let selected = persister.select().unwrap().unwrap();
    // The rename replaced the file, then syncing the directory failed.
    let bytes = format::encode(&selected.file).unwrap();
    assert_eq!(
        write(&path, &bytes, selected.written, 1),
        Ok(Persisted::Written)
    );
    assert_eq!(
        selected.finish(Err(PersistFailure::NotDurable)),
        Err(PersistFailure::NotDurable)
    );
    // Another process publishes a newer file before the retry.
    std::fs::write(path.file(), format::encode(&snapshot(99)).unwrap()).unwrap();
    assert_eq!(
        persister.attempt(deadline()).await,
        Ok(Persisted::NewerOnDisk)
    );
    assert_eq!(on_disk(&directory), 99);
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Idle));
}

#[tokio::test]
async fn an_unconfirmed_replacement_stays_queued_and_is_retried_over_its_own_file() {
    let directory = Directory::new();
    let path = directory.path();
    let persister = Persister::new(path.clone());
    persister.offer(snapshot(1));
    let selected = persister.select().unwrap().unwrap();
    let bytes = format::encode(&selected.file).unwrap();
    assert_eq!(
        write(&path, &bytes, selected.written, 1),
        Ok(Persisted::Written)
    );
    assert_eq!(
        selected.finish(Err(PersistFailure::NotDurable)),
        Err(PersistFailure::NotDurable)
    );
    // Offering the same version again changes nothing; the retry is queued.
    persister.offer(snapshot(1));
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Written));
    assert_eq!(on_disk(&directory), 1);
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Idle));
}

#[tokio::test]
async fn a_write_the_deadline_abandons_through_attempt_holds_admission_and_records_its_outcome() {
    let directory = Directory::new();
    let path = directory.path();
    let (persister, release) = Persister::held(path.clone());
    persister.offer(snapshot(1));
    assert_eq!(
        persister
            .attempt(Instant::now() + Duration::from_millis(20))
            .await,
        Err(PersistFailure::Deadline)
    );
    persister.offer(snapshot(2));
    assert_eq!(
        persister.attempt(deadline()).await,
        Err(PersistFailure::Busy)
    );
    release.send(()).unwrap();
    settled(&persister).await;
    assert_eq!(on_disk(&directory), 1);
    // The abandoned write recorded version 1, so version 2 keeps a newer file
    // another process publishes before the next attempt.
    std::fs::write(path.file(), format::encode(&snapshot(99)).unwrap()).unwrap();
    assert_eq!(
        persister.attempt(deadline()).await,
        Ok(Persisted::NewerOnDisk)
    );
    assert_eq!(on_disk(&directory), 99);
    assert_eq!(persister.attempt(deadline()).await, Ok(Persisted::Idle));
}

#[tokio::test]
async fn an_attempt_during_an_in_flight_write_is_refused_before_it_can_hold_stale_history() {
    let directory = Directory::new();
    let path = directory.path();
    let (persister, release_first) = Persister::held(path.clone());
    persister.offer(snapshot(1));
    assert_eq!(
        persister
            .attempt(Instant::now() + Duration::from_millis(20))
            .await,
        Err(PersistFailure::Deadline)
    );
    persister.offer(snapshot(2));
    let (reached, resume) = persister.pause_after_selection();
    let second = persister.attempt(deadline());
    tokio::pin!(second);
    // Admission is part of selection, so the second attempt is refused at
    // once. Were admission later than selection, it would stop at the pause
    // holding the first write's old history.
    let early = tokio::select! {
        outcome = &mut second => Some(outcome),
        _ = reached => None,
    };
    release_first.send(()).unwrap();
    settled(&persister).await;
    // Another process publishes a newer file before the second attempt goes on.
    std::fs::write(path.file(), format::encode(&snapshot(99)).unwrap()).unwrap();
    let outcome = match early {
        Some(outcome) => {
            // The attempt never reached the pause; disarm it so the next
            // attempt does not stop there.
            drop(resume);
            drop(lock_pause(&persister).take());
            outcome
        }
        None => {
            resume.send(()).unwrap();
            bounded(second).await
        }
    };
    assert_eq!(outcome, Err(PersistFailure::Busy));
    assert_eq!(on_disk(&directory), 99);
    assert_eq!(
        bounded(persister.attempt(deadline())).await,
        Ok(Persisted::NewerOnDisk)
    );
    assert_eq!(on_disk(&directory), 99);
}

/// Fails rather than hangs if an attempt stops for good.
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("the attempt finished within five seconds")
}
