use super::PgTransaction;
use super::migration;
use super::policy::{AuthorityPolicy, MigrationPolicy};
use super::policy::{ObjectPrivilege, quote_identifier};
use super::report::{SupportedSurface, UnsupportedSurface, VerificationError, VerificationReport};
use std::collections::{HashMap, HashSet, VecDeque};

mod database;
mod discovery;
mod evaluation;
mod ownership;
mod parameter_index;
mod privileges;
mod requests;
mod required;
mod selection;

use parameter_index::{ParameterIndex, index_parameters};

const PREDEFINED_ROLES: &[&str] = &[
    "pg_checkpoint",
    "pg_create_subscription",
    "pg_database_owner",
    "pg_execute_server_program",
    "pg_maintain",
    "pg_monitor",
    "pg_read_all_data",
    "pg_read_all_settings",
    "pg_read_all_stats",
    "pg_read_server_files",
    "pg_signal_autovacuum_worker",
    "pg_signal_backend",
    "pg_stat_scan_tables",
    "pg_use_reserved_connections",
    "pg_write_all_data",
    "pg_write_server_files",
];

#[derive(Clone, Debug)]
struct RoleInfo {
    oid: i64,
    name: String,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    replication: bool,
    bypassrls: bool,
}

#[derive(Clone, Copy, Debug)]
struct Membership {
    role: i64,
    member: i64,
    admin: bool,
    inherit: bool,
    set: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AclEntry {
    grantee: i64,
    privilege: ObjectPrivilege,
    grant_option: bool,
}

#[derive(Clone, Debug)]
struct RelationObject {
    oid: i64,
    schema: String,
    name: String,
    kind: String,
    owner: i64,
    acl: Vec<AclEntry>,
    columns: Vec<ColumnObject>,
}

#[derive(Clone, Debug)]
struct ColumnObject {
    name: String,
    acl: Vec<AclEntry>,
}

#[derive(Clone, Debug)]
struct SchemaObject {
    name: String,
    owner: i64,
    acl: Vec<AclEntry>,
}

#[derive(Clone, Debug)]
struct TypeObject {
    schema: String,
    name: String,
    owner: i64,
    relation_oid: i64,
    acl: std::sync::Arc<[AclEntry]>,
}

#[derive(Clone, Debug)]
struct TypeName {
    schema: String,
    name: String,
    array_element: Option<i64>,
}

#[derive(Clone, Debug)]
struct RoutineObject {
    schema: String,
    name: String,
    owner: i64,
    security_definer: bool,
    argument_types: Vec<i64>,
    acl: Vec<AclEntry>,
}

#[derive(Clone, Debug)]
struct DatabaseObject {
    name: String,
    owner: i64,
    acl: Vec<AclEntry>,
}

#[derive(Clone, Debug)]
struct ParameterObject {
    name: String,
    observable: bool,
    context: Option<String>,
    exists: bool,
    custom: bool,
    acl: Vec<AclEntry>,
}

#[derive(Clone, Debug)]
struct CatalogSnapshot {
    session_user: String,
    current_user: String,
    roles: Vec<RoleInfo>,
    memberships: Vec<Membership>,
    root: Option<i64>,
    schemas: Vec<SchemaObject>,
    relations: Vec<RelationObject>,
    types: Vec<TypeObject>,
    type_names: HashMap<i64, TypeName>,
    routines: Vec<RoutineObject>,
    database: DatabaseObject,
    parameters: database::ParameterCatalog,
}

struct RoleGraph<'a> {
    role_by_oid: HashMap<i64, &'a RoleInfo>,
    memberships_by_member: HashMap<i64, Vec<Membership>>,
    root: Option<i64>,
    database_owner: i64,
    database_owner_role: Option<i64>,
    invalid_ownership_role_catalog: bool,
    invalid_database_owner_membership: bool,
    active_role_oids: HashSet<i64>,
    active_role_infos: Vec<&'a RoleInfo>,
    capability_role_infos: Vec<&'a RoleInfo>,
    capability_role_oids: HashSet<i64>,
    active_superusers: Vec<i64>,
    admin_target_infos: Vec<&'a RoleInfo>,
}

