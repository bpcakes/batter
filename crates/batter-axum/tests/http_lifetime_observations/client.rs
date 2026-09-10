use super::resource::Events;
use std::net::{Shutdown, SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

pub struct Client {
    pub socket: TcpStream,
    pub bytes: Vec<u8>,
    events: Events,
}
impl Client {
    pub async fn connect(address: SocketAddr, events: Events) -> Self {
        Self {
            socket: TcpStream::connect(address).await.unwrap(),
            bytes: Vec::new(),
            events,
        }
    }
    pub async fn send(&mut self, request: &[u8]) {
        self.socket.write_all(request).await.unwrap();
    }
    pub async fn get(&mut self, path: &str) {
        self.send(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await;
    }
    pub async fn through(&mut self, marker: &[u8]) {
        let mut pending = PendingRead::new(
            self,
            format!("wire marker: {:?}", String::from_utf8_lossy(marker)),
        );
        while !pending
            .client
            .bytes
            .windows(marker.len())
            .any(|w| w == marker)
        {
            let client = &mut pending.client;
            assert_ne!(
                client.socket.read_buf(&mut client.bytes).await.unwrap(),
                0,
                "EOF before {marker:?}: {:?}",
                client.bytes
            );
        }
        pending.complete = true;
    }

    pub async fn headers(&mut self, status: u16) {
        self.through(b"\r\n\r\n").await;
        assert!(
            self.text().starts_with(&format!("HTTP/1.1 {status} ")),
            "{}",
            self.text()
        );
        self.events.record("headers");
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    pub async fn eof(&mut self) {
        let mut pending = PendingRead::new(self, "socket EOF".into());
        let client = &mut pending.client;
        client.socket.read_to_end(&mut client.bytes).await.unwrap();
        pending.complete = true;
        drop(pending);
        self.events.record("socket-eof");
    }
    pub async fn assert_pending(&mut self) {
        let mut byte = [0];
        assert!(
            timeout(
                crate::http_graceful::PENDING_WINDOW,
                self.socket.read(&mut byte)
            )
            .await
            .is_err(),
            "expected pending wire while handler/body release is withheld"
        );
    }
    pub async fn transport_close(&mut self) {
        // A native close racing an unread second request can reset the stream.
        let mut pending = PendingRead::new(self, "transport close".into());
        let client = &mut pending.client;
        let result = client.socket.read_to_end(&mut client.bytes).await;
        pending.complete = true;
        drop(pending);
        if let Err(error) = &result {
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                ),
                "{error}"
            );
        }
        self.events.record(if result.is_ok() {
            "socket-eof"
        } else {
            "socket-error"
        });
        if !self.bytes.is_empty() {
            assert!(self.text().starts_with("HTTP/1.1 503 "), "{}", self.text());
        }
    }
    pub fn close(self) {
        let socket = self.socket.into_std().unwrap();
        socket.shutdown(Shutdown::Both).unwrap();
        drop(socket);
        self.events.record("client-full-close");
    }
    pub async fn half_close(&mut self) {
        self.socket.shutdown().await.unwrap();
        self.events.record("client-write-half-close");
    }
    pub fn assert_fixed_complete(&self) {
        let split = self
            .bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&self.bytes[..split]).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(self.bytes.len() - split, length, "{}", self.text());
        self.events.record("message-complete");
    }
    pub fn assert_chunk_complete(&self) {
        assert!(
            self.text()
                .to_ascii_lowercase()
                .contains("transfer-encoding: chunked")
        );
        assert!(
            self.bytes.ends_with(b"5\r\nfirst\r\n0\r\n\r\n"),
            "{}",
            self.text()
        );
        self.events.record("message-complete");
    }
}

// Borrow the actual buffer so cancellation retains bytes read before the deadline.
struct PendingRead<'a> {
    client: &'a mut Client,
    label: String,
    complete: bool,
}
impl<'a> PendingRead<'a> {
    fn new(client: &'a mut Client, label: String) -> Self {
        use std::io::Write;
        let _ = writeln!(std::io::stderr().lock(), "lifetime wait: {label}");
        Self {
            client,
            label,
            complete: false,
        }
    }
}
impl Drop for PendingRead<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.client.events.interrupted(format!(
                "{}; wire: {:?}",
                self.label,
                self.client.text()
            ));
        }
    }
}
