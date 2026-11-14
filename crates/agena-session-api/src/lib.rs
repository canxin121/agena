//! Session operation contract shared by every transport.
//!
//! `Command` and `Query` are the single declaration of what a client may do to
//! a session. REST, WebSocket and plugins all route into these two enums, so a
//! new operation is added once and every `match` over them must handle it.

pub mod activity;
pub mod bridge;
pub mod business;
pub mod commands;
pub mod content;
pub mod extended_sessions;
pub mod interaction;
pub mod live;
pub mod management;
pub mod pagination;
pub mod part;
pub mod queries;
pub mod resource;
pub mod server_control;

pub use commands::*;
pub use queries::*;
pub use resource::*;
