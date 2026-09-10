use super::state::State;
use std::{io, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub struct Client {
    stream: TcpStream,
    pub state: State,
}

pub struct Head {
    pub status: u16,
    chunked: bool,
    length: Option<usize>,
}

impl Client {
    pub async fn connect(address: SocketAddr, state: State) -> io::Result<Self> {
        Ok(Self {
            stream: TcpStream::connect(address).await?,
            state,
        })
    }

    pub async fn send(&mut self, path: &str) -> io::Result<()> {
        self.raw(&format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"))
            .await
    }

    pub async fn raw(&mut self, request: &str) -> io::Result<()> {
        self.stream.write_all(request.as_bytes()).await
    }

    async fn line(&mut self) -> io::Result<String> {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n") {
            bytes.push(self.stream.read_u8().await?);
            if bytes.len() > 8192 {
                return Err(io::Error::other("HTTP line exceeded fixture bound"));
            }
        }
        String::from_utf8(bytes).map_err(io::Error::other)
    }

    pub async fn head(&mut self) -> io::Result<Head> {
        self.state.record("waiting-http-head");
        let status = self
            .line()
            .await?
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| io::Error::other("missing HTTP status"))?
            .parse()
            .map_err(io::Error::other)?;
        let mut head = Head {
            status,
            chunked: false,
            length: None,
        };
        loop {
            let line = self.line().await?.to_ascii_lowercase();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("content-length:") {
                head.length = Some(value.trim().parse().map_err(io::Error::other)?);
            }
            if line.trim() == "transfer-encoding: chunked" {
                head.chunked = true;
            }
        }
        self.state.record("headers-received");
        Ok(head)
    }

    pub async fn body(&mut self, head: &Head) -> io::Result<Vec<u8>> {
        self.state.record("waiting-http-body");
        let mut body = Vec::new();
        if head.chunked {
            loop {
                let line = self.line().await?;
                let size = usize::from_str_radix(line.trim(), 16).map_err(io::Error::other)?;
                if size == 0 {
                    assert_eq!(self.line().await?, "\r\n");
                    break;
                }
                if size > 65536 {
                    return Err(io::Error::other("chunk exceeded fixture bound"));
                }
                let start = body.len();
                body.resize(start + size, 0);
                self.stream.read_exact(&mut body[start..]).await?;
                assert_eq!(self.line().await?, "\r\n");
            }
        } else {
            let length = head
                .length
                .ok_or_else(|| io::Error::other("missing HTTP framing"))?;
            if length > 65536 {
                return Err(io::Error::other("body exceeded fixture bound"));
            }
            body.resize(length, 0);
            self.stream.read_exact(&mut body).await?;
        }
        self.state.record("message-complete");
        Ok(body)
    }

    pub async fn response(&mut self) -> io::Result<(u16, Vec<u8>)> {
        let head = self.head().await?;
        let body = self.body(&head).await?;
        Ok((head.status, body))
    }

    pub async fn eof(&mut self) -> io::Result<()> {
        self.state.record("waiting-socket-close");
        let mut byte = [0];
        match self.stream.read(&mut byte).await {
            Ok(0) => self.state.record("socket-eof"),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                ) =>
            {
                self.state.record("socket-error")
            }
            Ok(_) => return Err(io::Error::other("unexpected data before socket closure")),
            Err(error) => return Err(error),
        }
        self.state.record("socket-closed");
        Ok(())
    }

    pub fn disconnect(self) -> io::Result<()> {
        // SHUT_RDWR, not AsyncWriteExt::shutdown (which only closes the write half).
        let socket = self.stream.into_std()?;
        socket.shutdown(std::net::Shutdown::Both)?;
        drop(socket);
        self.state.record("client-full-close");
        Ok(())
    }
}
