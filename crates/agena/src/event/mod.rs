//! Agena domain event surface.
//!
//! Single unified stream: [`EventKind`] is the only event enum in the
//! system. Its variants reuse the payload structs that live next to it (UI
//! payloads in [`client`], history payloads in `crate::session::history`).
//!
//! All writes go through [`EventPublisher`]; all readers consume the
//! resulting [`DomainEvent`] envelopes.

mod client;
pub mod bus;
pub mod envelope;
pub mod error;
pub mod filter;
pub mod kind;
pub mod publisher;
pub mod sequence;
pub mod event_store;

pub use bus::{EventBus, InProcessEventBus, Subscription};
pub use envelope::EventMeta;
pub use error::{BusError, EventStoreError, PublishError};
pub use filter::{EventFilter, EventKindTag, KindMatcher, Scope};
pub use sequence::SequenceAllocator;
pub use event_store::{EventStore, StoreRange};

pub use client::{
    CommandBeginEvent, CommandContext, CommandEndEvent, CommandOutputDeltaEvent,
    CommandOutputStream, ErrorInfo, MessagePartDeltaEvent, MessagePartUpdatedEvent,
    PartDeltaField, RunFailedEvent, RunStartedEvent, StreamErrorEvent,
};

pub use kind::{DomainEvent, EventKind, EventPublisher, HISTORY_KINDS, UI_KINDS, PluginEventPayload, PublishContext, kinds_table, ui_kinds_table, history_kinds_table};
