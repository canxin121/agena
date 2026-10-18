//! Read-only transcript wire values.
//! Session mutations and content producers are exclusively kernel capabilities.
mod error;
mod types;
pub use error::StoreError;
pub use types::*;
