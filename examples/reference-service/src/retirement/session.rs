use super::{DatabaseIdentity, RetirementError};
use batter::command::CommandScope;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(test)]
mod tests;

/// Native APIs take a pool; this private owner permits only its first physical
/// connection. A failed connection cannot silently move maintenance to a new one.
pub(super) struct Session {
    pub pool: PgPool,
    replaced: Arc<AtomicBool>,
}

impl Session {
    pub fn new(
        scope: &mut CommandScope,
        options: PgConnectOptions,
    ) -> Result<Self, RetirementError> {
        let slot = scope.reserve_cleanup("retirement.pool")?;
        let connected = AtomicBool::new(false);
        let replaced = Arc::new(AtomicBool::new(false));
        let detected = replaced.clone();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .idle_timeout(None)
            .max_lifetime(None)
            .acquire_timeout(Duration::from_secs(2))
            .after_connect(move |_, _| {
                let replacement = connected.swap(true, Ordering::SeqCst);
                if replacement {
                    detected.store(true, Ordering::SeqCst);
                }
                Box::pin(async move {
                    if replacement {
                        // SQLx logs hook errors and retries. Never put native
                        // error contents here; the bounded caller retains failure.
                        Err(sqlx::Error::Protocol(
                            "maintenance session replacement refused".into(),
                        ))
                    } else {
                        Ok(())
                    }
                })
            })
            .connect_lazy_with(options.options([("search_path", "public")]));
        let closing = pool.clone();
        slot.register(move || async move {
            closing.close().await;
            Ok(())
        });
        Ok(Self { pool, replaced })
    }

    pub fn finish<T>(&self, result: Result<T, RetirementError>) -> Result<T, RetirementError> {
        if self.replaced.load(Ordering::SeqCst) {
            Err(RetirementError::SessionReplaced {
                original: result.err().map(Box::new),
            })
        } else {
            result
        }
    }

    pub async fn verify(
        &self,
        expected: DatabaseIdentity,
    ) -> Result<DatabaseIdentity, RetirementError> {
        let (system_identifier, database_oid): (i64, i64) = sqlx::query_as(
            "SELECT system_identifier, d.oid::bigint FROM pg_control_system(), pg_database d WHERE d.datname = current_database()"
        ).fetch_one(&self.pool).await?;
        let actual = DatabaseIdentity::new(
            system_identifier,
            u32::try_from(database_oid).map_err(|_| RetirementError::InvalidIdentity)?,
        )?;
        if actual == expected {
            Ok(actual)
        } else {
            Err(RetirementError::WrongDatabase { actual })
        }
    }
}
