//! Agena domain event surface.
//!
//! Single unified stream: [`EventKind`] is the only event enum in the
//! system. Its variants reuse the payload structs that live next to it (UI
//! payloads in [`client`], history payloads in
//! `crate::session::history`). Backed by
//! [`agena_event::DomainEvent`].
//!
//! All writes go through [`EventPublisher`]; all readers consume the
//! resulting [`DomainEvent`] envelopes.

mod client;
pub mod kind;

pub use client::{
    CommandBeginEvent, CommandContext, CommandEndEvent, CommandOutputDeltaEvent,
    CommandOutputStream, ErrorInfo, MessagePartDeltaEvent, MessagePartUpdatedEvent,
    PartDeltaField, RunFailedEvent, RunStartedEvent, StreamErrorEvent,
};

pub use kind::{DomainEvent, EventKind, EventPublisher, PluginEventPayload, PublishContext, kinds_table};
