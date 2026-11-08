//! The commands: what each reads or writes, and the report it ends with.

use batter_flags_core::{FlagKey, Version};
use batter_flags_postgres::{
    Actor, Change, ChangeReason, Coordination, Declaration, Expiry, FlipPolicy, KeyListing,
    PooledStore, Precondition, ProcessState, Reference, SnapshotReport, StoreError, Stored,
};
use serde_json::{Value, json};
use tokio::{sync::oneshot, time::Instant};

use crate::{
    exit::Exit,
    output::Report,
    plan::{AfterWrite, Plan},
};

/// A command.
#[derive(Debug, clap::Subcommand)]
pub(crate) enum Command {
    /// Lists every stored or declared key.
    List {
        #[command(flatten)]
        filter: ListFilter,
    },
    /// Shows one key; `revision` is the value `--if-revision` takes.
    Show { key: String },
    /// Sets a release or mode flag to a variant.
    Set {
        key: String,
        variant: String,
        #[command(flatten)]
        guard: Guard,
    },
    /// Engages or releases a kill switch.
    Kill {
        switch: String,
        state: KillState,
        #[command(flatten)]
        guard: Guard,
    },
    /// Archives a key no Live or Silent process declares.
    Archive {
        key: String,
        #[command(flatten)]
        guard: Guard,
    },
    /// Restores an archived key.
    Restore {
        key: String,
        #[command(flatten)]
        guard: Guard,
    },
    /// Shows a key's audited history.
    History { key: String },
    /// Shows the fleet, or whether a version reached every Live process.
    Fleet {
        #[arg(long, conflicts_with = "flag")]
        version: Option<u64>,
        #[arg(long)]
        flag: Option<String>,
    },
}

#[derive(Debug, clap::Args)]
#[group(id = "filter", multiple = false)]
pub(crate) struct ListFilter {
    /// Only declared release flags past their expiry on the server's date.
    #[arg(long)]
    expired: bool,
    /// Only keys whose declaring catalogs no process row references.
    #[arg(long)]
    orphaned: bool,
    /// Only declared keys with no stored row.
    #[arg(long)]
    declared_only: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum KillState {
    Engage,
    Release,
}

/// What every write requires: an actor, a reason, and exactly one of
/// `--if-revision N` or `--yes`, which [`Guard::change`] checks.
#[derive(Debug, clap::Args)]
pub(crate) struct Guard {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    reason: String,
    /// Write only if the key's revision is still this one.
    #[arg(long)]
    if_revision: Option<u64>,
    /// Write whatever the key's revision is.
    #[arg(long)]
    yes: bool,
    /// Accept a poll-by-poll rollout of a flip the catalog marks coordinated.
    #[arg(long)]
    accept_rolling_flip: bool,
}

impl Command {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Show { .. } => "show",
            Self::Set { .. } => "set",
            Self::Kill { .. } => "kill",
            Self::Archive { .. } => "archive",
            Self::Restore { .. } => "restore",
            Self::History { .. } => "history",
            Self::Fleet { .. } => "fleet",
        }
    }
}

/// Runs `plan` against `store` within `deadline`; `interrupt` stops a
/// write before its COMMIT.
pub(crate) async fn run(
    plan: Plan,
    store: &PooledStore,
    deadline: Instant,
    interrupt: oneshot::Receiver<()>,
) -> Report {
    let name = plan.name();
    match execute(plan, store, deadline, interrupt).await {
        Ok(report) => report,
        Err(Failure::Store(error)) => crate::writes::failed(name, &error),
    }
}

