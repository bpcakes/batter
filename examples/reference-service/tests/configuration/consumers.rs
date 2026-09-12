use crate::load;
use batter::{
    admission::{Admission, AdmissionError},
    cleanup::CleanupBudget,
    lifecycle::{ProcessAdmissionError, ShutdownBudget, ShutdownHandle},
    operation::OperationContext,
};
use std::{convert::Infallible, time::Duration};

fn budget() -> ShutdownBudget {
    let second = Duration::from_secs(1);
    ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn configured_request_policy_changes_actual_response_deadline() {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use tower::ServiceExt;
    for (millis, expected) in [
        ("5", StatusCode::SERVICE_UNAVAILABLE),
        ("50", StatusCode::OK),
    ] {
        let root = load(&[("BATTER_REQUEST_TIMEOUT_MS", millis)]).unwrap();
        let handle = ShutdownHandle::new();
        handle.mark_ready();
        let app = Router::new()
            .route(
                "/held",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    "done"
                }),
            )
            .layer(middleware::from_fn_with_state(
                root.request_policy(handle).unwrap(),
                batter_axum::request_admission,
            ));
        let response = app
            .oneshot(Request::builder().uri("/held").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn bulkhead_and_process_capacities_change_independent_native_admission() {
    for (bulk_limit, process_limit) in [(1, 3), (3, 1)] {
        let root = load(&[
            ("BATTER_BULKHEAD_CAPACITY", &bulk_limit.to_string()),
            ("BATTER_PROCESS_CAPACITY", &process_limit.to_string()),
        ])
        .unwrap();
        let bulkhead = root.bulkhead().unwrap();
        let context = OperationContext::new(Duration::from_secs(10)).unwrap();
        let mut held = Vec::new();
        for _ in 0..bulk_limit {
            held.push(bulkhead.enter(&context, Admission::Reject).await.unwrap());
        }
        assert!(matches!(
            bulkhead.enter(&context, Admission::Reject).await,
            Err(AdmissionError::Overloaded)
        ));
        let mut supervisor = root.supervisor(budget()).unwrap();
        supervisor
            .register("initialized", |signal| async move {
                signal.mark_started();
                signal.draining().await;
                Ok(())
            })
            .unwrap();
        let process = supervisor.process_handle().unwrap();
        let handle = supervisor.handle();
        handle.mark_ready();
        let running = supervisor.start();
        handle.wait_ready().await.unwrap();
        let mut finishes = Vec::new();
        let mut receipts = Vec::new();
        for _ in 0..process_limit {
            let (send, receive) = tokio::sync::oneshot::channel();
            finishes.push(send);
            receipts.push(
                process
                    .try_spawn("held", |_| async move {
                        receive.await.unwrap();
                        Ok::<_, Infallible>(())
                    })
                    .unwrap(),
            );
        }
        assert!(matches!(
            process.try_spawn("excess", |_| async { Ok::<_, Infallible>(()) }),
            Err(ProcessAdmissionError::Full)
        ));
        // Capacity remains separate: freeing one bulkhead permit cannot admit a finite task.
        held.pop();
        let permit = bulkhead.enter(&context, Admission::Reject).await.unwrap();
        assert!(matches!(
            process.try_spawn("still-full", |_| async { Ok::<_, Infallible>(()) }),
            Err(ProcessAdmissionError::Full)
        ));
        drop(permit);
        for finish in finishes {
            finish.send(()).unwrap();
        }
        for receipt in receipts {
            receipt.wait().await.unwrap();
        }
        handle.request();
        let report = running.wait().await.unwrap();
        assert!(report.is_success());
        assert_eq!(report.completed_process_tasks, process_limit);
    }
}

#[test]
fn managed_worker_preparation_consumes_validated_section() {
    super::process::native("native-worker");
}

pub(crate) async fn native_worker() {
    let root = load(&[
        ("JOBS_WORKER_ID", "configured-worker"),
        ("JOBS_MAX_GLOBAL_CONCURRENCY", "2"),
    ])
    .unwrap();
    let pool = root
        .pool_options()
        .connect_lazy_with(root.connect_options_from_process().unwrap());
    pool.close().await;
    // Local native initialization needs no database. Settings supply data; the
    // adapter owns launch and settlement through the process lifecycle.
    let catalog = runledger_runtime::catalog::JobCatalog::new();
    let config = root.worker().jobs_config().unwrap();
    assert_eq!(config.worker_id, "configured-worker");
    assert_eq!(config.max_global_concurrency, 2);
    let prepared = runledger_runtime::Supervisor::builder(&pool, config)
        .unwrap()
        .with_catalog(&catalog)
        .disable_scheduler()
        .disable_reaper()
        .prepare()
        .unwrap();
    let mut process = root.supervisor(budget()).unwrap();
    batter_runledger::register(
        &mut process,
        "worker",
        OperationContext::new(Duration::from_secs(3)).unwrap(),
        prepared,
    )
    .unwrap();
    let running = process.start();
    running.handle().mark_ready();
    let initialized =
        tokio::time::timeout(Duration::from_secs(3), running.handle().wait_ready()).await;
    let report = running.shutdown().await.unwrap();
    assert!(matches!(initialized, Ok(Ok(()))));
    assert!(report.is_success(), "{report}");
}
