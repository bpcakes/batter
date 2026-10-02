//! Shutdown completion follows destruction of the caller's owned stop future.

use super::{capture::Capture, shutdown_budget};
use batter_core::{
    lifecycle::{Readiness, Supervisor},
    telemetry::metrics::{CLEANUP_HOOKS, SHUTDOWN_DURATION, SHUTDOWNS, facade},
};
use std::{
    future::Future,
    marker::PhantomPinned,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

struct Stop {
    events: Arc<Mutex<Vec<&'static str>>>,
    panics: bool,
    _pin: PhantomPinned,
}

impl Future for Stop {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("stop destroyed");
        assert!(!self.panics, "stop future destructor panicked");
    }
}

#[tokio::test(start_paused = true)]
async fn shutdown_future_destruction_precedes_terminal_recording() {
    for panics in [false, true] {
        let capture = Capture::unbounded();
        let _recorder = facade::set_default_local_recorder(&capture);
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut supervisor = Supervisor::new(shutdown_budget());
        supervisor
            .register("worker", |startup| async move {
                let running = startup.acknowledge_started();
                running.draining().await;
                Ok(running.stopped())
            })
            .unwrap();
        let cleanup_events = events.clone();
        supervisor
            .on_cleanup("resource.close", move || async move {
                cleanup_events.lock().unwrap().push("cleanup");
                Ok(())
            })
            .unwrap();
        let status = supervisor.status();
        let outcome = tokio::spawn(supervisor.run_until(Stop {
            events: events.clone(),
            panics,
            _pin: PhantomPinned,
        }))
        .await;
        if panics {
            assert!(outcome.unwrap_err().is_panic());
        } else {
            assert!(outcome.unwrap().is_success());
        }
        assert_eq!(status.readiness(), Readiness::Stopped);
        assert_eq!(*events.lock().unwrap(), ["cleanup", "stop destroyed"]);
        assert_eq!(
            capture.count(CLEANUP_HOOKS, &[("outcome", "succeeded")]),
            1.0
        );
        assert_eq!(capture.count(SHUTDOWNS, &[]), 1.0);
        assert_eq!(
            capture.count(
                SHUTDOWNS,
                &[
                    ("cause", "requested"),
                    ("result", if panics { "panicked" } else { "success" })
                ]
            ),
            1.0
        );
        assert_eq!(
            capture.samples(SHUTDOWN_DURATION).len(),
            usize::from(!panics)
        );
    }
}
