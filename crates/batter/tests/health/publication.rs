use super::*;
use std::sync::{atomic::AtomicBool, mpsc};

struct ReadDuringDrop {
    reader: Arc<Mutex<Option<HealthReader<ReadDuringDrop>>>>,
    read_completed: Arc<AtomicBool>,
    threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}
impl Drop for ReadDuringDrop {
    fn drop(&mut self) {
        let reader = self.reader.lock().unwrap().take().unwrap();
        let (sent, received) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let _ = sent.send(reader.is_healthy());
        });
        self.threads.lock().unwrap().push(thread);
        // Bound the negative control: if publication holds its mutex across
        // this destructor, the read times out, then proceeds once Drop returns.
        // Join outside publication below; no blocked thread is abandoned.
        self.read_completed.store(
            received.recv_timeout(Duration::from_secs(1)) == Ok(true),
            Ordering::SeqCst,
        );
    }
}

#[tokio::test(start_paused = true)]
async fn replacing_a_failure_allows_reads_while_its_application_destructor_runs() {
    let reader_slot = Arc::new(Mutex::new(None));
    let read_completed = Arc::new(AtomicBool::new(false));
    let threads = Arc::new(Mutex::new(Vec::new()));
    let first = ReadDuringDrop {
        reader: reader_slot.clone(),
        read_completed: read_completed.clone(),
        threads: threads.clone(),
    };
    let mut error = Some(first);
    let monitor = HealthMonitor::new(policy(), move || {
        let result = error.take().map_or(Ok(()), Err);
        async move { result }
    });
    let reader = monitor.reader();
    *reader_slot.lock().unwrap() = Some(reader.clone());
    let handle = ShutdownHandle::new();
    let mut run = Box::pin(monitor.run(handle.signal()));
    assert!(poll_once(run.as_mut()).await.is_pending());
    assert_eq!(reader.snapshot().status(), HealthStatus::Failed);
    advance(policy().probe_delay()).await;
    assert!(poll_once(run.as_mut()).await.is_pending());
    let joins = std::mem::take(&mut *threads.lock().unwrap());
    assert_eq!(joins.len(), 1);
    for thread in joins {
        thread.join().unwrap();
    }
    assert!(
        read_completed.load(Ordering::SeqCst),
        "application destructor blocked a health reader"
    );
    handle.request();
    run.await;
}
