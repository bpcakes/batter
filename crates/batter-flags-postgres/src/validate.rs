use crate::{
    BlockingBuild, ErrorClass, Precondition, Presence, Refusal, StoreError, StoredState,
    UndeclaredBasis, WriteOutcome, write::Mutation,
};
use batter_flags_core::Version;
use batter_sqlx::PgScopedSql;
use sqlx::{Row, postgres::PgRow};

pub(crate) async fn apply(
    sql: &mut PgScopedSql<'_>,
    mutation: &Mutation,
    head: i64,
    next: i64,
) -> Result<WriteOutcome, StoreError> {
    let key = mutation.key().as_str();
    let stored = sqlx::query("SELECT kind, value_type, state, updated_version, archived_version FROM flags.flag_state WHERE name = $1")
        .bind(key).fetch_optional(sql.executor()).await?;
    let declarations = sqlx::query("SELECT e.kind, e.value_type, e.only_in, e.manifest_digest FROM flags.flag_catalog_entries e JOIN flags.flag_catalogs c USING(manifest_digest) WHERE e.key = $1 ORDER BY c.last_referenced_at DESC, e.manifest_digest")
        .bind(key).fetch_all(sql.executor()).await?;
    let Facts {
        kind,
        value_type,
        previous,
    } = match preflight(mutation, head, stored.as_ref(), &declarations)? {
        std::ops::ControlFlow::Break(outcome) => return Ok(outcome),
        std::ops::ControlFlow::Continue(facts) => facts,
    };
    let processes = sqlx::query("SELECT p.build, p.stopped_at IS NOT NULL AS stopped, p.expires_at > clock_timestamp() AS live, p.row_encoding, e.kind, e.value_type, e.variants FROM flags.flag_processes p JOIN flags.flag_catalog_entries e USING(manifest_digest) WHERE e.key = $1 ORDER BY p.build, p.process_id")
        .bind(key).fetch_all(sql.executor()).await?;
    let active: Vec<_> = processes
        .iter()
        .filter(|r| !r.get::<bool, _>("stopped"))
        .collect();
    if matches!(mutation, Mutation::Archive(_)) && !active.is_empty() {
        return Ok(WriteOutcome::Refused(Refusal::StillDeclared {
            builds: active.iter().map(|r| blocking(r)).collect(),
        }));
    }
    let state = match mutation {
        Mutation::Write(w) => w.state.clone(),
        _ => previous
            .clone()
            .ok_or_else(|| StoreError::classified(ErrorClass::InvalidData))?,
    };
    let blocking: Vec<_> = active
        .iter()
        .filter(|r| {
            r.get::<i16, _>("row_encoding") < 1
                || r.get::<String, _>("kind") != kind
                || r.get::<String, _>("value_type") != value_type
                || state
                    .get("variant")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|v| {
                        !r.get::<Vec<String>, _>("variants")
                            .iter()
                            .any(|known| known == v)
                    })
        })
        .map(|r| blocking(r))
        .collect();
    if !blocking.is_empty() {
        return Ok(WriteOutcome::Refused(
            Refusal::UndecodableByLiveDeclaration { builds: blocking },
        ));
    }
    let ahead: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM flags.flag_processes WHERE stopped_at IS NULL AND expires_at > clock_timestamp() AND snapshot_source = 'control_plane' AND snapshot_version > $1)")
        .bind(head).fetch_one(sql.executor()).await?;
    if ahead {
        return Ok(WriteOutcome::Refused(Refusal::StoreBehindFleet));
    }
    if stored.is_none() {
        sqlx::query("INSERT INTO flags.flag_state(name, kind, value_type, state, created_version, updated_version) VALUES ($1,$2,$3,$4,$5,$5)")
            .bind(key).bind(&kind).bind(&value_type).bind(&state).bind(next).execute(sql.executor()).await?;
    } else {
        let archive = if matches!(mutation, Mutation::Archive(_)) {
            Some(next)
        } else {
            None
        };
        sqlx::query("UPDATE flags.flag_state SET state = $2, updated_version = $3, archived_version = $4 WHERE name = $1")
            .bind(key).bind(&state).bind(next).bind(archive).execute(sql.executor()).await?;
    }
    let live_declaring = active.iter().filter(|r| r.get::<bool, _>("live")).count();
    let version = Version::new(next as u64);
    Ok(if live_declaring > 0 {
        WriteOutcome::Written {
            version,
            previous: previous.map(StoredState),
            live_declaring,
        }
    } else {
        WriteOutcome::WrittenUndeclared {
            version,
            basis: if declarations.is_empty() {
                UndeclaredBasis::NeverDeclared
            } else if processes.is_empty() {
                UndeclaredBasis::Orphaned
            } else {
                UndeclaredBasis::NoLiveDeclarer
            },
        }
    })
}
fn unchanged(value: i64) -> Result<WriteOutcome, StoreError> {
    Ok(WriteOutcome::Unchanged {
        version: crate::load::version(value)?,
    })
}
fn blocking(row: &PgRow) -> BlockingBuild {
    BlockingBuild {
        build: row.get("build"),
        presence: if row.get("live") {
            Presence::Live
        } else {
            Presence::Silent
        },
    }
}
fn known_encoding(state: &serde_json::Value) -> bool {
    state.as_object().is_some_and(|o| {
        o.len() == 2
            && o.get("encoding").and_then(serde_json::Value::as_u64) == Some(1)
            && (o.get("variant").is_some_and(serde_json::Value::is_string)
                || matches!(
                    o.get("switch").and_then(serde_json::Value::as_str),
                    Some("engaged" | "released")
                ))
    })
}

