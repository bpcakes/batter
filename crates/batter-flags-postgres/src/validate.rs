use crate::{
    ErrorClass, FlipPolicy, InvariantPolicy, Precondition, Presence, Refusal, StoreError,
    StoredState, UndeclaredBasis,
    prepared::{DeclarationEvidence, PendingWrite, Prepared, RollbackDecision},
    relations::Written,
    request::RequestedValue,
    state::{ExistingRow, Process, Shape, StoredRow, blocking},
    write::{Mutation, WritePolicy},
};
use batter_flags_core::Version;
use batter_sqlx::PgScopedSql;
use sqlx::{Row, postgres::PgRow};
use std::{num::NonZeroUsize, ops::ControlFlow};

enum ChangePlan {
    Create {
        shape: Shape,
        state: serde_json::Value,
    },
    Replace {
        previous: ExistingRow,
        state: serde_json::Value,
    },
    Archive(ExistingRow),
    Restore(ExistingRow),
}
impl ChangePlan {
    fn shape(&self) -> Shape {
        match self {
            Self::Create { shape, .. } => *shape,
            Self::Replace { previous, .. } | Self::Archive(previous) | Self::Restore(previous) => {
                previous.shape
            }
        }
    }
    fn state(&self) -> &serde_json::Value {
        match self {
            Self::Create { state, .. } | Self::Replace { state, .. } => state,
            Self::Archive(row) | Self::Restore(row) => &row.state,
        }
    }
    /// The state the change leaves its key serving; `None` for an archive,
    /// which leaves nothing served.
    fn written(&self) -> Option<Written<'_>> {
        match self {
            Self::Archive(_) => None,
            Self::Create { .. } | Self::Replace { .. } | Self::Restore(_) => {
                crate::relations::written(self.state())
            }
        }
    }
    fn previous(self) -> Option<StoredState> {
        match self {
            Self::Create { .. } => None,
            Self::Replace { previous, .. } | Self::Archive(previous) | Self::Restore(previous) => {
                Some(StoredState(previous.state))
            }
        }
    }
    async fn apply(
        &self,
        sql: &mut PgScopedSql<'_>,
        key: &str,
        next: i64,
    ) -> Result<(), StoreError> {
        let (kind, value_type) = self.shape().tags();
        match self {
            Self::Create { state, .. } => {
                sqlx::query("INSERT INTO flags.flag_state(name, kind, value_type, state, created_version, updated_version) VALUES ($1,$2,$3,$4,$5,$5)")
                    .bind(key).bind(kind).bind(value_type).bind(state).bind(next).execute(sql.executor()).await?;
            }
            Self::Replace { .. } | Self::Restore(_) => self.update(sql, key, next, None).await?,
            Self::Archive(_) => self.update(sql, key, next, Some(next)).await?,
        }
        Ok(())
    }
    // Nullable archive version is only the persisted wire representation.
    async fn update(
        &self,
        sql: &mut PgScopedSql<'_>,
        key: &str,
        next: i64,
        archive: Option<i64>,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE flags.flag_state SET state = $2, updated_version = $3, archived_version = $4 WHERE name = $1")
            .bind(key).bind(self.state()).bind(next).bind(archive).execute(sql.executor()).await?;
        Ok(())
    }
}

