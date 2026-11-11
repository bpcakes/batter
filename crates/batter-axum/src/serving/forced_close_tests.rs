use super::*;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(5);

fn connection(gate: &Arc<Gate>, capacity: usize) -> (ForcedCloseIo<DuplexStream>, DuplexStream) {
    let (server, client) = tokio::io::duplex(capacity);
    (ForcedCloseIo::new(server, gate.clone()), client)
}

fn registered(gate: &Gate) -> usize {
    lock(&gate.connections).parked.len()
}

fn assert_closed<T: std::fmt::Debug>(result: io::Result<T>) {
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
}

#[tokio::test]
async fn open_gate_passes_bytes_in_both_directions() {
    let gate = Gate::new();
    let (mut server, mut client) = connection(&gate, 64);
    client.write_all(b"request").await.unwrap();
    let mut read = [0_u8; 7];
    server.read_exact(&mut read).await.unwrap();
    assert_eq!(&read, b"request");
    server.write_all(b"response").await.unwrap();
    server.flush().await.unwrap();
    let mut written = [0_u8; 8];
    client.read_exact(&mut written).await.unwrap();
    assert_eq!(&written, b"response");
}

/// A split connection parks its halves in different tasks; closing must wake
/// and fail both, not only the one that parked last.
#[tokio::test]
async fn closing_wakes_and_fails_a_parked_read_and_a_parked_write() {
    let gate = Gate::new();
    let (server, _client) = connection(&gate, 8);
    let (mut reader, mut writer) = tokio::io::split(server);
    let reading = tokio::spawn(async move { reader.read(&mut [0_u8; 1]).await });
    // The peer never reads, so a write beyond the buffer stays pending.
    let writing = tokio::spawn(async move { writer.write_all(&[0_u8; 64]).await });
    // A current-thread runtime parks both spawned tasks before this resumes.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(!reading.is_finished() && !writing.is_finished());

    gate.close();
    assert_closed(timeout(WAIT, reading).await.unwrap().unwrap());
    assert_closed(timeout(WAIT, writing).await.unwrap().unwrap());
}

#[tokio::test]
async fn closed_gate_fails_every_operation_without_reaching_the_transport() {
    let gate = Gate::new();
    let (mut server, mut client) = connection(&gate, 64);
    client.write_all(b"late").await.unwrap();
    gate.close();
    // Readable bytes and free buffer space no longer matter.
    assert_closed(server.read(&mut [0_u8; 4]).await);
    assert_closed(server.write(b"late").await);
    assert_closed(server.flush().await);
    assert_closed(server.shutdown().await);
    // A connection accepted after closing is born closed.
    let (mut later, _peer) = connection(&gate, 64);
    assert_closed(later.read(&mut [0_u8; 1]).await);
}

#[tokio::test]
async fn dropping_a_connection_removes_its_registration() {
    let gate = Gate::new();
    let (first, _first_peer) = connection(&gate, 8);
    let (second, _second_peer) = connection(&gate, 8);
    assert_eq!(registered(&gate), 2);
    drop(first);
    assert_eq!(registered(&gate), 1);
    drop(second);
    assert_eq!(registered(&gate), 0);
    // Closing an empty registry is inert.
    gate.close();
}
