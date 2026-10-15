//! Backend integration contract. Application code uses [`crate::store::SessionStore`]
//! and the ready-made memory or SQLite factory. Implementing a persistence
//! backend never exposes the facade's cache, notification bus or mutable rows.

use std::sync::Arc;

pub use crate::store::engine::PersistenceEngine;
pub use crate::store::part_update::{
    prepare_part_update, prepare_run_completion, validate_run_content,
};

/// Wrap a backend in the sealed commit/cache/notification boundary.
pub fn session_store<E: PersistenceEngine + 'static>(
    engine: E,
    max_cached_sessions: usize,
    contents: crate::content::ContentHub,
) -> Arc<dyn crate::store::SessionStore> {
    Arc::new(
        crate::store::facade::SessionFacade::new(engine, max_cached_sessions)
            .with_contents(contents),
    )
}

/// Infrastructure fixtures can compare backend behavior without making a raw
/// engine or the facade's internal state available in normal builds.
#[cfg(feature = "test-support")]
pub use crate::store::in_memory::InMemoryEngine;

#[cfg(feature = "test-support")]
pub fn session_store_with_clock<E: PersistenceEngine + 'static>(
    engine: E,
    max_cached_sessions: usize,
    now_ms: impl Fn() -> i64 + Send + Sync + 'static,
) -> Arc<dyn crate::store::SessionStore> {
    use crate::store::facade::{MemoryLayer, NotificationBus, SessionFacade};
    Arc::new(SessionFacade::with_clock(
        engine,
        MemoryLayer::new(max_cached_sessions),
        NotificationBus::new(),
        now_ms,
    ))
}
