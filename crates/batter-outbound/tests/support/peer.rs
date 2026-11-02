//! Scripted loopback HTTP/1.1 fault peer.
//!
//! Every accepted connection runs the same [`Step`] script and then closes
//! normally. The peer counts accepted connections and keeps the raw bytes of
//! every complete request, so a test can assert that exactly one request, or
//! none, reached it. It parses only what this package sends: a request head
//! and an optional `Content-Length` body.
//!
//! Each peer and each [`RefusingPort`] holds its port for the whole test, so
//! a client can reach only the peer it was built for, even when many test
//! processes bind ports at once.

use batter_outbound::{Client, ConnectTimeout, Origin, RESERVE, ResponseLimit};
use std::{fmt, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::Instant,
};

/// How long a test waits for the peer to observe a connection event.
const OBSERVE: Duration = Duration::from_secs(5);

/// One action on an accepted connection.
#[derive(Clone, Debug)]
pub enum Step {
    /// Read one complete request. The script ends if the client closes first.
    Read,
    /// Read whatever arrives first without expecting HTTP, such as a TLS
    /// `ClientHello`.
    Sniff,
    /// Write these bytes.
    Write(Vec<u8>),
    /// Wait before the next step.
    Pause(Duration),
    /// Keep the connection open until the client closes it.
    Hold,
    /// Close abortively: a zero linger turns the close into a TCP reset.
    Reset,
}

/// A write step from a byte string.
pub fn write(bytes: impl AsRef<[u8]>) -> Step {
    Step::Write(bytes.as_ref().to_vec())
}

#[derive(Default)]
struct State {
    connections: usize,
    requests: Vec<Recorded>,
    sniffed: Vec<Vec<u8>>,
    writes: usize,
}

/// A running peer. Dropping it stops accepting and closes its connections.
pub struct Peer {
    address: SocketAddr,
    state: watch::Sender<State>,
    task: JoinHandle<()>,
}

impl Peer {
    pub async fn start(script: impl Into<Vec<Step>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("peer binds");
        let address = listener.local_addr().expect("peer address");
        let state = watch::Sender::new(State::default());
        let task = tokio::spawn(accept(listener, script.into(), state.clone()));
        Self {
            address,
            state,
            task,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// A plain HTTP client for this peer's loopback origin.
    pub fn client(&self) -> Client {
        loopback_client(self.address)
    }

    /// An HTTPS client for this peer's address, which speaks no TLS.
    pub fn tls_client(&self, connect: Duration) -> Client {
        let origin = Origin::https(&format!("https://{}", self.address)).expect("https origin");
        Client::new(origin, connect_timeout(connect)).expect("client builds")
    }

    pub fn connections(&self) -> usize {
        self.state.borrow().connections
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.borrow().requests.clone()
    }

    pub fn sniffed(&self) -> Vec<Vec<u8>> {
        self.state.borrow().sniffed.clone()
    }

    /// The only request this peer received. It waits for the first request,
    /// because an exchange can stop before the peer task records what it read.
    pub async fn single_request(&self) -> Recorded {
        self.wait_requests(1).await;
        let requests = self.requests();
        assert_eq!(requests.len(), 1, "requests: {requests:?}");
        assert_eq!(self.connections(), 1);
        requests.into_iter().next().expect("one request")
    }

    /// Wait until at least `count` complete requests arrived.
    pub async fn wait_requests(&self, count: usize) {
        self.wait(|state| state.requests.len() >= count).await;
    }

    /// Wait until at least `count` sniff steps completed.
    pub async fn wait_sniffed(&self, count: usize) {
        self.wait(|state| state.sniffed.len() >= count).await;
    }

    /// Wait until at least `count` write steps completed.
    pub async fn wait_writes(&self, count: usize) {
        self.wait(|state| state.writes >= count).await;
    }

    /// Give a wrongly started background request time to connect, then
    /// assert that nothing did.
    pub async fn assert_untouched(&self) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(self.connections(), 0, "a refused exchange connected");
    }

    async fn wait(&self, ready: impl FnMut(&State) -> bool) {
        let mut state = self.state.subscribe();
        tokio::time::timeout(OBSERVE, state.wait_for(ready))
            .await
            .expect("the peer observed the event in time")
            .expect("the peer state is open");
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept(listener: TcpListener, script: Vec<Step>, state: watch::Sender<State>) {
    // Dropping the set when the accept task is aborted aborts every connection.
    let mut connections = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        state.send_modify(|state| state.connections += 1);
        connections.spawn(serve(stream, script.clone(), state.clone()));
    }
}

async fn serve(mut stream: TcpStream, script: Vec<Step>, state: watch::Sender<State>) {
    let mut received = Vec::new();
    for step in script {
        match step {
            Step::Read => match read_request(&mut stream, &mut received).await {
                Some(request) => state.send_modify(|state| state.requests.push(request)),
                None => return,
            },
            Step::Sniff => {
                let mut first = vec![0; 16 * 1024];
                let read = stream.read(&mut first).await.unwrap_or(0);
                first.truncate(read);
                state.send_modify(|state| state.sniffed.push(first));
            }
            Step::Write(bytes) => {
                if stream.write_all(&bytes).await.is_err() {
                    return;
                }
                state.send_modify(|state| state.writes += 1);
            }
            Step::Pause(duration) => tokio::time::sleep(duration).await,
            Step::Hold => {
                let mut sink = [0; 1024];
                while matches!(stream.read(&mut sink).await, Ok(read) if read > 0) {}
                return;
            }
            Step::Reset => {
                stream.set_zero_linger().expect("zero linger");
                return;
            }
        }
    }
}

/// Read one request: its head and its `Content-Length` body. `None` when the
/// client closed or failed before the request was complete.
async fn read_request(stream: &mut TcpStream, received: &mut Vec<u8>) -> Option<Recorded> {
    let mut chunk = [0; 4096];
    loop {
        if let Some(head) = find(received, b"\r\n\r\n") {
            let length = head + 4 + content_length(&received[..head]);
            if received.len() >= length {
                let rest = received.split_off(length);
                return Some(Recorded(std::mem::replace(received, rest)));
            }
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => received.extend_from_slice(&chunk[..read]),
        }
    }
}

fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| {
            value.trim().parse().expect("numeric content-length")
        })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// One complete request exactly as the peer received it.
#[derive(Clone)]
pub struct Recorded(Vec<u8>);

impl Recorded {
    /// The request line, such as `GET /v1 HTTP/1.1`.
    pub fn request_line(&self) -> &str {
        self.head().split("\r\n").next().unwrap_or_default()
    }

