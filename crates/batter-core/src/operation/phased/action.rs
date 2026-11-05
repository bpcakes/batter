use std::{future::Future, pin::Pin};

/// Library-boxed future borrowing the current phase only.
type ActionFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// An owned async closure executed with the library's current phase scope.
///
/// Implemented for `async move |scope| { ... }` with a `Send` future. Output
/// cannot borrow the scope. Manual implementations are sealed; the library
/// supplies the phase and boxes its future. See [`super::PhasedOperation`].
pub trait PhaseAction<S, T, E>:
    for<'a> AsyncFnOnce(&'a mut S) -> Result<T, E> + Send + 'static
{
    #[doc(hidden)]
    fn invoke(self, scope: &mut S, _: private::Token) -> ActionFuture<'_, T, E>;
}

impl<S, T, E, F> PhaseAction<S, T, E> for F
where
    F: for<'a> private::SendAction<'a, S, T, E>
        + for<'a> AsyncFnOnce(&'a mut S) -> Result<T, E>
        + Send
        + 'static,
{
    fn invoke(self, scope: &mut S, _: private::Token) -> ActionFuture<'_, T, E> {
        Box::pin(self(scope))
    }
}

pub(super) mod private {
    use std::future::Future;

    pub struct Token;

    pub trait SendAction<'a, S: 'a, T, E>: FnOnce(&'a mut S) -> Self::Future {
        type Future: Future<Output = Result<T, E>> + Send + 'a;
    }

    impl<'a, S: 'a, T, E, F, Fut> SendAction<'a, S, T, E> for F
    where
        F: FnOnce(&'a mut S) -> Fut,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
    {
        type Future = Fut;
    }
}
