use std::time::Duration;

use sqlx::{ConnectOptions, postgres::PgPoolOptions};

use super::*;

// Every URL carries a password so SQLx never consults a passfile here.
const LEASE: &str = "postgres://owner:ownersecret@127.0.0.1:5439/pgh_probe_1\
    ?sslmode=disable&application_name=fixture&options=-c%20search_path%3Dpublic";

fn plan(pools: Vec<PlannedPool>) -> ConnectionPlan {
    let mut declaration = ConnectionPlan::declare(0);
    for pool in pools {
        declaration.pool(pool);
    }
    declaration.finish().expect("valid plan")
}

fn secret(value: &str) -> SecretString {
    SecretString::new(value)
}

fn one() -> PgPoolOptions {
    PgPoolOptions::new().max_connections(1)
}

fn identity(options: &PgConnectOptions) -> impl PartialEq + std::fmt::Debug {
    (
        options.get_host().to_owned(),
        options.get_port(),
        options.get_socket().cloned(),
        options.get_database().map(str::to_owned),
        format!("{:?}", options.get_ssl_mode()),
        options.get_application_name().map(str::to_owned),
        options.get_options().map(str::to_owned),
    )
}

fn password(options: &PgConnectOptions) -> Option<String> {
    options.to_url_lossy().password().map(str::to_owned)
}

#[test]
fn lease_login_resolves_to_the_parsed_lease_url() {
    let expected: PgConnectOptions = LEASE.parse().unwrap();
    let resolved = plan(vec![
        PlannedPool::lease_login(one()),
        PlannedPool::lease_login(one()),
    ])
    .resolve(LEASE, "pgh_probe_1")
    .unwrap();
    assert_eq!(resolved.len(), 2);
    for (_, options) in &resolved {
        assert_eq!(identity(options), identity(&expected));
        assert_eq!(options.get_username(), "owner");
        assert_eq!(password(options).as_deref(), Some("ownersecret"));
    }
}

#[test]
fn password_login_replaces_only_credentials() {
    let expected: PgConnectOptions = LEASE.parse().unwrap();
    let resolved = plan(vec![
        PlannedPool::lease_login(one()),
        PlannedPool::password_login(
            one().max_connections(3),
            "restricted",
            secret("loginsecret"),
        ),
    ])
    .resolve(LEASE, "pgh_probe_1")
    .unwrap();
    let (pool, restricted) = &resolved[1];
    assert_eq!(pool.get_max_connections(), 3);
    assert_eq!(identity(restricted), identity(&expected));
    assert_eq!(restricted.get_username(), "restricted");
    assert_eq!(password(restricted).as_deref(), Some("loginsecret"));
    // The owner pool keeps its own credentials.
    assert_eq!(resolved[0].1.get_username(), "owner");
}

#[test]
fn socket_hosts_survive_a_password_login() {
    for url in [
        "postgres://owner:ownersecret@localhost/pgh_probe_2?host=/var/run/postgresql",
        "postgres://owner:ownersecret@%2Fvar%2Frun%2Fpostgresql/pgh_probe_2",
    ] {
        let expected: PgConnectOptions = url.parse().unwrap();
        assert!(expected.get_socket().is_some());
        let resolved = plan(vec![PlannedPool::password_login(
            one(),
            "restricted",
            secret("x1"),
        )])
        .resolve(url, "pgh_probe_2")
        .unwrap();
        assert_eq!(identity(&resolved[0].1), identity(&expected));
    }
}

#[test]
fn a_lease_url_selecting_another_database_is_refused_before_construction() {
    let url = "postgres://owner:ownersecret@127.0.0.1:5439/pgh_probe_3?dbname=postgres";
    for pool in [
        PlannedPool::lease_login(one()),
        PlannedPool::password_login(one(), "restricted", secret("loginsecret")),
    ] {
        let error = plan(vec![pool])
            .resolve(url, "pgh_probe_3")
            .expect_err("query parameter retargets the database");
        let sqlx::Error::Configuration(source) = &error else {
            panic!("configuration error required");
        };
        assert!(source.is::<LeaseDatabaseMismatch>());
        let formatted = format!("{error} {error:?} {source} {source:?}");
        for value in ["pgh_probe_3", "postgres", "owner", "secret", "127.0.0.1"] {
            assert!(!formatted.contains(value), "formatting exposed {value}");
        }
    }
}

#[test]
fn an_unparseable_lease_url_keeps_its_native_parse_error() {
    let error = plan(vec![PlannedPool::lease_login(one())])
        .resolve("not a lease url", "pgh_probe_4")
        .expect_err("unparseable URL");
    assert!(
        matches!(error, sqlx::Error::Configuration(source) if !source.is::<LeaseDatabaseMismatch>())
    );
}

#[tokio::test]
async fn every_pool_registers_before_the_first_connectivity_check() {
    // A closed loopback port: construction opens nothing, the check then fails.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("postgres://owner:ownersecret@127.0.0.1:{port}/pgh_probe_5");
    let quick = || {
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(50))
    };
    let resolved = plan(vec![
        PlannedPool::lease_login(quick()),
        PlannedPool::password_login(quick(), "restricted", secret("loginsecret")),
    ])
    .resolve(&url, "pgh_probe_5")
    .unwrap();
    let mut registry = Vec::new();
    let pools = construct(resolved, &mut registry);
    assert_eq!(registry.len(), 2);
    assert!(registry.iter().all(|pool| pool.size() == 0));
    assert!(matches!(
        check(&pools).await,
        Err(sqlx::Error::PoolTimedOut)
    ));
    // Clones share the native owner, so closing the registry closes delivered pools.
    futures_util::future::join_all(registry.iter().map(PgPool::close)).await;
    assert!(pools.iter().all(PgPool::is_closed));
}

#[tokio::test]
async fn keys_select_their_own_pool_and_never_another_plans() {
    let mut first = ConnectionPlan::declare(0);
    let owner = first.pool(PlannedPool::lease_login(one()));
    let login = first.pool(PlannedPool::password_login(
        one(),
        "restricted",
        secret("x1"),
    ));
    let first = first.finish().unwrap();
    let mut second = ConnectionPlan::declare(0);
    let foreign = second.pool(PlannedPool::lease_login(one()));
    let second = second.finish().unwrap();
    assert_ne!(first.id(), second.id());
    // A clone is the same declaration, so its keys stay valid.
    assert_eq!(first.clone().id(), first.id());
    let options: PgConnectOptions = LEASE.parse().unwrap();
    let pools: Vec<PgPool> = (0..2)
        .map(|_| one().connect_lazy_with(options.clone()))
        .collect();
    assert!(lookup(&pools, first.id(), owner).is_some_and(|pool| std::ptr::eq(pool, &pools[0])));
    assert!(lookup(&pools, first.id(), login).is_some_and(|pool| std::ptr::eq(pool, &pools[1])));
    // `foreign` has index 0 in its own plan; it must not select `owner`.
    assert!(lookup(&pools, first.id(), foreign).is_none());
    assert!(lookup(&pools[..1], second.id(), foreign).is_some());
}

#[test]
fn failed_declarations_return_invalid_budget() {
    assert!(matches!(
        ConnectionPlan::declare(0).finish(),
        Err(FixtureError::InvalidBudget)
    ));
    let mut zero = ConnectionPlan::declare(0);
    zero.pool(PlannedPool::lease_login(
        PgPoolOptions::new().max_connections(0),
    ));
    assert!(matches!(zero.finish(), Err(FixtureError::InvalidBudget)));
}
