use super::policy::{ObjectPrivilege, PolicyError};
use crate::SqlxFailure;
use std::fmt;

/// The result of comparing one snapshot with the declared policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationStatus {
    /// Every requested surface was observed and no policy violation occurred.
    WithinDeclaredPolicy,
    /// At least one observed object, role or migration violates the policy.
    Violations,
    /// A requested observation was unavailable or an explicitly required
    /// unsupported surface prevented a complete conclusion.
    Incomplete,
}

/// A stable category for a safe, non-native verification finding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindingKind {
    MissingMigration,
    MigrationChecksumMismatch,
    UnsuccessfulMigration,
    UnexpectedMigration,
    DuplicateMigrationRow,
    /// The ledger exceeded the verifier's bounded row capacity.
    MigrationLedgerLimit,
    MissingLedgerColumn,
    LedgerColumnShape,
    /// Row-level security could filter migration rows and invalidate the
    /// ledger comparison.
    LedgerRowSecurity,
    RoleAttribute,
    RoleAdminOption,
    Ownership,
    Privilege,
    GrantOption,
    ProtectedParameter,
    MissingObject,
    /// A requested built-in or required custom parameter lacks observable context.
    /// Existence may be unknown or separately observed; no missing-object or
    /// missing-privilege verdict is inferred. The report is Incomplete.
    ParameterUnobservable,
    /// Effective current role lacks a required catalog privilege.
    MissingPrivilege,
    /// A SQLx ledger did not have exactly the six SQLx 0.9 columns and exact
    /// `version` primary key.
    SqlxLedgerShape,
    /// A scoped SECURITY DEFINER routine did not store exactly the required
    /// `search_path` configuration.
    SecurityDefinerSearchPath,
    /// A capability-reachable role owns the current database or an object in it.
    ReachableOwnership,
}

/// The exact non-inherited role attribute carried by a reachable execution
/// identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleAttribute {
    /// Unrestricted PostgreSQL superuser authority.
    Superuser,
    /// Permission to create databases.
    CreateDatabase,
    /// Permission to create and administer roles within PostgreSQL's rules.
    CreateRole,
    /// Permission to initiate replication and manage replication slots.
    Replication,
    /// Permission to bypass row-level security policies.
    BypassRowLevelSecurity,
    /// Membership in a catalog-known predefined role.
    PredefinedRole,
}

/// A structured finding with only policy/catalog identifiers, never a native
/// SQLx error or arbitrary server error string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    /// Stable category for this observation.
    pub kind: FindingKind,
    /// Catalog or policy object identity, when the category has one.
    /// Schema-qualified objects and columns quote each identifier component.
    /// Declared routines use canonical `RoutineSignature` rendering; security-
    /// definer configuration findings instead append `#` and the snapshot-local
    /// routine OID to the quoted schema and name so overloads remain distinct.
    /// Database, parameter and role names retain their category-specific spelling.
    pub object: Option<String>,
    /// Reachable role or PUBLIC identity, when the category has one.
    pub subject: Option<String>,
    /// Object privilege involved in the finding, when applicable.
    pub privilege: Option<ObjectPrivilege>,
    /// Exact non-inherited role attribute, when `kind` is `RoleAttribute`.
    pub role_attribute: Option<RoleAttribute>,
}

impl Finding {
    pub(crate) fn new(
        kind: FindingKind,
        object: impl Into<Option<String>>,
        subject: impl Into<Option<String>>,
        privilege: Option<ObjectPrivilege>,
    ) -> Self {
        Self {
            kind,
            object: object.into(),
            subject: subject.into(),
            privilege,
            role_attribute: None,
        }
    }

    pub(crate) fn role_attribute(role: String, attribute: RoleAttribute) -> Self {
        Self {
            kind: FindingKind::RoleAttribute,
            object: Some(role),
            subject: None,
            privilege: None,
            role_attribute: Some(attribute),
        }
    }
}

