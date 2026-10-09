//! One handler invocation's exit, observed separately from its deadline.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

type ExitHook = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    ended: bool,
    hooks: Vec<ExitHook>,
    waiters: Vec<(u64, Waker)>,
    next_waiter: u64,
}

impl Shared {
    // Hooks and all waker callbacks (including clone/drop) run outside this
    // lock, so a poisoned state is still consistent: every update is a single
    // field assignment or collection edit.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn has_ended(&self) -> bool {
        self.lock().ended
    }

    fn end(&self) -> JobInvocationHookPanics {
        let (hooks, waiters) = {
            let mut state = self.lock();
            if state.ended {
                return JobInvocationHookPanics(Vec::new());
            }
            state.ended = true;
            (
                std::mem::take(&mut state.hooks),
                std::mem::take(&mut state.waiters),
            )
        };
        // A failed wake must not discard the hooks that carry cancellation.
        // Both iterators are consumed in notification order, outside the lock.
        let wake_panics = waiters
            .into_iter()
            .filter_map(|(_, waker)| catch_unwind(AssertUnwindSafe(|| waker.wake())).err());
        let hook_panics = hooks
            .into_iter()
            .filter_map(|hook| catch_unwind(AssertUnwindSafe(hook)).err());
        JobInvocationHookPanics(wake_panics.chain(hook_panics).collect())
    }
}

/// Exclusive owner of one handler invocation's exit signal.
///
/// A runtime creates one owner for each handler invocation, exposes
/// [`Self::invocation`] through its [`super::JobExecutionServices`], and ends
/// the owner when that invocation exits: explicitly with [`Self::end`], or by
/// dropping it, including when the task driving the invocation is aborted or
/// destroyed. Ending wakes every [`JobInvocation::ended`] waiter and then runs
/// every registered exit hook once, in registration order. Each waiter wake or
/// hook panic is contained independently, so later notifications still run.
///
/// The owner is not `Clone`, and no observation can reach it, so handlers,
/// observers and detached tasks cannot end an invocation. A runtime that ends
/// it before the invocation exits, keeps it after the invocation exits, or
/// shares one owner between invocations breaks the observation contract.
///
/// ```
/// use runledger_core::jobs::JobInvocationOwner;
///
/// let owner = JobInvocationOwner::new();
/// let invocation = owner.invocation();
/// assert!(!invocation.has_ended());
/// let contained = owner.end();
/// assert!(contained.is_empty());
/// assert!(invocation.has_ended());
/// ```
///
/// ```compile_fail,E0599
/// fn cannot_duplicate(owner: runledger_core::jobs::JobInvocationOwner) {
///     let _ = owner.clone();
/// }
/// ```
#[must_use = "dropping the owner ends the invocation immediately"]
pub struct JobInvocationOwner {
    shared: Arc<Shared>,
}

impl JobInvocationOwner {
    /// Start one invocation's exit signal. It has not ended.
    pub fn new() -> Self {
        Self {
            shared: Arc::default(),
        }
    }

    /// Read-only observation of this invocation, without authority to end it.
    pub fn invocation(&self) -> JobInvocation {
        JobInvocation {
            shared: Arc::clone(&self.shared),
        }
    }

    /// End the invocation now and return the waiter-wake and exit-hook panics
    /// it contained.
    ///
    /// Dropping the owner ends the invocation the same way, but has nowhere to
    /// report contained panics. Containment catches unwinding panics; it does
    /// not suppress the panic hook or catch panics that abort the process.
    pub fn end(self) -> JobInvocationHookPanics {
        self.shared.end()
    }
}

impl Default for JobInvocationOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for JobInvocationOwner {
    fn drop(&mut self) {
        drop(self.shared.end());
    }
}

impl fmt::Debug for JobInvocationOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobInvocationOwner")
            .field("ended", &self.shared.has_ended())
            .finish()
    }
}

/// Cloneable, read-only observation of one handler invocation's exit.
///
/// The invocation ends when its runtime-owned execution exits, whatever the
/// cause: a returned result, a panic, the runtime deadline, lease loss, or the
/// runtime destroying the invocation. Clones and late observers see the same
/// signal; observing it after the end completes immediately. It carries no
/// authority to end the invocation, and dropping it changes nothing.
///
/// The end is a notification, not a join: it does not wait for work the
/// handler detached, undo external effects, or decide the durable outcome.
///
/// ```compile_fail,E0599
/// fn cannot_end(invocation: runledger_core::jobs::JobInvocation) {
///     let _ = invocation.end();
/// }
/// ```
#[derive(Clone)]
pub struct JobInvocation {
    shared: Arc<Shared>,
}

impl JobInvocation {
    /// Whether the invocation has already ended.
    #[must_use]
    pub fn has_ended(&self) -> bool {
        self.shared.has_ended()
    }

