//! Compatibility handle for VSH.
//!
//! New applications should depend on `vsh`. This crate contains no implementation;
//! it re-exports the exact same-version `vsh` crate.

pub use vsh_primary::*;
