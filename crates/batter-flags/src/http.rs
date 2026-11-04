//! Route-group gates and per-request flag views on the Batter Axum adapter.
//!
//! A gated route group answers with a library-owned response while its kill
//! switch is tripped; admitted handlers extract the one view captured for the
//! request rather than evaluating flags themselves.
//!
//! Scaffold: the gate builders and the extractor arrive with the HTTP task.