    /// Wait until the invocation ends. Ready immediately if it already has.
    pub fn ended(&self) -> JobInvocationEnded {
        JobInvocationEnded {
            shared: Arc::clone(&self.shared),
            waiter: None,
        }
    }

    /// Run `hook` exactly once when the invocation ends.
    ///
    /// The owner runs registered hooks synchronously, in registration order, on
    /// the thread that ends the invocation, which can be inside a destructor
    /// during task abort. Keep a hook brief, non-blocking and panic-free: the
    /// owner contains and reports unwinding panics. If the invocation has already
    /// ended, `hook` runs immediately on the caller's thread and a panic propagates.
    /// Each registration is retained until the invocation ends.
    pub fn on_end<F>(&self, hook: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut state = self.shared.lock();
        if !state.ended {
            state.hooks.push(Box::new(hook));
            return;
        }
        drop(state);
        hook();
    }
}

impl fmt::Debug for JobInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobInvocation")
            .field("ended", &self.has_ended())
            .finish()
    }
}

/// Future returned by [`JobInvocation::ended`].
///
/// Waker cloning, replacement and removal run their callbacks outside the
/// invocation's state lock, so they may read the invocation without deadlocking.
#[must_use = "futures do nothing unless polled"]
pub struct JobInvocationEnded {
    shared: Arc<Shared>,
    waiter: Option<u64>,
}

impl Future for JobInvocationEnded {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let waker = cx.waker().clone();
        let mut state = this.shared.lock();
        if state.ended {
            drop(state);
            this.waiter = None;
            return Poll::Ready(());
        }
        let current = this
            .waiter
            .and_then(|id| state.waiters.iter_mut().find(|(waiter, _)| *waiter == id));
        let replaced = if let Some((_, current)) = current {
            Some(std::mem::replace(current, waker))
        } else {
            let id = state.next_waiter;
            state.next_waiter = state.next_waiter.wrapping_add(1);
            state.waiters.push((id, waker));
            this.waiter = Some(id);
            None
        };
        drop(state);
        drop(replaced);
        Poll::Pending
    }
}

impl Drop for JobInvocationEnded {
    fn drop(&mut self) {
        if let Some(id) = self.waiter.take() {
            let removed = {
                let mut state = self.shared.lock();
                state
                    .waiters
                    .iter()
                    .position(|(waiter, _)| *waiter == id)
                    .map(|index| state.waiters.remove(index).1)
            };
            drop(removed);
        }
    }
}

impl fmt::Debug for JobInvocationEnded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobInvocationEnded")
            .field("ended", &self.shared.has_ended())
            .finish()
    }
}

/// Notification panics contained by [`JobInvocationOwner::end`]: waiter wakes
/// first, then exit hooks in registration order. Each one means that callback's
/// notification may not have completed. The existing type name is retained for
/// compatibility; formatting reports only the count.
///
/// Inspect payloads through [`Self::payloads`]. Dropping this report releases
/// string payloads but intentionally retains every other payload allocation:
/// an opaque `panic_any` payload can itself panic, even recursively, on drop.
/// This also applies when an owner is dropped without a report recipient.
#[must_use = "a contained notification panic means an observer may not have been notified"]
pub struct JobInvocationHookPanics(Vec<Box<dyn Any + Send>>);

impl JobInvocationHookPanics {
    /// Whether every waiter wake and exit hook returned normally.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of waiter wakes and exit hooks that panicked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Borrow the original payloads while this report retains disposal ownership.
    ///
    /// ```
    /// use runledger_core::jobs::JobInvocationOwner;
    /// let owner = JobInvocationOwner::new();
    /// owner.invocation().on_end(|| panic!("notification failed"));
    /// let panics = owner.end();
    /// assert_eq!(panics.payloads().len(), 1);
    /// assert!(panics.payloads().next().unwrap().is::<&'static str>());
    /// ```
    pub fn payloads(&self) -> impl ExactSizeIterator<Item = &(dyn Any + Send)> {
        self.0.iter().map(Box::as_ref)
    }

    /// Transfer the original payloads through a low-level ownership escape hatch.
    ///
    /// The caller now owns their disposal: dropping an opaque payload may execute
    /// arbitrary application code and panic. Prefer [`Self::payloads`] for
    /// diagnostics that should retain the report's disposal policy.
    #[must_use]
    pub fn into_payloads(mut self) -> Vec<Box<dyn Any + Send>> {
        std::mem::take(&mut self.0)
    }
}

impl Drop for JobInvocationHookPanics {
    fn drop(&mut self) {
        for payload in self.0.drain(..) {
            // Match native shutdown containment: never execute an opaque panic
            // payload's destructor inside a library-owned failure boundary.
            if !payload.is::<String>() && !payload.is::<&'static str>() {
                std::mem::forget(payload);
            }
        }
    }
}

impl fmt::Debug for JobInvocationHookPanics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("JobInvocationHookPanics")
            .field(&self.0.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
