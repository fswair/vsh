//! Bashkit is a guest frontend; the host remains VSH's only filesystem authority.
//!
//! `host` does not depend on Bashkit or Tokio. Build the separate executable with
//! `--no-default-features --features worker` to exclude VSH's commit/store authority.

mod protocol;

#[cfg(feature = "host")]
mod host;
#[cfg(feature = "host")]
pub use host::{BashCancellation, BashConfig, BashError, BashOutcome, SubprocessBash};

pub use protocol::BashLimits;

/// Immutable compatibility profile identifier bound into execution identity.
pub const PROFILE_ID: &str = protocol::PROFILE;

/// Build identity of the independent worker, also checked by its RPC handshake.
#[cfg(feature = "worker")]
pub const WORKER_VERSION: &str = protocol::WORKER_ID;

#[cfg(feature = "worker")]
mod guest;
#[cfg(feature = "worker")]
#[doc(hidden)]
pub use guest::worker_main;
