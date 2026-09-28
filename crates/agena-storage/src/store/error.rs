//! Error type for the parts-first store.
//!
//! One error type serves both the [`PersistenceEngine`](super::engine::PersistenceEngine)
//! contract and the [`SessionStore`] facade. It carries no database types, so
//! the backend-neutral contract crate stays free of SeaORM; the SQLite engine
//! maps `sea_orm::DbErr` into [`StoreError::Database`] at its boundary.

use std::fmt;

/// Errors returned by the persistence engine and the session facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The session or part does not exist.
    NotFound(String),
    /// The operation violates a session/part invariant (invalid lifecycle
    /// transition, terminal part updated, fork of a failed session, etc.).
    InvalidState(String),
    /// A database-level constraint was violated (a trigger fired, a CHECK
    /// failed, a unique key collided).
    Constraint(String),
    /// An optimistic-lock conflict (session version changed underneath us).
    Conflict(String),
    /// A transient SQLite lock conflict; the caller may retry.
    Busy,
    /// JSON serialization or deserialization failed.
    Serialization(String),
    /// An I/O error.
    Io(String),
    /// Any other backend error (SQLite/sqlx surfaced by the engine).
    Database(String),
}

impl StoreError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// Whether this error is transient and the operation may be retried.
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Busy)
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::InvalidState(message) => write!(f, "invalid state: {message}"),
            Self::Constraint(message) => write!(f, "constraint violation: {message}"),
            Self::Conflict(message) => write!(f, "conflict: {message}"),
            Self::Busy => write!(f, "database is busy; retry"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
            Self::Io(message) => write!(f, "i/o error: {message}"),
            Self::Database(message) => write!(f, "database error: {message}"),
        }
    }
}

impl std::error::Error for StoreError {}
