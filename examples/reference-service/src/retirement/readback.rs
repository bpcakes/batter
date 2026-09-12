use super::{DatabaseIdentity, RetirementError, driver::LEGACY_TYPE, session::Session};
use batter::{
    cleanup::CleanupBudget,
    command::{Command, CommandFuture, CommandScope},
    operation::OperationContext,
};
use sqlx::{postgres::PgConnectOptions, types::Uuid};

/// Prepare a separate read-only observation of one global legacy control job.
/// Await retirement first and retain its complete report while awaiting this
/// command. Readback never changes that primary outcome or authorizes replay.
/// Missing rows return `None`; cancellation retains the already-published primary
/// report and leaves this observation incomplete.
///
/// ```no_run
/// # async fn example(options: sqlx::postgres::PgConnectOptions,
/// # expected: batter_example_reference_service::retirement::DatabaseIdentity,
/// # job: sqlx::types::Uuid) -> Result<(), batter::BoxError> {
/// use batter::{command::check_command, cleanup::CleanupBudget, operation::OperationContext};
/// use std::time::Duration;
/// let second = Duration::from_secs(1);
/// let observation = batter_example_reference_service::retirement::readback(
///     options, expected, job, OperationContext::new(second * 5)?,
///     CleanupBudget::new(second, second, second)?).start();
/// let report = check_command(observation.wait().await)?;
/// // Inspect report.work; retain the original retirement report independently.
/// # Ok(()) }
/// ```
pub fn readback(
    options: PgConnectOptions,
    expected: DatabaseIdentity,
    job: Uuid,
    context: OperationContext,
    cleanup: CleanupBudget,
) -> Command<
    impl for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, Option<String>, RetirementError>
    + Send
    + 'static,
> {
    Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("retirement.readback")?;
            let session = Session::new(
                scope,
                options.options([("default_transaction_read_only", "on")]),
            )?;
            let result = async {
            session.verify(expected).await?;
            sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1 AND job_type = $2 AND organization_id IS NULL")
                .bind(job).bind(LEGACY_TYPE).fetch_optional(&session.pool).await.map_err(RetirementError::from)
        }.await;
            session.finish(result)
        })
    })
}
