use std::error::Error;
use std::fmt;

/// Active transaction evidence limits, independent of file I/O and guest heap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceLimits {
    /// Retained effects, read dependencies, write preconditions and adapter denials.
    pub max_records: u64,
    /// Accounted owned paths and conservative record/container storage bytes.
    /// This is not a snapshot, blob-content, allocator-overhead or process RSS cap.
    pub max_bytes: u64,
}

impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            max_records: 250_000,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Monotonic charged retention, not file bytes or measured process memory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvidenceUsage {
    /// Successfully reserved evidence records, including adapter observations.
    pub records: u64,
    /// Successfully reserved path and storage accounting bytes.
    pub bytes: u64,
}

/// Byte-only reservation for one temporary path collection or traversal frontier.
///
/// This does not consume retained-record counts or measured I/O. Adapters reserve
/// before copying paths; the counter conservatively retains previous reservations
/// until the collection's scope ends, even when individual entries are popped.
#[derive(Debug, Default)]
pub struct EvidenceBuffer {
    bytes: u64,
}

/// A terminal resource failure; the partial VFS cannot produce a canonical diff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceLimitExceeded {
    /// The combined retained record cap was exceeded.
    Records {
        /// Configured maximum.
        limit: u64,
        /// First rejected count, not a completed record count.
        attempted: u64,
    },
    /// The accounted storage cap was exceeded.
    Bytes {
        /// Configured maximum.
        limit: u64,
        /// First rejected charged byte count.
        attempted: u64,
    },
    /// Bounded container growth could not be allocated.
    Allocation,
}

impl fmt::Display for EvidenceLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Records { limit, attempted } => {
                write!(
                    formatter,
                    "filesystem evidence records limit exceeded: {attempted} > {limit}"
                )
            }
            Self::Bytes { limit, attempted } => {
                write!(
                    formatter,
                    "filesystem evidence bytes limit exceeded: {attempted} > {limit}"
                )
            }
            Self::Allocation => formatter.write_str("filesystem evidence allocation failed"),
        }
    }
}

impl Error for EvidenceLimitExceeded {}

#[derive(Clone, Copy, Default)]
pub(crate) struct EvidenceBudget {
    limits: EvidenceLimits,
    usage: EvidenceUsage,
    failure: Option<EvidenceLimitExceeded>,
}

impl EvidenceBudget {
    pub(crate) const fn limits(&self) -> EvidenceLimits {
        self.limits
    }

    pub(crate) const fn usage(&self) -> EvidenceUsage {
        self.usage
    }

    pub(crate) fn limit(&mut self, limits: EvidenceLimits) {
        self.limits = if self.usage == EvidenceUsage::default() {
            limits
        } else {
            EvidenceLimits {
                max_records: self.limits.max_records.min(limits.max_records),
                max_bytes: self.limits.max_bytes.min(limits.max_bytes),
            }
        };
        // Check a lowered bound without clearing a previous terminal failure.
        let _ = self.reserve(0, 0);
    }

    pub(crate) fn check(&self) -> Result<(), EvidenceLimitExceeded> {
        self.failure.map_or(Ok(()), Err)
    }

    pub(crate) fn reserve_buffer(
        &mut self,
        buffer: &mut EvidenceBuffer,
        bytes: u64,
    ) -> Result<(), EvidenceLimitExceeded> {
        self.check()?;
        let next = buffer.bytes.checked_add(bytes);
        let attempted = next.and_then(|value| self.usage.bytes.checked_add(value));
        if attempted.is_none_or(|value| value > self.limits.max_bytes) {
            return Err(self.stop(EvidenceLimitExceeded::Bytes {
                limit: self.limits.max_bytes,
                attempted: attempted.unwrap_or(u64::MAX),
            }));
        }
        buffer.bytes = next.expect("checked temporary byte count");
        Ok(())
    }

    pub(crate) fn stop(&mut self, failure: EvidenceLimitExceeded) -> EvidenceLimitExceeded {
        *self.failure.get_or_insert(failure)
    }

    pub(crate) fn reserve(
        &mut self,
        records: u64,
        bytes: u64,
    ) -> Result<(), EvidenceLimitExceeded> {
        self.check()?;
        let attempted_records = self.usage.records.checked_add(records);
        let attempted_bytes = self.usage.bytes.checked_add(bytes);
        let failure = if attempted_records.is_none_or(|value| value > self.limits.max_records) {
            Some(EvidenceLimitExceeded::Records {
                limit: self.limits.max_records,
                attempted: attempted_records.unwrap_or(u64::MAX),
            })
        } else if attempted_bytes.is_none_or(|value| value > self.limits.max_bytes) {
            Some(EvidenceLimitExceeded::Bytes {
                limit: self.limits.max_bytes,
                attempted: attempted_bytes.unwrap_or(u64::MAX),
            })
        } else {
            None
        };
        if let Some(failure) = failure {
            return Err(self.stop(failure));
        }
        self.usage.records = attempted_records.expect("checked record count");
        self.usage.bytes = attempted_bytes.expect("checked byte count");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{EvidenceBudget, EvidenceLimitExceeded, EvidenceLimits, EvidenceUsage};

    #[test]
    fn temporary_bytes_do_not_consume_records_and_overflow_is_sticky() {
        let mut budget = EvidenceBudget::default();
        budget.limit(EvidenceLimits {
            max_records: 1,
            max_bytes: u64::MAX,
        });
        budget.reserve(1, 0).unwrap();
        let mut buffer = super::EvidenceBuffer::default();
        budget.reserve_buffer(&mut buffer, u64::MAX).unwrap();
        assert_eq!(
            budget.usage(),
            EvidenceUsage {
                records: 1,
                bytes: 0
            }
        );
        let failure = EvidenceLimitExceeded::Bytes {
            limit: u64::MAX,
            attempted: u64::MAX,
        };
        assert_eq!(budget.reserve_buffer(&mut buffer, 1), Err(failure));
        assert_eq!(buffer.bytes, u64::MAX);
        assert_eq!(
            budget.reserve_buffer(&mut super::EvidenceBuffer::default(), 0),
            Err(failure)
        );
    }

    #[test]
    fn exact_boundaries_overflow_and_reconfiguration_remain_sticky() {
        for (records, bytes, added_records, added_bytes, expected) in [
            (
                3,
                10,
                1,
                0,
                EvidenceLimitExceeded::Records {
                    limit: 3,
                    attempted: 4,
                },
            ),
            (
                3,
                10,
                0,
                1,
                EvidenceLimitExceeded::Bytes {
                    limit: 10,
                    attempted: 11,
                },
            ),
            (
                u64::MAX,
                10,
                1,
                0,
                EvidenceLimitExceeded::Records {
                    limit: u64::MAX,
                    attempted: u64::MAX,
                },
            ),
            (
                3,
                u64::MAX,
                0,
                1,
                EvidenceLimitExceeded::Bytes {
                    limit: u64::MAX,
                    attempted: u64::MAX,
                },
            ),
        ] {
            let mut budget = EvidenceBudget::default();
            budget.limit(EvidenceLimits {
                max_records: records,
                max_bytes: bytes,
            });
            budget.reserve(records, bytes).unwrap();
            assert_eq!(budget.reserve(added_records, added_bytes), Err(expected));
            assert_eq!(budget.usage(), EvidenceUsage { records, bytes });
            budget.limit(EvidenceLimits {
                max_records: u64::MAX,
                max_bytes: u64::MAX,
            });
            assert_eq!(budget.reserve(0, 0), Err(expected));
        }
    }
}
