//! One setup poll may consume time before returning Ready; a select timer
//! cannot see that passage of time until the poll returns to its owner.

use super::*;
use tokio::io::AsyncReadExt;

#[tokio::test(start_paused = true)]
async fn a_setup_poll_returning_after_its_connect_timeout_never_hands_off_http() {
    for elapsed in [
        Duration::ZERO,
        Duration::from_secs(2),
        Duration::from_secs(12),
    ] {
        let started = Instant::now();
        let connect_deadline = started + Duration::from_secs(1);
        let gate = Gate {
            deadline: started + Duration::from_secs(10),
            is_cancelled: || false,
        };
        let (io, mut peer) = tokio::io::duplex(4096);
        let mut connected = Some(http1::handshake(TokioIo::new(io)).await.unwrap());
        let advance = tokio::time::advance(elapsed);
        tokio::pin!(advance);
        let setup = poll_fn(|cx| {
            // advance changes the paused clock before yielding. Polling it
            // here models synchronous setup work inside this one setup poll;
            // the enclosing biased select has already polled its timers.
            let _ = advance.as_mut().poll(cx);
            Poll::Ready(Ok(connected.take().expect("setup polled only until Ready")))
        });
        let timer = sleep_until(gate.deadline);
        let cancelled = std::future::pending();
        tokio::pin!(timer, cancelled);
        let outcome = match await_setup(
            setup,
            &gate,
            connect_deadline,
            timer.as_mut(),
            cancelled.as_mut(),
        )
        .await
        {
            Ok(connected) => {
                let request = http::Request::builder()
                    .uri("/effect")
                    .header("host", "localhost")
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                send_on(
                    connected,
                    request,
                    &gate,
                    ResponseLimit::new(1024).unwrap(),
                    timer,
                    cancelled,
                )
                .await
            }
            Err(failure) => failure.into_exchange(),
        };
        let mut received = Vec::new();
        peer.read_to_end(&mut received).await.unwrap();
        if elapsed.is_zero() {
            assert!(
                matches!(outcome, Exchange::Unknown(_)),
                "control never dispatched: {outcome:?}"
            );
            assert!(
                received.starts_with(b"GET /effect HTTP/1.1\r\n"),
                "control did not reach HTTP handoff"
            );
        } else if elapsed < Duration::from_secs(10) {
            assert!(
                matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::TimedOut),
                "{outcome:?}"
            );
            assert!(
                received.is_empty(),
                "setup completed late and handed off HTTP"
            );
        } else {
            assert!(
                matches!(&outcome, Exchange::NotStarted(refusal) if refusal.reason() == StartRefusal::DeadlineTooClose),
                "{outcome:?}"
            );
            assert!(
                received.is_empty(),
                "setup exceeded the exchange deadline and handed off HTTP"
            );
        }
    }
}
