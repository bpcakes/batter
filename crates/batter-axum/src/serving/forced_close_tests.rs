use super::*;
use batter_core::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ComponentExit, ManagedComponent, ManagedSettlement, ShutdownBudget, Supervisor},
    operation::OperationOwner,
};
use std::{future::poll_fn, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{oneshot, watch},
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(5);

/// Whether the gate of a server that has not finished is closed once the
/// supervisor has forced cancellation, aborted the component that outlasted
/// drain and stopped, and the serving task has had ample time to act.
async fn closed_after_forced_cancellation(cancel: Duration) -> bool {
    let cleanup = CleanupBudget::new(WAIT, WAIT, WAIT).unwrap();
    let mut supervisor = Supervisor::new(ShutdownBudget::new(WAIT, cancel, WAIT, cleanup).unwrap());
    supervisor
        .register("held", |startup| async move {
            let _running = startup.acknowledge_started();
            std::future::pending::<Result<ComponentExit, BoxError>>().await
        })
        .unwrap();
    let handle = supervisor.handle();
    let gate = Gate::new();
    let serving = tokio::spawn(serve(std::future::pending(), handle.signal(), gate.clone()));
    let running = supervisor.start();
    handle.status().wait_ready().await.unwrap();
    handle.request();
    let report = running.wait().await.unwrap();
    assert!(report.forced_cancellation);
    assert_eq!(report.abort_requested, ["held"]);
    tokio::time::sleep(WAIT).await;
    serving.abort();
    gate.is_closed()
}

/// With no interval left at forced cancellation the gate stays open, so the
/// wrapper abort does not race a close on any runtime flavour; a positive
/// allowance still closes it.
#[tokio::test(start_paused = true)]
async fn a_zero_cancel_allowance_never_closes_the_gate() {
    assert!(!closed_after_forced_cancellation(Duration::ZERO).await);
    assert!(closed_after_forced_cancellation(Duration::from_secs(2)).await);
}

/// A stop clock the test moves by hand: forced cancellation, and an abort
/// deadline that only moves earlier.
#[derive(Clone)]
struct ManualClock(Arc<watch::Sender<(bool, Option<Instant>)>>);

impl ManualClock {
    fn new(abort: Instant) -> Self {
        Self(Arc::new(watch::Sender::new((false, Some(abort)))))
    }

    fn cancel(&self) {
        self.0.send_modify(|state| state.0 = true);
    }

    fn tighten(&self, abort: Instant) {
        self.0.send_modify(|state| state.1 = Some(abort));
    }
}

impl StopClock for ManualClock {
    fn cancelled(&self) -> impl Future<Output = ()> + Send {
        let mut state = self.0.subscribe();
        async move {
            let _ = state.wait_for(|state| state.0).await;
        }
    }

    fn abort_deadline(&self) -> Option<Instant> {
        self.0.borrow().1
    }

    fn abort_deadline_before(&self, known: Instant) -> impl Future<Output = Instant> + Send {
        let mut state = self.0.subscribe();
        async move {
            let moved = state
                .wait_for(|state| state.1.is_some_and(|abort| abort < known))
                .await;
            match moved.ok().and_then(|state| state.1) {
                Some(abort) => abort,
                None => std::future::pending().await,
            }
        }
    }
}

/// On a paused clock: cancellation is observed with the abort 4 s away, so
/// the gate first waits for the 2 s midpoint. The abort then moves to 1 s,
/// and the gate closes at the recomputed 0.5 s, not at the original one.
#[tokio::test(start_paused = true)]
async fn the_close_is_recomputed_exactly_when_the_abort_deadline_moves_earlier() {
    let start = Instant::now();
    let clock = ManualClock::new(start + Duration::from_secs(4));
    let gate = Gate::new();
    let mut serving = pin!(serve(
        std::future::pending::<io::Result<()>>(),
        clock.clone(),
        gate.clone()
    ));
    clock.cancel();
    let first = poll_fn(|cx| Poll::Ready(serving.as_mut().poll(cx).is_pending())).await;
    assert!(first && !gate.is_closed());

    tokio::time::advance(Duration::from_millis(100)).await;
    clock.tighten(start + Duration::from_secs(1));
    let closed_at = loop {
        tokio::select! {
            biased;
            _ = serving.as_mut() => unreachable!("the serving future never returns"),
            () = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
        if gate.is_closed() {
            break Instant::now();
        }
    };
    let elapsed = closed_at - start;
    assert!(
        elapsed >= Duration::from_millis(500) && elapsed <= Duration::from_millis(501),
        "{elapsed:?}"
    );
}

/// A settled native component that reports success.
struct Settled;

impl ManagedSettlement for Settled {
    fn is_success(&self) -> bool {
        true
    }

    fn allows_dependency_cleanup(&self) -> bool {
        true
    }
}

/// The gate computes its close when it observes forced cancellation and
/// recomputes it when the abort deadline then moves earlier. A native
/// component here reveals an earlier stop only after the test has polled the
/// gate past its first computation, so that order is fixed. Holding the
/// component needs a multi-thread runtime and real time, so the outcome rests
/// on a wall-clock margin instead: the original close is about 15 s away and
/// the recomputed one about 0.5 s, against a 5 s wait. A gate that kept the
/// original close passes only after a stall of about 20 s before its first
/// poll; a correct one fails only if this task stalls for 5 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_close_is_recomputed_when_the_abort_deadline_moves_earlier_afterwards() {
    // Drain 50 ms and a 30 s cancel allowance: the first close is due about
    // 15 s after forced cancellation. The revealed stop is 29 s earlier, which
    // brings the abort to about 1 s after it and the close to about 0.5 s, or
    // to at once if the first poll came later than the new abort.
    let cleanup = CleanupBudget::new(WAIT, WAIT, WAIT).unwrap();
    let budget = ShutdownBudget::new(
        Duration::from_millis(50),
        Duration::from_secs(30),
        WAIT,
        cleanup,
    );
    let mut supervisor = Supervisor::new(budget.unwrap());
    let (release, released) = std::sync::mpsc::channel::<()>();
    let context = OperationOwner::new(WAIT).unwrap().into_context();
    supervisor
        .register_managed("native", context, move |_| {
            let (settle, settled) = oneshot::channel::<()>();
            let mut held = Some((released, settle));
            Ok(ManagedComponent::new(
                async { Ok::<(), BoxError>(()) },
                std::future::pending(),
                move |started| {
                    let Some((released, settle)) = held.take() else {
                        return started;
                    };
                    // Held past forced cancellation, on a worker handed over
                    // so the supervisor's driver keeps running.
                    let _ = tokio::task::block_in_place(|| released.recv_timeout(WAIT));
                    let _ = settle.send(());
                    started.checked_sub(Duration::from_secs(29)).unwrap()
                },
                async move {
                    let _ = settled.await;
                    Settled
                },
            ))
        })
        .unwrap();
    let handle = supervisor.handle();
    let signal = handle.signal();
    let running = supervisor.start();
    timeout(WAIT, handle.status().wait_ready())
        .await
        .unwrap()
        .unwrap();
    handle.request();
    timeout(WAIT, signal.cancelled()).await.unwrap();

    let gate = Gate::new();
    let mut serving = pin!(serve(
        std::future::pending::<io::Result<()>>(),
        signal,
        gate.clone()
    ));
    // Observes the cancellation and waits for the original midpoint.
    let first = poll_fn(|cx| Poll::Ready(serving.as_mut().poll(cx).is_pending())).await;
    assert!(first);
    assert!(!gate.is_closed());
    release.send(()).unwrap();
    timeout(WAIT, async {
        while !gate.is_closed() {
            tokio::select! {
                biased;
                _ = serving.as_mut() => unreachable!("the serving future never returns"),
                () = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    })
    .await
    .expect("closed at the recomputed midpoint, not the original one");
    let report = timeout(WAIT, running.wait()).await.unwrap().unwrap();
    assert!(report.forced_cancellation);
}

fn connection(gate: &Arc<Gate>, capacity: usize) -> (ForcedCloseIo<DuplexStream>, DuplexStream) {
    let (server, client) = tokio::io::duplex(capacity);
    (ForcedCloseIo::new(server, gate.clone()), client)
}

fn registered(gate: &Gate) -> usize {
    lock(&gate.connections).pollers.len()
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

/// A split connection is polled by different tasks per half; closing must
/// wake and fail both, not only the one that polled last.
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

/// Hyper can wait on a response body alone after a ready read left bytes of a
/// following request in its buffer, without polling the transport again.
/// That task's last I/O poll was ready, yet closing must wake it so the flush
/// on its next pass fails.
#[tokio::test]
async fn closing_wakes_a_task_whose_last_io_poll_was_ready() {
    let gate = Gate::new();
    let (mut server, mut client) = connection(&gate, 64);
    client.write_all(b"head and pipelined bytes").await.unwrap();
    let waiting = tokio::spawn(async move {
        let mut buffer = [0_u8; 64];
        let mut read = false;
        std::future::poll_fn(|cx| {
            if !read {
                let mut unfilled = ReadBuf::new(&mut buffer);
                let ready = Pin::new(&mut server).poll_read(cx, &mut unfilled);
                assert!(matches!(ready, Poll::Ready(Ok(()))));
                assert!(!unfilled.filled().is_empty());
                read = true;
                // Wait without I/O interest, as Hyper does with buffered bytes.
                return Poll::Pending;
            }
            Pin::new(&mut server).poll_flush(cx)
        })
        .await
    });
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        !waiting.is_finished(),
        "nothing wakes the task before closing"
    );

    gate.close();
    assert_closed(timeout(WAIT, waiting).await.unwrap().unwrap());
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
