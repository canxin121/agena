#![cfg_attr(not(feature = "server"), allow(dead_code))]

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub use server::{RegistryArtifact, RegistrySnapshot, router, serve};
