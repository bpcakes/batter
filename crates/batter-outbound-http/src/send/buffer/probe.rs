//! Test-only record of what each allocation held when it was released.
//!
//! Each allocation's own destructor reports here between the wipe and the
//! deallocation, so nothing reads freed memory. Records stay on the thread that
//! ran the destructor, which keeps parallel tests apart.

use std::cell::RefCell;

/// One allocation, observed by its destructor while still live.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::send) struct Released {
    held: Vec<u8>,
    allocated: usize,
    all_zero: bool,
}

thread_local! {
    static RELEASES: RefCell<Vec<Released>> = const { RefCell::new(Vec::new()) };
}

/// Record the bytes held before the wipe and the allocation as it reads after.
/// An empty boxed slice owns no allocation, so there is nothing to record.
pub(super) fn record(held: Vec<u8>, allocation: &[u8]) {
    if allocation.is_empty() {
        return;
    }
    let released = Released {
        held,
        allocated: allocation.len(),
        all_zero: allocation.iter().all(|byte| *byte == 0),
    };
    RELEASES.with_borrow_mut(|releases| releases.push(released));
}

/// Take what destructors on this thread recorded since the last call.
pub(in crate::send) fn take() -> Vec<Released> {
    RELEASES.take()
}

/// The record of an `allocated`-byte allocation that held `held` and was wiped.
pub(in crate::send) fn wiped(held: &[u8], allocated: usize) -> Released {
    Released {
        held: held.to_vec(),
        allocated,
        all_zero: true,
    }
}
