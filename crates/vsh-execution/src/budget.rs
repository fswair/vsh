use std::error::Error;
use std::fmt;
use std::time::Duration;

/// Per-execution budgets shared by guest adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionLimits {
    /// Maximum UTF-8 bytes accepted as one program.
    pub max_program_bytes: usize,
    /// Cumulative guest execution time; parent supervision remains authoritative.
    pub max_duration: Duration,
    /// Maximum guest recursion depth.
    pub max_recursion_depth: usize,
    /// Maximum interpreter heap bytes enforced by the supervised worker allocator.
    /// The process-local correctness harness cannot install a per-call global allocator.
    pub max_memory_bytes: usize,
    /// Maximum typed OS calls and high-level VSH tool calls serviced by the host adapter.
    pub max_os_calls: u64,
    /// Maximum cumulative bytes materialized by read and append operations.
    pub max_read_bytes: u64,
    /// Maximum cumulative bytes submitted by write and append operations.
    pub max_write_bytes: u64,
    /// Maximum payload bytes materialized by one typed read or write call.
    pub max_io_call_bytes: usize,
    /// Maximum UTF-8 bytes accepted in one guest-visible path.
    pub max_path_bytes: usize,
    /// Maximum cumulative directory entries returned to the guest.
    pub max_directory_entries: u64,
    /// Maximum retained effects, read dependencies, write preconditions and denials.
    pub max_evidence_records: u64,
    /// Maximum accounted filesystem evidence/storage bytes, not process RSS.
    pub max_evidence_bytes: u64,
    /// Maximum UTF-8 bytes retained from `print()` output.
    pub max_output_bytes: usize,
    /// Maximum deep host footprint of the returned guest value.
    pub max_result_bytes: usize,
    /// Maximum retained exception message, traceback and structured payload bytes.
    pub max_exception_bytes: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_program_bytes: 1024 * 1024,
            max_duration: Duration::from_secs(1),
            max_recursion_depth: 512,
            max_memory_bytes: 256 * 1024 * 1024,
            max_os_calls: 10_000,
            max_read_bytes: 64 * 1024 * 1024,
            max_write_bytes: 64 * 1024 * 1024,
            max_io_call_bytes: 4 * 1024 * 1024,
            max_path_bytes: 16 * 1024,
            max_directory_entries: 100_000,
            max_evidence_records: 250_000,
            max_evidence_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            max_result_bytes: 1024 * 1024,
            max_exception_bytes: 256 * 1024,
        }
    }
}

impl ExecutionLimits {
    /// Check source bytes before guest compilation or worker framing.
    ///
    /// # Errors
    /// Returns [`ExecutionLimitExceeded::ProgramBytes`] when the source exceeds its cap.
    pub fn check_program_bytes(self, bytes: usize) -> Result<(), ExecutionLimitExceeded> {
        if bytes > self.max_program_bytes {
            Err(ExecutionLimitExceeded::ProgramBytes {
                limit: u64::try_from(self.max_program_bytes).unwrap_or(u64::MAX),
                attempted: u64::try_from(bytes).unwrap_or(u64::MAX),
            })
        } else {
            Ok(())
        }
    }

    /// Project parent-side filesystem evidence caps without changing I/O budgets.
    #[must_use]
    pub const fn evidence_limits(self) -> vsh_vfs::EvidenceLimits {
        vsh_vfs::EvidenceLimits {
            max_records: self.max_evidence_records,
            max_bytes: self.max_evidence_bytes,
        }
    }
}

