//! Job-handler gates and pinned decisions over the native Runledger job types.
//!
//! A gated job type defers while its kill switch is tripped; a handler reads
//! one view per invocation and may pin a decision into its own row so a retry
//! replays the same decision. Native queue, claim and worker ownership stay in
//! Runledger.
//!
//! Scaffold: the wrappers and pin helpers arrive with the jobs task.
