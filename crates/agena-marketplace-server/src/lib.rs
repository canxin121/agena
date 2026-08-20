//! Optional typed mirror for the GitHub-backed Agena plugin marketplace. The
//! `server` feature serves the same standard index, release manifests, and
//! immutable artifacts consumed by `agena-plugin-marketplace` clients.

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub use server::{RegistryArtifact, RegistrySnapshot, router, serve};
