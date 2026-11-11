//! The operation-to-object inventory for this source version, read off the
//! `batter-flags-postgres` statements. Columns are sorted so a review diff
//! stays readable.

use batter_sqlx::verification::ObjectPrivilege;

/// One group of flags statements a login may be granted.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FlagsOperation {
    /// The loader's read: the counter and every unarchived state row.
    SnapshotRead,
    /// The heartbeat, catalog publication and the stop marker.
    Presence,
    /// The prune of expired process rows and unreferenced catalogs.
    PresencePrune,
    /// Audited set, kill, archive and restore.
    AuditedWrite,
    /// Catalogs, their entries and the state rows. Like every operator read,
    /// it first reads the installed schema revision.
    CatalogRead,
    /// The audit history, after the installed schema revision.
    HistoryRead,
    /// The fleet view: the installed schema revision, process rows, the head
    /// and which catalogs declare a key.
    FleetRead,
}

pub(super) struct Requirement {
    pub(super) relation: &'static str,
    pub(super) relation_privileges: &'static [ObjectPrivilege],
    pub(super) select: &'static [&'static str],
    pub(super) insert: &'static [&'static str],
    pub(super) update: &'static [&'static str],
}

const CONTROL: &str = "flag_control";
const STATE: &str = "flag_state";
const AUDIT: &str = "flag_audit";
const CATALOGS: &str = "flag_catalogs";
const ENTRIES: &str = "flag_catalog_entries";
const PROCESSES: &str = "flag_processes";

pub(super) const fn requirements(operation: FlagsOperation) -> &'static [Requirement] {
    match operation {
        FlagsOperation::SnapshotRead => SNAPSHOT_READ,
        FlagsOperation::Presence => PRESENCE,
        FlagsOperation::PresencePrune => PRESENCE_PRUNE,
        FlagsOperation::AuditedWrite => AUDITED_WRITE,
        FlagsOperation::CatalogRead => CATALOG_READ,
        FlagsOperation::HistoryRead => HISTORY_READ,
        FlagsOperation::FleetRead => FLEET_READ,
    }
}

const SNAPSHOT_READ: &[Requirement] = &[
    Requirement {
        relation: CONTROL,
        relation_privileges: &[],
        select: &["schema_revision", "singleton", "version"],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: STATE,
        relation_privileges: &[],
        select: &[
            "archived_version",
            "kind",
            "name",
            "state",
            "updated_version",
            "value_type",
        ],
        insert: &[],
        update: &[],
    },
];

const PRESENCE: &[Requirement] = &[
    // The key-share read needs UPDATE on one column, which the daily
    // reference refresh needs anyway.
    Requirement {
        relation: CATALOGS,
        relation_privileges: &[],
        select: &["last_referenced_at", "manifest_digest"],
        insert: &["manifest", "manifest_digest"],
        update: &["last_referenced_at"],
    },
    Requirement {
        relation: ENTRIES,
        relation_privileges: &[],
        select: &[],
        insert: &[
            "expiry",
            "flip_discipline",
            "key",
            "kind",
            "manifest_digest",
            "only_in",
            "value_type",
            "variants",
        ],
        update: &[],
    },
    // The upsert arbitrates on the process id and reads every refreshed
    // column through `EXCLUDED`; the stop marker locks the row by its id and
    // reads `stopped_at`.
    Requirement {
        relation: PROCESSES,
        relation_privileges: &[],
        select: &[
            "floor_engaged_keys",
            "floor_overflow",
            "held_keys",
            "held_overflow",
            "liveness_seconds",
            "process_id",
            "retention_seconds",
            "row_encoding",
            "snapshot_source",
            "snapshot_version",
            "stopped_at",
        ],
        insert: &[
            "application",
            "build",
            "declared_revision",
            "declared_shared_revision",
            "environment",
            "expires_at",
            "floor_engaged_keys",
            "floor_overflow",
            "held_keys",
            "held_overflow",
            "last_seen_at",
            "liveness_seconds",
            "manifest_digest",
            "process_id",
            "retention_seconds",
            "row_encoding",
            "snapshot_source",
            "snapshot_version",
        ],
        update: &[
            "expires_at",
            "floor_engaged_keys",
            "floor_overflow",
            "held_keys",
            "held_overflow",
            "last_seen_at",
            "liveness_seconds",
            "retention_seconds",
            "row_encoding",
            "snapshot_source",
            "snapshot_version",
            "stopped_at",
        ],
    },
];

