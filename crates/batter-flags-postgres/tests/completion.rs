//! Native completion evidence, wire acknowledgement loss, and bounded disposition.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "failure-contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_core::Toggle;
use batter_flags_postgres::*;
use fixture::*;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use std::{error::Error, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn write() -> Write {
    Write::explicit(
        key("change"),
        ExplicitValue::ModeToggle(Toggle::On),
        change(None),
    )
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn dropping_the_interrupt_sender_before_preparation_still_commits_once() {
    let fixture = Fixture::migrated().await;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    drop(sender);
    let outcome = fixture
        .store
        .submit(
            Request::Write(write()),
            WritePolicy::STRICT,
            deadline(),
            receiver,
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, WriteOutcome::WrittenUndeclared { version, .. } if version.get() == 1)
    );
    let loaded = fixture.store.load(deadline()).await.unwrap();
    assert_eq!(loaded.version().get(), 1);
    assert_eq!(loaded.rows().len(), 1);
    fixture.close().await;
}
fn sqlstate(mut error: &(dyn Error + 'static)) -> Option<String> {
    loop {
        if let Some(native) = error.downcast_ref::<sqlx::Error>() {
            return native
                .as_database_error()
                .and_then(|e| e.code())
                .map(|c| c.into_owned());
        }
        error = error.source()?;
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn deferred_notify_queue_sqlstate_is_known_failure_and_retires() {
    let f = Fixture::migrated().await;
    // Reproduce PostgreSQL's COMMIT ErrorResponse without filling a cluster-wide queue.
    sqlx::raw_sql("CREATE FUNCTION flags.simulate_queue_full() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION USING ERRCODE='54000', MESSAGE='private failure marker'; END $$; CREATE CONSTRAINT TRIGGER simulate_queue AFTER UPDATE ON flags.flag_control DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.simulate_queue_full();").execute(&f.pool).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let store = PooledStore::new(pool.clone(), timing());
    let error = store.write(write(), deadline()).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("54000"));
    for text in [
        format!("{error}"),
        format!("{error:?}"),
        format!("{error:#?}"),
    ] {
        assert!(!text.contains("private failure marker"));
    }
    assert_eq!(pool.size(), 0);
    assert_eq!(f.head().await, 0);
    assert!(
        f.store
            .history(None, batter_flags_core::Version::new(0), deadline())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    pool.close().await;
    f.close().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn deadline_during_commit_is_uncertain_and_retires() {
    let f = Fixture::migrated().await;
    sqlx::raw_sql("CREATE FUNCTION flags.slow_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(1); RETURN NULL; END $$; CREATE CONSTRAINT TRIGGER slow_commit AFTER UPDATE ON flags.flag_control DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.slow_commit();").execute(&f.pool).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let store = PooledStore::new(pool.clone(), timing());
    assert!(matches!(
        store
            .write(
                write(),
                tokio::time::Instant::now() + Duration::from_millis(200)
            )
            .await
            .unwrap(),
        WriteOutcome::Uncertain {
            cause: CommitUncertainty::Deadline,
            ..
        }
    ));
    assert_eq!(pool.size(), 0);
    pool.close().await;
    f.close().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn lost_commit_acknowledgement_is_uncertain_even_when_the_write_committed() {
    let f = Fixture::migrated().await;
    let target: PgConnectOptions = f.lease.database_url().parse().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = TcpStream::connect((target.get_host(), target.get_port()))
        .await
        .unwrap();
    let proxy = tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let (mut client_read, mut client_write) = client.into_split();
        let (mut server_read, mut server_write) = upstream.into_split();
        let outgoing = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
        });
        loop {
            let mut header = [0; 5];
            server_read.read_exact(&mut header).await.unwrap();
            let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
            assert!((4..=1_000_000).contains(&length));
            let mut payload = vec![0; length - 4];
            server_read.read_exact(&mut payload).await.unwrap();
            if header[0] == b'C' && payload == b"COMMIT\0" {
                // PostgreSQL has committed; intentionally suppress its acknowledgement.
                outgoing.abort();
                let _ = outgoing.await;
                drop(client_write);
                break;
            }
            client_write.write_all(&header).await.unwrap();
            client_write.write_all(&payload).await.unwrap();
        }
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            target
                .host("127.0.0.1")
                .port(address.port())
                .ssl_mode(PgSslMode::Disable),
        )
        .await
        .unwrap();
    let store = PooledStore::new(pool.clone(), timing());
    let outcome = store.write(write(), deadline()).await.unwrap();
    assert!(
        matches!(outcome,WriteOutcome::Uncertain { proposed_version, cause: CommitUncertainty::Native(_) } if proposed_version.get()==1)
    );
    tokio::time::timeout(Duration::from_secs(2), proxy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pool.size(), 0);
    assert_eq!(f.head().await, 1);
    pool.close().await;
    f.close().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn acknowledged_success_and_noop_use_batter_retirement_policy() {
    let f = Fixture::migrated().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let store = PooledStore::new(pool.clone(), timing());
    assert!(matches!(
        store.write(write(), deadline()).await.unwrap(),
        WriteOutcome::WrittenUndeclared { .. }
    ));
    assert_eq!(pool.size(), 0);
    assert!(matches!(
        store.write(write(), deadline()).await.unwrap(),
        WriteOutcome::Unchanged { .. }
    ));
    assert_eq!(pool.size(), 0);
    assert!(store.load(deadline()).await.is_ok());
    assert_eq!(pool.size(), 0);
    pool.close().await;
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn saturated_pool_acquisition_is_bounded_for_every_store_operation() {
    let f = Fixture::migrated().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let held = pool.acquire().await.unwrap();
    let limit = Duration::from_millis(75);
    let store = PooledStore::new(
        pool.clone(),
        StoreTiming::new(limit, limit, limit, limit).unwrap(),
    );
    let started = tokio::time::Instant::now();
    assert_eq!(
        store.write(write(), deadline()).await.unwrap_err().class(),
        ErrorClass::Deadline
    );
    assert_eq!(
        store.load(deadline()).await.unwrap_err().class(),
        ErrorClass::Deadline
    );
    assert_eq!(
        store
            .history(None, batter_flags_core::Version::new(0), deadline())
            .await
            .unwrap_err()
            .class(),
        ErrorClass::Deadline
    );
    assert_eq!(
        store.migrate(deadline()).await.unwrap_err().class(),
        ErrorClass::Deadline
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(f.head().await, 0);
    drop(held);
    pool.close().await;
    f.close().await;
}
