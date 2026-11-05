use batter::{
    BoxError,
    service::{self, Diagnostics, ServiceCompletion, ServiceOutcome},
    startup::{ApplicationError, Initializer, ScopedStartup},
};
use std::{fmt, net::SocketAddr};
use tokio::time::Instant;

pub(super) async fn run<D: Diagnostics>(
    startup: ScopedStartup<impl Initializer<ApplicationError, Output = SocketAddr>>,
    diagnostics: D,
    deadline: Instant,
) -> Result<(), BoxError> {
    let starting = service::start_with_readiness(startup, diagnostics, Some(deadline));
    let (owner, address) = starting.into_ready().await?;
    tracing::info!(%address, "HTTP listener bound");
    let completion = owner.wait().await;
    // Diagnostic failures remain advisory; lifecycle failures determine exit.
    tracing::debug!(diagnostics = ?completion.diagnostics(), "diagnostics completed");
    if matches!(completion.service(), ServiceOutcome::Shutdown(Ok(_))) {
        Ok(())
    } else {
        Err(Box::new(Failure(completion)))
    }
}

struct Failure<R>(ServiceCompletion<ApplicationError, R>);

impl<R> fmt::Debug for Failure<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}
impl<R> fmt::Display for Failure<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HTTP service failed; completion retained")
    }
}
impl<R> std::error::Error for Failure<R> {}
