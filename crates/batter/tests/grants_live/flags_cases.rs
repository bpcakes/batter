//! The flags control plane's operations under their selected grants: a
//! serving login and an operator login, each exactly within its policy.

use std::time::Duration;

use super::policy::{Composition, PublicPolicy};
use super::support::{
    Fixture, Result, exec, quote, require, require_denied, require_within_policy,
};
use batter::flags::core::{
    CatalogBuilder, FlagKey, SafeState, SettingsPrefix, StaleRule, SwitchState, Toggle, Version,
    low_level::BaseBuilder,
};
use batter::flags::grants::{FlagsOperation, FlagsRole};
use batter::flags::postgres::{
    Actor, Archive, Cadence, Change, ChangeReason, KeySample, PooledStore, Precondition,
    PresenceReport, ProcessLabel, ProcessLabels, ProcessPresence, PruneLimit, Refusal, Request,
    SnapshotReport, StoreTiming, Variant, Write, WriteOutcome, WritePolicy,
};
use sqlx::PgPool;
use tokio::time::Instant;

const SERVING: &[FlagsOperation] = FlagsRole::ServingProcess.operations();
const OPERATOR: &[FlagsOperation] = FlagsRole::OperatorCli.operations();
const WRITER: &[FlagsOperation] = &[FlagsOperation::AuditedWrite];
/// Each operator read alone, so a read that needs another operation's
/// privilege fails here rather than passing under the operator's union.
const READS: [(&str, FlagsOperation); 3] = [
    ("flags_catalog_read", FlagsOperation::CatalogRead),
    ("flags_history_read", FlagsOperation::HistoryRead),
    ("flags_fleet_read", FlagsOperation::FleetRead),
];

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

fn timing() -> Result<StoreTiming> {
    let second = Duration::from_secs(1);
    Ok(StoreTiming::new(
        second * 5,
        second * 5,
        second * 5,
        second * 5,
    )?)
}

fn presence(store: &PooledStore) -> Result<ProcessPresence> {
    let (mut catalog, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
    catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()?;
    let ledger_v2 = catalog.mode_flag("ledger_v2", Toggle::Off).declare()?;
    let ledger_audit = catalog.mode_flag("ledger_audit", Toggle::Off).declare()?;
    catalog.requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)?;
    let base = BaseBuilder::new(catalog.build(), serving)?.finish();
    let labels = ProcessLabels {
        application: ProcessLabel::new("service")?,
        environment: ProcessLabel::new("hosted")?,
        build: ProcessLabel::new("1.0.0")?,
    };
    let cadence = Cadence::new(Duration::from_secs(600), Duration::from_secs(600))?;
    Ok(ProcessPresence::new(store, labels, &base, cadence))
}

/// Names a failed store step by its class and SQLSTATE, never its message.
fn step<T>(
    name: &str,
    outcome: std::result::Result<T, batter::flags::postgres::StoreError>,
) -> Result<T> {
    outcome.map_err(|error| {
        super::support::fail(&format!(
            "{name}: {:?} {:?}",
            error.class(),
            error.sqlstate()
        ))
    })
}

