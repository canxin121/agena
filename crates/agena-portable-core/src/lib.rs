#![no_std]
//! Target-neutral Agena core metadata for platforms that cannot host the full
//! daemon/TUI process model.
//!
//! This crate deliberately has no allocator, OS, networking, or libc
//! dependency, so every Rust target with a distributed target component can
//! receive a real Agena-owned linkable artifact.

/// ABI version of the portable Agena core surface.
pub const ABI_VERSION: u32 = 1;
/// Agena release version embedded into this artifact.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Return the portable-core ABI version without requiring allocation or std.
#[inline]
pub const fn abi_version() -> u32 {
    ABI_VERSION
}
