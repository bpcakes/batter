//! Scoped application work inside the existing managed transaction.

use super::{
    AppliedManagedObjects, ManagedApplyError, ManagedObjectManifest, PublicExecute, apply, catalog,
};
use crate::{
    PgLease, PgScopedSql, PgTransactionError,
    atomic::scope::{ScopeSavepoint, validate_transaction},
};
use sqlx::PgConnection;
use std::{collections::BTreeMap, fmt};

/// A failed managed apply with application SQL. Formatting never prints a
/// concrete application or native cause; inspect fields/source explicitly.
/// No variant authorizes replay. Committed migrations are not rolled back.
pub enum ManagedApplyWithError<E> {
    /// Native migration/apply failure, including unconfirmed native COMMIT.
    Apply(ManagedApplyError),
    /// Application refusal after its savepoint was rolled back and the original
    /// transaction revalidated. No managed COMMIT was sent; retirement ends the
    /// uncommitted managed transaction, not the already committed migrations.
    Application(E),
    /// The original transaction could not be re-established or verified. A
    /// callback that issued COMMIT may already have published effects. Preserve
    /// its refusal too, if it returned one. Never classify this as known rollback.
    Unconfirmed {
        /// Retained continuity/recovery failure.
        error: PgTransactionError,
        /// Retained application refusal, independent of recovery failure.
        application: Option<E>,
    },
    /// The callback changed a managed definition/identity or ledger content,
    /// or restored PUBLIC execution contrary to the manifest. No COMMIT sent.
    ManagedStateChanged,
}

impl<E> From<ManagedApplyError> for ManagedApplyWithError<E> {
    fn from(error: ManagedApplyError) -> Self {
        Self::Apply(error)
    }
}

impl<E> fmt::Display for ManagedApplyWithError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Apply(_) => "managed apply failed",
            Self::Application(_) => "managed application SQL refused",
            Self::Unconfirmed { .. } => "managed application transaction is unconfirmed",
            Self::ManagedStateChanged => "application SQL changed managed state",
        })
    }
}

impl<E> fmt::Debug for ManagedApplyWithError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ManagedApplyWithError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Apply(error) => Some(error),
            Self::Application(error) => Some(error),
            Self::Unconfirmed { error, .. } => Some(error),
            Self::ManagedStateChanged => None,
        }
    }
}

pub(crate) async fn migrate_and_apply_with<E>(
    lease: PgLease,
    migrator: &sqlx::migrate::Migrator,
    manifest: &ManagedObjectManifest,
    work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<(), E>,
) -> Result<AppliedManagedObjects, ManagedApplyWithError<E>> {
    apply::migrate_and_apply(
        lease,
        migrator,
        manifest,
        async |connection, manifest| protected_work(connection, manifest, work).await,
        false,
    )
    .await
}

async fn protected_work<E>(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
    work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<(), E>,
) -> Result<(), ManagedApplyWithError<E>> {
    let xid: String = sqlx::query_scalar("SELECT pg_catalog.pg_current_xact_id()::text")
        .fetch_one(&mut *connection)
        .await
        .map_err(native)?;
    let settings = catalog::RenderSettings::apply(connection)
        .await
        .map_err(native)?;
    let before = State::read(connection, manifest).await?;
    settings.restore(connection).await.map_err(native)?;
    let guard = ScopeSavepoint::begin(connection).await.map_err(|error| {
        ManagedApplyWithError::Unconfirmed {
            error,
            application: None,
        }
    })?;
    let outcome = work(&mut PgScopedSql { connection }).await;
    // Releasing the parent also discards nested application savepoints. On a
    // rejection, recover an aborted SQL statement before checking the same XID.
    let recovery = if outcome.is_ok() {
        guard.release(connection).await
    } else {
        guard.rollback(connection).await
    };
    let recovery = match recovery {
        Ok(()) => validate_transaction(connection, &xid, None).await,
        Err(error) => Err(error),
    };
    if let Err(error) = recovery {
        return Err(ManagedApplyWithError::Unconfirmed {
            error,
            application: outcome.err(),
        });
    }
    outcome.map_err(ManagedApplyWithError::Application)?;
    // Fire deferred work before the final state check, not during COMMIT after
    // the definitions/ledger have already been certified.
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(native)?;
    settings.reapply(connection).await.map_err(native)?;
    let after = State::read(connection, manifest).await?;
    if before != after || !public_policy_holds(connection, manifest).await? {
        return Err(ManagedApplyWithError::ManagedStateChanged);
    }
    validate_transaction(connection, &xid, None)
        .await
        .map_err(|error| ManagedApplyWithError::Unconfirmed {
            error,
            application: None,
        })
}

fn native(error: sqlx::Error) -> ManagedApplyError {
    ManagedApplyError::Statement(error.into())
}

