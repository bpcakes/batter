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

/// The digest scheme is persisted: every ledger's `manifest` row holds a set
/// digest and applications compare it in release evidence. These values were
/// computed independently of `set_digest`, from the layout that
/// `ManagedObjectDigest`'s rustdoc states.
/// A failure here means the scheme changed: that is a `Breaking:` change with a
/// new tag, never a value to update in place.
#[test]
fn digest_scheme_matches_its_golden_vector() {
    const FUNCTION: &str = "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint LANGUAGE sql IMMUTABLE RETURN amount * 2;";
    const PROCEDURE: &str =
        "CREATE OR REPLACE PROCEDURE app.settle() LANGUAGE sql BEGIN ATOMIC SELECT 1; END;";
    const TRIGGER: &str = "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records FOR EACH ROW EXECUTE FUNCTION app.touch_updated_at();";
    const CONSTRAINT_TRIGGER: &str = "CREATE CONSTRAINT TRIGGER records_note_present AFTER INSERT OR UPDATE ON app.records DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION app.require_note();";
    let name = |name| Identifier::new(name).unwrap();
    let golden = |policy| {
        manifest(
            policy,
            vec![
                ManagedObject::function(total(), FUNCTION).unwrap(),
                ManagedObject::procedure(signature("app", "settle", &[]), PROCEDURE).unwrap(),
                ManagedObject::trigger(table(), name("records_touch"), TRIGGER).unwrap(),
                ManagedObject::constraint_trigger(
                    table(),
                    name("records_note_present"),
                    CONSTRAINT_TRIGGER,
                )
                .unwrap(),
            ],
            vec![
                Retirement::function(signature("app", "old_total", &["int8"])),
                Retirement::procedure(signature("app", "old_settle", &[])),
                Retirement::trigger(table(), name("records_old")),
                Retirement::constraint_trigger(table(), name("records_old_check")),
            ],
        )
    };
    let revoke = golden(PublicExecute::Revoke);
    // The inputs the independent computation hashed, in the order it hashed them.
    assert_eq!(
        revoke
            .objects()
            .iter()
            .map(|object| (object.kind().as_str(), object.identity()))
            .collect::<Vec<_>>(),
        [
            ("procedure", "\"app\".\"settle\"()"),
            ("function", "\"app\".\"total\"(\"pg_catalog\".\"int8\")"),
            (
                "constraint_trigger",
                "\"records_note_present\" ON \"app\".\"records\""
            ),
            ("trigger", "\"records_touch\" ON \"app\".\"records\""),
        ]
    );
    assert_eq!(
        revoke
            .retirements()
            .iter()
            .map(|retirement| (retirement.kind().as_str(), retirement.identity()))
            .collect::<Vec<_>>(),
        [
            ("procedure", "\"app\".\"old_settle\"()"),
            ("function", "\"app\".\"old_total\"(\"pg_catalog\".\"int8\")"),
            ("trigger", "\"records_old\" ON \"app\".\"records\""),
            (
                "constraint_trigger",
                "\"records_old_check\" ON \"app\".\"records\""
            ),
        ]
    );
    assert_eq!(
        revoke.digest().to_string(),
        "b2eb560b634ff7b07dc87092d2ea5b75e552bb4f2af48497a668e6be80c2fe47"
    );
    assert_eq!(
        golden(PublicExecute::Unmanaged).digest().to_string(),
        "5e282739ceece56b997f62edc5b5d8d9f7f9b6af8946be6eac37bda27431723d"
    );
}

/// Identities are digest input and ledger keys, so their rendering is part of
/// the persisted scheme: arguments joined by a comma and a space, `[]` after an
/// array's element type, every identifier in double quotes with an embedded
/// quote doubled. Computed independently, like the vector above.
#[test]
fn digest_scheme_pins_identity_rendering() {
    const FUNCTION: &str = "CREATE OR REPLACE FUNCTION app.\"Merge Totals\"(a bigint, b text[], c app.\"odd\"\"type\") RETURNS bigint LANGUAGE sql RETURN a;";
    const TRIGGER: &str = "CREATE OR REPLACE TRIGGER \"touch \"\"all\"\"\" BEFORE UPDATE ON app.\"Record Log\" FOR EACH ROW EXECUTE FUNCTION app.touch_updated_at();";
    let scalar = |schema, name| RoutineType::new(schema, name).unwrap();
    let array = |name| RoutineType::array("pg_catalog", name).unwrap();
    let merge = RoutineSignature::new(
        "app",
        "Merge Totals",
        [
            scalar("pg_catalog", "int8"),
            array("text"),
            scalar("app", "odd\"type"),
        ],
    )
    .unwrap();
    let old =
        RoutineSignature::new("app", "old", [array("int4"), scalar("pg_catalog", "int4")]).unwrap();
    let pinned = manifest(
        PublicExecute::Revoke,
        vec![
            ManagedObject::trigger(
                QualifiedName::new("app", "Record Log").unwrap(),
                Identifier::new("touch \"all\"").unwrap(),
                TRIGGER,
            )
            .unwrap(),
            ManagedObject::function(merge, FUNCTION).unwrap(),
        ],
        vec![Retirement::function(old)],
    );
    assert_eq!(
        pinned
            .objects()
            .iter()
            .map(ManagedObject::identity)
            .collect::<Vec<_>>(),
        [
            "\"app\".\"Merge Totals\"(\"pg_catalog\".\"int8\", \"pg_catalog\".\"text\"[], \"app\".\"odd\"\"type\")",
            "\"touch \"\"all\"\"\" ON \"app\".\"Record Log\"",
        ]
    );
    assert_eq!(
        pinned.retirements()[0].identity(),
        "\"app\".\"old\"(\"pg_catalog\".\"int4\"[], \"pg_catalog\".\"int4\")"
    );
    assert_eq!(
        pinned.digest().to_string(),
        "9d896bfc6e770472e32b8180f99f4d5dc950d379f78da05c49fe9aa9d3a98177"
    );
}

/// The `manifest` row also stores a digest of the rendered definition hashes,
/// which verification recomputes from the catalog. Its scheme is persisted
/// too: the tag bytes `batter.managed-objects.rendered.v1` and one NUL, the
/// entry count as a big-endian `u64`, then for each entry in byte order of its
/// identity the identity and the hash, each after its length as a big-endian
/// `u64`. Computed independently, like the vectors above.
#[test]
fn rendered_digest_scheme_matches_its_golden_vector() {
    let hex = |digest: [u8; 32]| Hex(&digest).to_string();
    assert_eq!(
        hex(rendered_set_digest([])),
        "6ad1aba728b8e04466c0f2bd9faff8f3178ea9c3bbc6518f242674e2946abe18"
    );
    let trigger = Sha256::digest(b"trigger definition");
    let function = Sha256::digest(b"function definition");
    assert_eq!(
        hex(rendered_set_digest([
            ("\"records_touch\" ON \"app\".\"records\"", &trigger[..]),
            ("\"app\".\"total\"(\"pg_catalog\".\"int8\")", &function[..]),
        ])),
        "f4c9d67fc608d27a9dd9f657a8b5d1839bfb700187e815e969dfdf814b449507"
    );
}