impl<'a> RoleGraph<'a> {
    fn new(
        roles: &'a [RoleInfo],
        memberships: &[Membership],
        root: Option<i64>,
        database_owner: i64,
    ) -> Self {
        let database_owner_role = roles
            .iter()
            .find(|role| role.name == "pg_database_owner")
            .map(|role| role.oid);
        let role_oids = roles.iter().map(|role| role.oid).collect::<HashSet<_>>();
        let invalid_ownership_role_catalog = database_owner_role.is_none()
            || !role_oids.contains(&database_owner)
            || memberships
                .iter()
                .any(|edge| !role_oids.contains(&edge.role) || !role_oids.contains(&edge.member));
        // PostgreSQL 18 represents this role's sole member implicitly and
        // forbids stored memberships both into and out of it. Keep the
        // implicit edge separate from catalog evidence so a malformed row
        // cannot be mistaken for the current-database-owner exception.
        let invalid_database_owner_membership = database_owner_role.is_some_and(|role| {
            memberships
                .iter()
                .any(|edge| edge.role == role || edge.member == role)
        });
        let role_by_oid = roles.iter().map(|role| (role.oid, role)).collect();
        let mut memberships_by_member: HashMap<i64, Vec<Membership>> = HashMap::new();
        for edge in memberships.iter().copied() {
            memberships_by_member
                .entry(edge.member)
                .or_default()
                .push(edge);
        }
        let mut graph = Self {
            role_by_oid,
            memberships_by_member,
            root,
            database_owner,
            database_owner_role,
            invalid_ownership_role_catalog,
            invalid_database_owner_membership,
            active_role_oids: HashSet::new(),
            active_role_infos: Vec::new(),
            capability_role_infos: Vec::new(),
            capability_role_oids: HashSet::new(),
            active_superusers: Vec::new(),
            admin_target_infos: Vec::new(),
        };
        if let Some(root) = root {
            let (active, capabilities, admin_targets) = graph.potential_reachability(root);
            let root_is_superuser = graph.role(root).is_some_and(|role| role.superuser);
            graph.active_role_infos = roles
                .iter()
                .filter(|role| {
                    if root_is_superuser {
                        role.oid == root
                    } else {
                        active.contains(&role.oid)
                    }
                })
                .collect();
            graph.capability_role_infos = roles
                .iter()
                .filter(|role| {
                    if root_is_superuser {
                        role.oid == root
                    } else {
                        capabilities.contains(&role.oid)
                    }
                })
                .collect();
            graph.admin_target_infos = roles
                .iter()
                .filter(|role| admin_targets.contains(&role.oid))
                .collect();
            graph.capability_role_oids = graph
                .capability_role_infos
                .iter()
                .map(|role| role.oid)
                .collect();
            graph.active_superusers = graph
                .active_role_infos
                .iter()
                .filter(|role| role.superuser)
                .map(|role| role.oid)
                .collect();
            graph.active_role_oids = active;
        }
        graph
    }

    /// Only ACL grantees, the owner and active superusers can contribute ordinary
    /// object authority. Keep role-attribute and parameter-context checks separate.
    fn acl_sources(
        &self,
        grantees: impl Iterator<Item = i64>,
        owner: Option<i64>,
    ) -> Vec<&RoleInfo> {
        let mut oids: Vec<_> = grantees
            .chain(owner)
            .chain(self.active_superusers.iter().copied())
            .filter(|oid| self.capability_role_oids.contains(oid))
            .collect();
        oids.sort_unstable();
        oids.dedup();
        oids.into_iter().filter_map(|oid| self.role(oid)).collect()
    }

