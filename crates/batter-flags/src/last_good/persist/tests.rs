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
