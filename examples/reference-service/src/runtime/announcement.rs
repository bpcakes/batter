//! Optional application-owned discovery, inside the existing startup deadline.
//! A datagram is a bind acknowledgement, not a readiness acknowledgement.

use std::{io, net::SocketAddr, path::Path};
use tokio::net::UnixDatagram;

pub(super) async fn publish(path: Option<&Path>, address: SocketAddr) -> io::Result<()> {
    let Some(path) = path else { return Ok(()) };
    let socket = UnixDatagram::unbound()?;
    let message = address.to_string();
    let written = socket.send_to(message.as_bytes(), path).await?;
    if written != message.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "incomplete listener announcement",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Duration};

    struct Receiver {
        socket: UnixDatagram,
        path: PathBuf,
    }

    impl Receiver {
        fn new() -> io::Result<Self> {
            // Short absolute path works with both Linux and macOS sockaddr_un.
            let path = PathBuf::from(format!("/tmp/batter-announce-{}", uuid::Uuid::now_v7()));
            let socket = UnixDatagram::bind(&path)?;
            Ok(Self { socket, path })
        }
    }

    impl Drop for Receiver {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[tokio::test]
    async fn optional_announcement_uses_only_the_explicit_socket() -> io::Result<()> {
        let receiver = Receiver::new()?;
        let address = "127.0.0.1:43210".parse().unwrap();
        publish(None, address).await?;
        let mut message = [0; 128];
        assert_eq!(
            receiver.socket.try_recv(&mut message).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        publish(Some(&receiver.path), address).await?;
        let read = receiver.socket.recv(&mut message).await?;
        assert_eq!(&message[..read], address.to_string().as_bytes());
        let missing = receiver.path.with_extension("missing");
        assert!(publish(Some(&missing), address).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn unread_receiver_cannot_hold_startup_past_its_deadline() -> io::Result<()> {
        let receiver = Receiver::new()?;
        let sender = std::os::unix::net::UnixDatagram::unbound()?;
        sender.set_nonblocking(true)?;
        let mut full = false;
        for _ in 0..100_000 {
            match sender.send_to(b"127.0.0.1:43210", &receiver.path) {
                Ok(_) => {}
                Err(error) if saturated(&error) => {
                    full = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        assert!(full, "the negative control must saturate the receiver");
        let address = "127.0.0.1:43210".parse().unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            publish(Some(&receiver.path), address),
        )
        .await;
        // Linux reports backpressure as WouldBlock; macOS can report ENOBUFS
        // immediately. Both must settle without an uncancellable writer.
        assert!(match result {
            Err(_) => true,
            Ok(Err(error)) => saturated(&error),
            Ok(Ok(())) => false,
        });
        // Cancellation leaves no blocking writer or spawned work. Draining the
        // socket cannot trigger a late publication from the dropped future.
        let mut message = [0; 128];
        while receiver.socket.try_recv(&mut message).is_ok() {}
        publish(Some(&receiver.path), address).await?;
        receiver.socket.recv(&mut message).await?;
        assert_eq!(
            receiver.socket.try_recv(&mut message).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        Ok(())
    }

    fn saturated(error: &io::Error) -> bool {
        error.kind() == io::ErrorKind::WouldBlock
            || (cfg!(target_os = "macos") && error.raw_os_error() == Some(55))
    }
}