/// Why execution stopped before a normal guest result was produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutionLimitExceeded {
    /// Program source exceeded its input cap before compilation.
    ProgramBytes {
        /// Configured maximum.
        limit: u64,
        /// Submitted UTF-8 byte count.
        attempted: u64,
    },
    /// Typed OS-call count exceeded its cap.
    OsCalls {
        /// Configured maximum.
        limit: u64,
        /// Count the next call would have reached.
        attempted: u64,
    },
    /// Cumulative materialized read bytes exceeded their cap.
    ReadBytes {
        /// Configured maximum.
        limit: u64,
        /// Byte count the operation would have reached.
        attempted: u64,
    },
    /// Cumulative submitted write bytes exceeded their cap.
    WriteBytes {
        /// Configured maximum.
        limit: u64,
        /// Byte count the operation would have reached.
        attempted: u64,
    },
    /// One typed read payload exceeded its per-call materialization cap.
    ReadCallBytes {
        /// Configured maximum.
        limit: u64,
        /// Bytes the call would materialize.
        attempted: u64,
    },
    /// One typed write payload exceeded its per-call decode cap.
    WriteCallBytes {
        /// Configured maximum.
        limit: u64,
        /// Submitted bytes in this call.
        attempted: u64,
    },
    /// One guest-visible path exceeded its UTF-8 byte cap.
    PathBytes {
        /// Configured maximum.
        limit: u64,
        /// Submitted path bytes.
        attempted: u64,
    },
    /// Cumulative returned directory entries exceeded their cap.
    DirectoryEntries {
        /// Configured maximum.
        limit: u64,
        /// Entry count the operation would have reached.
        attempted: u64,
    },
    /// Active filesystem evidence records exceeded their parent-side cap.
    EvidenceRecords {
        /// Configured maximum.
        limit: u64,
        /// First rejected reservation.
        attempted: u64,
    },
    /// Active filesystem evidence/storage accounting exceeded its byte cap.
    EvidenceBytes {
        /// Configured maximum.
        limit: u64,
        /// First rejected reservation, not measured RSS or file bytes.
        attempted: u64,
    },
    /// Streamed print output exceeded its retained UTF-8 byte cap.
    OutputBytes {
        /// Configured maximum.
        limit: u64,
        /// Bytes observed before stopping.
        attempted: u64,
    },
    /// A completed return value exceeded its deep host-footprint cap.
    ResultBytes {
        /// Configured maximum.
        limit: u64,
        /// Deep bytes visited before stopping.
        attempted: u64,
    },
    /// An escaping exception exceeded its host-output cap.
    ExceptionBytes {
        /// Configured maximum.
        limit: u64,
        /// Retained exception bytes.
        attempted: u64,
    },
}

impl fmt::Display for ExecutionLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, limit, attempted) = match *self {
            Self::ProgramBytes { limit, attempted } => ("program bytes", limit, attempted),
            Self::OsCalls { limit, attempted } => ("OS calls", limit, attempted),
            Self::ReadBytes { limit, attempted } => ("read bytes", limit, attempted),
            Self::WriteBytes { limit, attempted } => ("write bytes", limit, attempted),
            Self::ReadCallBytes { limit, attempted } => ("read call bytes", limit, attempted),
            Self::WriteCallBytes { limit, attempted } => ("write call bytes", limit, attempted),
            Self::PathBytes { limit, attempted } => ("path bytes", limit, attempted),
            Self::DirectoryEntries { limit, attempted } => ("directory entries", limit, attempted),
            Self::EvidenceRecords { limit, attempted } => {
                ("filesystem evidence records", limit, attempted)
            }
            Self::EvidenceBytes { limit, attempted } => {
                ("filesystem evidence bytes", limit, attempted)
            }
            Self::OutputBytes { limit, attempted } => ("output bytes", limit, attempted),
            Self::ResultBytes { limit, attempted } => ("result bytes", limit, attempted),
            Self::ExceptionBytes { limit, attempted } => ("exception bytes", limit, attempted),
        };
        if attempted == u64::MAX && limit == u64::MAX {
            write!(formatter, "{name} counter overflow at {limit}")
        } else {
            write!(formatter, "{name} limit exceeded: {attempted} > {limit}")
        }
    }
}

impl Error for ExecutionLimitExceeded {}

/// Host-side counters from one execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionStats {
    /// Typed OS calls and high-level VSH tool calls serviced.
    pub os_calls: u64,
    /// File bytes materialized for reads or copy-on-write append.
    pub read_bytes: u64,
    /// Payload bytes submitted to writes or appends.
    pub write_bytes: u64,
    /// Directory entries returned to the guest.
    pub directory_entries: u64,
    /// UTF-8 output bytes retained after completion.
    pub output_bytes: usize,
    /// Protected capability attempts denied before any VFS access.
    pub denied_accesses: u64,
    /// Deep host footprint of the final returned value.
    pub result_bytes: u64,
}

/// Monotonic host-side accounting for one guest execution.
pub struct ExecutionBudget {
    limits: ExecutionLimits,
    stats: ExecutionStats,
}