struct Facts {
    kind: String,
    value_type: String,
    previous: Option<serde_json::Value>,
}
fn preflight(
    mutation: &Mutation,
    head: i64,
    stored: Option<&PgRow>,
    declarations: &[PgRow],
) -> Result<std::ops::ControlFlow<WriteOutcome, Facts>, StoreError> {
    let row_version = stored.map_or(Ok(0), |r| r.try_get::<i64, _>("updated_version"))?;
    let previous = stored
        .map(|r| r.try_get::<serde_json::Value, _>("state"))
        .transpose()?;
    if previous.as_ref().is_some_and(|s| !known_encoding(s)) {
        return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
            Refusal::UnknownRowEncoding,
        )));
    }
    let archived = stored
        .and_then(|r| r.get::<Option<i64>, _>("archived_version"))
        .is_some();
    let shape = stored
        .or(declarations.first())
        .map(|r| {
            Ok::<_, sqlx::Error>((
                r.try_get::<String, _>("kind")?,
                r.try_get::<String, _>("value_type")?,
            ))
        })
        .transpose()?;
    let shape = shape.or_else(|| match mutation {
        Mutation::Write(w) => w.explicit.map(|(k, v)| (k.into(), v.into())),
        _ => None,
    });
    if let Mutation::Write(w) = mutation {
        if let Some((kind, value_type)) = &shape
            && ((kind == "switch") != w.switch
                || w.explicit
                    .is_some_and(|(k, v)| k != kind || v != value_type))
        {
            return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
                Refusal::WrongKind {
                    stored: format!("{kind}/{value_type}"),
                    requested: if w.switch { "switch" } else { "flag" }.into(),
                },
            )));
        }
        if archived {
            return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
                Refusal::Archived,
            )));
        }
        if previous.as_ref() == Some(&w.state) {
            return unchanged(row_version).map(std::ops::ControlFlow::Break);
        }
    } else if matches!(mutation, Mutation::Archive(_)) == archived || stored.is_none() {
        return unchanged(row_version).map(std::ops::ControlFlow::Break);
    }
    if let Precondition::UnchangedSince(v) = mutation.change().precondition {
        if v.get() > head as u64 {
            return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
                Refusal::RevisionAboveHead {
                    head: Version::new(head as u64),
                },
            )));
        }
        if row_version as u64 > v.get() {
            return Ok(std::ops::ControlFlow::Break(
                WriteOutcome::RevisionMismatch {
                    current: Version::new(row_version as u64),
                    head: Version::new(head as u64),
                },
            ));
        }
    }
    if declarations
        .iter()
        .any(|r| r.get::<Option<String>, _>("only_in").is_some())
    {
        return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
            Refusal::EnvironmentRestricted,
        )));
    }
    if declarations.is_empty() && matches!(mutation, Mutation::Write(w) if w.explicit.is_none()) {
        return Ok(std::ops::ControlFlow::Break(WriteOutcome::Refused(
            Refusal::Undeclared,
        )));
    }
    let (kind, value_type) =
        shape.ok_or_else(|| StoreError::classified(ErrorClass::InvalidData))?;
    Ok(std::ops::ControlFlow::Continue(Facts {
        kind,
        value_type,
        previous,
    }))
}
