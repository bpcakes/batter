//! Drop an actual COMMIT acknowledgement after PostgreSQL reports success.
use super::fixture::*;
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobEnqueueIntent, JobEnqueueIntentDisposition, JobScope};
use runledger_postgres::{PgAtomicError, PgAtomicUncertainty, RunledgerDatabase, run_atomic};
use sqlx::postgres::{PgPoolOptions, PgSslMode};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
async fn unconfirmed_commit_recovers_the_same_intent_without_duplicate_submission() {
    let f = Fixture::new("reconcile_commit").await;
    let source = f.service.pool().connect_options();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("complete fixture database operation");
    let port = listener
        .local_addr()
        .expect("proxy listener address")
        .port();
    let upstream = (source.get_host().to_owned(), source.get_port());
    let armed = Arc::new(AtomicBool::new(false));
    let guard = Arc::clone(&armed);
    // One lazy pool connection; no preflight connection or detached proxy tasks.
    let proxy = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (client, _) = listener.accept().await?;
            let server = TcpStream::connect(upstream).await?;
            let (mut client_read, mut client_write) = client.into_split();
            let (mut server_read, mut server_write) = server.into_split();
            let request = tokio::io::copy(&mut client_read, &mut server_write);
            let response = async {
                loop {
                    let tag = server_read.read_u8().await?;
                    let size = server_read.read_u32().await?;
                    if !(4..=1024*1024).contains(&size) { return Err(std::io::Error::other("invalid fixture frame size")); }
                    let mut body = vec![0; (size-4) as usize];
                    server_read.read_exact(&mut body).await?;
                    if guard.load(Ordering::Acquire) && tag == b'C' && body == b"COMMIT\0" {
                        return Ok::<_, std::io::Error>(true);
                    }
                    client_write.write_u8(tag).await?;
                    client_write.write_u32(size).await?;
                    client_write.write_all(&body).await?;
                }
            };
            tokio::select! { result = response => result, result = request => result.map(|_| false) }
        }).await.expect("bounded commit proxy")
    });
    let proxied = RunledgerDatabase::connect_lazy(
        (*source)
            .clone()
            .host("127.0.0.1")
            .port(port)
            .ssl_mode(PgSslMode::Disable),
        f.service.profile().clone(),
        PgPoolOptions::new().max_connections(1).min_connections(0),
    )
    .expect("construct validated fixture input");
    let payload = serde_json::json!({"value": 1});
    let request = JobEnqueueIntent::new(JobType::new(TYPE), &payload, "lost-ack");
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        run_atomic(&proxied, async |mut scope| {
            let result = scope.record_required_job_enqueue_intent(&request).await;
            assert!(result.is_ok());
            armed.store(true, Ordering::Release);
            result
        }),
    )
    .await
    .expect("bounded submission");
    assert!(
        proxy.await.expect("join commit proxy").expect("proxy I/O"),
        "proxy must observe and discard the server COMMIT acknowledgement"
    );
    let provisional = match result {
        Err(PgAtomicError::Uncertain(PgAtomicUncertainty::CommitUnconfirmed {
            output, ..
        })) => output,
        other => panic!("expected unconfirmed commit, got {other:?}"),
    };
    proxied.pool().close().await;
    let observed = retained(read(&f.service, JobScope::Global, provisional.intent_id()).await);
    assert_eq!(observed.id(), provisional.intent_id());
    let recovered = run_atomic(&f.service, async |mut scope| {
        scope.record_required_job_enqueue_intent(&request).await
    })
    .await
    .expect("complete fixture database operation");
    assert_eq!(recovered.intent_id(), observed.id());
    assert_eq!(
        recovered.disposition(),
        JobEnqueueIntentDisposition::Existing
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_enqueue_intents WHERE idempotency_key='lost-ack'",
    )
    .fetch_one(&f.pool)
    .await
    .expect("complete fixture database operation");
    assert_eq!(count, 1);
    f.finish().await;
}
