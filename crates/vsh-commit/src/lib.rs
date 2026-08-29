//! Capability-rooted revalidation and trusted commit engine for VSH.
//!
//! The committer is the only core component that can mutate a host workspace. It
//! consumes a non-cloneable reservation, revalidates only recorded dependencies,
//! persists an intent journal, moves replaced nodes into quarantine, installs content
//! with create-new semantics, and verifies the final state before marking a commit.

mod committer;
mod host;
mod journal;
mod plan;

pub use committer::{
    CommitConfig, CommitError, CommitReceipt, Committer, FaultInjector, FaultPoint, NoFaults,
    RecoveryConflict, RecoveryReport, RevalidationConflict, VerificationFailure,
};
pub use host::{HostError, SnapshotLimits};
pub use journal::JournalError;
pub use plan::{CommitPlan, CommitPlanError, PlanDecodeError};

#[cfg(test)]
mod tests;
