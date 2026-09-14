use super::super::AclEntry;
use super::super::required::tests::snapshot;
use super::*;
use crate::verification::report::FindingKind;

fn report(
    snapshot: CatalogSnapshot,
    policy: &AuthorityPolicy,
) -> crate::verification::VerificationReport {
    crate::verification::authority::evaluation::tests::run(super::super::evaluate_snapshot(
        snapshot,
        policy,
        Vec::new(),
        false,
    ))
    .unwrap()
}

fn assert_kind_drift_is_reported(
    report: &crate::verification::VerificationReport,
    object: &str,
    allowed_privilege: ObjectPrivilege,
) {
    assert_eq!(
        report.status(),
        crate::verification::VerificationStatus::Violations
    );
    assert!(report.findings().iter().any(|finding| {
        finding.kind == FindingKind::MissingObject && finding.object.as_deref() == Some(object)
    }));
    assert!(!report.findings().iter().any(|finding| {
        finding.kind == FindingKind::Privilege
            && finding.object.as_deref() == Some(object)
            && finding.privilege == Some(allowed_privilege)
    }));
}

#[test]
fn catalog_kind_drift_remains_a_finding_and_uses_the_observed_kind_defaults() {
    let name = QualifiedName::new("service", "records").unwrap();
    let object = name.quoted();

    let mut table_snapshot = snapshot();
    table_snapshot.relations[0].acl.push(AclEntry {
        grantee: 1,
        privilege: ObjectPrivilege::Select,
        grant_option: false,
    });
    let mut sequence_draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
    sequence_draft.sequences.push(SequencePolicy {
        sequence: name.clone(),
        privileges: Vec::new(),
        allow_owner: false,
    });
    sequence_draft.defaults.relations.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
    let table_report = report(table_snapshot, &sequence_draft.build().unwrap());
    assert_kind_drift_is_reported(&table_report, &object, ObjectPrivilege::Select);

    let mut sequence_snapshot = snapshot();
    sequence_snapshot.relations[0].kind = "S".to_owned();
    sequence_snapshot.relations[0].acl.push(AclEntry {
        grantee: 1,
        privilege: ObjectPrivilege::Usage,
        grant_option: false,
    });
    let mut relation_draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
    relation_draft.relations.push(RelationPolicy {
        relation: name,
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    });
    relation_draft.defaults.sequences.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
    let sequence_report = report(sequence_snapshot, &relation_draft.build().unwrap());
    assert_kind_drift_is_reported(&sequence_report, &object, ObjectPrivilege::Usage);
}

#[test]
fn catalog_kind_drift_reconciles_public_and_required_targets_after_reporting_them() {
    let name = QualifiedName::new("service", "records").unwrap();
    let object = name.quoted();
    let mut table_snapshot = snapshot();
    table_snapshot.relations[0].acl.push(AclEntry {
        grantee: 1,
        privilege: ObjectPrivilege::Select,
        grant_option: false,
    });
    let mut draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
    draft.defaults.relations.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
    draft.defaults.sequences.privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
    draft.public_grants.push(PublicGrant {
        object: PublicObject::Sequence(name.clone()),
        privilege: AllowedPrivilege::new(ObjectPrivilege::Usage, false),
    });
    draft.required_privileges.push(RequiredPrivilege {
        object: PublicObject::Sequence(name),
        privilege: ObjectPrivilege::Select,
    });
    let report = report(table_snapshot, &draft.build().unwrap());

    assert_kind_drift_is_reported(&report, &object, ObjectPrivilege::Select);
    assert!(report.findings().iter().any(|finding| {
        finding.kind == FindingKind::MissingPrivilege
            && finding.object.as_deref() == Some(object.as_str())
            && finding.privilege == Some(ObjectPrivilege::Select)
    }));
}

#[test]
fn relation_and_column_targets_drift_to_a_sequence_without_expansion_failure() {
    let name = QualifiedName::new("service", "records").unwrap();
    let object = name.quoted();
    let column = Identifier::new("value").unwrap();
    let column_object = PublicObject::Column(name.clone(), column);
    let mut sequence_snapshot = snapshot();
    sequence_snapshot.relations[0].kind = "S".to_owned();
    sequence_snapshot.relations[0].acl.push(AclEntry {
        grantee: 0,
        privilege: ObjectPrivilege::Usage,
        grant_option: false,
    });
    let mut draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
    draft.defaults.sequences.public_privileges =
        vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
    draft.public_grants.push(PublicGrant {
        object: PublicObject::Relation(name.clone()),
        privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
    });
    draft.public_overrides.push(PublicAllowance {
        object: column_object.clone(),
        privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)],
    });
    draft.required_privileges.push(RequiredPrivilege {
        object: column_object,
        privilege: ObjectPrivilege::Select,
    });
    let report = report(sequence_snapshot, &draft.build().unwrap());

    assert_kind_drift_is_reported(&report, &object, ObjectPrivilege::Usage);
    assert!(report.findings().iter().any(|finding| {
        finding.kind == FindingKind::MissingPrivilege
            && finding.privilege == Some(ObjectPrivilege::Select)
    }));
}

#[test]
fn declared_scope_kind_drift_remains_an_ordinary_report_without_discovery_defaults() {
    let name = QualifiedName::new("service", "records").unwrap();
    let object = name.quoted();
    let mut sequence_snapshot = snapshot();
    sequence_snapshot.relations[0].kind = "S".to_owned();
    sequence_snapshot.relations[0].acl.push(AclEntry {
        grantee: 0,
        privilege: ObjectPrivilege::Usage,
        grant_option: false,
    });
    let mut draft = AuthorityPolicyBuilder::new(DiscoveryScope::Declared);
    draft.relations.push(RelationPolicy {
        relation: name,
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    });
    let report = report(sequence_snapshot, &draft.build().unwrap());

    assert_eq!(
        report.status(),
        crate::verification::VerificationStatus::Violations
    );
    assert!(report.findings().iter().any(|finding| {
        finding.kind == FindingKind::MissingObject
            && finding.object.as_deref() == Some(object.as_str())
    }));
    assert!(report.findings().iter().any(|finding| {
        finding.kind == FindingKind::Privilege
            && finding.object.as_deref() == Some(object.as_str())
            && finding.privilege == Some(ObjectPrivilege::Usage)
    }));
}