    fn potential_reachability(&self, root: i64) -> (HashSet<i64>, HashSet<i64>, HashSet<i64>) {
        let mut active = HashSet::new();
        let mut capabilities = HashSet::new();
        let mut management = HashSet::new();
        let mut admin_targets = HashSet::new();
        let mut active_queue = VecDeque::from([root]);
        let mut capability_queue = VecDeque::new();
        let mut management_queue = VecDeque::new();

        while !(active_queue.is_empty()
            && capability_queue.is_empty()
            && management_queue.is_empty())
        {
            while let Some(member) = active_queue.pop_front() {
                if !active.insert(member) {
                    continue;
                }
                capability_queue.push_back(member);
                if self.role(member).is_some_and(|role| role.createrole) {
                    management_queue.push_back(member);
                }
                if member == self.database_owner
                    && let Some(database_owner_role) = self.database_owner_role
                {
                    active_queue.push_back(database_owner_role);
                }
                for edge in self
                    .memberships_by_member
                    .get(&member)
                    .into_iter()
                    .flatten()
                    .filter(|edge| edge.set)
                {
                    active_queue.push_back(edge.role);
                }
            }

            while let Some(member) = capability_queue.pop_front() {
                if !capabilities.insert(member) {
                    continue;
                }
                if member == self.database_owner
                    && let Some(database_owner_role) = self.database_owner_role
                {
                    capability_queue.push_back(database_owner_role);
                }
                for edge in self
                    .memberships_by_member
                    .get(&member)
                    .into_iter()
                    .flatten()
                {
                    if edge.inherit {
                        capability_queue.push_back(edge.role);
                    }
                    if edge.admin
                        && self.usable_admin_target(edge.role)
                        && admin_targets.insert(edge.role)
                    {
                        // The holder can grant this target back to the login
                        // with SET and INHERIT enabled.
                        active_queue.push_back(edge.role);
                    }
                }
            }

            while let Some(member) = management_queue.pop_front() {
                if !management.insert(member) {
                    continue;
                }
                if member == self.database_owner
                    && let Some(database_owner_role) = self.database_owner_role
                {
                    management_queue.push_back(database_owner_role);
                }
                for edge in self
                    .memberships_by_member
                    .get(&member)
                    .into_iter()
                    .flatten()
                {
                    management_queue.push_back(edge.role);
                    if edge.admin
                        && self.usable_admin_target(edge.role)
                        && admin_targets.insert(edge.role)
                    {
                        active_queue.push_back(edge.role);
                    }
                }
            }
        }
        (active, capabilities, admin_targets)
    }

    fn usable_admin_target(&self, oid: i64) -> bool {
        // PostgreSQL reserves grants and revocations of membership in a
        // superuser role to an already active superuser. An ADMIN-only edge
        // cannot be used to grant that target back with SET or INHERIT.
        self.role(oid).is_some_and(|role| !role.superuser)
    }

    fn role(&self, oid: i64) -> Option<&RoleInfo> {
        self.role_by_oid.get(&oid).copied()
    }

    fn active_roles(&self) -> &[&'a RoleInfo] {
        &self.active_role_infos
    }

    fn is_active_role(&self, oid: i64) -> bool {
        let Some(root) = self.root else {
            return false;
        };
        if self.role(root).is_some_and(|role| role.superuser) {
            return oid == root;
        }
        self.active_role_oids.contains(&oid)
    }

    fn capability_roles(&self) -> &[&'a RoleInfo] {
        &self.capability_role_infos
    }

    fn is_capability_role(&self, oid: i64) -> bool {
        self.capability_role_oids.contains(&oid)
    }

    fn role_name(&self, oid: i64) -> Option<&str> {
        self.role(oid).map(|role| role.name.as_str())
    }

    fn predefined_roles(&self) -> impl Iterator<Item = &'a RoleInfo> + '_ {
        self.capability_role_infos.iter().copied()
    }

    fn admin_target_roles(&self) -> &[&'a RoleInfo] {
        &self.admin_target_infos
    }

    fn has_active_superuser(&self) -> bool {
        self.active_role_infos.iter().any(|role| role.superuser)
    }

    fn has_unrecorded_ownership_capability(&self) -> bool {
        self.invalid_ownership_role_catalog
            || self.invalid_database_owner_membership
            || self.capability_role_infos.iter().any(|role| {
                role.superuser
                    || (Some(role.oid) != self.database_owner_role
                        && PREDEFINED_ROLES.contains(&role.name.as_str()))
            })
    }

    #[cfg(test)]
    fn can_admin_any_role(&self) -> bool {
        self.has_active_superuser() || !self.admin_target_infos.is_empty()
    }
}