/// A PostgreSQL capability whose catalog/ACL semantics are captured by this
/// generic verifier. Application-specific migration shape and security
/// policies remain outside this list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportedSurface {
    /// Required privileges evaluated for current-role INHERIT/PUBLIC authority.
    RequiredPrivileges,
    /// Bounded discovery of supported object kinds inside selected schemas.
    ObjectDiscovery,
    /// Migration ledger row and checksum checks.
    MigrationLedger,
    /// Explicit role-membership edges and reachability.
    RoleMembership,
    /// Reachable role attributes and predefined-role policy.
    RoleAttributes,
    /// Current-database ACLs.
    DatabaseAcl,
    /// Selected schema ACLs.
    SchemaAcl,
    /// Selected relation ACLs.
    RelationAcl,
    /// Selected column ACLs.
    ColumnAcl,
    /// Selected sequence ACLs.
    SequenceAcl,
    /// Selected user-defined type ACLs.
    TypeAcl,
    /// Selected routine ACLs and structural identities.
    RoutineAcl,
    /// Protected-parameter ACLs.
    ParameterAcl,
    /// PUBLIC ACL entries for declared objects and explicit parameter ACLs.
    PublicAcl,
    /// Direct and inherited ownership authority for declared objects.
    Ownership,
    /// Exact SQLx 0.9 ledger shape and selected history.
    SqlxMigrationLedger,
    /// Exact stored `search_path` configuration on scoped definer routines.
    SecurityDefinerConfiguration,
    /// Current-database ownership reachable from the authenticated login.
    CurrentDatabaseOwnership,
}

/// A requested or known boundary that this generic checker does not prove.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedSurface {
    /// A migration ledger participates in table inheritance in the captured
    /// snapshot. Combined and migration-only verification return Incomplete.
    /// Supported standalone ledgers are read with ONLY so a later attachment
    /// cannot introduce rows from outside that snapshot's relation membership.
    InheritedMigrationLedgers,
    /// Selected temporary namespaces, their objects, or routine argument types.
    /// Session-dependent namespace authority is outside the supported model.
    /// Includes `pg_temp`, `pg_temp_*` and `pg_toast_temp_*`; any such request
    /// returns Incomplete through all verifier entrypoints before ledger reads.
    TemporaryNamespaces,
    /// Function bodies, trigger behavior, and execution context are not inferred.
    /// Stored settings outside an explicit protected schema request are also
    /// outside the report.
    SecurityDefinerBody,
    /// Extension membership, upgrade behavior and extension-specific
    /// semantics are not inspected. ACLs for an extension-owned object are
    /// still checked when the caller declares that object.
    ExtensionSemantics,
    /// Role defaults and settings outside the inspected protected-parameter
    /// ACLs are not a proof of runtime behavior.
    RoleDefaults,
    /// Foreign-server definitions and ownership.
    ForeignServers,
    /// Foreign-data-wrapper definitions and ownership.
    ForeignDataWrappers,
    /// Tablespace ownership and access.
    Tablespaces,
    /// Large-object ownership and ACLs.
    LargeObjects,
    /// Other databases in the cluster.
    OtherDatabases,
    /// Publication definitions and membership.
    Publications,
    /// Event-trigger definitions and execution.
    EventTriggers,
    /// Procedural-language installation and trust.
    Languages,
    /// The connected server is outside the PostgreSQL 18 contract.
    PostgresVersion,
    /// A requested built-in or required custom parameter lacks observable context.
    /// This includes restricted pg_settings visibility and NO_SHOW_ALL settings,
    /// whose existence alone does not establish parameter authority, and custom
    /// names that may be placeholders, reserved prefixes or hidden definitions.
    ParameterVisibility,
    /// A requested protected ledger could not be coupled to the relation locked
    /// before the snapshot, including absent-then-created and pre-cursor name-
    /// replacement races.
    UnprotectedMigrationLedger,
    /// Current-database ownership catalog evidence had an unknown form and
    /// could not safely be classified.
    OwnershipCatalog,
}

