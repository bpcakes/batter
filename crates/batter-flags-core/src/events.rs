//! The evaluation event and the tally of evaluation errors.
//!
//! One `tracing` event is emitted per library evaluation, even when its result
//! is unchanged, named `feature_flag.evaluation` after the OpenTelemetry
//! semantic convention (v1.45.0). Its target is `batter::flags`, so
//! `batter=warn` filters it like the rest of Batter and `batter::flags=debug`
//! brings it back. It is emitted under the dispatcher and parent span captured
//! with the view, never under whatever happens to be current at evaluation.
//!
//! Attributes, all text: `feature_flag.key`, `feature_flag.result.variant`
//! (omitted when no variant was served), `feature_flag.result.reason`,
//! `feature_flag.version`, `feature_flag.set.id` (the catalog digest),
//! `feature_flag.provider.name`, `error.type` on an error and on no other
//! evaluation, and `batter.flags.cause` beside `flag_not_found` for a foreign or
//! not-owned handle. `feature_flag.context.id`, `feature_flag.result.value`,
//! `feature_flag.error.message` and `error.message` are never emitted, and no
//! event carries pin contents or any other application-supplied text.
//!
//! Levels: ordinary evaluations are DEBUG. A gate evaluation is DEBUG while the
//! switch is released and INFO while it is engaged. An error evaluation is WARN
//! the first time a handle meets a cause in a catalog's lifetime and DEBUG
//! afterwards; no other evaluation event is WARN. Every error evaluation is
//! counted whatever the level.

use std::{collections::HashSet, sync::Mutex};

use tracing::{Dispatch, Span};

use crate::{
    decision::{ErrorCause, Reason, SourceVersion},
    digest::CatalogDigest,
    vocabulary::CatalogId,
};

/// The target of every flags event.
pub(crate) const TARGET: &str = "batter::flags";

/// The `feature_flag.provider.name` of every evaluation.
pub(crate) const PROVIDER: &str = "batter-flags";

/// The dispatcher and parent span current when a view was captured.
pub(crate) struct EventContext {
    dispatch: Dispatch,
    parent: Span,
}

impl EventContext {
    pub(crate) fn capture() -> Self {
        Self {
            dispatch: tracing::dispatcher::get_default(Dispatch::clone),
            parent: Span::current(),
        }
    }

    fn scope<R>(&self, emit: impl FnOnce() -> R) -> R {
        tracing::dispatcher::with_default(&self.dispatch, || self.parent.in_scope(emit))
    }
}

/// Which surface evaluated, which decides an event's level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Surface {
    Ordinary,
    GateReleased,
    GateEngaged,
}

/// The handle an error evaluation was made for: its catalog and entry.
pub(crate) type HandleId = (CatalogId, usize);

/// One evaluation to report.
pub(crate) struct Evaluation<'a> {
    pub(crate) key: &'a str,
    pub(crate) variant: Option<&'a str>,
    pub(crate) reason: Reason,
    pub(crate) version: SourceVersion,
    pub(crate) error: Option<(ErrorCause, HandleId)>,
    pub(crate) surface: Surface,
}

#[derive(Clone, Copy)]
enum Severity {
    Warn,
    Info,
    Debug,
}

/// How often a handle has met a cause.
enum Occurrence {
    First,
    Repeat,
}

/// Counts every error evaluation and remembers which handle and cause pairs
/// have already been reported at WARN.
#[derive(Default)]
pub(crate) struct ErrorTally {
    state: Mutex<TallyState>,
}

#[derive(Default)]
struct TallyState {
    reported: HashSet<(HandleId, ErrorCause)>,
    counts: [u64; ErrorCause::ALL.len()],
}

impl ErrorTally {
    fn record(&self, handle: HandleId, cause: ErrorCause) -> Occurrence {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.counts[cause.index()] += 1;
        if state.reported.insert((handle, cause)) {
            Occurrence::First
        } else {
            Occurrence::Repeat
        }
    }

    pub(crate) fn count(&self, cause: ErrorCause) -> u64 {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.counts[cause.index()]
    }
}

/// Reports one evaluation.
pub(crate) fn emit(
    context: &EventContext,
    tally: &ErrorTally,
    set: &CatalogDigest,
    evaluation: &Evaluation<'_>,
) {
    let severity = match evaluation.error {
        Some((cause, handle)) => match tally.record(handle, cause) {
            Occurrence::First => Severity::Warn,
            Occurrence::Repeat => Severity::Debug,
        },
        None => match evaluation.surface {
            Surface::GateEngaged => Severity::Info,
            Surface::Ordinary | Surface::GateReleased => Severity::Debug,
        },
    };
    context.scope(|| fire(severity, set, evaluation));
}

/// One callsite per level: a callsite's level is fixed where it is written.
macro_rules! evaluation_event {
    ($level:expr, $set:expr, $evaluation:expr) => {
        tracing::event!(
            name: "feature_flag.evaluation",
            target: TARGET,
            $level,
            feature_flag.key = $evaluation.key,
            feature_flag.result.variant = $evaluation.variant,
            feature_flag.result.reason = $evaluation.reason.as_str(),
            feature_flag.version = tracing::field::display(&$evaluation.version),
            feature_flag.set.id = tracing::field::display($set),
            feature_flag.provider.name = PROVIDER,
            error.r#type = $evaluation.error.map(|(cause, _)| cause.error_type()),
            batter.flags.cause = $evaluation.error.and_then(|(cause, _)| cause.handle_cause()),
        )
    };
}

fn fire(severity: Severity, set: &CatalogDigest, evaluation: &Evaluation<'_>) {
    match severity {
        Severity::Warn => at_warn(set, evaluation),
        Severity::Info => at_info(set, evaluation),
        Severity::Debug => at_debug(set, evaluation),
    }
}

// One expansion per function: how much code `tracing::event!` expands to
// depends on the `tracing` features the whole build unifies, so a function
// holding all three would pass alone and fail inside the workspace.
fn at_warn(set: &CatalogDigest, evaluation: &Evaluation<'_>) {
    evaluation_event!(tracing::Level::WARN, set, evaluation);
}

fn at_info(set: &CatalogDigest, evaluation: &Evaluation<'_>) {
    evaluation_event!(tracing::Level::INFO, set, evaluation);
}

fn at_debug(set: &CatalogDigest, evaluation: &Evaluation<'_>) {
    evaluation_event!(tracing::Level::DEBUG, set, evaluation);
}