pub(super) async fn inspect_identities(
    transaction: &mut PgTransaction<'_>,
) -> Result<(String, String), VerificationError> {
    let (entry_session_user, current_user): (String, String) =
        sqlx::query_as("SELECT session_user::text, current_user::text")
            .fetch_one(&mut **transaction)
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
    // `session_user` can be masked by a superuser with SET SESSION
    // AUTHORIZATION. Restoring DEFAULT locally exposes the initially
    // authenticated database role, and rollback restores the caller's entry
    // state even when it was deliberately masked.
    sqlx::query("SET LOCAL SESSION AUTHORIZATION DEFAULT")
        .execute(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    let session_user: String = sqlx::query_scalar("SELECT session_user::text")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    // Catalog and migration visibility belongs to the effective identity at
    // the API boundary. Reconstruct that transaction-local state after
    // recovering the authenticated login, including a prior SET ROLE.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET LOCAL SESSION AUTHORIZATION {}",
        quote_identifier(&entry_session_user)
    )))
    .execute(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    if current_user != entry_session_user {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            quote_identifier(&current_user)
        )))
        .execute(&mut **transaction)
        .await
        .map_err(|error| VerificationError::Native(error.into()))?;
    }
    Ok((session_user, current_user))
}

/// Inspect all generic surfaces in one explicit transaction snapshot.
pub(crate) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    policy: &AuthorityPolicy,
    migration: Option<(&MigrationPolicy, migration::LedgerLock)>,
) -> Result<VerificationReport, VerificationError> {
    let mut evaluation = evaluation::Evaluation::new();
    inspect_with_evaluation(transaction, policy, migration, &mut evaluation).await
}

pub(crate) async fn inspect_with_evaluation(
    transaction: &mut PgTransaction<'_>,
    policy: &AuthorityPolicy,
    migration: Option<(&MigrationPolicy, migration::LedgerLock)>,
    evaluation: &mut evaluation::Evaluation,
) -> Result<VerificationReport, VerificationError> {
    let (session_user, current_user) = inspect_identities(transaction).await?;

    let mut findings = Vec::new();
    let includes_migrations = migration.is_some();
    if let Some((migration_policy, ledger_lock)) = migration {
        let unprotected =
            inspect_ledger(transaction, migration_policy, ledger_lock, &mut findings).await?;
        if unprotected {
            return Ok(VerificationReport::incomplete(
                vec![UnsupportedSurface::UnprotectedMigrationLedger],
                session_user,
                current_user,
            ));
        }
    }
    let snapshot = CatalogSnapshot::load(transaction, policy, session_user, current_user).await?;
    evaluate_snapshot_with(snapshot, policy, findings, includes_migrations, evaluation).await
}

/// The same production evaluator is used by loaded-snapshot runtime oracles.
#[cfg(test)]
async fn evaluate_snapshot(
    snapshot: CatalogSnapshot,
    policy: &AuthorityPolicy,
    findings: Vec<super::report::Finding>,
    includes_migrations: bool,
) -> Result<VerificationReport, VerificationError> {
    let evaluation = &mut evaluation::Evaluation::new();
    evaluate_snapshot_with(snapshot, policy, findings, includes_migrations, evaluation).await
}

async fn evaluate_snapshot_with(
    snapshot: CatalogSnapshot,
    policy: &AuthorityPolicy,
    mut findings: Vec<super::report::Finding>,
    includes_migrations: bool,
    evaluation: &mut evaluation::Evaluation,
) -> Result<VerificationReport, VerificationError> {
    let parameters = index_parameters(&snapshot.parameters);
    requests::inspect_requested_objects(evaluation, &snapshot, &parameters, policy, &mut findings)
        .await?;
    required::inspect(evaluation, &snapshot, &parameters, policy, &mut findings).await?;
    let expanded_authority = discovery::expand(evaluation, &snapshot, policy).await?;
    let Some(root) = snapshot.root else {
        return Err(VerificationError::MissingSessionUser);
    };
    let graph = RoleGraph::new(
        &snapshot.roles,
        &snapshot.memberships,
        Some(root),
        snapshot.database.owner,
    );
    requests::inspect_roles(evaluation, &graph, &expanded_authority, &mut findings).await?;
    requests::inspect_admin_options(evaluation, &graph, &expanded_authority, &mut findings).await?;
    requests::inspect_database(
        evaluation,
        &snapshot,
        &graph,
        &expanded_authority,
        &mut findings,
    )
    .await?;
    requests::inspect_schemas(
        evaluation,
        &snapshot,
        &graph,
        &expanded_authority,
        &mut findings,
    )
    .await?;
    requests::inspect_relations(
        evaluation,
        &snapshot,
        &graph,
        &expanded_authority,
        &mut findings,
    )
    .await?;
    requests::inspect_types(
        evaluation,
        &snapshot,
        &graph,
        &expanded_authority,
        &mut findings,
    )
    .await?;
    requests::inspect_routines(
        evaluation,
        &snapshot,
        &graph,
        &expanded_authority,
        &mut findings,
    )
    .await?;
    requests::inspect_parameters(
        evaluation,
        &snapshot,
        &parameters,
        &graph,
        &expanded_authority,
        policy.roles.allow_superuser,
        &mut findings,
    )
    .await?;

    evaluation.checkpoint(&findings).await?;
    Ok(build_report(
        snapshot,
        policy,
        findings,
        includes_migrations,
    ))
}

