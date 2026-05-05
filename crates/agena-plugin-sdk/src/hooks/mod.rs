//! Strongly-typed hook input / output structures. Plugin authors write against
//! these; the host serializes them as JSON-RPC params.

pub mod agent;
pub mod auth;
pub mod chat;
pub mod command;
pub mod config;
pub mod event;
pub mod notification;
pub mod permission;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod shell;
pub mod tool;

pub use agent::*;
pub use auth::*;
pub use chat::*;
pub use command::*;
pub use config::*;
pub use event::*;
pub use notification::*;
pub use permission::*;
pub use prompt::*;
pub use provider::*;
pub use session::*;
pub use shell::*;
pub use tool::*;
