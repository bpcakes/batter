//! The write path: one audited mutation, with the coordinated-flip and
//! invariant policies judged inside its transaction, and the report each
//! outcome and failure ends with.

use batter_flags_core::FlagKey;
use batter_flags_postgres::{
    Basis, Coordination, Deferral, ErrorClass, PooledStore, Refusal, Request, StoreError,
    UndeclaredBasis, UnjudgedCatalogs, UnmetInvariant, WriteOutcome, WritePolicy,
};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{commands::Failure, exit::Exit, output::Report};

/// Writes once. A key a retained catalog declares coordinated, and a state
/// that leaves a running declarer's invariant unmet, are refused in the
/// write's own transaction unless `policy` accepts them.
pub(crate) async fn write(
    name: &'static str,
    store: &PooledStore,
    deadline: Instant,
    interrupt: &mut batter_flags_postgres::WriteInterrupt,
    policy: WritePolicy,
    key: &FlagKey,
    request: Request,
) -> Result<Report, Failure> {
    match store
        .submit_with_interrupt(request, policy, deadline, interrupt)
        .await
    {
        Ok(outcome) => Ok(outcome_report(name, key, outcome)),
        Err(error) => Err(Failure::Store(error)),
    }
}

fn outcome_report(name: &'static str, key: &FlagKey, outcome: WriteOutcome) -> Report {
    match outcome {
        WriteOutcome::Written { write_id, live_declaring, deferral, .. } => deferred(
            Report::new(name, Exit::Ok, "written", format!("written at version {} (write {write_id})", write_id.version().get()))
                .with_data(json!({"version": write_id.version().get(), "write_id": write_id.to_string(), "live_declaring": live_declaring, "deferral": deferral_data(&deferral)})),
            key,
            &deferral,
        ),
        WriteOutcome::WrittenUndeclared { write_id, basis, deferral } => {
            let basis = match basis {
                UndeclaredBasis::NoLiveDeclarer => "no_live_declarer",
                UndeclaredBasis::Orphaned => "orphaned",
                UndeclaredBasis::NeverDeclared => "never_declared",
            };
            deferred(
                Report::new(name, Exit::Ok, "written_undeclared", format!("written at version {} (write {write_id})", write_id.version().get()))
                    .with_data(json!({"version": write_id.version().get(), "write_id": write_id.to_string(), "basis": basis, "deferral": deferral_data(&deferral)}))
                    .warn(format!("no Live process declares the key ({basis}); nothing applies it yet")),
                key,
                &deferral,
            )
        }
        WriteOutcome::Unchanged { version } => Report::new(name, Exit::Ok, "unchanged", format!("already in that state at revision {}", version.get()))
            .with_data(json!({"revision": version.get()})),
        WriteOutcome::RevisionMismatch { current, head } => Report::new(name, Exit::Precondition, "revision_mismatch",
            format!("nothing written: the key is at revision {}, the store head is {}", current.get(), head.get()))
            .with_data(json!({"current": current.get(), "head": head.get()})),
        WriteOutcome::Refused(refusal) => refused(name, key, refusal),
        WriteOutcome::Uncertain { proposed_write_id, .. } => Report::new(name, Exit::Uncertain, "uncertain",
            format!("the commit outcome is unknown. Check proposed write {proposed_write_id} with `batter-flags fleet --write {proposed_write_id}` and inspect `batter-flags history {key}` before considering a new write with --if-revision", key = key.as_str()))
            .with_data(json!({"proposed_version": proposed_write_id.version().get(), "proposed_write_id": proposed_write_id.to_string()})),
    }
}

