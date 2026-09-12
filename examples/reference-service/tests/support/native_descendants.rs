use super::{
    ProbeResult,
    native_hosted::{config, enqueue, process, status},
};
use batter::{cleanup::SkipReason, operation::OperationContext};
use runledger_core::prelude::async_trait;
use runledger_runtime::{JobLifecycleObserver, JobSucceededEvent};
use sqlx::PgPool;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

struct HeldCallback {
    entered: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    release: Mutex<mpsc::Receiver<()>>,
}
struct Finished(Arc<AtomicBool>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl JobLifecycleObserver for HeldCallback {
    async fn on_job_succeeded(&self, _: JobSucceededEvent) {
        let _finished = Finished(self.finished.clone());
        self.entered.store(true, Ordering::SeqCst);
        // A finite safety bound complements the runner's process watchdog.
        // This deliberately non-yielding callback cannot be stopped by task abort.
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
    }
}

pub async fn callback_outlives_wrapper(pool: PgPool) -> ProbeResult {
    batter_example_reference_service::schema::initialize_schema(&pool).await?;
    let catalog = super::worker::catalog(None);
    catalog.sync_definitions(&pool).await?;
    let job = enqueue(&pool).await?;
    let entered = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let callback_entered = entered.clone();
    let callback_finished = finished.clone();
    let (release, receiver) = mpsc::channel();
    let mut process = process(Duration::from_millis(20), Duration::from_millis(20));
    process.on_cleanup("dependency", || async {
        panic!("unjoined native callback must prevent dependency cleanup");
        #[allow(unreachable_code)]
        Ok(())
    })?;
    let native_pool = pool.clone();
    batter_runledger::register(
        &mut process,
        "native",
        OperationContext::new(Duration::from_secs(2))?,
        {
            runledger_runtime::Supervisor::builder(&native_pool, config())?
                .with_catalog(catalog)
                .disable_scheduler()
                .disable_reaper()
                .disable_intent_promoter()
                .with_job_lifecycle_observer(HeldCallback {
                    entered: callback_entered,
                    finished: callback_finished,
                    release: Mutex::new(receiver),
                })
                .prepare()?
        },
    )?;
    let running = process.start();
    // Waking this fixture directly from the blocking callback can strand it in
    // that worker's non-stealable LIFO slot. Observe via an independent timer so
    // the controller runs while the deliberately non-yielding callback is held.
    let entry = observe(&entered, Duration::from_secs(5)).await;
    let held_at_stop = !finished.load(Ordering::SeqCst);
    running.handle().request();
    let observed = tokio::time::timeout(Duration::from_secs(3), running.wait()).await;
    let held_at_report = !finished.load(Ordering::SeqCst);
    // Every assertion/error path releases the real callback before fixture teardown.
    let _ = release.send(());
    let settled = observe(&finished, Duration::from_secs(3)).await;
    let report = match observed {
        Ok(report) => report?,
        Err(_) => {
            let _retained = running.wait().await?;
            return Err("native adapter exceeded bounded process settlement".into());
        }
    };
    entry?;
    settled?;
    assert!(
        held_at_stop && held_at_report,
        "the actual callback must still be held during shutdown observation"
    );
    assert!(!report.is_success());
    assert!(
        report
            .cleanup
            .skipped
            .iter()
            .any(|entry| entry.reason == SkipReason::UnsafeTaskExit)
    );
    let native = report
        .managed
        .iter()
        .find(|entry| entry.name == "native")
        .ok_or("managed native record was discarded")?
        .outcome
        .settlement
        .as_ref()
        .ok_or("native report was discarded")?
        .downcast_ref::<batter_runledger::NativeReport>()
        .ok_or("native report identity changed")?;
    assert!(
        native
            .native
            .unjoined
            .iter()
            .any(|task| task.task == "terminal_observer" && task.abort_requested)
    );
    assert!(!native.native.is_cooperatively_stopped());
    assert_eq!(status(&pool, job).await?, "SUCCEEDED");
    Ok(())
}

async fn observe(flag: &AtomicBool, bound: Duration) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(bound, async {
        while !flag.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
}
