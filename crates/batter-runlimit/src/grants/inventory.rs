//! The supported operation-to-object inventory for this native source version.
//!
//! Each entry names the exact relation, the relation-level privileges PostgreSQL
//! cannot express per column, and the column lists the native statements of this
//! version read, insert and update. Entries are sorted by relation and column so
//! a review diff stays readable, and a native query or migration change is
//! expected to change this file.

use batter_sqlx::verification::ObjectPrivilege;

/// One independently selectable native Runlimit storage operation.
///
/// Selections are operations, never application role names, and there is no
/// administrator preset. Selecting an operation an application never invokes
/// provisions authority it does not need.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RunlimitOperation {
    /// Fixed-window quota admission, including first insert, renewal, counter
    /// increment, batch admission and denial.
    FixedWindowAdmission,
    /// Bounded deletion of expired fixed-window counters.
    FixedWindowExpiryCleanup,
    /// GCRA quota admission, including shard locking and counter upsert.
    GcraAdmission,
    /// Bounded deletion of expired GCRA counters.
    GcraExpiryCleanup,
    /// Authentication-attempt admission, settlement, transactional
    /// claim/finish, and the bounded expiry deletion those paths perform
    /// internally. Native Runlimit exposes no separate attempts cleanup call.
    AuthenticationAttempts,
}

pub(super) struct Requirement {
    pub(super) relation: &'static str,
    pub(super) relation_privileges: &'static [ObjectPrivilege],
    pub(super) select: &'static [&'static str],
    pub(super) insert: &'static [&'static str],
    pub(super) update: &'static [&'static str],
}

const FIXED_WINDOWS: &str = "runlimit_fixed_windows";
const CAPACITY_SHARDS: &str = "runlimit_capacity_shards";
const GCRA: &str = "runlimit_gcra";
const GCRA_SHARDS: &str = "runlimit_gcra_shards";
const ATTEMPTS: &str = "runlimit_attempts";

pub(super) const fn requirements(operation: RunlimitOperation) -> &'static [Requirement] {
    match operation {
        RunlimitOperation::FixedWindowAdmission => FIXED_WINDOW_ADMISSION,
        RunlimitOperation::FixedWindowExpiryCleanup => FIXED_WINDOW_EXPIRY_CLEANUP,
        RunlimitOperation::GcraAdmission => GCRA_ADMISSION,
        RunlimitOperation::GcraExpiryCleanup => GCRA_EXPIRY_CLEANUP,
        RunlimitOperation::AuthenticationAttempts => AUTHENTICATION_ATTEMPTS,
    }
}

const FIXED_WINDOW_ADMISSION: &[Requirement] = &[
    Requirement {
        relation: FIXED_WINDOWS,
        relation_privileges: &[],
        // The admission upsert arbitrates on the counter key and reads
        // every column it refreshes through `EXCLUDED`, which
        // PostgreSQL treats as reading the target relation's columns. The
        // generated `capacity_shard` is read from the caller's own shard input
        // and from the capacity ledger, never from this relation, so admission
        // alone does not select it.
        select: &[
            "config_fingerprint",
            "policy_id",
            "scope_id",
            "subject_key",
            "used",
            "window_expires_at",
            "window_started_at",
        ],
        insert: &[
            "config_fingerprint",
            "policy_id",
            "scope_id",
            "subject_key",
            "used",
            "window_expires_at",
            "window_started_at",
        ],
        update: &[
            "policy_id",
            "scope_id",
            "used",
            "window_expires_at",
            "window_started_at",
        ],
    },
    Requirement {
        relation: CAPACITY_SHARDS,
        relation_privileges: &[],
        select: &["capacity_shard", "row_count"],
        insert: &[],
        update: &["capacity_shard"],
    },
];

const FIXED_WINDOW_EXPIRY_CLEANUP: &[Requirement] = &[
    Requirement {
        relation: FIXED_WINDOWS,
        // The bounded delete identifies its victims by `ctid`. PostgreSQL checks
        // system-column reads against relation-level SELECT, never column
        // privileges, so this selection reads every counter column including the
        // retained policy and scope metadata.
        relation_privileges: &[ObjectPrivilege::Delete, ObjectPrivilege::Select],
        select: &[],
        insert: &[],
        update: &["window_expires_at"],
    },
    Requirement {
        relation: CAPACITY_SHARDS,
        relation_privileges: &[],
        select: &["capacity_shard"],
        insert: &[],
        update: &["capacity_shard"],
    },
];

const GCRA_ADMISSION: &[Requirement] = &[
    Requirement {
        relation: GCRA_SHARDS,
        relation_privileges: &[],
        select: &["capacity_shard", "observed_at_ms", "row_count"],
        insert: &[],
        update: &["observed_at_ms"],
    },
    Requirement {
        relation: GCRA,
        relation_privileges: &[],
        select: &[
            "config_fingerprint",
            "expires_at_ms",
            "subject_key",
            "tat_scaled",
        ],
        insert: &[
            "config_fingerprint",
            "expires_at_ms",
            "subject_key",
            "tat_scaled",
        ],
        update: &["expires_at_ms", "tat_scaled"],
    },
];

const GCRA_EXPIRY_CLEANUP: &[Requirement] = &[
    Requirement {
        relation: GCRA_SHARDS,
        relation_privileges: &[],
        select: &["capacity_shard", "observed_at_ms"],
        insert: &[],
        update: &["observed_at_ms"],
    },
    Requirement {
        relation: GCRA,
        relation_privileges: &[ObjectPrivilege::Delete],
        select: &[
            "capacity_shard",
            "config_fingerprint",
            "expires_at_ms",
            "subject_key",
        ],
        insert: &[],
        update: &[],
    },
];

const AUTHENTICATION_ATTEMPTS: &[Requirement] = &[Requirement {
    relation: ATTEMPTS,
    relation_privileges: &[ObjectPrivilege::Delete],
    select: &[
        "capacity_shard",
        "capacity_slot",
        "config_fingerprint",
        "failures",
        "last_failure_ms",
        "lease_token",
        "lease_until_ms",
        "quiet_ms",
        "retry_at_ms",
        "subject_key",
    ],
    insert: &[
        "capacity_slot",
        "config_fingerprint",
        "failures",
        "last_failure_ms",
        "lease_token",
        "lease_until_ms",
        "quiet_ms",
        "retry_at_ms",
        "subject_key",
    ],
    update: &[
        "failures",
        "last_failure_ms",
        "lease_token",
        "lease_until_ms",
        "retry_at_ms",
    ],
}];
