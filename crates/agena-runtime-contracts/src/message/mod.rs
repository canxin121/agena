//! Runtime-neutral message types shared across layers.

#![allow(clippy::module_inception)]

mod message;
mod metadata;
mod part;

// Shim: the retained part model lives at crate::part (see `part/mod.rs`).
pub use crate::part::*;
pub use message::Message;
pub use metadata::{MessageMetadata, MessageProviderState};
pub use part::MessagePart;
