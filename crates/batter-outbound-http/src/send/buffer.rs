//! Response-body storage that wipes each allocation before releasing it.
//!
//! This covers only the memory allocated here. Hyper's read buffer, TLS and
//! kernel buffers and any copy a caller makes are outside it.

use bytes::Bytes;
use zeroize::Zeroize;

/// A chunk would take the body past its limit, so it was not stored.
#[derive(Debug)]
pub(super) struct LimitExceeded;

/// The bytes of one response body, in one fixed heap allocation at a time.
///
/// A boxed slice has no `reserve` or `shrink` that could return populated
/// memory to the allocator, which leaves [`Drop`] as the only release path. That
/// covers an allocation replaced by growth, every early return from the body
/// read, a dropped exchange future and, through [`Bytes::from_owner`], the last
/// view of a returned body.
pub(super) struct ZeroizingBuffer {
    /// Always fully initialized; bytes at `filled..` were never written.
    storage: Box<[u8]>,
    filled: usize,
}

impl ZeroizingBuffer {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            storage: vec![0; capacity].into_boxed_slice(),
            filled: 0,
        }
    }

    /// Append `chunk`. A full allocation at least doubles, but never past `limit`.
    pub(super) fn append(&mut self, chunk: &[u8], limit: usize) -> Result<(), LimitExceeded> {
        let needed = self.filled.saturating_add(chunk.len());
        if needed > limit {
            return Err(LimitExceeded);
        }
        if needed > self.storage.len() {
            let target = needed.max(self.storage.len().saturating_mul(2)).min(limit);
            let mut grown = Self::with_capacity(target);
            grown.storage[..self.filled].copy_from_slice(self.as_ref());
            grown.filled = self.filled;
            // Assigning over `self` wipes the old allocation through its `Drop`.
            *self = grown;
        }
        self.storage[self.filled..needed].copy_from_slice(chunk);
        self.filled = needed;
        Ok(())
    }

    /// Give the bytes to a [`Bytes`], without copying or resizing them.
    ///
    /// Clones and slices of the result share this owner, and `Bytes` drops it
    /// once, when the last of them is released. There is deliberately no final
    /// shrink: it would copy and wipe the whole body in the exchange's last poll,
    /// inside `RESERVE`, to reclaim less than half of this allocation.
    pub(super) fn into_bytes(self) -> Bytes {
        Bytes::from_owner(self)
    }
}

impl AsRef<[u8]> for ZeroizingBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.storage[..self.filled]
    }
}

impl Drop for ZeroizingBuffer {
    fn drop(&mut self) {
        #[cfg(test)]
        let held = self.as_ref().to_vec();
        self.storage.as_mut().zeroize();
        // The allocation is still live here, so a test reads the result safely.
        #[cfg(test)]
        probe::record(held, &self.storage);
    }
}

#[cfg(test)]
pub(super) mod probe;
#[cfg(test)]
mod tests;
