//! A finite PostgreSQL query whose native pool close is published before use.
//! The database is externally provisioned and selected only by `DATABASE_URL`.
//! The executable root owns SIGINT and SIGTERM: either cancels the query, and
//! the registered pool close still runs under its own cleanup budget.

use batter::sqlx::{PgSessionProfile, SqlxFailure, pool_in, probe};
use batter::{
    BoxError, RegistrationError,
    cleanup::{CleanupBudget, CleanupReport},
    command::{Command, CommandFailure},
    operation::{OperationError, OperationOwner},
};
use sqlx::postgres::PgPoolOptions;
use std::{error::Error, fmt, process::ExitCode, time::Duration};

enum ExampleError {
    Registration(RegistrationError),
    Query(OperationError<SqlxFailure>),
    Pool(SqlxFailure),
}

impl From<RegistrationError> for ExampleError {
    fn from(error: RegistrationError) -> Self {
        Self::Registration(error)
    }
}

impl From<OperationError<SqlxFailure>> for ExampleError {
    fn from(error: OperationError<SqlxFailure>) -> Self {
        Self::Query(error)
    }
}

impl fmt::Display for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("owned PostgreSQL command failed")
    }
}

impl fmt::Debug for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for ExampleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Registration(error) => Some(error),
            Self::Query(error) => Some(error),
            Self::Pool(error) => Some(error),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<ExitCode, BoxError> {
    // First statement, unwrapped: prints each panic's location, never its payload.
    std::panic::set_hook(batter::redacting_panic_hook());
    let Some(database_url) = std::env::var_os("DATABASE_URL") else {
        eprintln!("DATABASE_URL is required");
        return Ok(ExitCode::FAILURE);
    };
    let Some(database_url) = database_url.to_str() else {
        eprintln!("DATABASE_URL must be valid Unicode");
        return Ok(ExitCode::FAILURE);
    };
    let connect_options = match batter::sqlx::connect_options(database_url) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{}", endpoint_refusal(&error));
            return Ok(ExitCode::FAILURE);
        }
    };

    let login = connect_options.get_username();
    let profile = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let second = Duration::from_secs(1);
    let context = OperationOwner::new(Duration::from_secs(5))?.into_context();
    let cleanup = CleanupBudget::new(second, second, second)?;
    let owner = Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("postgres.pool")?;
            let pool = pool_in(
                scope.reserve_cleanup("postgres.pool")?,
                PgPoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(2)),
                connect_options,
                profile,
            )
            .map_err(|error| ExampleError::Pool(error.into()))?;
            scope.stage("postgres.query")?;
            probe(pool.pool(), scope.context()).await?;
            Ok::<_, ExampleError>(())
        })
    })
    .start_with_unix_signals()?;

    // Default command diagnostics retain causes without formatting their contents.
    Ok(match owner.finish().await {
        Ok(success) => {
            println!(
                "query succeeded: true; cleanup succeeded: {}; command succeeded: true",
                success.cleanup().is_success()
            );
            ExitCode::SUCCESS
        }
        Err(CommandFailure::Report(report)) => {
            println!(
                "query succeeded: {}; cleanup succeeded: {}; command succeeded: {}",
                report.work.is_ok(),
                report.cleanup.as_ref().is_ok_and(CleanupReport::is_success),
                report.is_success()
            );
            ExitCode::FAILURE
        }
        Err(CommandFailure::Coordinator(_)) => {
            eprintln!("PostgreSQL command coordinator failed");
            ExitCode::FAILURE
        }
    })
}

/// Operator guidance for a refused `DATABASE_URL`, without its contents.
fn endpoint_refusal(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::Configuration(source) if source.is::<batter::sqlx::AmbientHostAddress>() => {
            "unset PGHOSTADDR: SQLx would connect to it instead of DATABASE_URL's host"
        }
        sqlx::Error::Configuration(source)
            if source.is::<batter::sqlx::HostAddressShadowedBySocket>() =>
        {
            "give DATABASE_URL a TCP host: a Unix socket would replace its hostaddr"
        }
        _ => "DATABASE_URL is not a valid PostgreSQL connection string",
    }
}
