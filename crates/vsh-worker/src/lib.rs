//! Cargo graph anchor for source distributions that build the worker binary.
//!
//! The executable remains the package's product. This intentionally empty library lets
//! the non-published `PyO3` build crate retain the worker as a build dependency, so Maturin
//! carries the complete worker package and lockfile edge into an installable sdist.
