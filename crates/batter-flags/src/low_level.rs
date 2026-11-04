//! Disclosed, non-canonical access to the live snapshot.
//!
//! The canonical paths capture one view per request, invocation or run from
//! the registered flags. The escape hatch here is deliberately weaker: a caller
//! that captures a view itself takes on the freshness and provenance
//! obligations its rustdoc discloses, and no protected surface accepts what it
//! produces.
//!
//! Scaffold: the capture function arrives with the declared-mode task.
