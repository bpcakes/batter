//! One interrupt protocol across preparation and commit validation.

pub(super) enum Interrupt {
    Never,
    On(tokio::sync::oneshot::Receiver<()>),
    Requested,
}

impl Interrupt {
    /// Cancellation of this borrow retains both pending and terminal state.
    pub(super) async fn wait(&mut self) {
        loop {
            match self {
                Self::Never => std::future::pending::<()>().await,
                Self::Requested => return,
                Self::On(receiver) => {
                    *self = match receiver.await {
                        Ok(()) => Self::Requested,
                        Err(_) => Self::Never,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Interrupt;

    async fn preparation(interrupt: &mut Interrupt) {
        tokio::select! {
            biased;
            () = interrupt.wait() => panic!("preparation was interrupted"),
            () = std::future::ready(()) => {},
        }
    }

    #[tokio::test]
    async fn a_closed_sender_stays_disabled_across_phase_waits() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut interrupt = Interrupt::On(receiver);
        drop(sender);
        preparation(&mut interrupt).await;
        preparation(&mut interrupt).await;
        assert!(
            matches!(interrupt, Interrupt::Never),
            "closed sender disables both phase waits"
        );
    }

    #[tokio::test]
    async fn a_request_between_phases_is_retained() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut interrupt = Interrupt::On(receiver);
        preparation(&mut interrupt).await;
        assert!(
            sender.send(()).is_ok(),
            "receiver remains live after preparation"
        );
        interrupt.wait().await;
        interrupt.wait().await;
        assert!(
            matches!(interrupt, Interrupt::Requested),
            "request is retained"
        );
    }
}
