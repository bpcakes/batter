//! Recording a process stop: the transition that consumes a presence.
use super::ProcessPresence;
use crate::{ErrorClass, StoreError};
use tokio::time::Instant;

/// Why a stop was not recorded, and whether the process may still be Live.
#[derive(Debug)]
pub enum StopFailure {
    /// The stop certainly did not commit. The presence is handed back, so the
    /// process can keep sending heartbeats or try to stop again.
    NotRecorded {
        /// Why it failed.
        error: StoreError,
        /// The unchanged capability.
        presence: Box<ProcessPresence>,
    },
    /// COMMIT may have taken effect. The presence is not handed back: the row
    /// may already be stopped, and a later heartbeat could recreate it after
    /// prune. Until prune removes it, the row reads Live until its expiry and
    /// Silent after, as an unstopped process would.
    Unknown {
        /// What made the outcome unknown.
        error: StoreError,
    },
}

impl std::fmt::Display for StopFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRecorded { .. } => f.write_str("flags process stop was not recorded"),
            Self::Unknown { .. } => f.write_str("flags process stop outcome is unknown"),
        }
    }
}

impl std::error::Error for StopFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotRecorded { error, .. } | Self::Unknown { error } => Some(error),
        }
    }
}

/// What recording a stop did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopOutcome {
    /// The process is now Stopped.
    Stopped,
    /// The process's row was already stopped, by something other than this
    /// presence, which is consumed by the stop it records.
    AlreadyStopped,
    /// No row exists: the process never sent a heartbeat, or prune removed it.
    NotRegistered,
}

impl ProcessPresence {
    /// Records, in the store that issued this presence, that the process
    /// stopped, consuming the presence. Its expiry is left as it was, so a
    /// stopped row is pruned on the same schedule as a silent one. The row is
    /// locked and read in the statement that marks it, so the outcome
    /// describes the row it changed.
    pub async fn stop(self, deadline: Instant) -> Result<StopOutcome, StopFailure> {
        let presence = self;
        let process = presence.process.as_uuid();
        let store = presence.store.clone();
        let marked = store
            .atomically(deadline, async |sql| {
                let stopped: Option<bool> = sqlx::query_scalar("WITH target AS (SELECT process_id, stopped_at IS NOT NULL AS stopped FROM flags.flag_processes WHERE process_id = $1 FOR UPDATE), \
                    marked AS (UPDATE flags.flag_processes p SET stopped_at = clock_timestamp() FROM target WHERE p.process_id = target.process_id AND NOT target.stopped RETURNING p.process_id) \
                    SELECT stopped FROM target")
                    .bind(process).fetch_optional(sql.executor()).await?;
                Ok(match stopped {
                    None => StopOutcome::NotRegistered,
                    Some(false) => StopOutcome::Stopped,
                    Some(true) => StopOutcome::AlreadyStopped,
                })
            })
            .await;
        match marked {
            Ok(outcome) => Ok(outcome),
            Err(error) if error.class() == ErrorClass::CommitUnknown => {
                Err(StopFailure::Unknown { error })
            }
            Err(error) => Err(StopFailure::NotRecorded {
                error,
                presence: Box::new(presence),
            }),
        }
    }
}
