use super::*;
use std::{
    future::Future,
    mem::discriminant,
    sync::{Arc, Barrier, atomic::AtomicUsize},
    task::{Context, Wake, Waker},
    thread,
};

fn ready() -> Shared {
    let state = Shared::new(true);
    state.start_driver();
    assert!(state.mark_ready());
    state
}

#[test]
fn all_startup_orders_require_the_driver_approval_and_acknowledgement() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let state = Shared::new(true);
        state.register_component();
        let startup = AtomicBool::new(false);
        for (index, event) in order.into_iter().enumerate() {
            match event {
                0 => state.start_driver(),
                1 => assert!(state.mark_ready()),
                2 => assert!(state.mark_started(&startup)),
                _ => unreachable!(),
            }
            assert_eq!(
                state.readiness(),
                if index == 2 {
                    Readiness::Ready
                } else {
                    Readiness::Starting
                },
                "startup order {order:?}, step {index}"
            );
        }
        assert!(!state.mark_started(&startup));
        assert!(!state.mark_ready());
    }
}

#[test]
fn transition_table_keeps_drain_and_stop_irreversible() {
    use Readiness::{Draining, Ready, Starting, Stopped};
    // Columns: driver start, application approval, drain, force, completion.
    let table = [
        (Starting, [Starting, Starting, Draining, Draining, Stopped]),
        (Ready, [Ready, Ready, Draining, Draining, Stopped]),
        (Draining, [Draining, Draining, Draining, Draining, Stopped]),
        (Stopped, [Stopped; 5]),
    ];
    for (initial, outcomes) in table {
        for (event, expected) in outcomes.into_iter().enumerate() {
            let state = Shared::new(true);
            match initial {
                Starting => {}
                Ready => {
                    state.start_driver();
                    state.mark_ready();
                }
                Draining => state.request(),
                Stopped => {
                    state.request();
                    state.stop_driver();
                }
            }
            match event {
                0 => state.start_driver(),
                1 => {
                    state.mark_ready();
                }
                2 => state.request(),
                3 => state.force_cancel(),
                4 => state.stop_driver(),
                _ => unreachable!(),
            }
            assert_eq!(state.readiness(), expected, "{initial:?}, event {event}");
        }
    }
}

#[test]
fn late_component_acknowledgement_cannot_revive_shutdown() {
    for stopped in [false, true] {
        let state = Shared::new(true);
        state.register_component();
        state.mark_ready();
        state.start_driver();
        state.request();
        if stopped {
            state.stop_driver();
        }
        let terminal = state.readiness();
        assert!(state.mark_started(&AtomicBool::new(false)));
        assert_eq!(state.readiness(), terminal);
    }
}

#[test]
fn concurrent_request_and_completion_keep_stopped() {
    let state = Arc::new(ready());
    state.request();
    let start = Arc::new(Barrier::new(3));
    let requesting = state.clone();
    let request_start = start.clone();
    let requester = thread::spawn(move || {
        request_start.wait();
        requesting.request();
    });
    let stopping = state.clone();
    let stop_start = start.clone();
    let coordinator = thread::spawn(move || {
        stop_start.wait();
        stopping.stop_driver();
    });
    start.wait();
    requester.join().unwrap();
    coordinator.join().unwrap();
    assert_eq!(state.readiness(), Readiness::Stopped);
}

#[test]
fn readiness_reads_remain_available_while_admission_is_held() {
    let state = Arc::new(ready());
    let admission = state.admission();
    let reading = state.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || send.send(reading.readiness()).unwrap());
    let observed = receive.recv_timeout(std::time::Duration::from_secs(2));
    // Release the guard and join even when a regression blocks the reader.
    drop(admission);
    reader.join().unwrap();
    assert_eq!(observed.unwrap(), Readiness::Ready);
}

#[test]
fn admission_table_preserves_closure_and_the_active_descendant_exception() {
    use ProcessAdmissionError::{Closed, NotReady, NotRunning};
    // State transitions, then expected root and active-descendant rejections.
    let cases: [(fn(&Shared), _, _); 7] = [
        (|_| {}, Some(NotRunning), Some(NotRunning)),
        (Shared::start_driver, Some(NotReady), None),
        (
            |state| {
                state.start_driver();
                state.mark_ready();
            },
            None,
            None,
        ),
        (
            |state| {
                state.start_driver();
                state.request();
            },
            Some(Closed),
            None,
        ),
        (Shared::force_cancel, Some(Closed), Some(Closed)),
        (Shared::fail_task, Some(Closed), Some(Closed)),
        (
            |state| {
                state.force_cancel();
                state.stop_driver();
            },
            Some(Closed),
            Some(Closed),
        ),
    ];
    for (transition, root, descendant) in cases {
        let state = Shared::new(true);
        transition(&state);
        let active = AtomicBool::new(true);
        let expired = AtomicBool::new(false);
        for (ancestor, expected) in [
            (None, root),
            (Some(&active), descendant),
            (Some(&expired), Some(Closed)),
        ] {
            let admission = state.admission();
            assert_eq!(
                admission
                    .check(ancestor, false)
                    .err()
                    .as_ref()
                    .map(discriminant),
                expected.as_ref().map(discriminant)
            );
            assert!(matches!(admission.check(ancestor, true), Err(Closed)));
        }
    }
}

struct LockProbe {
    state: Arc<Shared>,
    wakes: AtomicUsize,
}

impl Wake for LockProbe {
    fn wake(self: Arc<Self>) {
        assert!(
            self.state.admission.try_lock().is_ok(),
            "wake under admission lock"
        );
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn cancellation_and_readiness_wakers_run_outside_the_transition_lock() {
    let state = Arc::new(Shared::new(true));
    let probe = Arc::new(LockProbe {
        state: state.clone(),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(probe.clone());
    let mut cx = Context::from_waker(&waker);
    let mut ready = Box::pin(state.wait_ready());
    let mut drain = Box::pin(state.draining());
    let mut cancel = Box::pin(state.cancelled());
    assert!(ready.as_mut().poll(&mut cx).is_pending());
    assert!(drain.as_mut().poll(&mut cx).is_pending());
    assert!(cancel.as_mut().poll(&mut cx).is_pending());
    state.force_cancel();
    assert_eq!(probe.wakes.load(Ordering::SeqCst), 3);
    assert!(ready.as_mut().poll(&mut cx).is_ready());
    assert!(drain.as_mut().poll(&mut cx).is_ready());
    assert!(cancel.as_mut().poll(&mut cx).is_ready());
}
