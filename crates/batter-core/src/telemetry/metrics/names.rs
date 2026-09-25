//! Fixed-capacity, write-once tables of operation and task name labels.

use super::{INVALID_NAME, LABELS_COALESCED, NAME_CAPACITY, OVERFLOW_NAME};
use metrics::counter;
use std::sync::OnceLock;

pub(super) struct NameTable {
    domain: &'static str,
    slots: [OnceLock<&'static str>; NAME_CAPACITY],
}

impl NameTable {
    pub(super) const fn new(domain: &'static str) -> Self {
        Self {
            domain,
            slots: [const { OnceLock::new() }; NAME_CAPACITY],
        }
    }

    /// Return a bounded label for `name`, admitting it when a slot is free.
    ///
    /// Slots are write-once, filled in order and lock-free to read, so the
    /// first empty slot ends the lookup. Validation runs only before admitting.
    /// A writer that loses a race for a slot returns that slot's name when it
    /// is the same name, so each distinct name occupies exactly one slot.
    pub(super) fn label(&self, name: &'static str) -> &'static str {
        let mut first_empty = self.slots.len();
        for (index, slot) in self.slots.iter().enumerate() {
            match slot.get() {
                Some(existing) if std::ptr::eq(*existing, name) || *existing == name => {
                    return existing;
                }
                Some(_) => {}
                None => {
                    first_empty = index;
                    break;
                }
            }
        }
        if crate::validation::name(name).is_err() {
            self.coalesced("invalid");
            return INVALID_NAME;
        }
        for slot in &self.slots[first_empty..] {
            match slot.set(name) {
                Ok(()) => return name,
                Err(_) if slot.get() == Some(&name) => return name,
                Err(_) => {}
            }
        }
        self.coalesced("capacity");
        OVERFLOW_NAME
    }

    fn coalesced(&self, reason: &'static str) {
        counter!(LABELS_COALESCED, "domain" => self.domain, "reason" => reason).increment(1);
    }
}

pub(super) static OPERATION_NAMES: NameTable = NameTable::new("operation");
pub(super) static TASK_NAMES: NameTable = NameTable::new("task");
