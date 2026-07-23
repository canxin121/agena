//! Generic optional asynchronous service construction.

#[cfg(test)]
use std::future::Future;

/// Build an optional service only when its feature/configuration is enabled.
#[cfg(test)]
pub async fn build_optional<T, Build, BuildFuture>(enabled: bool, build: Build) -> Option<T>
where
    Build: FnOnce() -> BuildFuture,
    BuildFuture: Future<Output = T>,
{
    if enabled { Some(build().await) } else { None }
}
