//! `agena-rollout` — append-only JSONL session log + replay.
//!
//! Schema is intentionally agena-agnostic: each line is a self-contained
//! `RolloutFrame` with a sequence number, ISO-8601 timestamp, and an
//! enum-tagged `kind` payload.  The crate exposes a recorder
//! (write side, fsync per append), a reader (lazy line iterator), a
//! `list_sessions` directory walker, and a thin `Replayer` that returns
//! frames for a target session in order.

pub mod error;
pub mod frame;
pub mod reader;
pub mod recorder;

pub use error::{RolloutError, RolloutResult};
pub use frame::{RolloutFrame, RolloutKind, SessionMeta};
pub use reader::{RolloutReader, list_sessions};
pub use recorder::RolloutRecorder;