fn refused(name: &'static str, key: &FlagKey, refusal: Refusal) -> Report {
    let report = |code: &str, message: String, extra: Value| {
        let mut data = json!({"refusal": code});
        if let (Value::Object(data), Value::Object(extra)) = (&mut data, extra) {
            data.extend(extra);
        }
        Report::new(
            name,
            Exit::Precondition,
            "refused",
            format!("nothing written: {message}"),
        )
        .with_data(data)
    };
    let builds = |builds: &[batter_flags_postgres::BlockingBuild]| -> Value {
        builds
            .iter()
            .map(|b| {
                json!({"build": b.build, "presence": match b.presence {
                    batter_flags_postgres::Presence::Live => "live",
                    batter_flags_postgres::Presence::Silent => "silent",
                }})
            })
            .collect()
    };
    match refusal {
        Refusal::UnknownRowEncoding => report(
            "unknown_row_encoding",
            "the stored row uses a newer encoding".into(),
            Value::Null,
        ),
        Refusal::WrongKind { stored, requested } => report(
            "wrong_kind",
            format!("the key is a {stored}, not a {requested}"),
            json!({"stored": stored, "requested": requested}),
        ),
        Refusal::NotAToggleVariant => report(
            "not_a_toggle_variant",
            "the key is a toggle, which takes only on or off".into(),
            Value::Null,
        ),
        Refusal::Archived => report(
            "archived",
            "the key is archived; restore it first".into(),
            Value::Null,
        ),
        Refusal::StillDeclared { builds: blocking } => report(
            "still_declared",
            "Live or Silent processes still declare the key".into(),
            json!({"builds": builds(&blocking)}),
        ),
        Refusal::EnvironmentRestricted => report(
            "environment_restricted",
            "the key is declared for one environment class only".into(),
            Value::Null,
        ),
        Refusal::Undeclared => report(
            "undeclared",
            "no retained catalog declares the key".into(),
            Value::Null,
        ),
        Refusal::UndecodableByLiveDeclaration { builds: blocking } => report(
            "undecodable_by_live_declaration",
            format!(
                "a Live or Silent build cannot decode it; see `batter-flags fleet --flag {}`; a block by Silent builds only clears when prune deletes their rows, at their expiry plus their retention",
                key.as_str()
            ),
            json!({"builds": builds(&blocking)}),
        ),
        Refusal::RevisionAboveHead { current, head } => report(
            "revision_above_head",
            format!(
                "--if-revision is above the store head {}; the key is at revision {}",
                head.get(),
                current.get()
            ),
            json!({"current": current.get(), "head": head.get()}),
        ),
        Refusal::CoordinatedFlip => report(
            "coordinated",
            "the catalog marks this flip coordinated, but processes apply it poll by poll; pass --accept-rolling-flip to accept that".into(),
            Value::Null,
        ),
        Refusal::InvariantUnmet { unmet } => report(
            "invariant_unmet",
            format!(
                "processes would newly hold {} flag(s) at their declared base for unmet invariants ({}). Write the missing flag first, or pass --accept-invariant-hold to accept the hold",
                unmet.len(),
                unmet.iter().map(described).collect::<Vec<_>>().join("; ")
            ),
            json!({"unmet": unmet.iter().map(unmet_data).collect::<Vec<_>>()}),
        ),
        Refusal::InvariantsUnjudged { catalogs } => report(
            "invariants_unjudged",
            format!(
                "{}; pass --accept-invariant-hold to write unjudged once the fleet's holds are known",
                unjudged_text(catalogs)
            ),
            unjudged_data(catalogs),
        ),
    }
}

/// The written report, with one warning for each reason processes will not
/// serve the state as written for now.
fn deferred(mut report: Report, key: &FlagKey, deferral: &Deferral) -> Report {
    match deferral.flip {
        Coordination::Rolling => {}
        Coordination::Coordinated => {
            report = report.warn(format!(
                "{key} is declared coordinated: processes apply this write poll by poll, not together",
                key = key.as_str()
            ));
        }
    }
    for unmet in &deferral.unmet {
        report = report.warn(format!("held until met: {}", described(unmet)));
    }
    if let Some(catalogs) = deferral.unjudged {
        report = report.warn(unjudged_text(catalogs));
    }
    for switch in &deferral.overridden_by {
        report = report.warn(format!(
            "{switch} is engaged and governs {key}: processes serve its code default until {switch} is released",
            switch = switch.as_str(),
            key = key.as_str()
        ));
    }
    report
}

