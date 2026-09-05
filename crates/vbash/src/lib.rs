//! Native Rust facade for VSH.
//!
//! The execution runtime is introduced behind this crate as each security phase is
//! completed. The facade already exports the canonical types used by both Rust and
//! Python so adapters cannot invent parallel contracts.

mod artifact;
mod hook;
mod review;
mod runtime;

pub use artifact::ArtifactError;

pub use hook::{
    CommitHook, CommitPreparation, CommitResolution, HookBaseline, HookConfig, HookDecision,
    HookDecisionRecord, HookHandlerError, HookScope, HookVerdict, HookedRuntime, RequestEvent,
    ReviewContent,
};

pub use runtime::{
    ArtifactLimits, ExecutionBudget, Receipt, ReceiptDetail, RunMode, RunRequest, Runtime,
    RuntimeConfig, RuntimeDecision, StageTimings, VshError,
};

pub use vsh_commit::{
    CommitConfig, CommitError, CommitPlan, CommitPlanError, CommitReceipt, Committer,
    FaultInjector, FaultPoint, HostError, JournalError, NoFaults, PlanDecodeError,
    RecoveryConflict, RecoveryReport, RevalidationConflict, SnapshotLimits, VerificationFailure,
};

pub use vsh_monty::{
    DEFAULT_VIRTUAL_ROOT, ExecutionError, ExecutionLimitExceeded, ExecutionLimits,
    ExecutionOutcome, ExecutionStats, InProcessConfig, InProcessMonty, MontyFailurePhase,
    MontyObject, MontyType, OsFunctionCall, ResultCompatibility, ResultCompatibilityError,
    SubprocessConfig, SubprocessMonty, VirtualPathError, VirtualRoot, VirtualRootError,
    WorkerFailure, WorkerFailureKind, validate_result_compatibility,
};
pub use vsh_policy::{
    AccessKind, AccessSet, CallPolicy, DEFAULT_SECRET_PATTERNS, DeniedAccess, DenyManifest,
    DenyReason, PatternError, PolicyConfigError, PolicyDecision, PolicyInput, PolicyProfile,
    PolicyThresholds, ProtectedRule, RiskFlag, RiskManifest, RiskMetrics, TransactionIdentityInput,
    TransactionPolicy, bind_transaction, read_set_digest, write_set_digest,
};
pub use vsh_store::{
    ApprovalGrant, ApprovalGrantError, BlobStore, BlobStoreError, CommitReservation, DataDirectory,
    DataDirectoryError, FileStoreConfig, FileTransactionStore, MemoryTransactionStore,
    TransactionRecord, TransactionStore, TransactionStoreError,
};
pub use vsh_types::{
    ApprovalBinding, ApprovalId, BlobId, ContentVersion, DiffDigest, DiffEntry, DiffKind,
    DirectoryDigest, FileStamp, HookId, IntentDigest, NodeKind, NodeState, ParseDigestError,
    PlatformFileId, PolicyDigest, PrincipalId, ProgramDigest, ReadSetDigest, RequestEventId,
    RuntimeConfigDigest, SnapshotId, TransactionBinding, TransactionId, TransactionState,
    TransitionError, VPath, VPathError, WriteSetDigest,
};
pub use vsh_vfs::{
    BaseSnapshot, CanonicalDiff, CanonicalDiffMetrics, CapturedContent, ContentLoadError,
    ContentLoader, Effect, EffectEvent, EffectOrigin, ReadObservation, SnapshotBuilder,
    SnapshotError, SnapshotMetrics, VfsError, VfsMetrics, VirtualFs, WritePrecondition,
};

/// The VSH semantic version shared by native and Python packages.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Identify the implementation behind all public SDK surfaces.
#[must_use]
pub const fn engine_kind() -> &'static str {
    "rust"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exports_one_versioned_rust_engine() {
        assert_eq!(VERSION, "0.5.0");
        assert_eq!(engine_kind(), "rust");
        assert_eq!(VPath::parse("src/lib.rs").unwrap().as_str(), "src/lib.rs");
    }
}