// Each locking read needs UPDATE on one column of its table; entries go by
// `ON DELETE CASCADE`, which runs as the table owner.
const PRESENCE_PRUNE: &[Requirement] = &[
    Requirement {
        relation: PROCESSES,
        relation_privileges: &[ObjectPrivilege::Delete],
        select: &[
            "expires_at",
            "manifest_digest",
            "process_id",
            "retention_seconds",
        ],
        insert: &[],
        update: &["expires_at"],
    },
    Requirement {
        relation: CATALOGS,
        relation_privileges: &[ObjectPrivilege::Delete],
        select: &["last_referenced_at", "manifest_digest"],
        insert: &[],
        update: &["last_referenced_at"],
    },
];

const AUDITED_WRITE: &[Requirement] = &[
    // `UPDATE(version)` also authorizes the counter's `FOR UPDATE`.
    Requirement {
        relation: CONTROL,
        relation_privileges: &[],
        select: &["schema_revision", "singleton", "version"],
        insert: &[],
        update: &["version"],
    },
    Requirement {
        relation: STATE,
        relation_privileges: &[],
        select: &[
            "archived_version",
            "kind",
            "name",
            "state",
            "updated_version",
            "value_type",
        ],
        insert: &[
            "created_version",
            "kind",
            "name",
            "state",
            "updated_version",
            "value_type",
        ],
        update: &["archived_version", "state", "updated_version"],
    },
    // The flip discipline judges a coordinated flip; the manifest carries the
    // invariants and governing switches a write is judged against.
    Requirement {
        relation: ENTRIES,
        relation_privileges: &[],
        select: &[
            "flip_discipline",
            "key",
            "kind",
            "manifest_digest",
            "only_in",
            "value_type",
            "variants",
        ],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: CATALOGS,
        relation_privileges: &[],
        select: &["last_referenced_at", "manifest", "manifest_digest"],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: PROCESSES,
        relation_privileges: &[],
        select: &[
            "build",
            "expires_at",
            "manifest_digest",
            "process_id",
            "row_encoding",
            "snapshot_source",
            "snapshot_version",
            "stopped_at",
        ],
        insert: &[],
        update: &[],
    },
];

/// The guard every operator read runs first, in its own snapshot: the
/// installed schema revision (`PooledStore::preflight` and each read).
const READ_GUARD: Requirement = Requirement {
    relation: CONTROL,
    relation_privileges: &[],
    select: &["schema_revision", "singleton"],
    insert: &[],
    update: &[],
};

const CATALOG_READ: &[Requirement] = &[
    READ_GUARD,
    Requirement {
        relation: CATALOGS,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: ENTRIES,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: STATE,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    },
    // Whether any process row still references a declaring catalog: the
    // listing's `referenced` column.
    Requirement {
        relation: PROCESSES,
        relation_privileges: &[],
        select: &["manifest_digest"],
        insert: &[],
        update: &[],
    },
];

const HISTORY_READ: &[Requirement] = &[
    READ_GUARD,
    Requirement {
        relation: AUDIT,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    },
];

const FLEET_READ: &[Requirement] = &[
    Requirement {
        relation: PROCESSES,
        relation_privileges: &[ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: CONTROL,
        relation_privileges: &[],
        select: &["schema_revision", "singleton", "version"],
        insert: &[],
        update: &[],
    },
    Requirement {
        relation: ENTRIES,
        relation_privileges: &[],
        select: &["key", "manifest_digest"],
        insert: &[],
        update: &[],
    },
];