pub(crate) async fn apply(
    sql: &mut PgScopedSql<'_>,
    mutation: &Mutation,
    policy: WritePolicy,
    head: i64,
    next: i64,
) -> Result<Prepared, StoreError> {
    let key = mutation.key().as_str();
    let stored = StoredRow::read(sqlx::query("SELECT kind, value_type, state, updated_version, archived_version FROM flags.flag_state WHERE name = $1")
        .bind(key).fetch_optional(sql.executor()).await?)?;
    let declarations = sqlx::query("SELECT e.kind, e.value_type, e.only_in, e.flip_discipline, e.manifest_digest FROM flags.flag_catalog_entries e JOIN flags.flag_catalogs c USING(manifest_digest) WHERE e.key = $1 ORDER BY c.last_referenced_at DESC, e.manifest_digest")
        .bind(key).fetch_all(sql.executor()).await?;
    let plan = match preflight(mutation, policy.flip, head, stored, &declarations)? {
        ControlFlow::Break(decision) => return Ok(Prepared::Rollback(decision)),
        ControlFlow::Continue(plan) => plan,
    };
    let processes = sqlx::query("SELECT p.build, CASE WHEN p.stopped_at IS NOT NULL THEN 'stopped' WHEN p.expires_at > clock_timestamp() THEN 'live' ELSE 'silent' END AS presence, p.row_encoding, e.kind, e.value_type, e.variants FROM flags.flag_processes p JOIN flags.flag_catalog_entries e USING(manifest_digest) WHERE e.key = $1 ORDER BY p.build, p.process_id")
        .bind(key).fetch_all(sql.executor()).await?.into_iter().map(Process::read).collect::<Result<Vec<_>, _>>()?;
    let active: Vec<_> = processes.iter().filter_map(Process::active).collect();
    if matches!(plan, ChangePlan::Archive(_)) && !active.is_empty() {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(
            Refusal::StillDeclared {
                builds: active.iter().map(|(r, p)| blocking(r, *p)).collect(),
            },
        )));
    }
    let (kind, value_type) = plan.shape().tags();
    let blocking: Vec<_> = active
        .iter()
        .filter(|(r, _)| {
            r.get::<i16, _>("row_encoding") < 1
                || r.get::<String, _>("kind") != kind
                || r.get::<String, _>("value_type") != value_type
                || plan
                    .state()
                    .get("variant")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|v| {
                        !r.get::<Vec<String>, _>("variants")
                            .iter()
                            .any(|known| known == v)
                    })
        })
        .map(|(r, p)| blocking(r, *p))
        .collect();
    if !blocking.is_empty() {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(
            Refusal::UndecodableByLiveDeclaration { builds: blocking },
        )));
    }
    let ahead: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM flags.flag_processes WHERE stopped_at IS NULL AND expires_at > clock_timestamp() AND snapshot_source = 'control_plane' AND snapshot_version > $1)")
        .bind(head).fetch_one(sql.executor()).await?;
    if ahead {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(
            Refusal::StoreBehindFleet,
        )));
    }
    let written = plan.written();
    let mut deferral = match written {
        Some(written) if !active.is_empty() => {
            crate::relations::judge(sql, mutation.key(), written).await?
        }
        Some(_) | None => crate::Deferral::default(),
    };
    // A policy that refuses a coordinated flip refused it above; one that
    // accepts it says so in the outcome.
    if declarations
        .iter()
        .any(|r| r.get::<String, _>("flip_discipline") == "coordinated")
    {
        deferral.flip = crate::Coordination::Coordinated;
    }
    // A kill switch is never refused for what it holds: engaging one is the
    // emergency path, and its holds are reported instead.
    if let (InvariantPolicy::RefuseUnmet, Some(Written::Flag(_))) = (policy.invariants, written)
        && let Some(refusal) = invariant_refusal(&deferral)
    {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(refusal)));
    }
    plan.apply(sql, key, next).await?;
    let declaration = match NonZeroUsize::new(
        active
            .iter()
            .filter(|(_, presence)| *presence == Presence::Live)
            .count(),
    ) {
        Some(count) => DeclarationEvidence::Live {
            previous: plan.previous(),
            count,
        },
        None => DeclarationEvidence::Undeclared(if declarations.is_empty() {
            UndeclaredBasis::NeverDeclared
        } else if processes.is_empty() {
            UndeclaredBasis::Orphaned
        } else {
            UndeclaredBasis::NoLiveDeclarer
        }),
    };
    Ok(Prepared::Commit(PendingWrite {
        version: Version::new(next as u64),
        declaration,
        deferral,
    }))
}

