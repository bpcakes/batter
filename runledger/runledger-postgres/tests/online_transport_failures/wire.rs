#![allow(
    dead_code,
    reason = "shared relay targets select different fault scenarios"
)]

//! Narrow PostgreSQL relay for online-migration fault tests. All SQL executes
//! on the real fixture server; only response delivery or socket lifetime changes.

use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

#[derive(Clone, Copy)]
pub enum Fault {
    HoldPollResponse,
    DisconnectOnRestore,
    HoldIndexInspection(&'static str),
    HoldRepairTableLock,
    LoseRepairCommitResponse,
    DisconnectOnRepairRollback,
}

pub struct Relay {
    pub options: PgConnectOptions,
    reached: Arc<Notify>,
    release: Arc<Notify>,
    delivered: Arc<Notify>,
    task: JoinHandle<io::Result<()>>,
}

impl Relay {
    pub async fn start(options: &PgConnectOptions, fault: Fault) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("relay listener");
        let address = listener.local_addr().expect("relay address");
        let upstream = (options.get_host().to_owned(), options.get_port());
        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let delivered = Arc::new(Notify::new());
        let events = (reached.clone(), release.clone(), delivered.clone());
        let task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await?;
            let mut server = TcpStream::connect(upstream).await?;
            // SSL is disabled for this loopback-only fixture. Startup has no tag.
            let startup = body(&mut client).await?;
            write_body(&mut server, &startup).await?;
            let (mut from_client, mut to_client) = client.split();
            let (mut from_server, mut to_server) = server.split();
            let poll_started = AtomicBool::new(false);
            let repair_started = AtomicBool::new(false);
            let repair_commit = AtomicBool::new(false);
            let (reached, release, delivered) = events;
            let requests = async {
                loop {
                    let tag = from_client.read_u8().await?;
                    let bytes = body(&mut from_client).await?;
                    // Parse carries statement-name NUL then SQL NUL; a simple
                    // Query carries SQL NUL. Matching complete frames survives
                    // arbitrary TCP fragmentation without interpreting binds.
                    let sql = match tag {
                        b'P' => bytes.splitn(2, |b| *b == 0).nth(1).unwrap_or_default(),
                        b'Q' => &bytes,
                        _ => &[],
                    };
                    let sql = sql.split(|b| *b == 0).next().unwrap_or_default();
                    if let Fault::HoldIndexInspection(index) = fault
                        && tag == b'B'
                        && bytes
                            .windows(index.len())
                            .any(|part| part == index.as_bytes())
                    {
                        poll_started.store(true, Ordering::SeqCst);
                    }
                    if sql.starts_with(b"SET LOCAL lock_timeout = '100ms'") {
                        repair_started.store(true, Ordering::SeqCst);
                    }
                    if repair_started.load(Ordering::SeqCst) && sql.starts_with(b"COMMIT") {
                        repair_commit.store(true, Ordering::SeqCst);
                    }
                    match fault {
                        Fault::HoldRepairTableLock if sql.starts_with(b"LOCK TABLE ONLY ") => {
                            reached.notify_one();
                            release.notified().await;
                        }
                        Fault::DisconnectOnRepairRollback
                            if repair_started.load(Ordering::SeqCst)
                                && sql.starts_with(b"ROLLBACK") =>
                        {
                            reached.notify_one();
                            return Ok::<(), io::Error>(());
                        }
                        Fault::DisconnectOnRestore
                            if sql.starts_with(
                                b"SELECT pg_catalog.set_config('statement_timeout', $1",
                            ) =>
                        {
                            reached.notify_one();
                            return Ok::<(), io::Error>(());
                        }
                        Fault::HoldPollResponse
                            if sql.starts_with(b"SELECT pg_catalog.pg_try_advisory_lock(") =>
                        {
                            poll_started.store(true, Ordering::SeqCst)
                        }
                        _ => {}
                    }
                    to_server.write_u8(tag).await?;
                    write_body(&mut to_server, &bytes).await?;
                }
            };
            let responses = async {
                let mut held = false;
                loop {
                    let tag = from_server.read_u8().await?;
                    let bytes = body(&mut from_server).await?;
                    if matches!(fault, Fault::LoseRepairCommitResponse)
                        && repair_commit.load(Ordering::SeqCst)
                        && tag == b'C'
                    {
                        reached.notify_one();
                        return Ok::<(), io::Error>(());
                    }
                    let boundary = match fault {
                        Fault::HoldIndexInspection(_) => b'Z',
                        _ => b'D',
                    };
                    if !held && tag == boundary && poll_started.load(Ordering::SeqCst) {
                        held = true;
                        reached.notify_one();
                        release.notified().await;
                    }
                    to_client.write_u8(tag).await?;
                    write_body(&mut to_client, &bytes).await?;
                    if held && tag == b'Z' {
                        delivered.notify_one();
                    }
                }
            };
            // EOF or injected disconnection drops both directions, even while
            // the response gate is held. No backend is retained by the relay.
            let result = tokio::select! {
                result = requests => result,
                result = responses => result,
            };
            match result {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::BrokenPipe
                    ) =>
                {
                    Ok(())
                }
                result => result,
            }
        });
        Self {
            options: options
                .clone()
                .host("127.0.0.1")
                .port(address.port())
                .ssl_mode(PgSslMode::Disable),
            reached,
            release,
            delivered,
            task,
        }
    }

    pub async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(15), self.reached.notified())
            .await
            .expect("target query reaches fault boundary");
    }

    pub async fn release_response(&self) {
        self.release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), self.delivered.notified())
            .await
            .expect("complete successful response delivered");
    }

    pub async fn finish(&mut self) {
        tokio::time::timeout(Duration::from_secs(10), &mut self.task)
            .await
            .expect("relay ends with its connection")
            .expect("relay task")
            .expect("relay I/O");
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn body(input: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let length = input.read_u32().await?;
    if !(4..=16 * 1024 * 1024).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid fixture frame length",
        ));
    }
    let mut bytes = vec![0; (length - 4) as usize];
    input.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_body(
    output: &mut (impl tokio::io::AsyncWrite + Unpin),
    bytes: &[u8],
) -> io::Result<()> {
    output
        .write_u32(u32::try_from(bytes.len() + 4).expect("bounded fixture frame"))
        .await?;
    output.write_all(bytes).await
}
