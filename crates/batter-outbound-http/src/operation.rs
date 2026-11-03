//! Bridge from `batter_core::operation::OperationContext` (feature `operation`).

use crate::{
    exchange::{Exchange, NotStarted, StartRefusal},
    limits::ResponseLimit,
    request::PreparedRequest,
    send,
};
use batter_core::operation::{Interruption, OperationContext};

impl PreparedRequest {
    /// Send the request once under `context`'s deadline and cancellation.
    ///
    /// A context that is already cancelled returns
    /// [`crate::StartRefusal::Cancelled`] without sending. Cancellation after the
    /// start returns [`crate::UnknownReason::Cancelled`] or
    /// [`crate::BodyFailure::Cancelled`]. Otherwise this behaves like
    /// [`PreparedRequest::send`] with `context.deadline()`: the classification
    /// arrives at least [`crate::RESERVE`] before the context expires, apart from
    /// scheduling delay and any poll that blocks, such as macOS certificate
    /// verification (see the crate's Limits; source-inspected).
    ///
    /// Pass the work phase of a context whose finalization is reserved, and
    /// record the outcome in the finalization phase. `RESERVE` is not a
    /// recording budget, and a cancelled context, including one whose parent
    /// was cancelled, runs no recording step: the dispatch fence the
    /// application wrote before sending is what remains held.
    ///
    /// ```no_run
    /// use batter_outbound_http::{Client, ConnectTimeout, Origin, ResponseLimit, http::Method};
    /// use batter_core::operation::OperationOwner;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Build the client once per origin and prepare before the budget starts.
    /// let client = Client::new(
    ///     Origin::https("https://provider.example")?,
    ///     ConnectTimeout::new(Duration::from_secs(5))?,
    /// )?;
    /// let request = client.request(Method::POST, ["v1", "effects"]).prepare()?;
    /// let context = OperationOwner::new(Duration::from_secs(10))?.into_context();
    /// let phases = context.reserve_finalization(Duration::from_millis(500))?;
    /// // The application's durable dispatch fence is written before this send.
    /// let exchange = request
    ///     .send_in(phases.work(), ResponseLimit::new(16 * 1024)?)
    ///     .await;
    /// let label = exchange.label();
    /// phases
    ///     .finalization()
    ///     .run("example.record", |_| async move {
    ///         // Record `label` and settle or hold the fence here.
    ///         let _ = label;
    ///         Ok::<_, std::convert::Infallible>(())
    ///     })
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_in(self, context: &OperationContext, limit: ResponseLimit) -> Exchange {
        if matches!(context.check(), Err(Interruption::Cancelled)) {
            return Exchange::NotStarted(NotStarted::new(StartRefusal::Cancelled));
        }
        send::run(
            self.http,
            self.request,
            context.deadline(),
            limit,
            context.cancelled(),
        )
        .await
    }
}
