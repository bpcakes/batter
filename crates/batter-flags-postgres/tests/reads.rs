//! PostgreSQL 18 operator-read contracts: every read checks the store itself,
//! and a history read says whether more follow.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "independent database read assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_core::Version;
use batter_flags_postgres::*;
use fixture::*;

/// A store that was never migrated is refused the same way by every read and
/// by a write: schema revision 0, never a raw missing-relation error. The
/// listing and history reads used to depend on `preflight` having run first.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn every_read_and_write_refuses_an_unmigrated_store_as_revision_zero() {
    let f = Fixture::new().await;
    let behind = |operation: &str, error: StoreError| {
        assert_eq!(error.class(), ErrorClass::SchemaRevision, "{operation}");
        assert_eq!(error.installed_schema_revision(), Some(0), "{operation}");
    };
    behind(
        "preflight",
        f.store.preflight(deadline()).await.unwrap_err(),
    );
    behind("keys", f.store.keys(deadline()).await.unwrap_err());
    behind(
        "key",
        f.store
            .key(&key("fast_path"), deadline())
            .await
            .unwrap_err(),
    );
    behind(
        "history",
        f.store
            .history(None, Version::new(0), deadline())
            .await
            .unwrap_err(),
    );
    behind("fleet", f.store.fleet(deadline()).await.unwrap_err());
    behind(
        "write",
        f.store
            .write(set("fast_path", "on", None), deadline())
            .await
            .unwrap_err(),
    );
    f.close().await;
}

/// A key neither stored nor declared has no kind or value type to report.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn an_unknown_key_lists_no_kind_or_value_type() {
    let f = Fixture::migrated().await;
    let listing = f.store.key(&key("nowhere"), deadline()).await.unwrap();
    assert_eq!((listing.kind, listing.value_type), (None, None));
    assert_eq!(listing.stored, Stored::Absent);
    assert_eq!(listing.declaration, Declaration::Undeclared);
    f.close().await;
}

/// One page more than a page of audit rows: the first read says where to
/// continue, and the newest row is on the last page rather than lost.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_history_longer_than_a_page_says_where_to_continue() {
    let f = Fixture::migrated().await;
    // A catalog whose only process stopped declares the key, so every write
    // is accepted without a live declarer.
    f.seed("busy_flag", "mode", "toggle", "stopped").await;
    let writes = HISTORY_PAGE + 1;
    for write in 0..writes {
        let variant = if write % 2 == 0 { "on" } else { "off" };
        let outcome = f
            .store
            .write(set("busy_flag", variant, None), deadline())
            .await
            .unwrap();
        assert!(
            matches!(outcome, WriteOutcome::WrittenUndeclared { .. }),
            "{outcome:?}"
        );
    }
    let busy = key("busy_flag");
    let first = f
        .store
        .history(Some(&busy), Version::new(0), deadline())
        .await
        .unwrap();
    assert_eq!(first.entries.len(), HISTORY_PAGE);
    let last = first.entries.last().unwrap().version;
    assert_eq!(first.next, Continuation::After(last));
    // Each row carries when the database recorded it, in commit order.
    assert!(
        first
            .entries
            .windows(2)
            .all(|pair| pair[0].recorded_at <= pair[1].recorded_at)
    );
    let second = f
        .store
        .history(Some(&busy), last, deadline())
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.next, Continuation::Complete);
    assert_eq!(second.entries[0].version, Version::new(writes as u64));

    // Exactly a page ends complete, without an empty read to find out.
    let exact = f
        .store
        .history(Some(&busy), Version::new(1), deadline())
        .await
        .unwrap();
    assert_eq!(exact.entries.len(), HISTORY_PAGE);
    assert_eq!(exact.next, Continuation::Complete);
    f.close().await;
}
