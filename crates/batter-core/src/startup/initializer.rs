use super::{ProtectedStartupScope, StartupFuture, StartupScope};
use std::{future::Future, pin::Pin};

/// Library-boxed initializer future. The box lets it borrow its scope across
/// awaits and erases the application future's type depth.
type InitializerFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// An owned protected-startup initializer returning typed output.
///
/// Every owned `Send` async closure (`async move |scope| { ... }`) or `async fn`
/// taking `&mut ProtectedStartupScope` and returning `Result<T, E>` implements
/// this trait when its future is `Send` and `T: Send + 'static`. The library
/// boxes the future; callers never write `Box::pin`. Stable Rust cannot bound an
/// `AsyncFnOnce` future by `Send` directly, so this trait expresses that bound
/// and can name an initializer in a return type. It cannot be implemented
/// manually. The lower-level boxed [`super::StartupFuture`] closures accepted
/// by [`super::Startup::scoped`] implement it with `Output = ()`.
///
/// ```
/// use batter_core::{
///     cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext,
///     startup::{ApplicationError, Initializer, ScopedStartup, Startup},
/// };
///
/// fn bounded_startup(
///     supervisor: Supervisor,
///     context: OperationContext,
///     cleanup: CleanupBudget,
/// ) -> ScopedStartup<impl Initializer<ApplicationError, Output = u16>> {
///     Startup::protected(supervisor, context, cleanup, async |scope| {
///         scope.stage("settings.read")?;
///         let port: u16 = "8080".parse()?;
///         Ok::<_, ApplicationError>(port)
///     })
/// }
/// ```
///
/// A future that is not `Send` is rejected when the specification is built:
///
/// ```compile_fail,E0277
/// # use batter_core::{cleanup::CleanupBudget, lifecycle::Supervisor,
/// #     operation::OperationContext, startup::Startup};
/// fn local(supervisor: Supervisor, context: OperationContext, cleanup: CleanupBudget) {
///     let shared = std::rc::Rc::new(1_u8);
///     let _ = Startup::protected(supervisor, context, cleanup, async move |_scope| {
///         tokio::task::yield_now().await;
///         Ok::<_, std::io::Error>(*shared)
///     });
/// }
/// ```
///
/// Output cannot borrow the protected scope:
///
/// ```compile_fail
/// # use batter_core::{cleanup::CleanupBudget, lifecycle::Supervisor,
/// #     operation::OperationContext, startup::Startup};
/// fn escaping(supervisor: Supervisor, context: OperationContext, cleanup: CleanupBudget) {
///     let _ = Startup::protected(supervisor, context, cleanup, async |scope| {
///         Ok::<_, std::io::Error>(scope.registration())
///     });
/// }
/// ```
pub trait Initializer<E>: Send + 'static {
    /// Output delivered with the running owner after successful handoff.
    type Output: Send + 'static;

    #[doc(hidden)]
    fn initialize(
        self,
        scope: &mut ProtectedStartupScope,
        _: private::Token,
    ) -> InitializerFuture<'_, Self::Output, E>;
}

impl<F, T, E> Initializer<E> for F
where
    F: for<'a> private::SendInitializer<'a, E, Value = T> + Send + 'static,
    T: Send + 'static,
{
    type Output = T;

    fn initialize(
        self,
        scope: &mut ProtectedStartupScope,
        _: private::Token,
    ) -> InitializerFuture<'_, T, E> {
        Box::pin(self(scope))
    }
}

/// Calls one initializer form with the scope type its specification owns.
pub(super) trait Initialize<S, E> {
    type Output;
    fn initialize(self, scope: &mut S) -> InitializerFuture<'_, Self::Output, E>;
}

impl<F, E> Initialize<StartupScope, E> for F
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E>,
{
    type Output = ();
    fn initialize(self, scope: &mut StartupScope) -> StartupFuture<'_, E> {
        self(scope)
    }
}

impl<F: Initializer<E>, E> Initialize<ProtectedStartupScope, E> for F {
    type Output = F::Output;
    fn initialize(self, scope: &mut ProtectedStartupScope) -> InitializerFuture<'_, F::Output, E> {
        Initializer::initialize(self, scope, private::Token)
    }
}

pub(super) mod private {
    use super::ProtectedStartupScope;
    use std::future::Future;

    /// Unnameable call token: only this crate can drive an initializer.
    pub struct Token;

    /// Names one scope borrow's future so its `Send` bound can be stated.
    pub trait SendInitializer<'a, E>:
        FnOnce(&'a mut ProtectedStartupScope) -> Self::Future
    {
        type Value;
        type Future: Future<Output = Result<Self::Value, E>> + Send + 'a;
    }

    impl<'a, F, Fut, T, E> SendInitializer<'a, E> for F
    where
        F: FnOnce(&'a mut ProtectedStartupScope) -> Fut,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
    {
        type Value = T;
        type Future = Fut;
    }
}
