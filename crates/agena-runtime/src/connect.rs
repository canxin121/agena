//! Generic connection reuse and one-time initialization orchestration.

use std::{future::Future, sync::Arc};

/// Reuse an injected connection or create one, then optionally initialize it.
///
/// The runtime owns this lifecycle choreography; callers provide the concrete
/// connector and initializer so no database or application type is required
/// by this crate.
pub async fn connect_or_initialize<T, E, Connect, ConnectFuture, Initialize, InitializeFuture>(
    existing: Option<Arc<T>>,
    initialize: bool,
    connect: Connect,
    initialize_connection: Initialize,
) -> Result<Option<Arc<T>>, E>
where
    Connect: FnOnce() -> ConnectFuture,
    ConnectFuture: Future<Output = Result<T, E>>,
    Initialize: FnOnce(Arc<T>) -> InitializeFuture,
    InitializeFuture: Future<Output = Result<(), E>>,
{
    let connection = match existing {
        Some(connection) => connection,
        None => Arc::new(connect().await?),
    };

    if initialize {
        initialize_connection(Arc::clone(&connection)).await?;
    }

    Ok(Some(connection))
}