fn change() -> Result<Change> {
    Ok(Change::new(
        Actor::new("operator")?,
        ChangeReason::new("grant composition")?,
        Precondition::Unconditional,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn flags_serving_and_operator_logins_run_their_operations_and_nothing_more() -> Result {
    Fixture::run(async |fixture| {
        // The flags schema has a fixed name, so the case runs in a database
        // of its own, dropped whatever the case's outcome.
        let database = format!("{}_flags", fixture.names.owner);
        exec(
            &mut fixture.admin,
            format!("CREATE DATABASE {}", quote(&database)),
        )
        .await?;
        let outcome = in_database(fixture, &database).await;
        let dropped = exec(
            &mut fixture.admin,
            format!("DROP DATABASE {} WITH (FORCE)", quote(&database)),
        )
        .await;
        super::support::both(outcome, dropped)
    })
    .await
}

/// The logins of a case, in its own database.
struct Logins {
    admin: sqlx::PgConnection,
    serving: PgPool,
    operator: PgPool,
    /// The audited write alone, without the operator's catalog reads.
    writer: PgPool,
}

async fn in_database(fixture: &mut Fixture, database: &str) -> Result {
    let mut logins = provision(fixture, database).await?;
    let process = serve(&logins.serving).await?;
    judge(&logins.writer).await?;
    operate(&logins, process).await?;
    read_alone(fixture, &mut logins.admin, database).await?;
    deny(&logins).await?;
    prune_needs_its_lock_columns(fixture, &mut logins.admin, database).await?;
    // The serving login still loads: a failed prune never touches reads.
    step(
        "load after revoke",
        PooledStore::new(logins.serving.clone(), timing()?)
            .load(deadline())
            .await,
    )?;
    Ok(())
}

/// Migrates as the owner, then creates both logins exactly within policy.
async fn provision(fixture: &mut Fixture, database: &str) -> Result<Logins> {
    let owner = fixture.names.owner.clone();
    let mut admin = fixture.admin_on(database).await?;
    exec(
        &mut admin,
        format!("CREATE SCHEMA flags AUTHORIZATION {}", quote(&owner)),
    )
    .await?;
    // The migration's `CREATE SCHEMA IF NOT EXISTS` needs CREATE on the
    // database, which the owner holds only while it migrates.
    exec(
        &mut admin,
        format!(
            "GRANT CREATE ON DATABASE {} TO {}",
            quote(database),
            quote(&owner)
        ),
    )
    .await?;
    let owner_pool = fixture.pool_on(&owner, database).await?;
    step(
        "migrate",
        PooledStore::new(owner_pool, timing()?)
            .migrate(deadline())
            .await,
    )?;
    exec(
        &mut admin,
        format!(
            "REVOKE CREATE ON DATABASE {} FROM {}",
            quote(database),
            quote(&owner)
        ),
    )
    .await?;
    let mut pools = Vec::new();
    for (label, operations) in [
        ("flags_serving", SERVING),
        ("flags_operator", OPERATOR),
        ("flags_writer", WRITER),
    ] {
        let role = Composition::new(PublicPolicy::Deny)
            .with_flags(operations)
            .compile()?;
        let login = fixture.login_on(label, &role, &mut admin, database).await?;
        let pool = fixture.pool_on(&login, database).await?;
        require_within_policy(&pool, &role).await?;
        pools.push(pool);
    }
    let writer = pools
        .pop()
        .ok_or_else(|| super::support::fail("writer pool"))?;
    let operator = pools
        .pop()
        .ok_or_else(|| super::support::fail("operator pool"))?;
    let serving = pools
        .pop()
        .ok_or_else(|| super::support::fail("serving pool"))?;
    Ok(Logins {
        admin,
        serving,
        operator,
        writer,
    })
}

/// The serving selection loads, heartbeats and prunes; the process it
/// registers keeps the key declared.
async fn serve(pool: &PgPool) -> Result<ProcessPresence> {
    let store = PooledStore::new(pool.clone(), timing()?);
    step("serving load", store.load(deadline()).await)?;
    let mut process = presence(&store)?;
    let report = PresenceReport {
        snapshot: SnapshotReport::ControlPlane(Version::new(0)),
        held: KeySample::default(),
        floor_engaged: KeySample::default(),
    };
    step("heartbeat", process.heartbeat(&report, deadline()).await)?;
    step(
        "second heartbeat",
        process.heartbeat(&report, deadline()).await,
    )?;
    step(
        "prune",
        store.prune(PruneLimit::new(100)?, deadline()).await,
    )?;
    Ok(process)
}

/// The audited write alone judges a flag write against the running
/// declarer's catalog: the coordinated-flip refusal reads its flip
/// discipline, and the invariant judgement its manifest and the stored rows.
async fn judge(writer: &PgPool) -> Result {
    let store = PooledStore::new(writer.clone(), timing()?);
    let strict = async |name: &str| {
        let (_keep, interrupt) = tokio::sync::oneshot::channel();
        let write = Write::set(FlagKey::new(name)?, Variant::new("on")?, change()?);
        step(
            name,
            store
                .submit(
                    Request::Write(write),
                    WritePolicy::STRICT,
                    deadline(),
                    interrupt,
                )
                .await,
        )
    };
    require!(
        matches!(
            strict("ledger_v2").await?,
            WriteOutcome::Refused(Refusal::InvariantUnmet { .. })
        ),
        "the dependent is refused before its requirement"
    )?;
    for name in ["ledger_audit", "ledger_v2"] {
        require!(
            matches!(strict(name).await?, WriteOutcome::Written { .. }),
            "the requirement, then the dependent"
        )?;
    }
    Ok(())
}

/// The operator selection runs every operation the command issues: a first
/// write, a change, a refused and a successful archive and a restore, each
/// reaching its insert or update, and every read.
async fn operate(logins: &Logins, process: ProcessPresence) -> Result {
    let store = PooledStore::new(logins.operator.clone(), timing()?);
    step("preflight", store.preflight(deadline()).await)?;
    let halt = FlagKey::new("halt")?;
    for state in [SwitchState::Engaged, SwitchState::Released] {
        let written = step(
            "write",
            store
                .write(Write::kill(halt.clone(), state, change()?), deadline())
                .await,
        )?;
        require!(
            matches!(written, WriteOutcome::Written { .. }),
            "the operator inserts, then updates"
        )?;
    }
    let refused = step(
        "archive while declared",
        store
            .archive(Archive::new(halt.clone(), change()?), deadline())
            .await,
    )?;
    require!(
        matches!(refused, WriteOutcome::Refused(_)),
        "a declared key is not archived"
    )?;
    // The declaring process records its stop, as a deploy would stop it.
    process
        .stop(deadline())
        .await
        .map_err(|_| super::support::fail("stop"))?;
    let archived = step(
        "archive",
        store
            .archive(Archive::new(halt.clone(), change()?), deadline())
            .await,
    )?;
    require!(
        matches!(archived, WriteOutcome::WrittenUndeclared { .. }),
        "a key only a stopped process declares is archived"
    )?;
    let restored = step(
        "restore",
        store
            .restore(Archive::new(halt.clone(), change()?), deadline())
            .await,
    )?;
    require!(
        matches!(restored, WriteOutcome::WrittenUndeclared { .. }),
        "an archived key is restored"
    )?;
    require!(
        step(
            "history",
            store
                .history(Some(&halt), Version::new(0), deadline())
                .await
        )?
        .entries
        .len()
            == 4,
        "four audited changes"
    )?;
    require!(
        !step("keys", store.keys(deadline()).await)?.is_empty(),
        "catalog read"
    )?;
    step("key", store.key(&halt, deadline()).await)?;
    step("fleet", store.fleet(deadline()).await)?;
    step(
        "fleet declaring",
        store.fleet_declaring(&halt, deadline()).await,
    )?;
    step("operator load", store.load(deadline()).await)?;
    Ok(())
}

/// A login granted exactly one read operation runs that operation's reads,
/// each of which first checks the installed schema revision in its snapshot.
async fn read_alone(
    fixture: &mut Fixture,
    admin: &mut sqlx::PgConnection,
    database: &str,
) -> Result {
    let halt = FlagKey::new("halt")?;
    for (label, operation) in READS {
        let role = Composition::new(PublicPolicy::Deny)
            .with_flags(&[operation])
            .compile()?;
        let login = fixture.login_on(label, &role, admin, database).await?;
        let pool = fixture.pool_on(&login, database).await?;
        require_within_policy(&pool, &role).await?;
        let store = PooledStore::new(pool, timing()?);
        step(
            &format!("{label} preflight"),
            store.preflight(deadline()).await,
        )?;
        match operation {
            FlagsOperation::CatalogRead => {
                require!(
                    !step(&format!("{label} keys"), store.keys(deadline()).await)?.is_empty(),
                    "catalog read alone lists the keys"
                )?;
                step(&format!("{label} key"), store.key(&halt, deadline()).await)?;
            }
            FlagsOperation::HistoryRead => {
                let page = step(
                    &format!("{label} history"),
                    store
                        .history(Some(&halt), Version::new(0), deadline())
                        .await,
                )?;
                require!(
                    page.entries.len() == 4,
                    "history read alone reads the audit"
                )?;
            }
            FlagsOperation::FleetRead => {
                step(&format!("{label} fleet"), store.fleet(deadline()).await)?;
                step(
                    &format!("{label} fleet declaring"),
                    store.fleet_declaring(&halt, deadline()).await,
                )?;
            }
            other => return Err(super::support::fail(&format!("{other:?} is not a read"))),
        }
    }
    Ok(())
}

/// Neither login writes the audit or the counter outside the guarded path,
/// and neither owns anything it could disable or replace.
async fn deny(logins: &Logins) -> Result {
    for pool in [&logins.serving, &logins.operator] {
        require_denied(pool, "INSERT INTO flags.flag_audit(version, flag_name, operation, actor, reason, guard, after_state, recorded_at) VALUES (99, 'halt', 'set', 'a', 'b', 'unconditional', '{}', now())").await?;
        require_denied(
            pool,
            "ALTER TABLE flags.flag_state DISABLE TRIGGER audit_state",
        )
        .await?;
        require_denied(pool, "CREATE OR REPLACE FUNCTION flags.guard_state() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$").await?;
    }
    require_denied(
        &logins.serving,
        "UPDATE flags.flag_control SET version = version + 1",
    )
    .await
}

/// A prune login without one of its lock columns fails for want of
/// privilege. (The serving login's heartbeat columns would still authorize
/// the lock, so the dependency shows only on its own.)
async fn prune_needs_its_lock_columns(
    fixture: &mut Fixture,
    admin: &mut sqlx::PgConnection,
    database: &str,
) -> Result {
    for (table, revoked) in [
        ("flag_processes", "(expires_at)"),
        ("flag_catalogs", "(last_referenced_at)"),
    ] {
        let role = Composition::new(PublicPolicy::Deny)
            .with_flags(&[FlagsOperation::PresencePrune])
            .compile()?;
        let label = format!("flags_prune_{}", table.trim_start_matches("flag_"));
        let pruner = fixture.login_on(&label, &role, admin, database).await?;
        let store = PooledStore::new(fixture.pool_on(&pruner, database).await?, timing()?);
        step(
            "prune with every column",
            store.prune(PruneLimit::new(100)?, deadline()).await,
        )?;
        exec(
            admin,
            format!(
                "REVOKE UPDATE {revoked} ON flags.{table} FROM {}",
                quote(&pruner)
            ),
        )
        .await?;
        let refused = store.prune(PruneLimit::new(100)?, deadline()).await;
        require!(
            refused
                .as_ref()
                .err()
                .and_then(batter::flags::postgres::StoreError::sqlstate)
                .as_deref()
                == Some("42501"),
            &format!("prune without UPDATE{revoked} on {table} fails with 42501"),
        )?;
    }
    Ok(())
}
