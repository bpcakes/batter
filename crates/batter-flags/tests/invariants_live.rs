//! Control-plane invariants against a disposable PostgreSQL 18 database: a
//! registered serving process holds each published flag that leaves a
//! `requires`, `conflicts` or `both_or_none` invariant unmet, or that needs
//! settings it lacked, reports each in its heartbeat's held keys, and
//! serves a governed flag's code default while its switch is engaged.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::time::Duration;

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use batter_flags::{
    control_plane::{
        ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, LoadPolicy, Presence,
        RefreshTiming, RegisteredFlags, register_in,
    },
    core::{
        CatalogBuilder, CivilDate, Expiry, FlagKey, Layer, ModeFlag, Reason, ReleaseFlag,
        SafeState, SettingsPrefix, Source, StaleRule, SwitchState, Toggle, Version,
    },
    declared,
    low_level::capture_now,
    postgres::{
        Actor, Change, ChangeReason, PooledStore, Precondition, StoreTiming, Variant, Write,
        WriteOutcome,
    },
};
use batter_sqlx::{PgSessionProfile, pool_in};
use postgres_test_harness::{HarnessConfig, PostgresHarness};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::time::Instant;

const SECOND: Duration = Duration::from_secs(1);

struct Flags {
    ledger_v2: ModeFlag<Toggle>,
    fast_lane: ModeFlag<Toggle>,
    slow_lane: ModeFlag<Toggle>,
    pair_a: ModeFlag<Toggle>,
    pair_b: ModeFlag<Toggle>,
    new_checkout: ReleaseFlag<Toggle>,
    outbound: ReleaseFlag<Toggle>,
}

