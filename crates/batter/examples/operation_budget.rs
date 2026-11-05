//! Owned work/finalization phases and per-execution retry jitter.

use batter::retry::{self, ReplaySafety, RetryDecision, RetryOptions, RetryPolicy};
use std::{collections::hash_map::RandomState, hash::BuildHasher, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // First statement, unwrapped: prints each panic's location, never its payload.
    std::panic::set_hook(batter::redacting_panic_hook());
    let parent = batter::operation::OperationOwner::new(Duration::from_secs(1))?.into_context();
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
    let operation = batter::operation::PhasedOperation::new(
        &parent,
        Duration::from_millis(200),
        async move |work| {
            retry::execute_with_options(
                work,
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
            .await
        },
        async |finalization| {
            // Only this callback can access the finalization context. Work
            // evidence is retained even when an earlier ? returned an error.
            finalization
                .context()
                .run("example.finalize", |_| async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok::<_, std::io::Error>(())
                })
                .await
        },
    )?;
    let running = operation.start();
    let report = running.wait().await?;

    println!(
        "work succeeded: {}; finalization succeeded: {}",
        report.work().is_success(),
        report.finalization().is_success()
    );
    // Keep both typed results available until the application's reporting step.
    if !report.is_success() {
        return Err(std::io::Error::other("operation or finalization failed").into());
    }
    Ok(())
}
