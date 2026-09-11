use std::time::Duration;

use batter_sqlx::test_support::{ConnectionPlan, FixtureError, FixtureSuite};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::oneshot;

use super::{
    ProbeResult,
    fixture_run::{present, wait_absent},
};

const BOUND: Duration = Duration::from_secs(5);

pub async fn waiter_loss() -> ProbeResult {
    let harness = super::harness().await?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let (send, receive) = oneshot::channel();
    let run = FixtureSuite::new(harness.clone()).start(move |scope| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
            let db = scope.empty(&plan).await?;
            let connection = db.pools()[0]
                .acquire()
                .await
                .map_err(FixtureError::Observe)?;
            send.send((
                connection,
                db.pools()[0].clone(),
                db.database_name().to_owned(),
            ))
            .unwrap_or_else(|_| panic!("observer alive"));
            Err::<(), _>(FixtureError::ObservationTimeout)
        })
    });
    let (connection, pool, name) = tokio::time::timeout(BOUND, receive).await??;
    tokio::time::timeout(BOUND, pool.close_event()).await?;
    drop(run); // Only report observation is lost. The cleanup driver is live.
    let pending = tokio::time::timeout(Duration::from_millis(40), pool.close())
        .await
        .is_err();
    let retained = present(&admin, &name).await;
    drop(connection);
    tokio::time::timeout(BOUND, pool.close()).await?;
    let removed = wait_absent(&admin, &name).await;
    let drained = harness.drain_deferred_cleanup().await;
    admin.close().await;
    removed?;
    drained?;
    assert!(pending && retained?);
    Ok(())
}

pub async fn runtime_loss() -> ProbeResult {
    let harness = super::harness().await?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let owned_harness = harness.clone();
    let (send_run, receive_run) = oneshot::channel();
    let (send_resource, receive_resource) = oneshot::channel();
    let (stop, stopped) = oneshot::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let run = FixtureSuite::new(owned_harness).start(move |scope| {
                Box::pin(async move {
                    let plan =
                        ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                    let db = scope.empty(&plan).await?;
                    let connection = db.pools()[0]
                        .acquire()
                        .await
                        .map_err(FixtureError::Observe)?;
                    send_resource
                        .send((
                            connection,
                            db.pools()[0].clone(),
                            db.database_name().to_owned(),
                        ))
                        .unwrap_or_else(|_| panic!("observer alive"));
                    Ok::<_, FixtureError>(())
                })
            });
            send_run
                .send(run)
                .unwrap_or_else(|_| panic!("waiter receiver alive"));
            let _ = stopped.await;
        });
        // Actual runtime destruction cancels the cleanup driver and drops its
        // captured native lease. Upstream Drop can queue destructive cleanup.
    });
    let mut run = tokio::time::timeout(BOUND, receive_run).await??;
    let (connection, pool, name) = tokio::time::timeout(BOUND, receive_resource).await??;
    tokio::time::timeout(BOUND, pool.close_event()).await?;
    let pending = run
        .wait_for(Duration::from_millis(40))
        .await
        .expect("fixture driver joined")
        .is_none();
    let retained = present(&admin, &name).await;
    stop.send(()).expect("runtime owner alive");
    tokio::task::spawn_blocking(move || thread.join().expect("runtime thread joined")).await?;
    let cancelled = tokio::time::timeout(BOUND, run.wait())
        .await?
        .is_err_and(|error| error.is_cancelled());
    let repeated = run.wait().await.is_err_and(|error| error.is_cancelled());
    let drained = harness.drain_deferred_cleanup().await;
    // Keep the checkout alive: database disappearance comes from native lease
    // Drop, not successful Pool::close or the fixture driver's finish path.
    let removed_while_held = wait_absent(&admin, &name).await;
    drop(connection);
    admin.close().await;
    drained?;
    removed_while_held?;
    assert!(pending && retained? && cancelled && repeated);
    Ok(())
}