/// What a strict flag write is refused for: flags it would newly hold, then
/// catalogs nothing could be judged in.
fn invariant_refusal(deferral: &crate::Deferral) -> Option<Refusal> {
    if !deferral.unmet.is_empty() {
        return Some(Refusal::InvariantUnmet {
            unmet: deferral.unmet.clone(),
        });
    }
    NonZeroUsize::new(deferral.unjudged_catalogs)
        .map(|catalogs| Refusal::InvariantsUnjudged { catalogs })
}
fn preflight(
    mutation: &Mutation,
    flip: FlipPolicy,
    head: i64,
    stored: StoredRow,
    declarations: &[PgRow],
) -> Result<ControlFlow<RollbackDecision, ChangePlan>, StoreError> {
    let refuse = |r| Ok(ControlFlow::Break(RollbackDecision::Refused(r)));
    let row_version = stored.version();
    if let Some(row) = stored.existing()
        && !row.shape.known_encoding(&row.state)
    {
        return refuse(Refusal::UnknownRowEncoding);
    }
    let shape = match stored.existing() {
        Some(row) => Some(row.shape),
        None => declarations.first().map(Shape::read).transpose()?,
    };
    if let Mutation::Write(w) = mutation
        && let Some(shape) = shape
        && !shape.accepts(&w.value)
    {
        let (kind, value_type) = shape.tags();
        let requested = match &w.value {
            RequestedValue::DeclaredSwitch(_)
            | RequestedValue::Explicit(crate::ExplicitValue::Switch(_)) => "switch",
            RequestedValue::DeclaredFlag(_)
            | RequestedValue::Explicit(
                crate::ExplicitValue::ReleaseToggle(_)
                | crate::ExplicitValue::ReleaseEnum(_)
                | crate::ExplicitValue::ModeToggle(_)
                | crate::ExplicitValue::ModeEnum(_),
            ) => "flag",
        };
        return refuse(Refusal::WrongKind {
            stored: format!("{kind}/{value_type}"),
            requested: requested.into(),
        });
    }
    match (mutation, &stored) {
        (Mutation::Write(_), StoredRow::Archived(_)) => return refuse(Refusal::Archived),
        (Mutation::Write(w), StoredRow::Active(row)) if row.state == w.value.state() => {
            return Ok(ControlFlow::Break(RollbackDecision::Unchanged(
                crate::load::version(row_version)?,
            )));
        }
        (Mutation::Archive(_), StoredRow::Absent | StoredRow::Archived(_))
        | (Mutation::Restore(_), StoredRow::Absent | StoredRow::Active(_)) => {
            return Ok(ControlFlow::Break(RollbackDecision::Unchanged(
                crate::load::version(row_version)?,
            )));
        }
        (Mutation::Write(_), StoredRow::Absent | StoredRow::Active(_))
        | (Mutation::Archive(_), StoredRow::Active(_))
        | (Mutation::Restore(_), StoredRow::Archived(_)) => {}
    }
    if let Precondition::UnchangedSince(v) = mutation.change().precondition {
        if v.get() > head as u64 {
            return refuse(Refusal::RevisionAboveHead {
                current: Version::new(row_version as u64),
                head: Version::new(head as u64),
            });
        }
        if row_version as u64 > v.get() {
            return Ok(ControlFlow::Break(RollbackDecision::RevisionMismatch {
                current: Version::new(row_version as u64),
                head: Version::new(head as u64),
            }));
        }
    }
    if let Some(refusal) = declaration_refusal(mutation, flip, declarations) {
        return refuse(refusal);
    }
    Ok(ControlFlow::Continue(match (mutation, stored) {
        (Mutation::Write(w), StoredRow::Absent) => {
            let shape = match (&w.value, shape) {
                (RequestedValue::Explicit(v), None) => Shape::explicit(v),
                (_, Some(shape)) => shape,
                (RequestedValue::DeclaredFlag(_) | RequestedValue::DeclaredSwitch(_), None) => {
                    return Err(StoreError::classified(ErrorClass::InvalidData));
                }
            };
            ChangePlan::Create {
                shape,
                state: w.value.state(),
            }
        }
        (Mutation::Write(w), StoredRow::Active(previous)) => ChangePlan::Replace {
            previous,
            state: w.value.state(),
        },
        (Mutation::Archive(_), StoredRow::Active(row)) => ChangePlan::Archive(row),
        (Mutation::Restore(_), StoredRow::Archived(row)) => ChangePlan::Restore(row),
        (Mutation::Write(_), StoredRow::Archived(_))
        | (Mutation::Archive(_), StoredRow::Absent | StoredRow::Archived(_))
        | (Mutation::Restore(_), StoredRow::Absent | StoredRow::Active(_)) => {
            return Err(StoreError::classified(ErrorClass::InvalidData));
        }
    }))
}

/// What the retained declarations, read in this transaction after the
/// counter lock, refuse: an environment restriction, an undeclared value
/// write, then a coordinated flip the request did not accept.
fn declaration_refusal(
    mutation: &Mutation,
    flip: FlipPolicy,
    declarations: &[PgRow],
) -> Option<Refusal> {
    if declarations
        .iter()
        .any(|r| r.get::<Option<String>, _>("only_in").is_some())
    {
        return Some(Refusal::EnvironmentRestricted);
    }
    if declarations.is_empty()
        && matches!(mutation, Mutation::Write(w) if matches!(w.value, RequestedValue::DeclaredFlag(_) | RequestedValue::DeclaredSwitch(_)))
    {
        return Some(Refusal::Undeclared);
    }
    if let (Mutation::Write(_), FlipPolicy::RefuseCoordinated) = (mutation, flip)
        && declarations
            .iter()
            .any(|r| r.get::<String, _>("flip_discipline") == "coordinated")
    {
        return Some(Refusal::CoordinatedFlip);
    }
    None
}
