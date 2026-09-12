//! Offline retirement of the former global startup-control jobs.
//!
//! Stop old deployments and revoke their restart authority before invocation.
//! This command observes database state; it cannot inspect deployment authority.
//! Native definition disable prevents the inspected legacy additive catalog sync
//! and enqueue path from producing new controls. Arbitrary SQL or administrative
//! re-enabling remains outside that contract. Production startup never calls this.

mod driver;
mod readback;
mod report;
mod session;
pub use readback::readback;

pub use report::{
    CancellationFailure, DatabaseIdentity, DefinitionState, RetirementError, RetirementReport,
};

use batter::{
    cleanup::CleanupBudget,
    command::{Command, CommandFuture, CommandScope},
    operation::OperationContext,
};
use sqlx::postgres::PgConnectOptions;

/// Prepare owned finite retirement work with separate cleanup. No acquisition or
/// database mutation occurs until the returned command starts. Use one dedicated
/// maintenance connection to a direct PostgreSQL endpoint; other target-database
/// client or unknown backends and prepared transactions prevent retirement. Physical
/// connection replacement stops this invocation. Publish and retain the completed
/// report before invoking separate [`readback`] after a cancellation failure.
///
/// ```no_run
/// # async fn example(options: sqlx::postgres::PgConnectOptions) -> Result<(), batter::BoxError> {
/// use batter::{cleanup::CleanupBudget, command::check_command, operation::OperationContext};
/// use batter_example_reference_service::retirement::{prepare, DatabaseIdentity};
/// use std::time::Duration;
/// let second = Duration::from_secs(1);
/// // Supply identity from the deployment's known target, not an unchecked URL.
/// let expected = DatabaseIdentity::new(123, 456)?;
/// let command = prepare(options, expected, OperationContext::new(second * 30)?,
///     CleanupBudget::new(second * 3, second * 3, second)?) .start();
/// let report = check_command(command.wait().await)?;
/// assert!(report.work.is_ok());
/// # Ok(()) }
/// ```
pub fn prepare(
    options: PgConnectOptions,
    expected: DatabaseIdentity,
    context: OperationContext,
    cleanup: CleanupBudget,
) -> Command<
    impl for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, RetirementReport, RetirementError>
    + Send
    + 'static,
> {
    Command::new(context, cleanup, move |scope| {
        Box::pin(driver::run(scope, options, expected))
    })
}
