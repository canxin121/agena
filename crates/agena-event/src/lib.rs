//! # agena-event
//!
//! Generic event-bus & store primitives that back agena's unified domain
//! event stream. This crate is intentionally **payload-agnostic**: it owns the
//! envelope (`DomainEvent<K>`), filtering, in-process pub/sub
//! (`InProcessEventBus`), and the `EventStore` trait. Concrete event-kind
//! enums (e.g. `agena::event::EventKind`) live in the `agena` core crate so
//! they can reference domain types without dragging them down here.

pub mod bus;
pub mod envelope;
pub mod error;
pub mod filter;
pub mod publisher;
pub mod sequence;
pub mod store;

pub use bus::{EventBus, InProcessEventBus, Subscription};
pub use envelope::{DomainEvent, EventMeta};
pub use error::{BusError, EventStoreError, PublishError};
pub use filter::{EventFilter, EventKindTag, KindMatcher, Scope};
pub use publisher::EventPublisher;
pub use sequence::SequenceAllocator;
pub use store::{EventStore, StoreRange};
