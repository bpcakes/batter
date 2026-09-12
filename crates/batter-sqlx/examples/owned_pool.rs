//! A finite PostgreSQL query whose native pool close is published before use.
//! The database is externally provisioned and selected only by `DATABASE_URL`.

use batter::{
    RegistrationError,
    cleanup::CleanupBudget,
    command::Command,
    operation::{OperationContext, OperationError},
};
use batter_sqlx::{SqlxFailure, pool_in, probe};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{error::Error, fmt, process::ExitCode, str::FromStr, time::Duration};

enum ExampleError {
    Registration(RegistrationError),
    Query(OperationError<SqlxFailure>),
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
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let Some(database_url) = std::env::var_os("DATABASE_URL") else {
        eprintln!("DATABASE_URL is required");
        return ExitCode::FAILURE;
    };
    let Some(database_url) = database_url.to_str() else {
        eprintln!("DATABASE_URL must be valid Unicode");
        return ExitCode::FAILURE;
    };
    let connect_options = match PgConnectOptions::from_str(database_url) {
        Ok(options) => options,
        Err(_) => {
            eprintln!("DATABASE_URL is not a valid PostgreSQL connection string");
            return ExitCode::FAILURE;
        }
    };

    let second = Duration::from_secs(1);
    let context =
        OperationContext::new(Duration::from_secs(5)).expect("constant operation budget is valid");
    let cleanup =
        CleanupBudget::new(second, second, second).expect("constant cleanup budget is valid");
    let command = Command::new(context, cleanup, move |scope| {
        Box::pin(async move {
            scope.stage("postgres.pool")?;
            let pool = pool_in(
                scope.reserve_cleanup("postgres.pool")?,
                PgPoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(2)),
                connect_options,
            );
            scope.stage("postgres.query")?;
            probe(&pool, scope.context()).await?;
            Ok::<_, ExampleError>(())
        })
    })
    .start();

    match command.wait().await {
        Ok(report) => {
            println!(
                "query succeeded: {}; cleanup succeeded: {}; command succeeded: {}",
                report.work.is_ok(),
                report
                    .cleanup
                    .as_ref()
                    .is_ok_and(batter::cleanup::CleanupReport::is_success),
                report.is_success()
            );
            if report.is_success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(_) => {
            eprintln!("PostgreSQL command coordinator failed");
            ExitCode::FAILURE
        }
    }
}
