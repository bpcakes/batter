use super::super::super::policy::is_valid_custom_parameter_name;
use super::super::super::work::WorkSet;
use super::super::parameter_index::ParameterWork;
use super::*;

#[derive(Clone, Debug, Default)]
pub(in crate::verification::authority) struct ParameterCatalog {
    objects: Vec<ParameterObject>,
    work: ParameterWork,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::verification::authority) struct DuplicateParameterName;

impl ParameterCatalog {
    #[cfg(test)]
    pub(in crate::verification::authority) fn try_from_objects(
        objects: Vec<ParameterObject>,
    ) -> Result<Self, DuplicateParameterName> {
        let mut names = HashSet::with_capacity(objects.len());
        for object in &objects {
            if !names.insert(object.name.as_str()) {
                return Err(DuplicateParameterName);
            }
        }
        Ok(Self {
            objects,
            work: ParameterWork::default(),
        })
    }

    #[cfg(test)]
    pub(in crate::verification::authority) fn with_work(mut self, work: ParameterWork) -> Self {
        self.work = work;
        self
    }

    pub(in crate::verification::authority) fn iter(
        &self,
    ) -> impl Iterator<Item = &ParameterObject> {
        self.objects.iter().inspect(|_| self.record_work())
    }

    pub(in crate::verification::authority) fn record_work(&self) {
        self.work.record();
    }

    pub(in crate::verification::authority) fn len(&self) -> usize {
        self.objects.len()
    }
}

struct ParameterCatalogBuilder {
    names: WorkSet<String, ParameterWork>,
    catalog: ParameterCatalog,
}

impl ParameterCatalogBuilder {
    fn with_capacity(capacity: usize, work: ParameterWork) -> Self {
        Self {
            names: WorkSet::with_capacity(capacity, work.clone()),
            catalog: ParameterCatalog {
                objects: Vec::with_capacity(capacity),
                work,
            },
        }
    }

    fn insert_if_absent(&mut self, object: ParameterObject) {
        self.catalog.record_work();
        if self.names.insert(object.name.clone()) {
            self.catalog.objects.push(object);
        }
    }

    fn finish(self) -> ParameterCatalog {
        self.catalog
    }
}

/// pg_settings visibility belongs to the effective role, not to SET-reachable
/// identities or the originally authenticated role's dormant privileges.
pub(super) fn settings_visible(
    roles: &[RoleInfo],
    memberships: &[Membership],
    current_user: &str,
) -> bool {
    let Some(current) = roles.iter().find(|role| role.name == current_user) else {
        return false;
    };
    if current.superuser {
        return true;
    }
    let Some(settings) = roles
        .iter()
        .find(|role| role.name == "pg_read_all_settings")
    else {
        return false;
    };
    let mut by_member: HashMap<i64, Vec<i64>> = HashMap::new();
    for edge in memberships.iter().filter(|edge| edge.inherit) {
        by_member.entry(edge.member).or_default().push(edge.role);
    }
    let mut pending = vec![current.oid];
    let mut visited = HashSet::new();
    while let Some(member) = pending.pop() {
        if member == settings.oid {
            return true;
        }
        if visited.insert(member) {
            pending.extend(by_member.get(&member).into_iter().flatten().copied());
        }
    }
    false
}

