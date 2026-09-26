//! Fixed-capacity, write-once, hashed tables of operation and task names.

use super::{
    Coalesce, INVALID_NAME, LABELS_COALESCED, NAME_CAPACITY, NameDomain, OVERFLOW_NAME, vocabulary,
};
use metrics::counter;
use std::sync::OnceLock;

pub(super) struct NameTable {
    domain: NameDomain,
    slots: [OnceLock<&'static str>; NAME_CAPACITY],
}

/// FNV-1a over the name bytes; stable and allocation-free.
fn home_slot(name: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (hash % NAME_CAPACITY as u64) as usize
}

impl NameTable {
    pub(super) const fn new(domain: NameDomain) -> Self {
        Self {
            domain,
            slots: [const { OnceLock::new() }; NAME_CAPACITY],
        }
    }

    /// Return a bounded label for `name`, admitting it when a slot is free.
    ///
    /// Each name probes linearly from its hashed home slot. Slots are
    /// write-once and never evicted, so an empty slot in a name's probe
    /// sequence proves that the name was never admitted beyond it. A known
    /// name is usually found at its home slot; validation runs only before an
    /// admission attempt. A writer that loses a race for a slot returns that
    /// slot's name when it is the same name, so each distinct name occupies
    /// exactly one slot.
    pub(super) fn label(&self, name: &'static str) -> &'static str {
        let home = home_slot(name);
        let probes = (0..NAME_CAPACITY).map(|offset| &self.slots[(home + offset) % NAME_CAPACITY]);
        let mut validated = false;
        for slot in probes {
            if let Some(existing) = slot.get() {
                if std::ptr::eq(*existing, name) || *existing == name {
                    return existing;
                }
                continue;
            }
            if !validated {
                if crate::validation::name(name).is_err() {
                    return self.coalesced(Coalesce::Invalid, INVALID_NAME);
                }
                validated = true;
            }
            match slot.set(name) {
                Ok(()) => return name,
                Err(_) if slot.get() == Some(&name) => return name,
                Err(_) => {}
            }
        }
        if !validated && crate::validation::name(name).is_err() {
            return self.coalesced(Coalesce::Invalid, INVALID_NAME);
        }
        self.coalesced(Coalesce::Capacity, OVERFLOW_NAME)
    }

    fn coalesced(&self, reason: Coalesce, label: &'static str) -> &'static str {
        let domain = vocabulary::name_domain(self.domain);
        let reason = vocabulary::coalesce_reason(reason);
        counter!(LABELS_COALESCED, "domain" => domain, "reason" => reason).increment(1);
        label
    }
}

pub(super) static OPERATION_NAMES: NameTable = NameTable::new(NameDomain::Operation);
pub(super) static TASK_NAMES: NameTable = NameTable::new(NameDomain::Task);
