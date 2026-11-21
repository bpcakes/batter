//! A real server rejection followed by a missing savepoint recovery acknowledgement.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "failure-contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_core::{Toggle, Version};
use batter_flags_postgres::*;
use fixture::*;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use std::{error::Error, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

enum Relay {
    Forwarding,
    RejectionObserved,
}
enum Operation {
    Write,
    Load,
    History,
}

struct RecoveryProxy {
    pool: sqlx::PgPool,
    observed: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl RecoveryProxy {
    async fn connect(target: PgConnectOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = TcpStream::connect((target.get_host(), target.get_port()))
            .await
            .unwrap();
        let (suppressed, observed) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let proxy = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            let (mut client_read, mut client_write) = client.into_split();
            let (mut server_read, mut server_write) = upstream.into_split();
            let outgoing = tokio::spawn(async move {
                let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
            });
            let mut state = Relay::Forwarding;
            loop {
                let mut header = [0; 5];
                server_read.read_exact(&mut header).await.unwrap();
                let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
                assert!((4..=1_000_000).contains(&length));
                let mut payload = vec![0; length - 4];
                server_read.read_exact(&mut payload).await.unwrap();
                match (&state, header[0], payload.as_slice()) {
                    (Relay::RejectionObserved, b'C', b"ROLLBACK\0") => {
                        // The database acknowledged savepoint recovery; the owner
                        // cannot know it because this wire response is withheld.
                        suppressed.send(()).unwrap();
                        let _ = released.await;
                        break;
                    }
                    (Relay::Forwarding, b'E', _) => state = Relay::RejectionObserved,
                    (Relay::Forwarding | Relay::RejectionObserved, _, _) => {}
                }
                client_write.write_all(&header).await.unwrap();
                client_write.write_all(&payload).await.unwrap();
            }
            outgoing.abort();
            let _ = outgoing.await;
        });
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(
                target
                    .username("bounded_reader")
                    .host("127.0.0.1")
                    .port(address.port())
                    .ssl_mode(PgSslMode::Disable),
            )
            .await
            .unwrap();
        Self {
            pool,
            observed,
            release,
            task: proxy,
        }
    }
    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(1), self.observed)
            .await
            .unwrap()
            .unwrap();
        self.release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), self.task)
            .await
            .unwrap()
            .unwrap();
        self.pool.close().await;
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn observed_rejections_survive_write_load_and_history_recovery_deadlines() {
    for operation in [Operation::Write, Operation::Load, Operation::History] {
        let f = Fixture::migrated().await;
        let login = f.writer("bounded_reader").await;
        login.close().await;
        sqlx::raw_sql("REVOKE ALL ON flags.flag_control, flags.flag_audit FROM bounded_reader")
            .execute(&f.pool)
            .await
            .unwrap();
        let proxy = RecoveryProxy::connect(f.lease.database_url().parse().unwrap()).await;
        let store = PooledStore::new(proxy.pool.clone(), timing());
        let budget = tokio::time::Instant::now() + Duration::from_secs(1);
        let error = match operation {
            Operation::Write => store
                .write(
                    Write::explicit(
                        key("value"),
                        ExplicitValue::ModeToggle(Toggle::On),
                        change(None),
                    ),
                    budget,
                )
                .await
                .unwrap_err(),
            Operation::Load => store.load(budget).await.unwrap_err(),
            Operation::History => store
                .history(None, Version::new(0), budget)
                .await
                .unwrap_err(),
        };
        let retained = error
            .source()
            .unwrap()
            .downcast_ref::<RollbackFailure>()
            .unwrap();
        assert_eq!(retained.cleanup().class(), ErrorClass::Deadline);
        let native = retained
            .operation()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<sqlx::Error>()
            .unwrap();
        assert_eq!(
            native.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
        for text in [
            format!("{error}"),
            format!("{error:?}"),
            format!("{error:#?}"),
        ] {
            assert!(!text.contains("permission denied"));
            assert!(!text.contains("bounded_reader"));
        }
        assert_eq!(proxy.pool.size(), 0);
        proxy.finish().await;
        assert_eq!(f.head().await, 0);
        f.drop_role("bounded_reader").await;
        f.close().await;
    }
}
