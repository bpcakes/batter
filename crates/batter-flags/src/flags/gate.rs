//! The checked kill-switch gate every protected surface reads through.
//!
//! A gate over a kill switch has two parts that must agree: the switch must be
//! one these flags can answer for, and the reading must come from the same
//! frozen view the work is then evaluated under. Left as two calls, each surface
//! (an HTTP route group, a job wrapper, a periodic component) has to remember
//! both. Here they are one type. A [`SwitchGate`] is built by
//! [`SwitchGate::single`], [`Flags::gate`] or [`Flags::gate_any`], which refuse a switch of another
//! catalog, a switch this process kind does not own and a switch listed twice;
//! and it can only be read through [`SwitchGate::enter`], which captures one view
//! and takes every reading from it. A surface holds a `SwitchGate` and never a
//! bare `KillSwitch`, so it has nothing unchecked to read.

#![allow(
    dead_code,
    reason = "the HTTP and job gate constructors require their optional features"
)]

use batter_flags_core::{
    KillSwitch, SnapshotView,
    low_level::{GateReading, HandleRefusal, gate_switch},
};

use super::Flags;
use crate::gate_error::GateError;

/// One or more kill switches, checked against the flags they were built from.
///
/// Cloning is cheap: it shares the flags and copies the handles.
#[derive(Clone)]
pub(crate) struct SwitchGate {
    flags: Flags,
    first: KillSwitch,
    rest: Box<[KillSwitch]>,
}

/// The view one unit of work is gated under, and what the gate read from it.
///
/// The reading and the view are one capture, so the work is evaluated under the
/// state it was admitted under, and a switch engaged a moment later reaches the
/// next unit of work, not this one.
pub(crate) struct GatedView {
    pub(crate) view: SnapshotView,
    pub(crate) reading: GateReading,
}

impl Flags {
    /// A gate over one kill switch.
    ///
    /// # Errors
    ///
    /// [`GateError::ForeignHandle`] for a switch of another catalog and
    /// [`GateError::NotOwned`] for one this process kind does not own.
    pub(crate) fn gate(&self, switch: &KillSwitch) -> Result<SwitchGate, GateError> {
        SwitchGate::single(self, switch)
            .map_err(|refusal| GateError::refused(switch.key(), refusal))
    }

    /// A gate over several kill switches, tripped while any of them is.
    ///
    /// # Errors
    ///
    /// As for [`gate`](Self::gate), and [`GateError::DuplicateSwitch`] for a
    /// switch listed more than once. The first switch that fails, in the order
    /// listed, is the one reported.
    pub(crate) fn gate_any(
        &self,
        first: &KillSwitch,
        rest: &[&KillSwitch],
    ) -> Result<SwitchGate, GateError> {
        let mut listed: Vec<&KillSwitch> = Vec::with_capacity(1 + rest.len());
        for switch in std::iter::once(first).chain(rest.iter().copied()) {
            self.check(switch)?;
            if listed.contains(&switch) {
                return Err(GateError::DuplicateSwitch {
                    key: switch.key().clone(),
                });
            }
            listed.push(switch);
        }
        Ok(SwitchGate {
            flags: self.clone(),
            first: first.clone(),
            rest: rest.iter().map(|switch| (*switch).clone()).collect(),
        })
    }

    /// Whether these flags can read `switch` at all.
    fn check(&self, switch: &KillSwitch) -> Result<(), GateError> {
        self.reader
            .base()
            .check_switch(switch)
            .map_err(|refusal| GateError::refused(switch.key(), refusal))
    }
}

impl SwitchGate {
    /// Check one switch without introducing the multi-switch constructor's
    /// duplicate refusal into a single-switch surface.
    pub(crate) fn single(flags: &Flags, switch: &KillSwitch) -> Result<Self, HandleRefusal> {
        flags.reader.base().check_switch(switch)?;
        Ok(Self {
            flags: flags.clone(),
            first: switch.clone(),
            rest: Box::new([]),
        })
    }

    /// Captures one view and reads every switch of the gate from it.
    ///
    /// Each switch is read through core's one gate function, so each read emits
    /// its own event (INFO while engaged, DEBUG while released). The reading is
    /// the one that decides: [`Tripped`](GateReading::Tripped) if any switch is
    /// tripped, else [`AwaitingPublication`](GateReading::AwaitingPublication)
    /// if any is awaiting, else [`Open`](GateReading::Open). Among equal
    /// readings the earlier switch decides, except that an awaiting error or
    /// engaged decision wins over a released default. HTTP may serve defaults
    /// before publication, so aggregation must retain any reason to withhold it.
    pub(crate) fn enter(&self) -> GatedView {
        let view = self.flags.capture();
        let mut deciding = gate_switch(&view, &self.first);
        for switch in &self.rest {
            let reading = gate_switch(&view, switch);
            if precedence(&reading) > precedence(&deciding) {
                deciding = reading;
            }
        }
        GatedView {
            view,
            reading: deciding,
        }
    }
}

/// Explicit ordering of the evidence that withholds work. Awaiting readings
/// retain the most restrictive decision for surfaces that may serve defaults.
#[derive(Eq, Ord, PartialEq, PartialOrd)]
enum GatePriority {
    Open,
    AwaitingReleased,
    AwaitingEngaged,
    AwaitingError,
    Tripped,
}

fn precedence(reading: &GateReading) -> GatePriority {
    match reading {
        GateReading::Open(_) => GatePriority::Open,
        GateReading::AwaitingPublication(decision) => {
            if decision.error().is_some() {
                GatePriority::AwaitingError
            } else if decision.value().is_engaged() {
                GatePriority::AwaitingEngaged
            } else {
                GatePriority::AwaitingReleased
            }
        }
        GateReading::Tripped(_) => GatePriority::Tripped,
    }
}

#[cfg(test)]
mod tests;
