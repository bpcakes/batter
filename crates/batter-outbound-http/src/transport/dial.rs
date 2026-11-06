//! Bounded address fallback, driven only by the owning exchange future.

use std::{future::Future, io, net::SocketAddr, time::Duration};
use tokio::time::{Instant, sleep, timeout_at};

const FAMILY_DELAY: Duration = Duration::from_millis(300);

pub(super) async fn connect<T, D, F>(
    origin: &crate::Origin,
    addresses: Vec<SocketAddr>,
    deadline: Instant,
    dial: F,
) -> io::Result<T>
where
    D: Future<Output = io::Result<T>>,
    F: Fn(SocketAddr) -> D,
{
    let addresses: Vec<_> = addresses
        .into_iter()
        .filter(|address| origin.permits_address(address.ip()))
        .collect();
    let Some(first) = addresses.first() else {
        return Err(io::ErrorKind::AddrNotAvailable.into());
    };
    let prefer_v6 = first.is_ipv6();
    let (preferred, alternate): (Vec<_>, Vec<_>) = addresses
        .into_iter()
        .partition(|address| address.is_ipv6() == prefer_v6);
    if alternate.is_empty() {
        return group(preferred, deadline, &dial).await;
    }
    // Leave the other family time even when the whole remaining setup budget
    // is shorter than the ordinary 300 ms fallback delay.
    let delay = FAMILY_DELAY.min(deadline.saturating_duration_since(Instant::now()) / 2);
    let preferred = group(preferred, deadline, &dial);
    let delay = sleep(delay);
    tokio::pin!(preferred, delay);
    tokio::select! {
        biased;
        result = &mut preferred => match result {
            Ok(stream) => return Ok(stream),
            // An outright failure should start the other family immediately.
            Err(_) => return group(alternate, deadline, &dial).await,
        },
        () = &mut delay => {}
    }
    let alternate = group(alternate, deadline, &dial);
    tokio::pin!(alternate);
    // Both futures are local. Returning drops the loser, including its socket,
    // before the selected connection can begin TLS or receive an HTTP request.
    tokio::select! {
        result = &mut preferred => match result {
            Ok(stream) => Ok(stream),
            Err(_) => alternate.await,
        },
        result = &mut alternate => match result {
            Ok(stream) => Ok(stream),
            Err(_) => preferred.await,
        },
    }
}

async fn group<T, D, F>(addresses: Vec<SocketAddr>, deadline: Instant, dial: &F) -> io::Result<T>
where
    D: Future<Output = io::Result<T>>,
    F: Fn(SocketAddr) -> D,
{
    let count = addresses.len();
    let mut last_error = io::ErrorKind::AddrNotAvailable.into();
    for (index, address) in addresses.into_iter().enumerate() {
        let now = Instant::now();
        if now >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let remaining = u32::try_from(count - index).unwrap_or(u32::MAX);
        let attempt_deadline = now + (deadline - now) / remaining;
        match timeout_at(attempt_deadline, dial(address)).await {
            Ok(Ok(stream)) => {
                // timeout_at polls its inner future first. A setup poll may
                // itself consume the budget, so clock time remains authoritative.
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                return Ok(stream);
            }
            Ok(Err(error)) => last_error = error,
            Err(_) => last_error = io::ErrorKind::TimedOut.into(),
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod public_tests;