/// A successful snapshot report.
pub struct VerificationReport {
    pub(crate) status: VerificationStatus,
    pub(crate) findings: Vec<Finding>,
    pub(crate) supported: Vec<SupportedSurface>,
    pub(crate) unsupported: Vec<UnsupportedSurface>,
    pub(crate) session_user: String,
    pub(crate) current_user: String,
}

impl VerificationReport {
    pub(crate) fn new(
        findings: Vec<Finding>,
        supported: Vec<SupportedSurface>,
        mut unsupported: Vec<UnsupportedSurface>,
        session_user: String,
        current_user: String,
        required: &[super::policy::RequiredSurface],
    ) -> Self {
        let required_missing = required.iter().any(|surface| {
            unsupported.iter().any(|candidate| {
                matches!(
                    (surface, candidate),
                    (
                        super::policy::RequiredSurface::SecurityDefinerBody,
                        UnsupportedSurface::SecurityDefinerBody,
                    ) | (
                        super::policy::RequiredSurface::ExtensionSemantics,
                        UnsupportedSurface::ExtensionSemantics,
                    ) | (
                        super::policy::RequiredSurface::RoleDefaults,
                        UnsupportedSurface::RoleDefaults,
                    ) | (
                        super::policy::RequiredSurface::ForeignServers,
                        UnsupportedSurface::ForeignServers,
                    ) | (
                        super::policy::RequiredSurface::ForeignDataWrappers,
                        UnsupportedSurface::ForeignDataWrappers,
                    ) | (
                        super::policy::RequiredSurface::Tablespaces,
                        UnsupportedSurface::Tablespaces,
                    ) | (
                        super::policy::RequiredSurface::LargeObjects,
                        UnsupportedSurface::LargeObjects,
                    ) | (
                        super::policy::RequiredSurface::OtherDatabases,
                        UnsupportedSurface::OtherDatabases,
                    ) | (
                        super::policy::RequiredSurface::Publications,
                        UnsupportedSurface::Publications,
                    ) | (
                        super::policy::RequiredSurface::EventTriggers,
                        UnsupportedSurface::EventTriggers,
                    ) | (
                        super::policy::RequiredSurface::Languages,
                        UnsupportedSurface::Languages,
                    )
                )
            })
        });
        let parameter_unobservable = findings
            .iter()
            .any(|finding| finding.kind == FindingKind::ParameterUnobservable);
        if parameter_unobservable && !unsupported.contains(&UnsupportedSurface::ParameterVisibility)
        {
            unsupported.push(UnsupportedSurface::ParameterVisibility);
        }
        let inherently_incomplete = unsupported.iter().any(|surface| {
            matches!(
                surface,
                UnsupportedSurface::UnprotectedMigrationLedger
                    | UnsupportedSurface::OwnershipCatalog
            )
        });
        let status = if parameter_unobservable || inherently_incomplete {
            VerificationStatus::Incomplete
        } else if !findings.is_empty() {
            VerificationStatus::Violations
        } else if required_missing {
            VerificationStatus::Incomplete
        } else {
            VerificationStatus::WithinDeclaredPolicy
        };
        Self {
            status,
            findings,
            supported,
            unsupported,
            session_user,
            current_user,
        }
    }

    pub(crate) fn incomplete(
        unsupported: Vec<UnsupportedSurface>,
        session_user: String,
        current_user: String,
    ) -> Self {
        Self {
            status: VerificationStatus::Incomplete,
            findings: Vec::new(),
            supported: Vec::new(),
            unsupported,
            session_user,
            current_user,
        }
    }

    /// Return the policy verdict for this snapshot.
    pub fn status(&self) -> VerificationStatus {
        self.status
    }

    /// Return structured observed policy violations.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Return the explicitly bounded surfaces not proven by this checker.
    pub fn unsupported(&self) -> &[UnsupportedSurface] {
        &self.unsupported
    }

    /// Return the catalog and ACL surfaces evaluated by this report.
    pub fn supported(&self) -> &[SupportedSurface] {
        &self.supported
    }

