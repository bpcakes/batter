//! Explicit work/finalization phases and per-execution retry jitter.

use batter::{
    operation::OperationContext,
    retry::{self, ReplaySafety, RetryDecision, RetryOptions, RetryPolicy},
};
use std::{collections::hash_map::RandomState, hash::BuildHasher, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parent = batter::operation::OperationOwner::new(Duration::from_secs(1))?.into_context();
    let phases = parent.reserve_finalization(Duration::from_millis(200))?;
    let policy = RetryPolicy::new(3, Duration::from_millis(10), Duration::from_millis(50))?;

    // Each execution gets independently keyed state. This is a local jitter
    // source, not a cryptographic protocol; an application's RNG also fits.
    let random = RandomState::new();
    let mut draw = 0_u64;
    let retry_options = RetryOptions::new()
        .with_attempt_maximum(Duration::from_millis(100))?
        .with_jitter(move || {
            draw = draw.wrapping_add(1);
            random.hash_one(draw)
        });
    let work = retry::execute_with_options(
        phases.work(),
        "example.read",
        ReplaySafety::Idempotent,
        &policy,
        retry_options,
        |attempt| async move {
            if attempt.number == 1 {
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            } else {
                // The 100 ms cap is derived again when this attempt starts;
                // it cannot extend the already shortened work deadline.
                tokio::time::sleep(Duration::from_millis(25)).await;
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