pub(super) async fn load_parameters(
    transaction: &mut PgTransaction<'_>,
    requested: &[String],
    settings_visible: bool,
) -> Result<ParameterCatalog, VerificationError> {
    let rows = sqlx::query_as::<_, (Option<String>, i64, String, bool)>(
        "SELECT CASE WHEN octet_length(p.parname) <= 1024 THEN translate(p.parname, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')::text END, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_parameter_acl AS p
         CROSS JOIN LATERAL pg_catalog.aclexplode(p.paracl) AS acl LIMIT 10001",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into())).and_then(bounded_rows)?;
    let mut grouped: HashMap<String, Vec<AclEntry>> = HashMap::new();
    for (name, grantee, privilege, grant_option) in rows {
        let name = name.ok_or(VerificationError::CatalogCapacity)?;
        if let Some(privilege) = map_privilege(AclKind::Parameter, &privilege) {
            grouped.entry(name).or_default().push(AclEntry {
                grantee,
                privilege,
                grant_option,
            });
        }
    }
    let mut selected_names = requested.iter().cloned().collect::<HashSet<_>>();
    selected_names.extend(grouped.keys().cloned());
    if selected_names.is_empty() {
        return Ok(ParameterCatalog::default());
    }
    let mut selected_names = selected_names.into_iter().collect::<Vec<_>>();
    selected_names.sort();
    let settings = sqlx::query_as::<_, (String, String)>(
        "SELECT translate(name, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')::text, context::text
         FROM pg_catalog.pg_settings
         WHERE translate(name, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz') = ANY($1::text[]) LIMIT 10001",
    )
    .bind(&selected_names)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?
    .into_iter()
    .collect::<HashMap<_, _>>();
    // NO_SHOW_ALL settings remain absent even with full settings visibility.
    // Probe only existence, never transfer values or infer parameter context.
    let mut presence = HashMap::new();
    if settings_visible {
        let absent = selected_names
            .iter()
            .filter(|name| !settings.contains_key(*name) && !is_valid_custom_parameter_name(name))
            .cloned()
            .collect::<Vec<_>>();
        if !absent.is_empty() {
            presence = sqlx::query_as::<_, (String, bool)>(
                "SELECT requested.name, pg_catalog.current_setting(requested.name, true) IS NOT NULL
                 FROM pg_catalog.unnest($1::text[]) AS requested(name) LIMIT 10001"
            ).bind(&absent).fetch_all(&mut **transaction).await
                .map_err(|error| VerificationError::Native(error.into())).and_then(bounded_rows)?
                .into_iter().collect();
        }
    }
    Ok(build_parameter_objects(
        grouped,
        settings,
        selected_names,
        presence,
    ))
}

fn build_parameter_objects(
    grouped: HashMap<String, Vec<AclEntry>>,
    settings: HashMap<String, String>,
    selected_names: Vec<String>,
    presence: HashMap<String, bool>,
) -> ParameterCatalog {
    build_parameter_objects_with_work(
        grouped,
        settings,
        selected_names,
        presence,
        ParameterWork::default(),
    )
}

fn build_parameter_objects_with_work(
    grouped: HashMap<String, Vec<AclEntry>>,
    settings: HashMap<String, String>,
    selected_names: Vec<String>,
    presence: HashMap<String, bool>,
    work: ParameterWork,
) -> ParameterCatalog {
    let capacity = selected_names.len().max(grouped.len()).max(settings.len());
    // Only this builder can finish a production catalog. Its private name set
    // makes duplicate captured identities unrepresentable after the merge.
    let mut catalog = ParameterCatalogBuilder::with_capacity(capacity, work);
    for (name, acl) in grouped {
        let context = settings.get(&name).cloned();
        catalog.insert_if_absent(ParameterObject {
            observable: context.is_some()
                || presence.get(&name) == Some(&false)
                || is_valid_custom_parameter_name(&name),
            custom: context.is_none() && is_valid_custom_parameter_name(&name),
            context,
            exists: presence.get(&name) != Some(&false),
            name,
            acl,
        });
    }
    for (name, context) in settings {
        catalog.insert_if_absent(ParameterObject {
            name,
            observable: true,
            context: Some(context),
            exists: true,
            custom: false,
            acl: Vec::new(),
        });
    }
    for name in selected_names {
        catalog.insert_if_absent(ParameterObject {
            observable: presence.get(&name) == Some(&false)
                || is_valid_custom_parameter_name(&name),
            exists: presence.get(&name) == Some(&true) || is_valid_custom_parameter_name(&name),
            custom: is_valid_custom_parameter_name(&name),
            name,
            context: None,
            acl: Vec::new(),
        });
    }
    catalog.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn absent_builtin_requires_full_visibility_but_custom_placeholder_remains_observable() {
        for visible in [false, true] {
            let objects = build_parameter_objects(
                HashMap::new(),
                HashMap::new(),
                vec![
                    "missing_parameter".to_owned(),
                    "extension.placeholder".to_owned(),
                ],
                if visible {
                    HashMap::from([("missing_parameter".to_owned(), false)])
                } else {
                    HashMap::new()
                },
            );
            let missing = objects
                .iter()
                .find(|object| object.name == "missing_parameter")
                .unwrap();
            assert!(!missing.exists);
            assert_eq!(missing.observable, visible);
            let custom = objects
                .iter()
                .find(|object| object.name == "extension.placeholder")
                .unwrap();
            assert!(custom.exists && custom.observable && custom.custom);
        }
    }

    #[test]
    fn builtin_presence_without_context_is_still_incomplete() {
        let objects = build_parameter_objects(
            HashMap::new(),
            HashMap::new(),
            vec!["role".to_owned()],
            HashMap::from([("role".to_owned(), true)]),
        );
        let object = objects.iter().next().unwrap();
        assert!(object.exists);
        assert!(!object.observable);
        assert!(!object.custom);
    }

    #[test]
    fn loaded_extension_parameter_does_not_gain_custom_defaults_from_an_acl_row() {
        let acl = AclEntry {
            grantee: 42,
            privilege: ObjectPrivilege::Set,
            grant_option: false,
        };
        let objects = build_parameter_objects(
            HashMap::from([("extension.setting".to_owned(), vec![acl])]),
            HashMap::from([("extension.setting".to_owned(), "superuser".to_owned())]),
            vec!["extension.setting".to_owned()],
            HashMap::new(),
        );

        assert_eq!(objects.len(), 1);
        let object = objects.iter().next().unwrap();
        assert_eq!(object.context.as_deref(), Some("superuser"));
        assert!(!object.custom);
        assert!(object.exists);
        assert_eq!(object.acl, vec![acl]);
    }

    #[test]
    fn checked_catalog_rejects_conflicting_duplicate_parameter_names() {
        let first = ParameterObject {
            name: "work_mem".to_owned(),
            observable: true,
            context: Some("user".to_owned()),
            exists: true,
            custom: false,
            acl: Vec::new(),
        };
        let mut conflicting = first.clone();
        conflicting.observable = false;
        conflicting.context = None;
        conflicting.exists = false;

        assert!(matches!(
            ParameterCatalog::try_from_objects(vec![first, conflicting]),
            Err(DuplicateParameterName)
        ));
    }

    #[test]
    fn near_capacity_catalog_assembly_indexes_each_source_name_once() {
        const PARAMETER_COUNT: usize = 10_000;
        let mut selected_names = (0..PARAMETER_COUNT)
            .map(|ordinal| format!("parameter_{ordinal}"))
            .collect::<Vec<_>>();
        selected_names[9_997] = "extension.placeholder".to_owned();
        let settings = selected_names[..5_000]
            .iter()
            .cloned()
            .map(|name| (name, "sighup".to_owned()))
            .collect::<HashMap<_, _>>();
        let acl = AclEntry {
            grantee: 42,
            privilege: ObjectPrivilege::Set,
            grant_option: true,
        };
        let grouped = selected_names[..2_500]
            .iter()
            .cloned()
            .map(|name| (name, vec![acl]))
            .collect::<HashMap<_, _>>();
        let operations = Arc::new(AtomicUsize::new(0));
        let objects = build_parameter_objects_with_work(
            grouped,
            settings,
            selected_names,
            HashMap::from([
                ("parameter_9998".to_owned(), true),
                ("parameter_9999".to_owned(), false),
            ]),
            ParameterWork::counted(Arc::clone(&operations)),
        );

        assert_eq!(objects.len(), PARAMETER_COUNT);
        // One deduplication decision per source name. Catalog scans also count
        // each visited object; HashSet allocation and rehashing never count.
        assert_eq!(operations.load(Ordering::Relaxed), 17_500);
        let granted = objects
            .iter()
            .find(|object| object.name == "parameter_0")
            .unwrap();
        assert_eq!(granted.acl, vec![acl]);
        assert_eq!(granted.context.as_deref(), Some("sighup"));
        assert!(granted.exists && granted.observable && !granted.custom);
        let custom = objects
            .iter()
            .find(|object| object.name == "extension.placeholder")
            .unwrap();
        assert!(custom.exists && custom.observable && custom.custom);
        let hidden = objects
            .iter()
            .find(|object| object.name == "parameter_9998")
            .unwrap();
        assert!(hidden.exists && !hidden.observable && !hidden.custom);
        let missing = objects
            .iter()
            .find(|object| object.name == "parameter_9999")
            .unwrap();
        assert!(!missing.exists && missing.observable && !missing.custom);
    }
}
