//! Async LSP client used by agena tools.
//!
//! Speaks Language Server Protocol over stdio with the per-server child
//! process model that LSP itself standardises. Limited to a curated subset
//! of LSP requests — definition, references, diagnostics, hover — that
//! cover the agena tool API; we are not building a full editor here.
//!
//! Layout mirrors `agena-mcp-client`: a low-level `transport` (in-memory
//! or stdio), a JSON-RPC bookkeeping `client`, a multi-server `registry`,
//! and `server_spec` as the typed config glue.

pub mod client;
pub mod error;
pub mod protocol;
pub mod registry;
pub mod server_spec;
pub mod transport;

pub use client::{LspClient, ServerNotification};
pub use error::{LspError, LspResult};
pub use registry::{LspRegistry, ResolveError};
pub use server_spec::LspServerSpec;
pub use transport::{LspTransport, StdioTransport};

pub use lsp_types;
