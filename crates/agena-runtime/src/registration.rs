//! Generic snapshot-scoped asynchronous registration helpers.

use std::future::Future;

/// Spawn a cancellable batch of registrations and retain the guard with the
/// snapshot's runtime service bundle.
pub fn spawn_registration_batch<I, F, Fut>(entries: I, mut register: F) -> crate::AbortOnDrop
where
    I: IntoIterator + Send + 'static,
    I::IntoIter: Send + 'static,
    I::Item: Send + 'static,
    F: FnMut(I::Item) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    crate::spawn_abortable(async move {
        for entry in entries {
            register(entry).await;
        }
    })
}