/// Catalog evidence excludes routine ownership/ACLs, which are the intended
/// application changes, but includes definitions, retired identities and the
/// ledger's physical identity, shape, every evidence column and timestamp.
#[derive(PartialEq, Eq)]
struct State {
    ledger: i64,
    rows: BTreeMap<String, (String, Vec<u8>, Vec<u8>)>,
    times: BTreeMap<String, String>,
    live: BTreeMap<String, catalog::LiveObject>,
    definitions: Vec<(String, i64, [u8; 32])>,
}

impl State {
    async fn read(
        connection: &mut PgConnection,
        manifest: &ManagedObjectManifest,
    ) -> Result<Self, ManagedApplyError> {
        let (ledger, kind) = catalog::named_relation(connection, manifest.ledger())
            .await
            .map_err(native)?
            .ok_or(ManagedApplyError::LedgerShape)?;
        if kind != "r"
            || catalog::inspect_ledger_shape(connection, ledger)
                .await
                .map_err(native)?
                != catalog::LedgerShape::Exact
        {
            return Err(ManagedApplyError::LedgerShape);
        }
        let rows = catalog::load_ledger_rows(connection, manifest.ledger())
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.identity,
                    (row.kind, row.source_checksum, row.rendered_checksum),
                )
            })
            .collect();
        let times = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(format!(
            "SELECT identity, applied_at::text FROM ONLY {} LIMIT {}",
            manifest.ledger().quoted(),
            super::MAX_MANAGED_OBJECTS + 2,
        )))
        .persistent(false)
        .fetch_all(&mut *connection)
        .await
        .map_err(native)?
        .into_iter()
        .collect();
        let live = catalog::resolve_live(connection, manifest)
            .await?
            .into_iter()
            .collect();
        let definitions = apply::installed_objects(connection, manifest)
            .await?
            .into_iter()
            .map(|(object, oid, hash)| (object.identity().to_owned(), oid, hash))
            .collect();
        Ok(Self {
            ledger,
            rows,
            times,
            live,
            definitions,
        })
    }
}

async fn public_policy_holds(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
) -> Result<bool, ManagedApplyError> {
    if manifest.public_execute() != PublicExecute::Revoke {
        return Ok(true);
    }
    let live = catalog::resolve_live(connection, manifest).await?;
    let routines = manifest
        .objects()
        .iter()
        .filter(|object| object.kind().is_routine())
        .filter_map(|object| {
            live.get(object.identity())
                .filter(|live| live.kind == Some(object.kind()))
                .map(|live| live.oid)
        })
        .collect::<Vec<_>>();
    Ok(catalog::public_execute_holders(connection, &routines)
        .await
        .map_err(native)?
        .is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::ManagedObject;
    use super::*;
    use crate::verification::{QualifiedName, RoutineSignature};
    use std::error::Error;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn application_errors_retain_causes_without_formatting_them() {
        let marker = "private-application-marker";
        let errors = [
            ManagedApplyWithError::Application(std::io::Error::other(marker)),
            ManagedApplyWithError::Unconfirmed {
                error: sqlx::Error::Protocol(marker.into()).into(),
                application: Some(std::io::Error::other(marker)),
            },
            ManagedApplyWithError::Apply(ManagedApplyError::CommitUnconfirmed(
                sqlx::Error::Protocol(marker.into()).into(),
            )),
            ManagedApplyWithError::ManagedStateChanged,
        ];
        for error in &errors {
            assert!(!format!("{error} {error:?}").contains(marker));
        }
        assert_eq!(errors[0].source().unwrap().to_string(), marker);
        assert!(matches!(
            errors[2],
            ManagedApplyWithError::Apply(ManagedApplyError::CommitUnconfirmed(_))
        ));
        let ManagedApplyWithError::Unconfirmed { application, .. } = &errors[1] else {
            unreachable!()
        };
        assert_eq!(application.as_ref().unwrap().to_string(), marker);
    }

    #[test]
    fn unpolled_application_work_is_small_send_and_inert() {
        let manifest = ManagedObjectManifest::new(
            QualifiedName::new("app", "objects").unwrap(),
            PublicExecute::Revoke,
            [ManagedObject::function(
                RoutineSignature::new("app", "total", []).unwrap(),
                "CREATE OR REPLACE FUNCTION app.total() RETURNS bigint LANGUAGE sql RETURN 1",
            )
            .unwrap()],
            [],
        )
        .unwrap();
        let migrator = sqlx::migrate::Migrator::with_migrations(Vec::new());
        // No connection is needed until poll; no callback may run at creation.
        let lease = PgLease { connection: None };
        let called = AtomicBool::new(false);
        let future = lease.migrate_and_apply_with(&migrator, &manifest, async |_| {
            let bytes = [7_u8; 16 * 1024];
            tokio::task::yield_now().await;
            std::hint::black_box(bytes);
            called.store(true, Ordering::SeqCst);
            Ok::<_, std::io::Error>(())
        });
        fn is_send<T: Send>(_: &T) {}
        is_send(&future);
        assert!(std::mem::size_of_val(&future) <= 128);
        drop(future);
        assert!(!called.load(Ordering::SeqCst));
    }
}
