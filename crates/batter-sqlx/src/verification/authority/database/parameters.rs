use super::super::super::policy::is_valid_custom_parameter_name;
use super::*;

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
) -> Result<Vec<ParameterObject>, VerificationError> {
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
        return Ok(Vec::new());
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
) -> Vec<ParameterObject> {
    let mut result: Vec<ParameterObject> = grouped
        .into_iter()
        .map(|(name, acl)| {
            let context = settings.get(&name).cloned();
            ParameterObject {
                observable: context.is_some()
                    || presence.get(&name) == Some(&false)
                    || is_valid_custom_parameter_name(&name),
                custom: context.is_none() && is_valid_custom_parameter_name(&name),
                context,
                exists: presence.get(&name) != Some(&false),
                name,
                acl,
            }
        })
        .collect();
    for (name, context) in settings {
        if !result.iter().any(|parameter| parameter.name == name) {
            result.push(ParameterObject {
                name,
                observable: true,
                context: Some(context),
                exists: true,
                custom: false,
                acl: Vec::new(),
            });
        }
    }
    for name in selected_names {
        if !result.iter().any(|parameter| parameter.name == name) {
            result.push(ParameterObject {
                observable: presence.get(&name) == Some(&false)
                    || is_valid_custom_parameter_name(&name),
                exists: presence.get(&name) == Some(&true) || is_valid_custom_parameter_name(&name),
                custom: is_valid_custom_parameter_name(&name),
                name,
                context: None,
                acl: Vec::new(),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(objects[0].exists);
        assert!(!objects[0].observable);
        assert!(!objects[0].custom);
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
        assert_eq!(objects[0].context.as_deref(), Some("superuser"));
        assert!(!objects[0].custom);
        assert!(objects[0].exists);
    }
}
