//! Private Unix fixture directories. Existing paths are never reused.
use std::{
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct TempDir(Option<PathBuf>);

impl TempDir {
    pub fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "batter-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match Self::create(path) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                result => return result,
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "fixture directory collisions",
        ))
    }

    pub fn create(path: PathBuf) -> io::Result<Self> {
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(Some(path)))
    }

    pub fn path(&self) -> &Path {
        self.0.as_deref().expect("open fixture directory")
    }

    pub fn close(mut self) -> io::Result<()> {
        fs::remove_dir_all(self.0.take().expect("open fixture directory"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Panic-path fallback only; normal test completion explicitly checks close.
        // An already closed directory is absent. Never mask the original panic.
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}
