pub mod contract;

pub fn launch(
    context: batter::operation::OperationContext,
    cleanup: batter::cleanup::CleanupBudget,
    acquire: contract::Acquire,
) -> batter::command::RunningCommand<u32, contract::Failure> {
    batter::command::Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            let slot = scope.reserve_cleanup("consumer.resource")?;
            let contract::Acquired { work, finish } = acquire().await?;
            slot.register(finish);
            work.await
        })
    })
    .start()
}

pub fn launch_within(
    total: batter::operation::OperationContext,
    cleanup: batter::cleanup::CleanupBudget,
    acquire: contract::Acquire,
) -> Result<batter::command::RunningCommand<u32, contract::Failure>, batter::ConfigurationError> {
    Ok(
        batter::command::Command::within(total, cleanup, move |scope| {
            Box::pin(async move {
                let slot = scope.reserve_cleanup("consumer.resource")?;
                let contract::Acquired { work, finish } = acquire().await?;
                slot.register(finish);
                work.await
            })
        })?
        .start(),
    )
}

pub fn register_native(
    process: &mut batter::lifecycle::Supervisor,
    pool: &sqlx::PgPool,
    config: runledger_runtime::config::JobsConfig,
    registry: runledger_runtime::registry::JobRegistry,
    startup: batter::operation::OperationContext,
) -> Result<(), batter::BoxError> {
    let prepared = runledger_runtime::Supervisor::builder(pool, config)?
        .with_registry(registry)
        .prepare()?;
    batter_runledger::register(process, "consumer.worker", startup, prepared)?;
    Ok(())
}
