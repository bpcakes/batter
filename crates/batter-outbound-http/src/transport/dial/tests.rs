use super::*;
use std::{cell::RefCell, future::pending, rc::Rc};

fn addresses() -> [SocketAddr; 2] {
    [
        "[::1]:10001".parse().unwrap(),
        "127.0.0.1:10002".parse().unwrap(),
    ]
}

#[tokio::test(start_paused = true)]
async fn a_stalled_family_cannot_starve_a_healthy_alternative() {
    for reverse in [false, true] {
        for budget in [Duration::from_secs(2), Duration::from_millis(200)] {
            let mut candidates = addresses();
            if reverse {
                candidates.reverse();
            }
            let started = Instant::now();
            let attempts = RefCell::new(Vec::new());
            let attempts_ref = &attempts;
            let result = connect(
                candidates.to_vec(),
                started + budget,
                |address| async move {
                    attempts_ref.borrow_mut().push(address);
                    if address == candidates[0] {
                        pending::<io::Result<SocketAddr>>().await
                    } else {
                        Ok(address)
                    }
                },
            )
            .await
            .unwrap();
            assert_eq!(result, candidates[1], "the reachable family was not used");
            assert_eq!(
                *attempts.borrow(),
                candidates,
                "resolved family preference changed"
            );
            assert!(
                Instant::now() - started <= Duration::from_millis(300).min(budget / 2),
                "fallback used up the setup budget"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn outright_failure_starts_the_other_family_without_waiting() {
    let candidates = addresses();
    let started = Instant::now();
    let result = connect(
        candidates.to_vec(),
        started + Duration::from_secs(2),
        |address| async move {
            if address == candidates[0] {
                Err(io::ErrorKind::ConnectionRefused.into())
            } else {
                Ok(address)
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(result, candidates[1], "reachable alternative failed");
    assert_eq!(
        Instant::now(),
        started,
        "an immediate failure incurred a fallback delay"
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_address_cannot_starve_later_addresses_of_the_same_family() {
    let candidates: Vec<SocketAddr> = (1..=3)
        .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
        .collect();
    let started = Instant::now();
    let attempts = RefCell::new(Vec::new());
    let attempts_ref = &attempts;
    let result = connect(
        candidates.clone(),
        started + Duration::from_secs(3),
        |address| async move {
            attempts_ref.borrow_mut().push(address);
            match address.port() {
                1 => pending::<io::Result<SocketAddr>>().await,
                2 => Err(io::ErrorKind::ConnectionRefused.into()),
                _ => Ok(address),
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(
        result, candidates[2],
        "a stalled same-family address hid a healthy one"
    );
    assert_eq!(
        *attempts.borrow(),
        candidates,
        "resolved address order changed"
    );
    assert!(
        Instant::now() < started + Duration::from_secs(3),
        "the healthy address got no time"
    );
}

#[derive(Debug)]
struct Candidate {
    address: SocketAddr,
    closed: Rc<RefCell<Vec<SocketAddr>>>,
}

impl Drop for Candidate {
    fn drop(&mut self) {
        self.closed.borrow_mut().push(self.address);
    }
}

#[tokio::test(start_paused = true)]
async fn either_family_can_win_and_losing_candidates_are_dropped_before_return() {
    for alternate_wins in [false, true] {
        let candidates = addresses();
        let closed = Rc::new(RefCell::new(Vec::new()));
        let result = connect(
            candidates.to_vec(),
            Instant::now() + Duration::from_secs(2),
            |address| {
                let candidate = Candidate {
                    address,
                    closed: closed.clone(),
                };
                async move {
                    let duration = if address == candidates[0] {
                        Duration::from_millis(400)
                    } else if alternate_wins {
                        Duration::ZERO
                    } else {
                        Duration::from_secs(1)
                    };
                    sleep(duration).await;
                    Ok(candidate)
                }
            },
        )
        .await
        .unwrap();
        let winner = usize::from(alternate_wins);
        assert_eq!(
            result.address, candidates[winner],
            "wrong family won the race"
        );
        assert_eq!(
            *closed.borrow(),
            [candidates[1 - winner]],
            "losing setup survived the handoff"
        );
        drop(result);
        assert_eq!(
            closed.borrow().len(),
            2,
            "winning connection was not released"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn all_stalled_candidates_end_at_the_shared_deadline() {
    let candidates = addresses();
    let closed = Rc::new(RefCell::new(Vec::new()));
    let deadline = Instant::now() + Duration::from_secs(1);
    let error = connect(candidates.to_vec(), deadline, |address| {
        let candidate = Candidate {
            address,
            closed: closed.clone(),
        };
        async move {
            let _held = candidate;
            pending::<io::Result<Candidate>>().await
        }
    })
    .await
    .unwrap_err();
    assert_eq!(
        error.kind(),
        io::ErrorKind::TimedOut,
        "a shared setup timeout lost its classification"
    );
    assert_eq!(
        Instant::now(),
        deadline,
        "candidate timeouts extended the shared budget"
    );
    assert_eq!(
        closed.borrow().len(),
        2,
        "setup retained a stalled candidate"
    );
}
