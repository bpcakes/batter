use super::*;
use crate::managed::{ManagedObject, ManagedObjectKind, Retirement};
use crate::verification::{
    AuthorityPolicyBuilder, Identifier, MigrationPolicy, PlanError, RoutineSignature,
    SchemaInspectionPolicy, SequencePolicy, SqlxLedgerMode, SqlxMigrationManifest,
    VerificationPlan,
};

fn ledger() -> QualifiedName {
    QualifiedName::new("app", "managed_objects").unwrap()
}

fn total() -> RoutineSignature {
    RoutineSignature::new("app", "total", []).unwrap()
}

fn manifest(policy: PublicExecute) -> ManagedObjectManifest {
    ManagedObjectManifest::new(
        ledger(),
        policy,
        [
            ManagedObject::function(
                total(),
                "CREATE OR REPLACE FUNCTION app.total() RETURNS bigint LANGUAGE sql RETURN 1;",
            )
            .unwrap(),
            ManagedObject::trigger(
                QualifiedName::new("app", "records").unwrap(),
                Identifier::new("records_touch").unwrap(),
                "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records
                 FOR EACH ROW EXECUTE FUNCTION app.touch();",
            )
            .unwrap(),
        ],
        [Retirement::function(
            RoutineSignature::new("app", "old_total", []).unwrap(),
        )],
    )
    .unwrap()
}

/// Ledger rows exactly as a successful apply of `manifest` records them.
fn applied_rows(manifest: &ManagedObjectManifest) -> Vec<LedgerRow> {
    let mut rows = manifest
        .objects()
        .iter()
        .enumerate()
        .map(|(index, object)| LedgerRow {
            identity: object.identity().to_owned(),
            kind: object.kind().as_str().to_owned(),
            source_checksum: object.checksum().to_vec(),
            rendered_checksum: vec![u8::try_from(index).unwrap(); 32],
        })
        .collect::<Vec<_>>();
    let rendered = managed::rendered_set_digest(
        rows.iter()
            .map(|row| (row.identity.as_str(), row.rendered_checksum.as_slice())),
    );
    rows.push(LedgerRow {
        identity: SET_ROW.to_owned(),
        kind: SET_ROW.to_owned(),
        source_checksum: manifest.digest().as_bytes().to_vec(),
        rendered_checksum: rendered.to_vec(),
    });
    rows
}

fn kinds(findings: &[Finding]) -> Vec<(FindingKind, Option<&str>)> {
    findings
        .iter()
        .map(|finding| (finding.kind, finding.object.as_deref()))
        .collect()
}

