use super::{
    InspectionFragment, PgTransaction, VerificationError,
    report::{Finding, FindingKind, SupportedSurface, UnsupportedSurface},
    request::SchemaInspectionPolicy,
};
use std::collections::HashMap;

const MAX_CONFIGURATION_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONFIGURATION_ENTRY_BYTES: usize = 1_024;
const MAX_CONFIGURATION_ROWS: usize = 10_000;

#[derive(Debug)]
struct RoutineSettings {
    object: String,
    search_paths: Vec<Vec<u8>>,
}

pub(super) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    policy: &SchemaInspectionPolicy,
) -> Result<InspectionFragment, VerificationError> {
    let schemas = policy
        .schemas()
        .iter()
        .map(|schema| schema.as_str())
        .collect::<Vec<_>>();
    let observed_schemas = sqlx::query_scalar::<_, String>(
        "SELECT n.nspname::text
         FROM pg_catalog.pg_namespace AS n
         WHERE n.nspname = ANY($1::text[])",
    )
    .bind(&schemas)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    let rows = sqlx::query_as::<_, (i64, String, String, Option<Vec<u8>>, Option<i64>)>(
        "SELECT p.oid::bigint, n.nspname::text, p.proname::text,
                substring(pg_catalog.convert_to(config.value, 'UTF8') FROM 1 FOR 1025),
                octet_length(pg_catalog.convert_to(config.value, 'UTF8'))::bigint
         FROM pg_catalog.pg_proc AS p
         JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         LEFT JOIN LATERAL unnest(p.proconfig) WITH ORDINALITY
              AS config(value, position) ON true
         WHERE p.prosecdef AND n.nspname = ANY($1::text[])
         ORDER BY p.oid, config.position
         LIMIT 10001",
    )
    .bind(&schemas)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    if !configuration_is_within_bounds(rows.len(), rows.iter().map(|row| row.4)) {
        return Err(VerificationError::CatalogCapacity);
    }

    let row_count = rows.len();
    let mut routines = HashMap::<i64, RoutineSettings>::new();
    for (oid, schema, name, setting, _) in rows {
        let routine = routines.entry(oid).or_insert_with(|| RoutineSettings {
            object: routine_object(&schema, &name, oid),
            search_paths: Vec::new(),
        });
        if let Some(setting) = setting
            && setting.starts_with(b"search_path=")
        {
            routine.search_paths.push(setting);
        }
    }
    let mut fragment = InspectionFragment {
        supported: vec![SupportedSurface::SecurityDefinerConfiguration],
        unsupported: vec![UnsupportedSurface::SecurityDefinerBody],
        evaluated_items: observed_schemas.len().saturating_add(row_count),
        ..InspectionFragment::default()
    };
    inspect_schema_presence(policy, observed_schemas, &mut fragment.findings);
    evaluate(policy, routines.into_values(), &mut fragment.findings);
    Ok(fragment)
}

fn configuration_is_within_bounds(
    row_count: usize,
    lengths: impl IntoIterator<Item = Option<i64>>,
) -> bool {
    if row_count > MAX_CONFIGURATION_ROWS {
        return false;
    }
    lengths
        .into_iter()
        .try_fold(0usize, |total, length| {
            let length = usize::try_from(length.unwrap_or(0)).ok()?;
            if length > MAX_CONFIGURATION_ENTRY_BYTES {
                return None;
            }
            total.checked_add(length)
        })
        .is_some_and(|bytes| bytes <= MAX_CONFIGURATION_BYTES)
}

fn routine_object(schema: &str, name: &str, oid: i64) -> String {
    format!("{}.{}#{oid}", quote(schema), quote(name))
}

fn inspect_schema_presence(
    policy: &SchemaInspectionPolicy,
    observed_schemas: impl IntoIterator<Item = String>,
    findings: &mut Vec<Finding>,
) {
    let observed = observed_schemas
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    for schema in policy
        .schemas()
        .iter()
        .filter(|schema| !observed.contains(schema.as_str()))
    {
        findings.push(Finding::new(
            FindingKind::MissingObject,
            Some(format!("schema:{}", quote(schema.as_str()))),
            None::<String>,
            None,
        ));
    }
}

fn evaluate(
    policy: &SchemaInspectionPolicy,
    routines: impl IntoIterator<Item = RoutineSettings>,
    findings: &mut Vec<Finding>,
) {
    for routine in routines {
        if routine.search_paths.len() != 1
            || routine.search_paths[0].as_slice() != policy.expected_search_path().as_bytes()
        {
            findings.push(Finding::new(
                FindingKind::SecurityDefinerSearchPath,
                Some(routine.object),
                None::<String>,
                None,
            ));
        }
    }
}

fn quote(identifier: &str) -> String {
    super::policy::quote_identifier(identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::Identifier;

    fn policy() -> SchemaInspectionPolicy {
        SchemaInspectionPolicy::canonical([Identifier::new("service").unwrap()]).unwrap()
    }

    #[test]
    fn exact_search_path_is_required_once_and_unrelated_settings_are_ignored() {
        let exact = RoutineSettings {
            object: "exact".to_owned(),
            search_paths: vec![b"search_path=pg_catalog, pg_temp".to_vec()],
        };
        let missing = RoutineSettings {
            object: "missing".to_owned(),
            search_paths: Vec::new(),
        };
        let duplicate = RoutineSettings {
            object: "duplicate".to_owned(),
            search_paths: vec![
                b"search_path=pg_catalog, pg_temp".to_vec(),
                b"search_path=pg_catalog, pg_temp".to_vec(),
            ],
        };
        let alternate = RoutineSettings {
            object: "alternate".to_owned(),
            search_paths: vec![b"search_path=pg_catalog,pg_temp".to_vec()],
        };
        let mut findings = Vec::new();
        evaluate(
            &policy(),
            [exact, missing, duplicate, alternate],
            &mut findings,
        );
        assert_eq!(findings.len(), 3);
        assert!(!findings.iter().any(|finding| {
            finding.kind == FindingKind::SecurityDefinerSearchPath
                && finding.object.as_deref() == Some("exact")
        }));
    }

    #[test]
    fn every_selected_schema_must_exist() {
        let policy = SchemaInspectionPolicy::canonical([
            Identifier::new("audit").unwrap(),
            Identifier::new("service").unwrap(),
        ])
        .unwrap();
        let mut findings = Vec::new();
        inspect_schema_presence(&policy, ["service".to_owned()], &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::MissingObject);
        assert_eq!(findings[0].object.as_deref(), Some("schema:\"audit\""));
    }

    #[test]
    fn configuration_bounds_cover_rows_entries_and_aggregate_bytes() {
        assert!(configuration_is_within_bounds(1, [Some(1_024)]));
        assert!(!configuration_is_within_bounds(10_001, []));
        assert!(!configuration_is_within_bounds(1, [Some(1_025)]));
        assert!(!configuration_is_within_bounds(
            8_193,
            std::iter::repeat_n(Some(1_024), 8_193)
        ));
    }

    #[test]
    fn routine_object_is_snapshot_specific_and_unambiguous() {
        assert_eq!(
            routine_object("odd schema", "routine\"name", 42),
            "\"odd schema\".\"routine\"\"name\"#42"
        );
    }
}