/// `ledger_v2` on requires `ledger_audit` on, `fast_lane` on conflicts with
/// `slow_lane` on, `pair_a` and `pair_b` are both or none, `new_checkout` is
/// governed by `halt`, and `outbound` on needs `OUTBOUND_URL`, which this
/// process lacks.
fn resolved() -> (declared::Resolved, Flags) {
    let (mut catalog, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let expiry = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let mut toggle = |key: &str| catalog.mode_flag(key, Toggle::Off).declare().unwrap();
    let ledger_v2 = toggle("ledger_v2");
    let ledger_audit = toggle("ledger_audit");
    let fast_lane = toggle("fast_lane");
    let slow_lane = toggle("slow_lane");
    let pair_a = toggle("pair_a");
    let pair_b = toggle("pair_b");
    let halt = catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let new_checkout = catalog
        .release_flag("new_checkout", Toggle::Off, expiry)
        .governed_by(&halt)
        .declare()
        .unwrap();
    let outbound = catalog
        .release_flag("outbound", Toggle::Off, expiry)
        .requires_settings(Toggle::On, &["OUTBOUND_URL"])
        .declare()
        .unwrap();
    catalog
        .requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)
        .unwrap();
    catalog
        .conflicts(&fast_lane, Toggle::On, &slow_lane, Toggle::On)
        .unwrap();
    catalog.both_or_none(&pair_a, &pair_b).unwrap();
    let resolved = declared::resolve(
        catalog.build(),
        &SettingsSource::from_pairs([]).unwrap(),
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap();
    let flags = Flags {
        ledger_v2,
        fast_lane,
        slow_lane,
        pair_a,
        pair_b,
        new_checkout,
        outbound,
    };
    (resolved, flags)
}

fn timing() -> StoreTiming {
    StoreTiming::new(SECOND * 2, SECOND * 2, SECOND, SECOND * 2).unwrap()
}

/// Writes every state in order, accepting each hold, and returns the head.
async fn write(store: &PooledStore, states: &[(&str, &str)]) -> Version {
    let mut head = Version::new(0);
    for (name, value) in states {
        let change = Change::new(
            Actor::new("operator").unwrap(),
            ChangeReason::new("invariants test").unwrap(),
            Precondition::Unconditional,
        );
        let key = FlagKey::new(name).unwrap();
        let write = match *value {
            "engaged" => Write::kill(key, SwitchState::Engaged, change),
            "released" => Write::kill(key, SwitchState::Released, change),
            variant => Write::set(key, Variant::new(variant).unwrap(), change),
        };
        match store
            .write(write, Instant::now() + SECOND * 10)
            .await
            .unwrap()
        {
            WriteOutcome::Written { version, .. } => head = version,
            other => panic!("{name}: {other:?}"),
        }
    }
    head
}

/// Waits until the process applied `head` and its heartbeat reported it,
/// then returns the held keys that heartbeat reported.
async fn applied(flags: &RegisteredFlags, probe: &PgPool, head: Version) -> Vec<String> {
    for _ in 0..300 {
        let published = matches!(capture_now(flags).source(), Source::ControlPlane { version, .. } if version == head);
        let row: Option<(Option<i64>, Vec<String>)> = sqlx::query_as(
            "SELECT snapshot_version, held_keys FROM flags.flag_processes WHERE process_id = $1",
        )
        .bind(flags.process().as_uuid())
        .fetch_optional(probe)
        .await
        .unwrap();
        if let Some((Some(version), mut held)) = row
            && published
            && version == i64::try_from(head.get()).unwrap()
        {
            held.sort();
            return held;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the process never reported version {}", head.get());
}

/// A serving process of [`resolved`] that polls every 2 s.
async fn serving(options: PgConnectOptions) -> (RegisteredFlags, RunningSupervisor, Flags) {
    let (resolved, f) = resolved();
    let budget = CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(SECOND * 5, SECOND * 5, SECOND * 5, budget).unwrap());
    let login = options.get_username().to_owned();
    let profile = PgSessionProfile::new(
        &login,
        &login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let pool = pool_in(
        supervisor.reserve_cleanup("database").unwrap(),
        PgPoolOptions::new().max_connections(4),
        options,
        profile,
    )
    .unwrap();
    let flags = register_in(
        &mut supervisor,
        ControlPlaneStore::new(&pool, timing()),
        resolved,
        LoadPolicy::start_on_defaults(RefreshTiming::new(SECOND * 2, SECOND * 30).unwrap()),
        Presence::new(
            ApplicationName::new("service").unwrap(),
            BuildId::new("1.0.0+invariants").unwrap(),
            HeartbeatCadence::new(SECOND * 5, SECOND * 600).unwrap(),
        ),
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    (flags, running, f)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn every_relation_is_held_or_overridden_in_the_control_plane_and_reported() {
    let url = std::env::var("BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL")
        .expect("explicit disposable PostgreSQL 18 URL required");
    let harness = PostgresHarness::start(
        HarnessConfig::new("flags_invariant")
            .unwrap()
            .with_admin_database_url(url),
    )
    .await
    .unwrap();
    let lease = harness.empty_database().await.unwrap();
    let options: PgConnectOptions = lease.database_url().parse().unwrap();
    let probe = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    let store = PooledStore::new(probe.clone(), timing());
    store.migrate(Instant::now() + SECOND * 30).await.unwrap();

    let (flags, running, f) = serving(options).await;

    // Each dependent published without its partner, a conflicting pair, half
    // a both-or-none pair, a governed flag under its engaged switch and a
    // variant whose settings this process lacks.
    let head = write(
        &store,
        &[
            ("ledger_v2", "on"),
            ("slow_lane", "on"),
            ("fast_lane", "on"),
            ("pair_a", "on"),
            ("halt", "engaged"),
            ("new_checkout", "on"),
            ("outbound", "on"),
        ],
    )
    .await;
    let held = applied(&flags, &probe, head).await;
    assert_eq!(held, ["fast_lane", "ledger_v2", "outbound", "pair_a"]);
    let view = capture_now(&flags);
    for (flag, expected) in [
        (&f.ledger_v2, Toggle::Off),
        (&f.fast_lane, Toggle::Off),
        (&f.slow_lane, Toggle::On),
        (&f.pair_a, Toggle::Off),
        (&f.pair_b, Toggle::Off),
    ] {
        assert_eq!(view.evaluate(flag).value(), expected, "{flag:?}");
    }
    assert_eq!(view.evaluate(&f.outbound).value(), Toggle::Off);
    let governed = view.evaluate(&f.new_checkout);
    assert_eq!(
        (governed.value(), governed.reason()),
        (Toggle::Off, Reason::Disabled)
    );
    assert!(
        matches!(governed.layer(), Layer::Override { by } if by.as_str() == "halt"),
        "{:?}",
        governed.layer()
    );

    // Meeting every relation serves each value as written; only the
    // variant this process lacks settings for stays held.
    let head = write(
        &store,
        &[
            ("ledger_audit", "on"),
            ("slow_lane", "off"),
            ("pair_b", "on"),
            ("halt", "released"),
        ],
    )
    .await;
    let held = applied(&flags, &probe, head).await;
    assert_eq!(held, ["outbound"]);
    let view = capture_now(&flags);
    for flag in [&f.ledger_v2, &f.fast_lane, &f.pair_a, &f.pair_b] {
        assert_eq!(view.evaluate(flag).value(), Toggle::On, "{flag:?}");
    }
    assert_eq!(view.evaluate(&f.new_checkout).value(), Toggle::On);
    assert_eq!(view.evaluate(&f.outbound).value(), Toggle::Off);

    running.shutdown_checked().await.unwrap();
    probe.close().await;
    lease.cleanup().await.unwrap();
    harness.shutdown().await.unwrap();
}