fn build_report(
    snapshot: CatalogSnapshot,
    policy: &AuthorityPolicy,
    findings: Vec<super::report::Finding>,
    includes_migrations: bool,
) -> VerificationReport {
    let mut supported = vec![
        SupportedSurface::RoleMembership,
        SupportedSurface::RoleAttributes,
        SupportedSurface::DatabaseAcl,
        SupportedSurface::SchemaAcl,
        SupportedSurface::RelationAcl,
        SupportedSurface::ColumnAcl,
        SupportedSurface::SequenceAcl,
        SupportedSurface::TypeAcl,
        SupportedSurface::RoutineAcl,
        SupportedSurface::ParameterAcl,
        SupportedSurface::PublicAcl,
        SupportedSurface::Ownership,
    ];
    if includes_migrations {
        supported.push(SupportedSurface::MigrationLedger);
    }
    if !policy.required_privileges.is_empty() {
        supported.push(SupportedSurface::RequiredPrivileges);
    }
    if !matches!(policy.discovery, super::policy::DiscoveryScope::Declared) {
        supported.push(SupportedSurface::ObjectDiscovery);
    }
    VerificationReport::new(
        findings,
        supported,
        vec![
            UnsupportedSurface::SecurityDefinerBody,
            UnsupportedSurface::ExtensionSemantics,
            UnsupportedSurface::RoleDefaults,
            UnsupportedSurface::ForeignServers,
            UnsupportedSurface::ForeignDataWrappers,
            UnsupportedSurface::Tablespaces,
            UnsupportedSurface::LargeObjects,
            UnsupportedSurface::OtherDatabases,
            UnsupportedSurface::Publications,
            UnsupportedSurface::EventTriggers,
            UnsupportedSurface::Languages,
        ],
        snapshot.session_user,
        snapshot.current_user,
        &policy.required_surfaces,
    )
}

async fn inspect_ledger(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
    ledger_lock: migration::LedgerLock,
    findings: &mut Vec<super::report::Finding>,
) -> Result<bool, VerificationError> {
    match ledger_lock {
        migration::LedgerLock::Ready(protected) => {
            return migration::inspect(&mut *transaction, policy, protected, findings).await;
        }
        migration::LedgerLock::Missing | migration::LedgerLock::WrongKind => {
            findings.push(super::report::Finding::new(
                if matches!(ledger_lock, migration::LedgerLock::Missing) {
                    super::report::FindingKind::MissingMigration
                } else {
                    super::report::FindingKind::LedgerColumnShape
                },
                Some(policy.ledger.quoted()),
                None::<String>,
                None,
            ))
        }
        migration::LedgerLock::Unprotected => {
            unreachable!("unprotected ledgers return before inspection")
        }
    }
    Ok(false)
}

pub(crate) async fn inspect_migrations(
    transaction: &mut PgTransaction<'_>,
    policy: &MigrationPolicy,
    ledger_lock: migration::LedgerLock,
) -> Result<VerificationReport, VerificationError> {
    let (session_user, current_user) = inspect_identities(transaction).await?;
    let mut findings = Vec::new();
    if inspect_ledger(transaction, policy, ledger_lock, &mut findings).await? {
        return Ok(VerificationReport::incomplete(
            vec![UnsupportedSurface::UnprotectedMigrationLedger],
            session_user,
            current_user,
        ));
    }
    Ok(VerificationReport::new(
        findings,
        vec![SupportedSurface::MigrationLedger],
        Vec::new(),
        session_user,
        current_user,
        &[],
    ))
}

pub(super) fn supports_postgres_18(server_version_num: i32) -> bool {
    (180_000..190_000).contains(&server_version_num)
}

pub(crate) use evaluation::Evaluation;
pub(crate) use ownership::inspect as inspect_ownership;
#[cfg(test)]
mod tests;
