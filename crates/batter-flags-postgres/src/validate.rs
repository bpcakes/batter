use crate::{
    ErrorClass, FlipPolicy, InvariantPolicy, Precondition, Presence, Refusal, StoreError,
    StoredState, UndeclaredBasis,
    declared::{Declaration, Declared},
    prepared::{DeclarationEvidence, PendingWrite, Prepared, RollbackDecision},
    relations::Written,
    request::RequestedValue,
    state::{ExistingRow, Shape, StoredRow},
    write::{Mutation, WritePolicy},
};
use batter_flags_core::Version;
use batter_sqlx::PgScopedSql;
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
) -> Result<Prepared, StoreError> {
    let key = mutation.key().as_str();
    let stored = match StoredRow::read(sqlx::query("SELECT kind, value_type, state::text, updated_version, archived_version FROM flags.flag_state WHERE name = $1")
        .bind(key).fetch_optional(sql.executor()).await?)? {
        ControlFlow::Continue(row) => row,
        ControlFlow::Break(refusal) => return Ok(Prepared::Rollback(RollbackDecision::Refused(refusal))),
    };
    // The declarations, their processes, their catalogs and the fleet's
    // reports come from one statement, so every judgement below, and the
    // version the write is numbered with, sees the same fleet.
    let declared = Declared::read(sql, key, head).await?;
    let declarations = &declared.declarations;
    let plan = match preflight(mutation, policy.flip, head, stored, &declared)? {
        ControlFlow::Break(decision) => return Ok(Prepared::Rollback(decision)),
        ControlFlow::Continue(plan) => plan,
    };
    let active: Vec<_> = declared.active().collect();
    if matches!(plan, ChangePlan::Archive(_)) && !active.is_empty() {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(
            Refusal::StillDeclared {
                builds: active.iter().map(|d| d.blocking()).collect(),
            },
        )));
    }
    let (kind, value_type) = plan.shape().tags();
    let blocking: Vec<_> = active
        .iter()
        .filter(|d| {
            d.row_encoding < 1
                || d.kind != kind
                || d.value_type != value_type
                || plan
                    .state()
                    .get("variant")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|v| !d.variants.iter().any(|known| known == v))
        })
        .map(|d| d.blocking())
        .collect();
    if !blocking.is_empty() {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(
            Refusal::UndecodableByLiveDeclaration { builds: blocking },
        )));
    }
    let written = plan.written();
    let mut deferral = match written {
        Some(written) if !active.is_empty() => {
            crate::relations::judge(sql, mutation.key(), written, &declared.manifests).await?
        }
        Some(_) | None => crate::Deferral::default(),
    };
    // A policy that refuses a coordinated flip refused it above; one that
    // accepts it says so in the outcome.
    if declarations.iter().any(|d| d.coordinated) {
        deferral.flip = crate::Coordination::Coordinated;
    }
    // A kill switch is never refused for what it holds: engaging one is the
    // emergency path, and its holds are reported instead.
    if let (InvariantPolicy::RefuseUnmet, Some(Written::Flag(_))) = (policy.invariants, written)
        && let Some(refusal) = invariant_refusal(&deferral)
    {
        return Ok(Prepared::Rollback(RollbackDecision::Refused(refusal)));
    }
    // After a restore or a failover to an older head, a Live process can
    // have applied a version above it and would refuse anything lower.
    // Numbering this write above every such process lets the fleet converge
    // on it; otherwise the next version is the head's successor. A report is
    // the serving role's own claim, so one at or beyond the window above the
    // head was not read: it can neither refuse this write nor move the counter
    // further than the guard trigger allows.
    let next = declared
        .ahead
        .unwrap_or(head)
        .checked_add(1)
        .ok_or_else(|| StoreError::classified(ErrorClass::InvalidData))?;
    sqlx::query("UPDATE flags.flag_control SET version = $1 WHERE singleton")
        .bind(next)
        .execute(sql.executor())
        .await?;
    plan.apply(sql, key, next).await?;
    let declaration = match NonZeroUsize::new(
        active
            .iter()
            .filter(|d| d.presence == Presence::Live)
            .count(),
    ) {
        Some(count) => DeclarationEvidence::Live {
            previous: plan.previous(),
            count,
        },
        None => DeclarationEvidence::Undeclared(if declarations.is_empty() {
            UndeclaredBasis::NeverDeclared
        } else if declared.processes.is_empty() {
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
    deferral
        .unjudged
        .map(|catalogs| Refusal::InvariantsUnjudged { catalogs })
}
/// The stored row's shape, or the one a new row takes: a running declarer's,
/// because the most recently referenced catalog can be a stopped build's (a
/// heartbeat refreshes a catalog's reference time at most daily), and a
/// retained catalog's when nothing that has not stopped declares the key.
fn shape(stored: &StoredRow, declared: &Declared) -> Result<Option<Shape>, StoreError> {
    match (stored.existing(), declared.running_shape()) {
        (Some(row), _) => Ok(Some(row.shape)),
        (None, Some(shape)) => shape.map(Some),
        (None, None) => declared
            .declarations
            .first()
            .map(Declaration::shape)
            .transpose(),
    }
}

fn preflight(
    mutation: &Mutation,
    flip: FlipPolicy,
    head: i64,
    stored: StoredRow,
    declared: &Declared,
) -> Result<ControlFlow<RollbackDecision, ChangePlan>, StoreError> {
    let declarations = &declared.declarations;
    let refuse = |r| Ok(ControlFlow::Break(RollbackDecision::Refused(r)));
    let row_version = stored.version();
    let shape = shape(&stored, declared)?;
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
    if let Mutation::Write(w) = mutation
        && let Some(shape) = shape
        && !shape.closed_vocabulary_admits(&w.value)
    {
        return refuse(Refusal::NotAToggleVariant);
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
/// write, then a coordinated flip the request did not accept. A restore is a
/// flip too: declaring processes move to the restored value poll by poll.
fn declaration_refusal(
    mutation: &Mutation,
    flip: FlipPolicy,
    declarations: &[Declaration],
) -> Option<Refusal> {
    if declarations.iter().any(|d| d.restricted) {
        return Some(Refusal::EnvironmentRestricted);
    }
    if declarations.is_empty()
        && matches!(mutation, Mutation::Write(w) if matches!(w.value, RequestedValue::DeclaredFlag(_) | RequestedValue::DeclaredSwitch(_)))
    {
        return Some(Refusal::Undeclared);
    }
    if let (Mutation::Write(_) | Mutation::Restore(_), FlipPolicy::RefuseCoordinated) =
        (mutation, flip)
        && declarations.iter().any(|d| d.coordinated)
    {
        return Some(Refusal::CoordinatedFlip);
    }
    None
}
