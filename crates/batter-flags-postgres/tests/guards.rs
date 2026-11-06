//! PostgreSQL 18 guards contracts.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent database guard assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_postgres::*;
use fixture::*;
use sqlx::{Connection, PgConnection};

const META: &str = "SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','guard probe',true),set_config('batter_flags.guard','unconditional',true);";
const INSERT: &str = "INSERT INTO flags.flag_state(name,kind,value_type,state,created_version,updated_version) VALUES ('raw','mode','toggle','{\"encoding\":1,\"variant\":\"on\"}',1,1);";
async fn rejected(connection: &mut PgConnection, sql: String, state: &str) {
    let error = sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&mut *connection)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some(state),
        "guard statement failed: {error}"
    );
    sqlx::raw_sql("ROLLBACK").execute(connection).await.unwrap();
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn nonowner_writer_cannot_bypass_audit_counter_or_append_only_guards() {
    let f = Fixture::migrated().await;
    let writer = f.writer("flags_guard_writer").await;
    let mut c = writer.acquire().await.unwrap();
    let identity:(bool,bool)=sqlx::query_as("SELECT rolsuper, pg_has_role(current_user, (SELECT relowner FROM pg_class WHERE oid='flags.flag_state'::regclass),'USAGE') FROM pg_roles WHERE rolname=current_user").fetch_one(&mut *c).await.unwrap();
    assert_eq!(identity, (false, false));
    rejected(
        &mut c,
        "BEGIN; UPDATE flags.flag_control SET version = version - 1; COMMIT".into(),
        "P0001",
    )
    .await;
    rejected(
        &mut c,
        "BEGIN; UPDATE flags.flag_control SET version = version + 1; COMMIT".into(),
        "P0001",
    )
    .await;
    rejected(&mut c, format!("BEGIN; {META} {INSERT} COMMIT"), "P0001").await;
    rejected(
        &mut c,
        format!("BEGIN; UPDATE flags.flag_control SET version = version + 1; {INSERT} COMMIT"),
        "P0001",
    )
    .await;
    rejected(
        &mut c,
        format!(
            "BEGIN; {META} UPDATE flags.flag_control SET version = version + 1; {} COMMIT",
            INSERT.replace("1,1)", "1,2)")
        ),
        "P0001",
    )
    .await;
    rejected(&mut c,"INSERT INTO flags.flag_audit(version,flag_name,operation,actor,reason,guard,after_state) VALUES (500,'x','create','a','r','unconditional','{}')".into(),"42501").await;
    for statement in [
        "UPDATE flags.flag_audit SET actor = 'x'",
        "DELETE FROM flags.flag_audit",
    ] {
        rejected(&mut c, statement.into(), "42501").await;
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "BEGIN; {META} UPDATE flags.flag_control SET version = version + 1; {INSERT} COMMIT"
    )))
    .execute(&mut *c)
    .await
    .unwrap();
    // The same physical session retains empty GUC placeholders after SET LOCAL.
    rejected(&mut c,"BEGIN; UPDATE flags.flag_control SET version = version + 1; UPDATE flags.flag_state SET state='{\"encoding\":1,\"variant\":\"off\"}', updated_version=2 WHERE name='raw'; COMMIT".into(),"P0001").await;
    rejected(
        &mut c,
        format!(
            "BEGIN; {META} UPDATE flags.flag_state SET updated_version=2 WHERE name='raw'; COMMIT"
        ),
        "P0001",
    )
    .await;
    sqlx::raw_sql("GRANT TRUNCATE ON ALL TABLES IN SCHEMA flags TO flags_guard_writer")
        .execute(&f.pool)
        .await
        .unwrap();
    for table in [
        "flag_control",
        "flag_state",
        "flag_audit",
        "flag_catalogs",
        "flag_catalog_entries",
        "flag_processes",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT TRUNCATE ON flags.{table} TO flags_guard_writer"
        )))
        .execute(&f.pool)
        .await
        .unwrap();
        rejected(&mut c, format!("TRUNCATE flags.{table} CASCADE"), "P0001").await;
    }
    assert_eq!(f.head().await, 1);
    drop(c);
    writer.close().await;
    let mut owner = PgConnection::connect(f.lease.database_url()).await.unwrap();
    rejected(&mut owner,"INSERT INTO flags.flag_audit(version,flag_name,operation,actor,reason,guard,after_state) VALUES (500,'x','create','a','r','unconditional','{}')".into(),"P0001").await;
    owner.close().await.unwrap();
    f.drop_role("flags_guard_writer").await;
    f.close().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn notify_is_transactional_and_payload_is_only_bounded_counter() {
    let f = Fixture::migrated().await;
    let channel = f.store.load(deadline()).await.unwrap().channel().to_owned();
    assert!(channel.len() <= 63 && channel.starts_with("batter_flags_"));
    let long_channel:String=sqlx::query_scalar("SELECT 'batter_flags_' || left(encode(sha256(convert_to(repeat('x',63),'UTF8')),'hex'),16)")
        .fetch_one(&f.pool).await.unwrap();
    let mut listener = sqlx::postgres::PgListener::connect(f.lease.database_url())
        .await
        .unwrap();
    listener.listen(&channel).await.unwrap();
    listener.listen(&long_channel).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "BEGIN; {META} UPDATE flags.flag_control SET version=version+1; {INSERT} ROLLBACK"
    )))
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.recv())
            .await
            .is_err()
    );
    let _ = f
        .store
        .write(
            Write::explicit(
                key("committed"),
                ExplicitValue::ModeToggle(batter_flags_core::Toggle::On),
                change(None),
            ),
            deadline(),
        )
        .await
        .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.channel(), channel);
    assert_eq!(message.payload(), "1");
    assert!(i64::MAX.to_string().len() < 8000);
    drop(listener);
    f.close().await;
}