    /// Return the initially authenticated PostgreSQL database role.
    ///
    /// Verification recovers this identity with transaction-local session
    /// authorization even when the caller changed `session_user` beforehand.
    pub fn session_user(&self) -> &str {
        &self.session_user
    }

    /// Return the effective PostgreSQL identity present when verification began.
    pub fn current_user(&self) -> &str {
        &self.current_user
    }

    /// Whether the observed snapshot satisfied the declared policy.
    pub fn is_within_declared_policy(&self) -> bool {
        self.status == VerificationStatus::WithinDeclaredPolicy
    }
}

impl fmt::Debug for VerificationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerificationReport")
            .field("status", &self.status)
            .field("finding_count", &self.findings.len())
            .field("supported_count", &self.supported.len())
            .field("unsupported_count", &self.unsupported.len())
            .field("session_user", &"<redacted>")
            .field("current_user", &"<redacted>")
            .finish()
    }
}

/// A redacted failure from validation, native SQLx, or transaction rollback.
pub enum VerificationError {
    InvalidPolicy(PolicyError),
    Native(SqlxFailure),
    /// The authenticated session identity was not present in the captured
    /// role catalog. Verification fails closed instead of treating OID zero or
    /// PUBLIC as the runtime principal.
    MissingSessionUser,
    /// A catalog query exceeded 10,000 rows or a parameter name exceeded 1,024 bytes.
    CatalogCapacity,
    /// Captured evaluation exceeded one million visits, 100,000 findings, or
    /// 16 MiB of finding payload (struct size plus object/subject string bytes).
    /// No partial report is returned; the owned checkout is retired.
    EvaluationCapacity,
    /// Checkout retained a SQLx-managed transaction.
    ConnectionState,
    Rollback {
        primary: Option<Box<Self>>,
        source: SqlxFailure,
    },
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPolicy(_) => "invalid PostgreSQL verification policy",
            Self::Native(_) => "PostgreSQL verification query failed",
            Self::ConnectionState => "verification checkout retained managed transaction state",
            Self::CatalogCapacity => "PostgreSQL verification catalog capacity exceeded",
            Self::EvaluationCapacity => "PostgreSQL verification evaluation capacity exceeded",
            Self::MissingSessionUser => "PostgreSQL session user was absent from the role catalog",
            Self::Rollback { .. } => "PostgreSQL verification rollback failed",
        })
    }
}

impl fmt::Debug for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPolicy(_) => "VerificationError::InvalidPolicy",
            Self::Native(_) => "VerificationError::Native",
            Self::ConnectionState => "VerificationError::ConnectionState",
            Self::CatalogCapacity => "VerificationError::CatalogCapacity",
            Self::EvaluationCapacity => "VerificationError::EvaluationCapacity",
            Self::MissingSessionUser => "VerificationError::MissingSessionUser",
            Self::Rollback {
                primary: Some(_), ..
            } => "VerificationError::RollbackWithPrimary",
            Self::Rollback { primary: None, .. } => "VerificationError::Rollback",
        })
    }
}

impl std::error::Error for VerificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPolicy(error) => Some(error),
            Self::Native(error) => Some(error),
            Self::MissingSessionUser
            | Self::CatalogCapacity
            | Self::EvaluationCapacity
            | Self::ConnectionState => None,
            Self::Rollback {
                primary: Some(primary),
                ..
            } => Some(primary),
            Self::Rollback {
                primary: None,
                source,
            } => Some(source),
        }
    }
}

impl VerificationError {
    /// Return the inspection failure retained alongside a rollback failure.
    ///
    /// [`std::error::Error::source`] exposes this primary cause first; this
    /// accessor preserves the rollback cause separately when both operations
    /// fail.
    pub fn primary(&self) -> Option<&VerificationError> {
        match self {
            Self::Rollback {
                primary: Some(primary),
                ..
            } => Some(primary),
            _ => None,
        }
    }