pub(crate) enum Failure {
    Store(StoreError),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

async fn execute(
    plan: Plan,
    store: &PooledStore,
    deadline: Instant,
    interrupt: oneshot::Receiver<()>,
) -> Result<Report, Failure> {
    match plan {
        Plan::List(filter) => list(store, deadline, &filter).await,
        Plan::Show(key) => {
            let listing = store.key(&key, deadline).await?;
            Ok(Report::new(
                "show",
                Exit::Ok,
                "ok",
                format!("revision {}", listing.revision.get()),
            )
            .with_data(listed(&listing)))
        }
        Plan::History(key) => history(store, deadline, &key).await,
        Plan::Fleet { version, flag } => {
            crate::fleet::report::<Failure>(store, deadline, version, flag.as_ref()).await
        }
        Plan::Write {
            name,
            key,
            request,
            flip,
            after,
        } => {
            let report =
                crate::writes::write(name, store, deadline, interrupt, flip, &key, request).await?;
            Ok(match (after, report.outcome) {
                (AfterWrite::FloorWarning, "written" | "written_undeclared") => {
                    crate::fleet::floor_warning(store, deadline, &key, report).await
                }
                _ => report,
            })
        }
    }
}

async fn list(
    store: &PooledStore,
    deadline: Instant,
    filter: &ListFilter,
) -> Result<Report, Failure> {
    let keys: Vec<Value> = store
        .keys(deadline)
        .await?
        .iter()
        .filter(|listing| filter.admits(listing))
        .map(listed)
        .collect();
    Ok(
        Report::new("list", Exit::Ok, "ok", format!("{} keys", keys.len()))
            .with_data(Value::Array(keys)),
    )
}

async fn history(store: &PooledStore, deadline: Instant, key: &FlagKey) -> Result<Report, Failure> {
    let entries: Vec<Value> = store
        .history(Some(key), Version::new(0), deadline)
        .await?
        .iter()
        .map(|entry| {
            json!({
                "version": entry.version.get(),
                "operation": entry.operation,
                "actor": entry.actor,
                "reason": entry.reason,
                "guard": entry.guard,
                "recorded_by": entry.recorded_by,
                "after": entry.after.value(),
            })
        })
        .collect();
    Ok(Report::new(
        "history",
        Exit::Ok,
        "ok",
        format!("{} entries", entries.len()),
    )
    .with_data(Value::Array(entries)))
}

impl Guard {
    pub(crate) fn flip(&self) -> FlipPolicy {
        if self.accept_rolling_flip {
            FlipPolicy::AcceptRolling
        } else {
            FlipPolicy::RefuseCoordinated
        }
    }

    pub(crate) fn change(&self) -> Result<Change, String> {
        let actor = Actor::new(&self.actor)
            .map_err(|_| "--actor must be 1 to 128 bytes without control characters".to_owned())?;
        let reason = ChangeReason::new(&self.reason).map_err(|_| {
            "--reason must be 1 to 1024 bytes without control characters".to_owned()
        })?;
        let precondition = match (self.if_revision, self.yes) {
            (Some(revision), false) => Precondition::UnchangedSince(Version::new(revision)),
            (None, true) => Precondition::Unconditional,
            _ => {
                return Err("pass exactly one of --if-revision N or --yes".to_owned());
            }
        };
        Ok(Change::new(actor, reason, precondition))
    }
}

impl ListFilter {
    fn admits(&self, listing: &KeyListing) -> bool {
        match (
            &listing.declaration,
            self.expired,
            self.orphaned,
            self.declared_only,
        ) {
            (_, false, false, false) => true,
            (
                Declaration::Retained {
                    expiry: Expiry::Passed(_),
                    ..
                },
                true,
                _,
                _,
            ) => true,
            (
                Declaration::Retained {
                    reference: Reference::Orphaned,
                    ..
                },
                _,
                true,
                _,
            ) => true,
            (Declaration::Retained { .. }, _, _, true) => listing.stored == Stored::Absent,
            _ => false,
        }
    }
}

fn listed(listing: &KeyListing) -> Value {
    let (state, stored) = match &listing.stored {
        Stored::Absent => (Value::Null, "absent"),
        Stored::Active(state) => (state.value().clone(), "active"),
        Stored::Archived(state) => (state.value().clone(), "archived"),
    };
    let declaration = match &listing.declaration {
        Declaration::Undeclared => json!({"declared": false}),
        Declaration::Retained {
            coordinated,
            expiry,
            reference,
        } => json!({
            "declared": true,
            "coordinated": *coordinated == Coordination::Coordinated,
            "expiry": match expiry { Expiry::None => Value::Null, Expiry::On(d) | Expiry::Passed(d) => json!(d) },
            "expired": matches!(expiry, Expiry::Passed(_)),
            "orphaned": *reference == Reference::Orphaned,
        }),
    };
    json!({
        "key": listing.key,
        "kind": listing.kind,
        "value_type": listing.value_type,
        "revision": listing.revision.get(),
        "stored": stored,
        "state": state,
        "declaration": declaration,
    })
}

pub(crate) fn snapshot_text(report: SnapshotReport) -> (&'static str, Option<u64>) {
    match report {
        SnapshotReport::NotLoaded => ("not_loaded", None),
        SnapshotReport::PersistedFile(version) => ("persisted_file", Some(version.get())),
        SnapshotReport::ControlPlane(version) => ("control_plane", Some(version.get())),
    }
}

pub(crate) fn state_text(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Live => "live",
        ProcessState::Silent => "silent",
        ProcessState::Stopped => "stopped",
    }
}
