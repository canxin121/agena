//! # agena-runtime-session-core
//!
//! Persistence- and execution-neutral session model types.
//!
//! This crate holds the core session data model ([`model`]), the session
//! module structure without pulling in persistence engines, execution
//! services, or provider adapters. It is the
//! dependency root for the heavier [`agena_runtime_session`] crate.

pub use agena_runtime_contracts::{authorization, message};
pub mod model;
pub use model::*;
pub mod session {
    pub use crate::model::*;
}
