//! Primary Rust SDK for VSH.
//!
//! The implementation remains isolated in `vsh-runtime`; this crate is the stable
//! application-facing registry handle.

pub use vsh_runtime_core::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_handle_reexports_the_native_runtime() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert_eq!(engine_kind(), "rust");
    }
}
