use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// One request's cancellation/commit arbitration. Use a new handle per request.
/// Cancellation wins only before commit entry; an entered commit must be joined
/// and reconciled rather than reported as a cancelled, uncommitted operation.
#[derive(Clone, Debug, Default)]
pub struct ExecutionCancellation(Arc<AtomicU8>);

impl ExecutionCancellation {
    /// Request cancellation. Returns false once commit has already entered.
    #[must_use]
    pub fn cancel(&self) -> bool {
        match self
            .0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(1) => true,
            Err(_) => false,
        }
    }

    /// Whether cancellation won before commit entry.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }

    /// Atomically enter the host commit boundary, unless cancellation won.
    #[must_use]
    pub fn enter_commit(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Whether the host entered commit and its actual outcome must be joined.
    #[must_use]
    pub fn commit_entered(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }
}

#[cfg(test)]
mod tests {
    use super::ExecutionCancellation;

    #[test]
    fn cancellation_and_commit_are_mutually_exclusive_and_shared() {
        let cancelled = ExecutionCancellation::default();
        assert!(cancelled.clone().cancel());
        assert!(cancelled.is_cancelled());
        assert!(!cancelled.enter_commit());
        let entered = ExecutionCancellation::default();
        assert!(entered.enter_commit());
        assert!(!entered.cancel());
        assert!(entered.clone().commit_entered());
        assert!(!entered.is_cancelled());
        assert!(!entered.enter_commit());
    }
}
