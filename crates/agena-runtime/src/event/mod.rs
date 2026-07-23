//! Agena domain event surface.
//!
//! Single unified stream: [`EventKind`] is the only event enum in the
//! system. Its variants reuse the payload structs that live next to it (UI
//! payloads in [`client`], history payloads in `crate::session::history`).
//!
//! All writes go through [`EventPublisher`]; all readers consume the
//! resulting [`DomainEvent`] envelopes.

pub(crate) mod bridge;
pub mod bus;
mod client;
pub mod error;
pub mod kind;
pub mod publisher;

pub use bus::{EventBus, InProcessEventBus, Subscription};
pub use client::MessagePartCheckpointedEvent;

pub use kind::{DomainEvent, EventKind, EventPublisher, PluginEventPayload, PublishContext};
