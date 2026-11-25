//! An upgraded connection's concrete I/O type. The registered server wraps
//! the listener's I/O, so an application that recovers its socket by downcast
//! names the wrapper and unwraps it, keeping the bytes Hyper already read.

use super::*;
use axum::{
    extract::Request,
    http::{StatusCode, header},
    response::IntoResponse,
    serve::Listener,
};
use batter_axum::low_level::ForcedCloseIo;
use hyper_util::{rt::TokioIo, server::conn::auto::upgrade::downcast};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};

const HEAD: &[u8] =
    b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: raw\r\n\r\n";
/// The upgraded protocol's first bytes, which the peer may send straight
/// after the request head.
const EARLY: &[u8] = b"early;";

/// What the application's own task found at the downcast: what the wrapped
/// I/O reports before it is recovered and the bytes Hyper had already read
/// past the request head, or why it recovered nothing.
type Recovered<T> = Result<(T, Bytes), &'static str>;

/// Answer 101 and hand the connection to a task of the application's own.
/// It recovers the listener's I/O by its concrete type and reports what it
/// found, then reads the protocol's first bytes, those Hyper buffered before
/// any from the I/O, echoes them to the peer and writes once more when
/// `resume` fires.
fn upgrade_route<I, T>(
    router: Router,
    describe: fn(&I) -> T,
    recovered: oneshot::Sender<Recovered<T>>,
    resume: oneshot::Receiver<()>,
) -> Router
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: Send + 'static,
{
    let channels = Arc::new(Mutex::new(Some((recovered, resume))));
    router.route(
        "/",
        get(move |mut request: Request| async move {
            let (recovered, resume) = channels.lock().unwrap().take().unwrap();
            let upgrading = hyper::upgrade::on(&mut request);
            tokio::spawn(async move {
                let upgraded = upgrading.await.unwrap();
                let upgraded = match downcast::<TokioIo<I>>(upgraded) {
                    Ok(_) => {
                        let _ = recovered.send(Err("the listener's bare I/O type matched"));
                        return;
                    }
                    Err(upgraded) => upgraded,
                };
                let Ok(parts) = downcast::<TokioIo<ForcedCloseIo<I>>>(upgraded) else {
                    let _ = recovered.send(Err("the forced-close wrapper did not match"));
                    return;
                };
                let buffered = parts.read_buf;
                let wrapped = parts.io.into_inner();
                let described = describe(wrapped.get_ref());
                let mut io = wrapped.into_inner();
                let _ = recovered.send(Ok((described, buffered.clone())));
                // The buffered bytes come first; the I/O has only the rest.
                let mut protocol = buffered.to_vec();
                while protocol.len() < EARLY.len() {
                    assert_ne!(io.read_buf(&mut protocol).await.unwrap(), 0);
                }
                io.write_all(b"recovered;").await.unwrap();
                io.write_all(&protocol).await.unwrap();
                let _ = resume.await;
                io.write_all(b"after shutdown;").await.unwrap();
            });
            (
                StatusCode::SWITCHING_PROTOCOLS,
                [(header::CONNECTION, "upgrade"), (header::UPGRADE, "raw")],
            )
                .into_response()
        }),
    )
}

async fn read_until(client: &mut (impl AsyncRead + Unpin), marker: &[u8]) -> String {
    let mut bytes = Vec::new();
    timeout(WAIT, async {
        while !bytes.windows(marker.len()).any(|window| window == marker) {
            assert_ne!(client.read_buf(&mut bytes).await.unwrap(), 0);
        }
    })
    .await
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

/// The recovered connection is the application's: the server shuts down
/// joined without it and the connection still writes afterwards.
async fn outlives_the_server(
    served: Served,
    resume: oneshot::Sender<()>,
    client: &mut (impl AsyncRead + Unpin),
) {
    let report = served.shut_down().await;
    served.assert_joined_and_cleaned(&report);
    resume.send(()).unwrap();
    let text = read_until(client, b"after shutdown;").await;
    assert!(text.ends_with("after shutdown;"), "{text}");
}

#[tokio::test]
async fn an_upgraded_connection_is_recovered_through_the_forced_close_wrapper() {
    upgraded_socket(register_http).await;
}

#[tokio::test]
async fn peer_server_upgraded_connection_is_recovered_through_the_same_wrapper() {
    upgraded_socket(register_http_with_connect_info_in).await;
}

/// The upgraded I/O is `TokioIo<ForcedCloseIo<TcpStream>>` through both
/// registrations, never the listener's bare `TokioIo<TcpStream>`. The
/// recovered socket is the accepted one, and the protocol bytes the peer sent
/// with its upgrade request reach the application whole and in order. How
/// they divide between the downcast's buffer and the socket is TCP's to
/// decide, so no division is asserted here.
async fn upgraded_socket(register: RegisterHttp) {
    let (recovered, found) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let served = Served::start(register, CANCEL, |router| {
        let peer: fn(&TcpStream) -> SocketAddr = |socket| socket.peer_addr().unwrap();
        upgrade_route(router, peer, recovered, resumed)
    })
    .await;
    let mut client = served.request(&[HEAD, EARLY].concat()).await;
    let text = read_until(&mut client, b"recovered;early;").await;
    assert!(text.starts_with("HTTP/1.1 101 "), "{text}");
    let (peer, buffered) = timeout(WAIT, found).await.unwrap().unwrap().unwrap();
    assert_eq!(peer, client.local_addr().unwrap());
    assert!(EARLY.starts_with(&buffered), "{buffered:?}");

    outlives_the_server(served, resume, &mut client).await;
}

/// A listener over one in-memory connection. A write to its peer that fits
/// the pipe arrives whole or not at all, and a read with room takes all of
/// it, so a test decides exactly which bytes the server can have read; a
/// socket leaves that to TCP.
struct OneConnection(Option<DuplexStream>);

impl Listener for OneConnection {
    type Io = DuplexStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (DuplexStream, SocketAddr) {
        match self.0.take() {
            Some(io) => (io, SocketAddr::from(([127, 0, 0, 1], 0))),
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(SocketAddr::from(([127, 0, 0, 1], 0)))
    }
}

/// The peer writes the request head with the protocol's first three bytes,
/// and the rest only once the application reported its downcast. Hyper
/// 1.11.1 reads with kibibytes of room, so the read that completes the head
/// takes those three bytes with it: they come back in the downcast's buffer,
/// exactly, and the application reads them before the rest from the I/O.
#[tokio::test]
async fn bytes_read_with_the_upgrade_request_come_back_in_the_downcasts_buffer() {
    let (sent, later) = EARLY.split_at(3);
    let (server, mut client) = tokio::io::duplex(1024);
    let (recovered, found) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let served = Served::start(
        move |supervisor, name, _bound, app| {
            register_http(supervisor, name, OneConnection(Some(server)), app)
        },
        CANCEL,
        |router| {
            let nothing: fn(&DuplexStream) = |_| {};
            upgrade_route(router, nothing, recovered, resumed)
        },
    )
    .await;
    client.write_all(&[HEAD, sent].concat()).await.unwrap();
    let ((), buffered) = timeout(WAIT, found).await.unwrap().unwrap().unwrap();
    assert_eq!(&buffered[..], sent);
    client.write_all(later).await.unwrap();
    let text = read_until(&mut client, b"recovered;early;").await;
    assert!(text.starts_with("HTTP/1.1 101 "), "{text}");

    outlives_the_server(served, resume, &mut client).await;
}
