//! One interrupt protocol across preparation and commit validation.

/// A command's interrupt, retained across reads, write preparation and settlement.
///
/// ```
/// # async fn example() {
/// let (request, receiver) = tokio::sync::oneshot::channel();
/// let mut interrupt = batter_flags_postgres::WriteInterrupt::new(receiver);
/// request.send(()).unwrap();
/// interrupt.wait().await;
/// interrupt.wait().await;
/// # }
/// ```
pub struct WriteInterrupt(Signal);

enum Signal {
    Never,
    On(tokio::sync::oneshot::Receiver<()>),
    Requested,
}

impl WriteInterrupt {
    /// Listen for one request. Dropping the sender disables the interrupt.
    pub fn new(receiver: tokio::sync::oneshot::Receiver<()>) -> Self {
        Self(Signal::On(receiver))
    }

    pub(super) fn never() -> Self {
        Self(Signal::Never)
    }

    /// Cancellation of this borrow retains both pending and terminal state.
    pub async fn wait(&mut self) {
        loop {
            match &mut self.0 {
                Signal::Never => std::future::pending::<()>().await,
                Signal::Requested => return,
                Signal::On(receiver) => {
                    self.0 = match receiver.await {
                        Ok(()) => Signal::Requested,
                        Err(_) => Signal::Never,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Signal, WriteInterrupt};

    async fn preparation(interrupt: &mut WriteInterrupt) {
        tokio::select! {
            biased;
            () = interrupt.wait() => panic!("preparation was interrupted"),
            () = std::future::ready(()) => {},
        }
    }

    #[tokio::test]
    async fn a_closed_sender_stays_disabled_across_phase_waits() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut interrupt = WriteInterrupt::new(receiver);
        drop(sender);
        preparation(&mut interrupt).await;
        preparation(&mut interrupt).await;
        assert!(
            matches!(interrupt.0, Signal::Never),
            "closed sender disables both phase waits"
        );
    }

    #[tokio::test]
    async fn a_request_between_phases_is_retained() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut interrupt = WriteInterrupt::new(receiver);
        preparation(&mut interrupt).await;
        assert!(
            sender.send(()).is_ok(),
            "receiver remains live after preparation"
        );
        interrupt.wait().await;
        interrupt.wait().await;
        assert!(
            matches!(interrupt.0, Signal::Requested),
            "request is retained"
        );
    }
}