impl ExecutionBudget {
    /// Return the immutable limits governing this ledger.
    #[must_use]
    pub const fn limits(&self) -> ExecutionLimits {
        self.limits
    }

    /// Return a copy of current execution counters.
    #[must_use]
    pub const fn stats(&self) -> ExecutionStats {
        self.stats
    }

    /// Record a policy denial; serviced-request limits bound denial evidence.
    pub fn record_denial(&mut self) {
        self.stats.denied_accesses = self.stats.denied_accesses.saturating_add(1);
    }

    /// Charge all guest output, including output returned by a nested frontend.
    ///
    /// # Errors
    /// Returns the output limit before accepting bytes beyond the shared ceiling.
    pub fn charge_output(&mut self, bytes: usize) -> Result<(), ExecutionLimitExceeded> {
        let attempted = self.stats.output_bytes.saturating_add(bytes);
        if attempted > self.limits.max_output_bytes {
            return Err(ExecutionLimitExceeded::OutputBytes {
                limit: self.limits.max_output_bytes as u64,
                attempted: attempted as u64,
            });
        }
        self.stats.output_bytes = attempted;
        Ok(())
    }

    /// Start an empty ledger with explicit limits.
    #[must_use]
    pub const fn new(limits: ExecutionLimits) -> Self {
        Self {
            limits,
            stats: ExecutionStats {
                os_calls: 0,
                read_bytes: 0,
                write_bytes: 0,
                directory_entries: 0,
                output_bytes: 0,
                denied_accesses: 0,
                result_bytes: 0,
            },
        }
    }

    /// Charge os call work before servicing it.
    ///
    /// # Errors
    /// Returns the corresponding limit error without updating this counter.
    pub fn charge_os_call(&mut self) -> Result<(), ExecutionLimitExceeded> {
        charge(
            &mut self.stats.os_calls,
            1,
            self.limits.max_os_calls,
            |limit, attempted| ExecutionLimitExceeded::OsCalls { limit, attempted },
        )
    }

    /// Charge read work before servicing it.
    ///
    /// # Errors
    /// Returns the corresponding limit error without updating this counter.
    pub fn charge_read(&mut self, bytes: u64) -> Result<(), ExecutionLimitExceeded> {
        let call_limit = u64::try_from(self.limits.max_io_call_bytes).unwrap_or(u64::MAX);
        if bytes > call_limit {
            return Err(ExecutionLimitExceeded::ReadCallBytes {
                limit: call_limit,
                attempted: bytes,
            });
        }
        charge(
            &mut self.stats.read_bytes,
            bytes,
            self.limits.max_read_bytes,
            |limit, attempted| ExecutionLimitExceeded::ReadBytes { limit, attempted },
        )
    }

    /// Charge write work before servicing it.
    ///
    /// # Errors
    /// Returns the corresponding limit error without updating this counter.
    pub fn charge_write(&mut self, bytes: usize) -> Result<(), ExecutionLimitExceeded> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let call_limit = u64::try_from(self.limits.max_io_call_bytes).unwrap_or(u64::MAX);
        if bytes > call_limit {
            return Err(ExecutionLimitExceeded::WriteCallBytes {
                limit: call_limit,
                attempted: bytes,
            });
        }
        charge(
            &mut self.stats.write_bytes,
            bytes,
            self.limits.max_write_bytes,
            |limit, attempted| ExecutionLimitExceeded::WriteBytes { limit, attempted },
        )
    }

    /// Charge directory entries work before servicing it.
    ///
    /// # Errors
    /// Returns the corresponding limit error without updating this counter.
    pub fn charge_directory_entries(
        &mut self,
        entries: usize,
    ) -> Result<(), ExecutionLimitExceeded> {
        let entries = u64::try_from(entries).unwrap_or(u64::MAX);
        charge(
            &mut self.stats.directory_entries,
            entries,
            self.limits.max_directory_entries,
            |limit, attempted| ExecutionLimitExceeded::DirectoryEntries { limit, attempted },
        )
    }
}

