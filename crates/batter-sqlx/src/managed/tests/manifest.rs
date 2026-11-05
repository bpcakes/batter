use super::*;

#[test]
fn digest_is_order_independent_and_covers_policy_sources_and_retirements() {
    let touch = trigger(TOUCH).unwrap();
    let base = manifest(
        PublicExecute::Revoke,
        vec![function(TOTAL).unwrap(), touch.clone()],
        vec![Retirement::function(signature("app", "old", &[]))],
    );
    let reordered = manifest(
        PublicExecute::Revoke,
        vec![touch.clone(), function(TOTAL).unwrap()],
        vec![Retirement::function(signature("app", "old", &[]))],
    );
    assert_eq!(base.digest(), reordered.digest());
    assert_eq!(base.objects(), reordered.objects());
    assert_eq!(base.objects()[0].identity(), total().as_str());
    assert_eq!(base.objects()[1].kind(), ManagedObjectKind::Trigger);

    let relocated = ManagedObjectManifest::new(
        ManagedRevision::new(1).expect("valid revision"),
        QualifiedName::new("audit", "managed_objects").unwrap(),
        PublicExecute::Revoke,
        base.objects().to_vec(),
        base.retirements().to_vec(),
    )
    .unwrap();
    assert_eq!(base.digest(), relocated.digest());
    assert_ne!(base.lock_key(), relocated.lock_key());

    for different in [
        manifest(
            PublicExecute::Unmanaged,
            base.objects().to_vec(),
            base.retirements().to_vec(),
        ),
        manifest(PublicExecute::Revoke, base.objects().to_vec(), Vec::new()),
        manifest(
            PublicExecute::Revoke,
            vec![
                touch.clone(),
                function(
                    "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
                     LANGUAGE sql IMMUTABLE RETURN amount * 3;",
                )
                .unwrap(),
            ],
            base.retirements().to_vec(),
        ),
        manifest(
            PublicExecute::Revoke,
            vec![touch],
            vec![
                Retirement::function(signature("app", "old", &[])),
                Retirement::function(total()),
            ],
        ),
    ] {
        assert_ne!(base.digest(), different.digest());
    }
    assert_eq!(base.digest().to_string().len(), 64);
    assert!(
        base.digest()
            .to_string()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn identities_checksums_and_diagnostics_are_canonical() {
    let object = trigger(TOUCH).unwrap();
    assert_eq!(
        object.identity(),
        "\"records_touch\" ON \"app\".\"records\""
    );
    assert_eq!(object.source(), TOUCH);
    assert_eq!(
        object.checksum(),
        &<[u8; 32]>::from(Sha256::digest(TOUCH.as_bytes()))
    );
    let debug = format!("{object:?}");
    assert!(debug.contains("records_touch") && !debug.contains("EXECUTE FUNCTION"));
    let retirement = Retirement::procedure(signature("app", "settle", &["int4"]));
    assert_eq!(retirement.kind(), ManagedObjectKind::Procedure);
    assert_eq!(
        retirement.identity(),
        "\"app\".\"settle\"(\"pg_catalog\".\"int4\")"
    );
    assert_eq!(ManagedObjectKind::Procedure.as_str(), "procedure");
    let error = ManagedObjectError::DuplicateObject {
        identity: "\"app\".\"total\"()".to_owned(),
    };
    assert!(error.to_string().contains("\"app\".\"total\"()"));
}