#[test]
fn an_applied_ledger_has_no_row_findings() {
    let manifest = manifest(PublicExecute::Revoke);
    let mut findings = Vec::new();
    compare_rows(&manifest, &applied_rows(&manifest), &mut findings);
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn row_comparison_reports_missing_mismatched_unexpected_and_set_rows() {
    let manifest = manifest(PublicExecute::Revoke);
    let total = manifest.objects()[0].identity().to_owned();
    let touch = manifest.objects()[1].identity().to_owned();
    let quoted = ledger().quoted();

    let mut rows = applied_rows(&manifest);
    rows.retain(|row| row.identity != touch);
    let mut findings = Vec::new();
    compare_rows(&manifest, &rows, &mut findings);
    // The set row no longer agrees with the object rows either.
    assert_eq!(
        kinds(&findings),
        [
            (FindingKind::ManagedObjectSetDigest, Some(quoted.as_str())),
            (FindingKind::MissingManagedObject, Some(touch.as_str())),
        ]
    );

    for mutate in [
        (|row: &mut LedgerRow| row.source_checksum[0] ^= 1) as fn(&mut LedgerRow),
        |row| row.kind = "procedure".to_owned(),
    ] {
        let mut rows = applied_rows(&manifest);
        mutate(rows.iter_mut().find(|row| row.identity == total).unwrap());
        let mut findings = Vec::new();
        compare_rows(&manifest, &rows, &mut findings);
        assert_eq!(
            kinds(&findings),
            [(
                FindingKind::ManagedObjectChecksumMismatch,
                Some(total.as_str())
            )]
        );
    }

    let mut rows = applied_rows(&manifest);
    rows.push(LedgerRow {
        identity: "\"app\".\"stale\"()".to_owned(),
        kind: "function".to_owned(),
        source_checksum: vec![0; 32],
        rendered_checksum: vec![0; 32],
    });
    let mut findings = Vec::new();
    compare_rows(&manifest, &rows, &mut findings);
    assert_eq!(
        kinds(&findings),
        [(
            FindingKind::UnexpectedManagedObject,
            Some("\"app\".\"stale\"()")
        )]
    );

    for mutate in [
        (|row: &mut LedgerRow| row.source_checksum[0] ^= 1) as fn(&mut LedgerRow),
        |row| row.rendered_checksum[0] ^= 1,
        |row| row.kind = "function".to_owned(),
        |row| row.identity = "set".to_owned(),
    ] {
        let mut rows = applied_rows(&manifest);
        mutate(rows.iter_mut().find(|row| row.identity == SET_ROW).unwrap());
        let mut findings = Vec::new();
        compare_rows(&manifest, &rows, &mut findings);
        assert_eq!(
            findings[0].kind,
            FindingKind::ManagedObjectSetDigest,
            "{findings:?}"
        );
    }

    // Another manifest's set digest is a set finding even when rows agree.
    let unmanaged = ManagedObjectManifest::new(
        ledger(),
        PublicExecute::Unmanaged,
        manifest.objects().to_vec(),
        manifest.retirements().to_vec(),
    )
    .unwrap();
    let mut findings = Vec::new();
    compare_rows(&unmanaged, &applied_rows(&manifest), &mut findings);
    assert_eq!(
        kinds(&findings),
        [(FindingKind::ManagedObjectSetDigest, Some(quoted.as_str()))]
    );
}

fn live(manifest: &ManagedObjectManifest) -> (HashMap<String, LiveObject>, HashMap<i64, [u8; 32]>) {
    let mut live = HashMap::new();
    let mut hashes = HashMap::new();
    for (index, object) in manifest.objects().iter().enumerate() {
        let oid = i64::try_from(index).unwrap() + 100;
        live.insert(
            object.identity().to_owned(),
            LiveObject {
                oid,
                kind: Some(object.kind()),
                touched: false,
            },
        );
        hashes.insert(oid, [u8::try_from(index).unwrap(); 32]);
    }
    (live, hashes)
}

#[test]
fn catalog_comparison_detects_drift_absence_kind_and_present_retirements() {
    let manifest = manifest(PublicExecute::Revoke);
    let rows = applied_rows(&manifest);
    let total = manifest.objects()[0].identity().to_owned();
    let (live_objects, hashes) = live(&manifest);
    let mut findings = Vec::new();
    compare_catalog(&manifest, &rows, &live_objects, &hashes, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");

    let mut changed = hashes.clone();
    changed.get_mut(&100).unwrap()[0] ^= 1;
    let mut absent = live_objects.clone();
    absent.remove(&total);
    let mut procedure = live_objects.clone();
    procedure.get_mut(&total).unwrap().kind = Some(ManagedObjectKind::Procedure);
    for (live_objects, hashes) in [
        (&live_objects, &changed),
        (&absent, &hashes),
        (&procedure, &hashes),
    ] {
        let mut findings = Vec::new();
        compare_catalog(&manifest, &rows, live_objects, hashes, &mut findings);
        assert_eq!(
            kinds(&findings),
            [(FindingKind::ManagedObjectDrift, Some(total.as_str()))]
        );
    }

    // Without a recorded row there is no recorded hash to drift from.
    let unrecorded = rows
        .iter()
        .filter(|row| row.identity != total)
        .cloned()
        .collect::<Vec<_>>();
    let mut findings = Vec::new();
    compare_catalog(&manifest, &unrecorded, &absent, &hashes, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");

    let retired = manifest.retirements()[0].identity().to_owned();
    let mut present = live_objects.clone();
    present.insert(
        retired.clone(),
        LiveObject {
            oid: 7,
            kind: Some(ManagedObjectKind::Function),
            touched: false,
        },
    );
    let mut findings = Vec::new();
    compare_catalog(&manifest, &rows, &present, &hashes, &mut findings);
    assert_eq!(
        kinds(&findings),
        [(
            FindingKind::RetiredManagedObjectPresent,
            Some(retired.as_str())
        )]
    );

    // A procedure or aggregate under the retired function's identity is a
    // different object, not the retired one.
    for kind in [Some(ManagedObjectKind::Procedure), None] {
        present.get_mut(&retired).unwrap().kind = kind;
        let mut findings = Vec::new();
        compare_catalog(&manifest, &rows, &present, &hashes, &mut findings);
        assert!(findings.is_empty(), "{findings:?}");
    }
}

#[test]
fn plans_reject_duplicate_and_conflicting_managed_ledgers() {
    let managed = manifest(PublicExecute::Revoke);
    let other = ManagedObjectManifest::new(
        QualifiedName::new("app", "other_objects").unwrap(),
        PublicExecute::Revoke,
        managed.objects().to_vec(),
        [],
    )
    .unwrap();
    assert!(matches!(
        VerificationPlan::managed_objects(&managed).with_managed_objects(&other),
        Err(PlanError::ManagedObjectsAlreadySelected)
    ));

    let sqlx = SqlxMigrationManifest::new(ledger(), SqlxLedgerMode::Exact, []).unwrap();
    let generic = MigrationPolicy::new(ledger(), []).unwrap();
    assert!(matches!(
        VerificationPlan::sqlx_migrations(&sqlx).with_managed_objects(&managed),
        Err(PlanError::ConflictingLedger)
    ));
    assert!(matches!(
        VerificationPlan::managed_objects(&managed).with_sqlx_migrations(&sqlx),
        Err(PlanError::ConflictingLedger)
    ));
    assert!(matches!(
        VerificationPlan::managed_objects(&managed).with_migrations(&generic),
        Err(PlanError::ConflictingLedger)
    ));

    let sequence = AuthorityPolicyBuilder {
        sequences: vec![SequencePolicy {
            sequence: ledger(),
            privileges: Vec::new(),
            allow_owner: false,
        }],
        ..AuthorityPolicyBuilder::default()
    }
    .build()
    .unwrap();
    assert!(matches!(
        VerificationPlan::authority(&sequence).with_managed_objects(&managed),
        Err(PlanError::ConflictingRelationKind)
    ));
    assert!(matches!(
        VerificationPlan::managed_objects(&managed).with_authority(&sequence),
        Err(PlanError::ConflictingRelationKind)
    ));

    let sqlx = SqlxMigrationManifest::new(
        QualifiedName::new("app", "_sqlx_migrations").unwrap(),
        SqlxLedgerMode::Exact,
        [],
    )
    .unwrap();
    let schema = SchemaInspectionPolicy::canonical([Identifier::new("app").unwrap()]).unwrap();
    let authority = AuthorityPolicyBuilder::default().build().unwrap();
    let combined = VerificationPlan::authority(&authority)
        .with_sqlx_migrations(&sqlx)
        .unwrap()
        .with_schema_inspection(&schema)
        .unwrap()
        .with_managed_objects(&managed)
        .unwrap();
    assert!(combined.managed_objects_manifest().is_some());
    assert!(combined.sqlx_migration().is_some());
}
