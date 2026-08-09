//! Legacy shim. The retained part model now lives at [`crate::part`];
//! this module re-exports it so historical paths keep resolving.
//!
//! `MessagePart` is the v1 structure and remains here until its removal.

mod message_part;

pub use crate::part::*;
pub use message_part::MessagePart;
