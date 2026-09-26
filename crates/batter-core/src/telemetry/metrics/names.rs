//! Fixed-capacity, write-once, hashed tables of operation and task names.

use super::{Coalesce, INVALID_NAME, NAME_CAPACITY, NameDomain, OVERFLOW_NAME, coalesced};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

/// A bounded name label and its series index: an admitted slot, or one of the
/// two placeholders after the slots.
#[derive(Clone, Copy)]
pub(super) struct NameLabel {
    pub(super) index: usize,
    pub(super) text: &'static str,
}

const INVALID: NameLabel = NameLabel {
    index: NAME_CAPACITY,
    text: INVALID_NAME,
};
const OVERFLOW: NameLabel = NameLabel {
    index: NAME_CAPACITY + 1,
    text: OVERFLOW_NAME,
};

pub(super) struct NameTable {
    domain: NameDomain,
    slots: [OnceLock<&'static str>; NAME_CAPACITY],
    /// Each admitted name's hash, published after its slot; zero until then.
    hashes: [AtomicU64; NAME_CAPACITY],
}

/// FNV-1a over the name bytes, forced nonzero so zero means "unpublished".
fn hash(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash.max(1)
}

impl NameTable {
    pub(super) const fn new(domain: NameDomain) -> Self {
        Self {
            domain,
            slots: [const { OnceLock::new() }; NAME_CAPACITY],
            hashes: [const { AtomicU64::new(0) }; NAME_CAPACITY],
        }
    }

    /// Return a bounded label for `name`, admitting it when a slot is free.
    ///
    /// Each name probes linearly from its hashed home slot. Slots are
    /// write-once and never evicted, so an empty slot in a name's probe
    /// sequence proves that the name was never admitted beyond it. Occupied
    /// slots are skipped by comparing published hashes; strings are compared
    /// only on a hash match. A known name is usually found at its home slot;
    /// the worst case, an absent name in a full table, is one pass of integer
    /// comparisons followed by one validation. A writer that loses a race for
    /// a slot returns that slot's name when it is the same name, so each
    /// distinct name occupies exactly one slot.
    pub(super) fn label(&self, name: &'static str) -> NameLabel {
        let hashed = hash(name);
        let home = (hashed % NAME_CAPACITY as u64) as usize;
        let mut valid = None;
        for offset in 0..NAME_CAPACITY {
            let index = (home + offset) % NAME_CAPACITY;
            let slot = &self.slots[index];
            if let Some(existing) = slot.get() {
                if Self::same(
                    self.hashes[index].load(Ordering::Acquire),
                    hashed,
                    existing,
                    name,
                ) {
                    return NameLabel {
                        index,
                        text: existing,
                    };
                }
                continue;
            }
            if !*valid.get_or_insert_with(|| crate::validation::name(name).is_ok()) {
                return self.coalesce(Coalesce::Invalid, INVALID);
            }
            match slot.set(name) {
                Ok(()) => {
                    self.hashes[index].store(hashed, Ordering::Release);
                    return NameLabel { index, text: name };
                }
                Err(_) if slot.get() == Some(&name) => return NameLabel { index, text: name },
                Err(_) => {}
            }
        }
        if !valid.unwrap_or_else(|| crate::validation::name(name).is_ok()) {
            return self.coalesce(Coalesce::Invalid, INVALID);
        }
        self.coalesce(Coalesce::Capacity, OVERFLOW)
    }

    /// An unpublished hash (zero) falls back to comparing the strings.
    fn same(published: u64, hashed: u64, existing: &str, name: &str) -> bool {
        (published == 0 || published == hashed)
            && (std::ptr::eq(existing, name) || existing == name)
    }

    fn coalesce(&self, reason: Coalesce, label: NameLabel) -> NameLabel {
        coalesced(self.domain, reason);
        label
    }
}

pub(super) static OPERATION_NAMES: NameTable = NameTable::new(NameDomain::Operation);
pub(super) static TASK_NAMES: NameTable = NameTable::new(NameDomain::Task);
