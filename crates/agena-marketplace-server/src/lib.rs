//! Backend for the plugin marketplace. The `server` feature builds an
//! axum app that serves the registry index and signed plugin tarballs
//! consumed by `agena-plugin-marketplace` clients.

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub use server::{RegistryArtifact, RegistrySnapshot, router, serve};
