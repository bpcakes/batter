//! The read-only usage report both bounded stores return.

/// Current bounded-store usage.
///
/// With the `serde` feature, this serializes as an object containing `entries`,
/// `capacity`, and `shard_count`. Telemetry is intentionally serialize-only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryStoreStats {
    entries: usize,
    capacity: usize,
    shard_count: usize,
}

impl MemoryStoreStats {
    pub(crate) const fn from_parts(entries: usize, capacity: usize, shard_count: usize) -> Self {
        Self {
            entries,
            capacity,
            shard_count,
        }
    }

    /// Returns the number of stored counters.
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the hard maximum number of stored counters.
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// Returns the number of independently locked shards.
    pub const fn shard_count(self) -> usize {
        self.shard_count
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct MemoryStoreStatsWire {
    entries: usize,
    capacity: usize,
    shard_count: usize,
}

#[cfg(feature = "serde")]
impl serde::Serialize for MemoryStoreStats {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &MemoryStoreStatsWire {
                entries: self.entries(),
                capacity: self.capacity(),
                shard_count: self.shard_count(),
            },
            serializer,
        )
    }
}
