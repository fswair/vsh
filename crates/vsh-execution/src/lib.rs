//! Guest-independent namespace, accounting and filesystem authority.
//!
//! This crate has no interpreter, async runtime, worker process or host committer.
//! Adapters translate guest values at their edge; the workspace stays parent-owned.

mod budget;
mod cancellation;
mod gateway;
mod path;

pub use budget::{ExecutionBudget, ExecutionLimitExceeded, ExecutionLimits, ExecutionStats};
pub use cancellation::ExecutionCancellation;
pub use gateway::{
    FsGateway, GatewayError, ObservedEntry, OpenMode, authorize_path, check_path_bytes,
    retain_denial,
};
pub use path::{DEFAULT_VIRTUAL_ROOT, VirtualPathError, VirtualRoot, VirtualRootError};

/// Security-semantic version bound into guest execution configuration fingerprints.
pub const FS_GATEWAY_VERSION: &str = "vsh-fs-gateway-v3";
