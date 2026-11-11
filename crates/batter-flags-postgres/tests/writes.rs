//! PostgreSQL 18 writes contracts.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent contract assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_core::{SwitchState, Toggle, Version};
use batter_flags_postgres::*;
use fixture::*;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
#[allow(
    clippy::too_many_lines,
    reason = "one chronological oracle verifies audit and version continuity across every outcome"
)]
async fn all_write_and_archive_outcomes_preserve_counter_and_audit() {
    let f = Fixture::migrated().await;
    assert!(matches!(
        f.store
            .write(set("missing", "on", None), deadline())
            .await
            .unwrap(),
        WriteOutcome::Refused(Refusal::Undeclared)
    ));
    assert!(matches!(
        f.store
            .write(
                Write::explicit(
                    key("explicit"),
                    ExplicitValue::ReleaseToggle(Toggle::On),
                    change(Some(0))
                ),
                deadline()
            )
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared {
            basis: UndeclaredBasis::NeverDeclared,
            ..
        }
    ));
    for (name, presence, basis) in [
        ("silent_flag", "silent", UndeclaredBasis::NoLiveDeclarer),
        ("stopped_flag", "stopped", UndeclaredBasis::NoLiveDeclarer),
        ("orphan_flag", "orphan", UndeclaredBasis::Orphaned),
    ] {
        f.seed(name, "mode", "toggle", presence).await;
        assert!(
            matches!(f.store.write(set(name,"on",Some(0)),deadline()).await.unwrap(),WriteOutcome::WrittenUndeclared { basis: b, .. } if b == basis)
        );
    }
    f.seed("live_flag", "release", "toggle", "live").await;
    assert!(
        matches!(f.store.write(set("live_flag","on",Some(0)),deadline()).await.unwrap(),WriteOutcome::Written { version, previous: None, live_declaring: 1, .. } if version == Version::new(5))
    );
    let before = f.head().await;
    assert!(
        matches!(f.store.write(set("live_flag","on",Some(0)),deadline()).await.unwrap(),WriteOutcome::Unchanged { version } if version == Version::new(5))
    );
    assert!(
        matches!(f.store.write(set("live_flag","off",Some(0)),deadline()).await.unwrap(),WriteOutcome::RevisionMismatch { current, head } if current == Version::new(5) && head == Version::new(5))
    );
    assert!(
        matches!(f.store.write(set("live_flag","off",Some(99)),deadline()).await.unwrap(),WriteOutcome::Refused(Refusal::RevisionAboveHead { head, .. }) if head == Version::new(5))
    );
    assert!(matches!(
        f.store
            .write(
                Write::kill(key("live_flag"), SwitchState::Engaged, change(None)),
                deadline()
            )
            .await
            .unwrap(),
        WriteOutcome::Refused(Refusal::WrongKind { .. })
    ));
    assert!(
        matches!(f.store.write(set("live_flag","other",None),deadline()).await.unwrap(),WriteOutcome::Refused(Refusal::UndecodableByLiveDeclaration { builds }) if builds.len() == 1 && builds[0].presence == Presence::Live)
    );
    for name in ["live_flag", "silent_flag"] {
        assert!(
            matches!(f.store.archive(Archive::new(key(name),change(None)),deadline()).await.unwrap(),WriteOutcome::Refused(Refusal::StillDeclared { builds }) if builds.len() == 1)
        );
    }
    assert_eq!(f.head().await, before);
    assert_eq!(
        f.store
            .history(None, Version::new(0), deadline())
            .await
            .unwrap()
            .entries
            .len(),
        5
    );
    assert!(
        matches!(f.store.archive(Archive::new(key("stopped_flag"),change(None)),deadline()).await.unwrap(),WriteOutcome::WrittenUndeclared { version, .. } if version == Version::new(6))
    );
    assert!(matches!(
        f.store
            .write(set("stopped_flag", "off", None), deadline())
            .await
            .unwrap(),
        WriteOutcome::Refused(Refusal::Archived)
    ));
    assert!(matches!(
        f.store
            .archive(Archive::new(key("stopped_flag"), change(None)), deadline())
            .await
            .unwrap(),
        WriteOutcome::Unchanged { .. }
    ));
    assert_eq!(f.store.load(deadline()).await.unwrap().rows().len(), 4);
    assert!(matches!(
        f.store
            .restore(
                Archive::new(key("stopped_flag"), change(Some(5))),
                deadline()
            )
            .await
            .unwrap(),
        WriteOutcome::RevisionMismatch { .. }
    ));
    assert!(
        matches!(f.store.restore(Archive::new(key("stopped_flag"),change(Some(6))),deadline()).await.unwrap(),WriteOutcome::WrittenUndeclared { version, .. } if version == Version::new(7))
    );
    assert!(matches!(
        f.store
            .restore(Archive::new(key("stopped_flag"), change(None)), deadline())
            .await
            .unwrap(),
        WriteOutcome::Unchanged { .. }
    ));
    assert!(
        matches!(f.store.archive(Archive::new(key("absent"),change(None)),deadline()).await.unwrap(),WriteOutcome::Unchanged { version } if version == Version::new(0))
    );
    let audit = f
        .store
        .history(Some(&key("stopped_flag")), Version::new(0), deadline())
        .await
        .unwrap()
        .entries;
    assert_eq!(
        audit
            .iter()
            .map(|r| r.operation.as_str())
            .collect::<Vec<_>>(),
        ["create", "archive", "restore"]
    );
    assert_eq!(audit[2].actor, "test-actor");
    assert_eq!(audit[2].guard, "unchanged_since:6");
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn refusal_order_future_encoding_restrictions_and_fleet_regression() {
    let f = Fixture::migrated().await;
    f.seed("restricted", "release", "toggle", "orphan").await;
    sqlx::query(
        "UPDATE flags.flag_catalog_entries SET only_in = 'development' WHERE key = 'restricted'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store
            .write(set("restricted", "on", None), deadline())
            .await
            .unwrap(),
        WriteOutcome::Refused(Refusal::EnvironmentRestricted)
    ));
    f.seed("future", "release", "toggle", "live").await;
    sqlx::raw_sql("BEGIN; SELECT set_config('batter_flags.actor','fixture',true),set_config('batter_flags.reason','future encoding',true),set_config('batter_flags.guard','unconditional',true); UPDATE flags.flag_control SET version = version + 1; INSERT INTO flags.flag_state(name,kind,value_type,state,created_version,updated_version) VALUES ('future','release','toggle','{\"encoding\":2,\"variant\":\"off\"}',1,1); COMMIT;")
        .execute(&f.pool).await.unwrap();
    assert!(matches!(
        f.store
            .write(
                Write::kill(key("future"), SwitchState::Engaged, change(None)),
                deadline()
            )
            .await
            .unwrap(),
        WriteOutcome::Refused(Refusal::UnknownRowEncoding)
    ));
    f.seed("normal", "mode", "toggle", "silent").await;
    assert!(
        matches!(f.store.write(set("normal","unlisted",None),deadline()).await.unwrap(),WriteOutcome::Refused(Refusal::UndecodableByLiveDeclaration { builds }) if builds[0].presence == Presence::Silent)
    );
    sqlx::query("UPDATE flags.flag_processes SET snapshot_version=50 WHERE build='live'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        matches!(
            f.store
                .write(set("normal", "on", None), deadline())
                .await
                .unwrap(),
            WriteOutcome::Written { version, .. } | WriteOutcome::WrittenUndeclared { version, .. } if version == Version::new(51)
        ),
        "a write behind a Live process is numbered above it"
    );
    assert_eq!(f.head().await, 51);
    sqlx::query(
        "UPDATE flags.flag_processes SET snapshot_source='persisted_file' WHERE build='live'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    // A process serving a file accepts any control-plane version, so it
    // moves no number.
    let after_file = f
        .store
        .write(set("normal", "off", None), deadline())
        .await
        .unwrap();
    assert!(
        matches!(&after_file, WriteOutcome::Written { version, .. } | WriteOutcome::WrittenUndeclared { version, .. } if *version == Version::new(52)),
        "{after_file:?}"
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn concurrent_first_writes_and_poller_observe_commit_order() {
    let f = Fixture::migrated().await;
    f.seed("race", "mode", "toggle", "live").await;
    let (a, b) = tokio::join!(
        f.store.write(set("race", "on", Some(0)), deadline()),
        f.store.write(set("race", "off", Some(0)), deadline())
    );
    let results = [a.unwrap(), b.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, WriteOutcome::Written { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, WriteOutcome::RevisionMismatch { .. }))
            .count(),
        1
    );
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..20 {
        let store = f.store.clone();
        tasks.spawn(async move {
            store
                .write(
                    Write::explicit(
                        key(&format!("concurrent_{n}")),
                        ExplicitValue::ModeToggle(Toggle::On),
                        change(Some(0)),
                    ),
                    deadline(),
                )
                .await
                .unwrap()
        });
    }
    let mut last = Version::new(0);
    let mut seen = Vec::new();
    while !tasks.is_empty() {
        let rows = f
            .store
            .history(None, last, deadline())
            .await
            .unwrap()
            .entries;
        for row in rows {
            assert_eq!(row.version.get(), last.get() + 1);
            last = row.version;
            seen.push(last);
        }
        if let Some(result) = tasks.join_next().await {
            assert!(matches!(
                result.unwrap(),
                WriteOutcome::WrittenUndeclared { .. }
            ));
        }
    }
    for row in f
        .store
        .history(None, last, deadline())
        .await
        .unwrap()
        .entries
    {
        assert_eq!(row.version.get(), last.get() + 1);
        last = row.version;
        seen.push(last);
    }
    assert_eq!(seen.len(), 21);
    assert_eq!(f.head().await, 21);
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn absent_row_cas_and_lower_encoding_declarers_are_refused_without_consuming_versions() {
    let f = Fixture::migrated().await;
    f.seed("older_live", "mode", "toggle", "live").await;
    f.seed("older_silent", "mode", "toggle", "silent").await;
    sqlx::query("UPDATE flags.flag_processes SET row_encoding=0")
        .execute(&f.pool)
        .await
        .unwrap();
    for name in ["older_live", "older_silent"] {
        assert!(
            matches!(f.store.write(set(name,"on",Some(0)),deadline()).await.unwrap(), WriteOutcome::Refused(Refusal::UndecodableByLiveDeclaration { builds }) if builds.len()==1)
        );
        assert!(
            matches!(f.store.write(set(name,"on",Some(u64::MAX)),deadline()).await.unwrap(), WriteOutcome::Refused(Refusal::RevisionAboveHead { head, .. }) if head.get()==0)
        );
    }
    assert_eq!(f.head().await, 0);
    assert!(
        f.store
            .history(None, Version::new(0), deadline())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    sqlx::query("UPDATE flags.flag_processes SET stopped_at=clock_timestamp()")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .write(set("older_silent", "on", Some(0)), deadline())
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared { .. }
    ));
    // Stopped rows cannot block archive. A restarted live declarer changes restore classification.
    assert!(matches!(
        f.store
            .archive(Archive::new(key("older_silent"), change(None)), deadline())
            .await
            .unwrap(),
        WriteOutcome::WrittenUndeclared { .. }
    ));
    f.seed("older_silent", "mode", "toggle", "live").await;
    assert!(matches!(
        f.store
            .restore(Archive::new(key("older_silent"), change(None)), deadline())
            .await
            .unwrap(),
        WriteOutcome::Written {
            previous: Some(_),
            live_declaring: 1,
            ..
        }
    ));
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn restore_retains_archived_value_until_incompatible_declarers_stop() {
    for presence in [Presence::Live, Presence::Silent] {
        let f = Fixture::migrated().await;
        f.seed("archived_flag", "mode", "enum", "stopped").await;
        assert!(matches!(
            f.store.write(set("archived_flag", "express", Some(0)), deadline()).await.unwrap(),
            WriteOutcome::WrittenUndeclared { version, .. } if version.get() == 1
        ));
        assert!(matches!(
            f.store.archive(Archive::new(key("archived_flag"), change(Some(1))), deadline()).await.unwrap(),
            WriteOutcome::WrittenUndeclared { version, .. } if version.get() == 2
        ));
        // A new build declares a narrower catalog after the old build stops.
        let build = match presence {
            Presence::Live => "live",
            Presence::Silent => "silent",
        };
        f.seed("new_catalog", "mode", "enum", build).await;
        sqlx::query("UPDATE flags.flag_catalog_entries SET key='archived_flag', variants=ARRAY['off','on'] WHERE key='new_catalog'")
            .execute(&f.pool).await.unwrap();
        let expected = serde_json::json!({"encoding": 1, "variant": "express"});
        let before = f
            .store
            .history(None, Version::new(0), deadline())
            .await
            .unwrap()
            .entries;
        assert_eq!(before.len(), 2);
        assert!(matches!(
            f.store.restore(Archive::new(key("archived_flag"), change(Some(2))), deadline()).await.unwrap(),
            WriteOutcome::Refused(Refusal::UndecodableByLiveDeclaration { builds })
                if builds.len() == 1 && builds[0].build == build && builds[0].presence == presence
        ));
        let archived: (serde_json::Value, i64, i64) = sqlx::query_as(
            "SELECT state, updated_version, archived_version FROM flags.flag_state WHERE name='archived_flag'",
        ).fetch_one(&f.pool).await.unwrap();
        assert_eq!(archived, (expected.clone(), 2, 2));
        assert_eq!(f.head().await, 2);
        assert!(f.store.load(deadline()).await.unwrap().rows().is_empty());
        assert_eq!(
            f.store
                .history(None, Version::new(0), deadline())
                .await
                .unwrap()
                .entries
                .len(),
            before.len()
        );
        // The same incompatible catalog cannot block once its process stops.
        sqlx::query("UPDATE flags.flag_processes SET stopped_at=clock_timestamp() WHERE build=$1")
            .bind(build)
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(matches!(
            f.store.restore(Archive::new(key("archived_flag"), change(Some(2))), deadline()).await.unwrap(),
            WriteOutcome::WrittenUndeclared { version, basis: UndeclaredBasis::NoLiveDeclarer, .. }
                if version.get() == 3
        ));
        let restored: (serde_json::Value, i64, Option<i64>) = sqlx::query_as(
            "SELECT state, updated_version, archived_version FROM flags.flag_state WHERE name='archived_flag'",
        ).fetch_one(&f.pool).await.unwrap();
        assert_eq!(restored, (expected, 3, None));
        assert_eq!(f.head().await, 3);
        assert_eq!(f.store.load(deadline()).await.unwrap().rows().len(), 1);
        let after = f
            .store
            .history(None, Version::new(0), deadline())
            .await
            .unwrap()
            .entries;
        assert_eq!(
            after
                .iter()
                .map(|r| r.operation.as_str())
                .collect::<Vec<_>>(),
            ["create", "archive", "restore"]
        );
        f.close().await;
    }
}
