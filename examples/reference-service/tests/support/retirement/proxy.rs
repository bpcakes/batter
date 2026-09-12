use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

/// Test-only transport fault: forward definition-disable commit, then discard
/// the cancellation COMMIT acknowledgement after PostgreSQL has sent it.
pub(super) struct CommitAckProxy {
    pub port: u16,
    stop: oneshot::Sender<()>,
    task: JoinHandle<io::Result<bool>>,
}

impl CommitAckProxy {
    pub async fn start(host: String, port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let proxy_port = listener.local_addr()?.port();
        let (stop, mut stopping) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (client, _) = tokio::select! {
                _ = &mut stopping => return Ok(false),
                result = listener.accept() => result?,
            };
            drop(listener);
            let server = TcpStream::connect((host.as_str(), port)).await?;
            let (mut client_read, mut client_write) = client.into_split();
            let (mut server_read, mut server_write) = server.into_split();
            // Both copying futures are owned here. Returning drops them and all
            // four socket halves; there are no detached relay descendants.
            tokio::select! {
                _ = &mut stopping => Ok(false),
                result = tokio::io::copy(&mut client_read, &mut server_write) => result.map(|_| false),
                result = discard_second_commit(&mut server_read, &mut client_write) => result,
            }
        });
        Ok(Self {
            port: proxy_port,
            stop,
            task,
        })
    }

    pub async fn finish(self) -> Result<bool, batter::BoxError> {
        let _ = self.stop.send(());
        Ok(self.task.await??)
    }
}

async fn discard_second_commit(
    server: &mut tokio::net::tcp::OwnedReadHalf,
    client: &mut tokio::net::tcp::OwnedWriteHalf,
) -> io::Result<bool> {
    let mut commits = 0;
    loop {
        let tag = server.read_u8().await?;
        let length = server.read_u32().await?;
        if !(4..=16_777_216).contains(&length) {
            return Err(io::Error::other("unexpected test PostgreSQL frame length"));
        }
        let mut payload = vec![0; (length - 4) as usize];
        server.read_exact(&mut payload).await?;
        if tag == b'C' && payload == b"COMMIT\0" {
            commits += 1;
            if commits == 2 {
                return Ok(true);
            }
        }
        client.write_u8(tag).await?;
        client.write_u32(length).await?;
        client.write_all(&payload).await?;
    }
}