    /// Return the native rollback failure retained by a rollback result.
    pub fn rollback_source(&self) -> Option<&SqlxFailure> {
        match self {
            Self::Rollback { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn unsupported_coverage_is_incomplete_only_when_required() {
        let report = VerificationReport::new(
            Vec::new(),
            vec![SupportedSurface::RoleMembership],
            vec![UnsupportedSurface::SecurityDefinerBody],
            "session".to_owned(),
            "current".to_owned(),
            &[],
        );
        assert_eq!(report.status(), VerificationStatus::WithinDeclaredPolicy);

        let report = VerificationReport::new(
            Vec::new(),
            vec![SupportedSurface::RoleMembership],
            vec![UnsupportedSurface::SecurityDefinerBody],
            "session".to_owned(),
            "current".to_owned(),
            &[crate::verification::policy::RequiredSurface::SecurityDefinerBody],
        );
        assert_eq!(report.status(), VerificationStatus::Incomplete);
    }

    #[test]
    fn unsupported_server_version_is_always_explicitly_incomplete() {
        let report = VerificationReport::incomplete(
            vec![UnsupportedSurface::PostgresVersion],
            String::new(),
            String::new(),
        );
        assert_eq!(report.status(), VerificationStatus::Incomplete);
        assert_eq!(report.unsupported(), &[UnsupportedSurface::PostgresVersion]);
    }

    #[test]
    fn findings_make_the_report_a_violation() {
        let report = VerificationReport::new(
            vec![Finding::new(
                FindingKind::Privilege,
                Some("public.ledger".to_owned()),
                Some("public".to_owned()),
                Some(ObjectPrivilege::Select),
            )],
            Vec::new(),
            Vec::new(),
            "session".to_owned(),
            "current".to_owned(),
            &[],
        );
        assert_eq!(report.status(), VerificationStatus::Violations);
    }

    #[test]
    fn unobservable_parameter_preserves_findings_and_requires_incomplete_status() {
        let findings = vec![
            Finding::new(
                FindingKind::Privilege,
                Some("service.records".to_owned()),
                None::<String>,
                Some(ObjectPrivilege::Delete),
            ),
            Finding::new(
                FindingKind::ParameterUnobservable,
                Some("hidden_setting".to_owned()),
                Some("current".to_owned()),
                None,
            ),
        ];
        let report = VerificationReport::new(
            findings.clone(),
            Vec::new(),
            Vec::new(),
            "session".to_owned(),
            "current".to_owned(),
            &[],
        );
        assert_eq!(report.status(), VerificationStatus::Incomplete);
        assert_eq!(report.findings(), findings);
        assert_eq!(
            report.unsupported(),
            &[UnsupportedSurface::ParameterVisibility]
        );
    }

    #[test]
    fn unprotected_ledger_and_unknown_ownership_evidence_are_incomplete() {
        for surface in [
            UnsupportedSurface::UnprotectedMigrationLedger,
            UnsupportedSurface::OwnershipCatalog,
        ] {
            let report = VerificationReport::new(
                Vec::new(),
                Vec::new(),
                vec![surface],
                "session".to_owned(),
                "current".to_owned(),
                &[],
            );
            assert_eq!(report.status(), VerificationStatus::Incomplete);
        }
    }

    #[test]
    fn native_failures_are_redacted_but_remain_in_the_source_chain() {
        let error = VerificationError::Native(SqlxFailure::from(sqlx::Error::Protocol(
            "verification-secret-marker".into(),
        )));
        assert!(!format!("{error} {error:?}").contains("verification-secret-marker"));
        assert!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<SqlxFailure>())
                .is_some()
        );
    }

    #[test]
    fn rollback_retains_both_primary_and_rollback_failures() {
        let primary = VerificationError::Native(SqlxFailure::from(sqlx::Error::Protocol(
            "primary-secret".into(),
        )));
        let rollback = SqlxFailure::from(sqlx::Error::Protocol("rollback-secret".into()));
        let error = VerificationError::Rollback {
            primary: Some(Box::new(primary)),
            source: rollback,
        };
        assert!(matches!(
            error.primary(),
            Some(VerificationError::Native(_))
        ));
        assert!(error.rollback_source().is_some());
        assert!(error.source().is_some());
    }
}
