//! Strongly-typed hook input / output structures. Plugin authors write against
//! these; the host serializes them as JSON-RPC params.

pub mod tool;
pub mod chat;
pub mod event;
pub mod auth;
pub mod provider;
pub mod permission;
pub mod command;
pub mod shell;
pub mod config;
pub mod session;
pub mod prompt;
pub mod agent;

pub use agent::*;
pub use auth::*;
pub use chat::*;
pub use command::*;
pub use config::*;
pub use event::*;
pub use permission::*;
pub use prompt::*;
pub use provider::*;
pub use session::*;
pub use shell::*;
pub use tool::*;
