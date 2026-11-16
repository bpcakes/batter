//! The process stop clock as a shutdown signal reads it: the abort deadline,
//! its moves, and when forced cancellation began.

use super::ShutdownSignal;

impl ShutdownSignal {
    /// When the owning supervisor stops waiting for cooperative cancellation
    /// and requests abort of unfinished direct tasks: its recorded stop time
    /// plus its drain and cancel allowances.
    ///
    /// `None` before drain is requested, and always for a standalone handle,
    /// which no supervisor budget governs. A component that owns descendants
    /// can use the remaining interval to end them itself, so that it returns
    /// joined rather than aborted; an abort makes the supervisor skip
    /// dependent cleanup. This is an observation, not a guarantee: a tighter
    /// stop clock can move the instant earlier, never later, so read it again
    /// after waiting or wait for that move with
    /// [`Self::abort_deadline_before`]; scheduling delay can pass it
    /// unnoticed; and an abandoned driver cancels without phases.
    ///
    /// ```
    /// use batter_core::{cleanup::CleanupBudget,
    ///     lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor}};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let second = std::time::Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    /// let handle = supervisor.handle();
    /// let signal = handle.signal();
    /// assert!(signal.abort_deadline().is_none());
    /// let before = tokio::time::Instant::now();
    /// handle.request();
    /// let abort = signal.abort_deadline().expect("drain was requested");
    /// assert!(abort >= before + 2 * second);
    /// assert!(ShutdownHandle::new_unapproved().signal().abort_deadline().is_none());
    /// # Ok(()) }
    /// ```
    pub fn abort_deadline(&self) -> Option<tokio::time::Instant> {
        self.shared.abort_deadline()
    }

    /// Wait until [`Self::abort_deadline`] is earlier than `known`, and return
    /// the new instant.
    ///
    /// The deadline moves only earlier, when a component reveals a stop time
    /// before the recorded one, such as a native runtime that had already
    /// begun stopping. Work scheduled against a deadline it read waits on this
    /// beside that work and reschedules when it resolves. It resolves at once
    /// if the deadline is already earlier, and never for a standalone handle,
    /// which no supervisor budget governs. Read-only: waiting changes nothing.
    ///
    /// ```
    /// use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let second = std::time::Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    /// let handle = supervisor.handle();
    /// let signal = handle.signal();
    /// handle.request();
    /// let abort = signal.abort_deadline().expect("drain was requested");
    /// // Nothing revealed an earlier stop, so the deadline has not moved.
    /// let moved = signal.abort_deadline_before(abort);
    /// assert!(tokio::time::timeout(second / 100, moved).await.is_err());
    /// // A caller holding a later instant learns the current one at once.
    /// assert_eq!(signal.abort_deadline_before(abort + second).await, abort);
    /// # Ok(()) }
    /// ```
    pub async fn abort_deadline_before(&self, known: tokio::time::Instant) -> tokio::time::Instant {
        self.shared.abort_before(known).await
    }

    /// When forced cooperative cancellation was first requested: when the
    /// owning supervisor's drain phase ended, or when an emergency stop, such
    /// as an abandoned driver's, requested it. The drain phase ends when its
    /// allowance runs out if a direct task outlasts it, and earlier once every
    /// direct task has finished.
    ///
    /// `None` until then. The supervisor's driver records the instant as it
    /// requests the cancellation, before anyone can observe it, so work that
    /// shares the cancel allowance with the operations cancelled then, such as
    /// closing connections halfway to [`Self::abort_deadline`], measures from
    /// here rather than from its own, possibly later, observation of
    /// [`Self::cancelled`]. The driver's own scheduling delay can still make
    /// the instant later than the drain allowance. Read-only, like the
    /// deadline.
    ///
    /// ```
    /// use batter_core::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter_core::BoxError> {
    /// let allowance = std::time::Duration::from_millis(20);
    /// let cleanup = CleanupBudget::new(allowance, allowance, allowance)?;
    /// let budget = ShutdownBudget::new(allowance, allowance, allowance, cleanup)?;
    /// let mut supervisor = Supervisor::new(budget);
    /// let signal = supervisor.handle().signal();
    /// supervisor.register("worker", |startup| async move {
    ///     let running = startup.acknowledge_started();
    ///     // Outlasts drain, so the supervisor forces cancellation.
    ///     running.cancelled().await;
    ///     Ok(running.stopped())
    /// })?;
    /// let (handle, process) = (supervisor.handle(), supervisor.start());
    /// handle.status().wait_ready().await.expect("the worker acknowledged");
    /// assert!(signal.forced_cancellation_at().is_none());
    /// handle.request();
    /// let abort = signal.abort_deadline().expect("drain was requested");
    /// let _report = process.wait().await.expect("the report is published");
    /// let forced = signal.forced_cancellation_at().expect("the drain phase ended");
    /// // The worker outlasted drain, so cancellation began no earlier than the
    /// // drain allowance after the stop; a busy runtime can only delay it.
    /// assert!(forced + allowance >= abort);
    /// # Ok(()) }
    /// ```
    pub fn forced_cancellation_at(&self) -> Option<tokio::time::Instant> {
        self.shared.cancelled_at()
    }
}