    /// Every value of header `name`, compared case-insensitively, in order.
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.head()
            .split("\r\n")
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .filter(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
            .collect()
    }

    pub fn body(&self) -> &[u8] {
        &self.0[self.head_len() + 4..]
    }

    /// Whether `needle` occurs anywhere in the raw request.
    pub fn contains(&self, needle: &str) -> bool {
        find(&self.0, needle.as_bytes()).is_some()
    }

    fn head(&self) -> &str {
        std::str::from_utf8(&self.0[..self.head_len()]).expect("an ASCII request head")
    }

    fn head_len(&self) -> usize {
        find(&self.0, b"\r\n\r\n").expect("a recorded request has a head")
    }
}

impl fmt::Debug for Recorded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", String::from_utf8_lossy(&self.0))
    }
}

/// A loopback port that refuses connections. Its socket stays bound without
/// listening, so the kernel cannot give the port to another test's listener,
/// in this or another process, while this value lives. A released port can
/// be reassigned at once, and a request sent to it would reach that peer.
pub struct RefusingPort {
    address: SocketAddr,
    _bound: TcpSocket,
}

impl RefusingPort {
    pub fn bind() -> Self {
        let socket = TcpSocket::new_v4().expect("socket");
        socket
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .expect("bind");
        let address = socket.local_addr().expect("address");
        Self {
            address,
            _bound: socket,
        }
    }

    /// A plain HTTP client for this refusing port.
    pub fn client(&self) -> Client {
        loopback_client(self.address)
    }
}

/// A plain HTTP client for a loopback `address`, with a two-second connect
/// timeout.
pub fn loopback_client(address: SocketAddr) -> Client {
    let origin = Origin::loopback(address).expect("loopback origin");
    Client::new(origin, connect_timeout(Duration::from_secs(2))).expect("client builds")
}

pub fn connect_timeout(timeout: Duration) -> ConnectTimeout {
    ConnectTimeout::new(timeout).expect("valid connect timeout")
}

pub fn limit(bytes: usize) -> ResponseLimit {
    ResponseLimit::new(bytes).expect("valid response limit")
}

pub fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// The caller's deadline `budget` from now.
pub fn after(budget: Duration) -> Instant {
    Instant::now() + budget
}

/// The exchange stopped at its own deadline, `RESERVE` before the caller's,
/// and returned before the caller's deadline passed: the remaining slack is
/// all scheduling delay, and it stays below `RESERVE`.
pub fn assert_within_reserve(deadline: Instant, returned: Instant) {
    assert!(
        returned >= deadline - RESERVE,
        "returned {:?} before the exchange deadline",
        (deadline - RESERVE) - returned
    );
    assert!(
        returned < deadline,
        "returned {:?} after the caller's deadline",
        returned - deadline
    );
}
