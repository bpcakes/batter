//! Explicit work/finalization phases and per-execution retry jitter.

use batter::{
    operation::OperationContext,
    retry::{self, ReplaySafety, RetryDecision, RetryPolicy},
};
use std::{collections::hash_map::RandomState, hash::BuildHasher, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parent = OperationContext::new(Duration::from_secs(1))?;
    let phases = parent.reserve_finalization(Duration::from_millis(200))?;
    let policy = RetryPolicy::new(3, Duration::from_millis(10), Duration::from_millis(50))?;

    // Each execution gets independently keyed state. This is a local jitter
    // source, not a cryptographic protocol; an application's RNG also fits.
    let random = RandomState::new();
    let mut draw = 0_u64;
    let work = retry::execute_with_jitter(
        phases.work(),
        "example.read",
        ReplaySafety::Idempotent,
        &policy,
        move || {
            draw = draw.wrapping_add(1);
            random.hash_one(draw)
        },
        |attempt| async move {
            if attempt.number == 1 {
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            } else {
                Ok(42)
            }
        },
        |error| match error.kind() {
            std::io::ErrorKind::ConnectionRefused => RetryDecision::Retry,
            _ => RetryDecision::Stop,
        },
    )
    .await;

    // Observe both outcomes. Returning with `work?` before this await would
    // skip finalization on an error. Parent cancellation can still stop either
    // phase; this example does not promise cleanup if its outer task is dropped.
    let finalized = phases
        .finalization()
        .run("example.finalize", |_| async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<_, std::io::Error>(())
        })
        .await;
    println!(
        "work succeeded: {}; finalization succeeded: {}",
        work.is_ok(),
        finalized.is_ok()
    );
    // Keep both typed results available until the application's reporting step.
    if work.is_err() || finalized.is_err() {
        return Err(std::io::Error::other("operation or finalization failed").into());
    }
    Ok(())
}