fn charge<E>(
    used: &mut u64,
    amount: u64,
    limit: u64,
    error: impl FnOnce(u64, u64) -> E,
) -> Result<(), E> {
    let Some(attempted) = used.checked_add(amount) else {
        return Err(error(limit, u64::MAX));
    };
    if attempted > limit {
        Err(error(limit, attempted))
    } else {
        *used = attempted;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_limits_reject_nonempty_work_without_changing_counters() {
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_os_calls: 0,
            max_read_bytes: 0,
            max_write_bytes: 0,
            max_directory_entries: 0,
            ..ExecutionLimits::default()
        });
        budget.charge_read(0).unwrap();
        budget.charge_write(0).unwrap();
        budget.charge_directory_entries(0).unwrap();
        assert!(matches!(
            budget.charge_os_call(),
            Err(ExecutionLimitExceeded::OsCalls {
                limit: 0,
                attempted: 1
            })
        ));
        assert!(matches!(
            budget.charge_read(1),
            Err(ExecutionLimitExceeded::ReadBytes {
                limit: 0,
                attempted: 1
            })
        ));
        assert!(matches!(
            budget.charge_write(1),
            Err(ExecutionLimitExceeded::WriteBytes {
                limit: 0,
                attempted: 1
            })
        ));
        assert!(matches!(
            budget.charge_directory_entries(1),
            Err(ExecutionLimitExceeded::DirectoryEntries {
                limit: 0,
                attempted: 1
            })
        ));
        assert_eq!(budget.stats(), ExecutionStats::default());
    }

    #[test]
    fn per_call_limit_is_checked_before_cumulative_charge() {
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_io_call_bytes: 4,
            max_read_bytes: 6,
            max_write_bytes: 6,
            ..ExecutionLimits::default()
        });
        assert!(matches!(
            budget.charge_read(5),
            Err(ExecutionLimitExceeded::ReadCallBytes {
                limit: 4,
                attempted: 5
            })
        ));
        assert!(matches!(
            budget.charge_write(5),
            Err(ExecutionLimitExceeded::WriteCallBytes {
                limit: 4,
                attempted: 5
            })
        ));
        assert_eq!(budget.stats(), ExecutionStats::default());
        budget.charge_read(4).unwrap();
        budget.charge_write(4).unwrap();
        assert!(matches!(
            budget.charge_read(3),
            Err(ExecutionLimitExceeded::ReadBytes {
                limit: 6,
                attempted: 7
            })
        ));
        assert!(matches!(
            budget.charge_write(3),
            Err(ExecutionLimitExceeded::WriteBytes {
                limit: 6,
                attempted: 7
            })
        ));
        budget.charge_read(2).unwrap();
        budget.charge_write(2).unwrap();
        assert_eq!(budget.stats().read_bytes, 6);
        assert_eq!(budget.stats().write_bytes, 6);
    }

    #[test]
    fn cumulative_counters_never_wrap_or_saturate_to_unlimited() {
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_read_bytes: u64::MAX,
            max_io_call_bytes: usize::MAX,
            ..ExecutionLimits::default()
        });
        budget.stats.read_bytes = u64::MAX - 1;
        budget.charge_read(1).unwrap();
        assert!(matches!(
            budget.charge_read(1),
            Err(ExecutionLimitExceeded::ReadBytes {
                limit: u64::MAX,
                attempted: u64::MAX
            })
        ));
        assert_eq!(budget.stats().read_bytes, u64::MAX);
        budget.stats.os_calls = u64::MAX;
        budget.limits.max_os_calls = u64::MAX;
        assert!(matches!(
            budget.charge_os_call(),
            Err(ExecutionLimitExceeded::OsCalls { .. })
        ));
    }

    #[test]
    fn work_and_denials_are_monotonic_and_snapshots_are_owned() {
        let mut budget = ExecutionBudget::new(ExecutionLimits {
            max_os_calls: 2,
            max_directory_entries: 3,
            ..ExecutionLimits::default()
        });
        budget.charge_os_call().unwrap();
        budget.record_denial();
        let first = budget.stats();
        budget.charge_os_call().unwrap();
        budget.record_denial();
        budget.charge_directory_entries(3).unwrap();
        assert!(budget.charge_os_call().is_err());
        assert!(matches!(
            budget.charge_directory_entries(1),
            Err(ExecutionLimitExceeded::DirectoryEntries {
                limit: 3,
                attempted: 4
            })
        ));
        assert_eq!(first.os_calls, 1);
        assert_eq!(first.denied_accesses, 1);
        assert_eq!(budget.stats().os_calls, 2);
        assert_eq!(budget.stats().denied_accesses, 2);
        assert_eq!(budget.stats().directory_entries, 3);
    }
}