/// Why running declarer catalogs went unjudged, one clause per cause with
/// its own remedy: only a newer manifest is fixed by upgrading the command.
fn unjudged_text(catalogs: UnjudgedCatalogs) -> String {
    let mut causes = Vec::new();
    if catalogs.undecodable() > 0 {
        causes.push(format!(
            "{} running declarer catalog(s) come from a newer build this command cannot read (upgrading batter-flags reads them)",
            catalogs.undecodable()
        ));
    }
    if catalogs.held_governors() > 0 {
        causes.push(format!(
            "{} running declarer catalog(s) hold more than four governing kill switches whose rows their processes cannot read (only writing those switches from a build that reads them clears this)",
            catalogs.held_governors()
        ));
    }
    format!("{}, so their invariants were not judged", causes.join("; "))
}

fn unjudged_data(catalogs: UnjudgedCatalogs) -> Value {
    json!({
        "catalogs": catalogs.total().get(),
        "undecodable": catalogs.undecodable(),
        "held_governors": catalogs.held_governors(),
    })
}

fn described(unmet: &UnmetInvariant) -> String {
    let judged = match unmet.basis {
        Basis::StoredRows => "judged by stored rows",
        Basis::CodeDefault => "judged against a code default",
    };
    format!(
        "{held} held, as {flag} {variant} {rule} {other} {other_variant}, {judged}",
        held = unmet.held.as_str(),
        flag = unmet.flag.as_str(),
        variant = unmet.variant,
        rule = unmet.rule.as_str(),
        other = unmet.other.as_str(),
        other_variant = unmet.other_variant,
    )
}

fn unmet_data(unmet: &UnmetInvariant) -> Value {
    json!({
        "held": unmet.held.as_str(),
        "rule": unmet.rule.as_str(),
        "flag": unmet.flag.as_str(),
        "variant": unmet.variant,
        "other": unmet.other.as_str(),
        "other_variant": unmet.other_variant,
        "basis": match unmet.basis {
            Basis::StoredRows => "stored_rows",
            Basis::CodeDefault => "code_default",
        },
    })
}

fn deferral_data(deferral: &Deferral) -> Value {
    json!({
        "unmet": deferral.unmet.iter().map(unmet_data).collect::<Vec<_>>(),
        "overridden_by": deferral.overridden_by.iter().map(FlagKey::as_str).collect::<Vec<_>>(),
        "unjudged_catalogs": deferral.unjudged.map_or(0, |catalogs| catalogs.total().get()),
        "unjudged": deferral.unjudged.map(unjudged_data),
        "flip": match deferral.flip {
            Coordination::Rolling => "rolling",
            Coordination::Coordinated => "coordinated",
        },
    })
}

/// A store failure: certain, so nothing was written, except an unknown commit.
pub(crate) fn failed(name: &'static str, error: &StoreError) -> Report {
    match (error.class(), error.sqlstate()) {
        (ErrorClass::Interrupted, _) => Report::new(
            name,
            Exit::Failed,
            "interrupted",
            "interrupted before COMMIT; nothing written",
        ),
        (ErrorClass::CommitUnknown, _) => {
            Report::new(name, Exit::Uncertain, "uncertain", "the outcome is unknown")
        }
        (ErrorClass::Standby, _) => Report::new(
            name,
            Exit::Failed,
            "failed",
            "the database is a standby; nothing written",
        ),
        (ErrorClass::SchemaRevision, _) => Report::new(
            name,
            Exit::Failed,
            "failed",
            "the flags schema is older than this command requires; nothing written",
        ),
        (ErrorClass::Deadline, _) => Report::new(
            name,
            Exit::Failed,
            "failed",
            "the timeout passed before COMMIT; nothing written",
        ),
        (_, Some(code)) => Report::new(
            name,
            Exit::Failed,
            "failed",
            format!("the database refused the operation (SQLSTATE {code}); nothing written"),
        )
        .with_data(json!({"sqlstate": code})),
        (class, None) => Report::new(
            name,
            Exit::Failed,
            "failed",
            format!("the operation failed ({class:?}); nothing written"),
        ),
    }
}
