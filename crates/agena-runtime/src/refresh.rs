//! Generic cancellable refresh choreography.

use std::future::Future;

use tokio_util::sync::CancellationToken;

/// Run a refresh, discard its result when cancellation/shutdown wins, then
/// reload the surrounding runtime before returning the refreshed value.
pub async fn run_cancellable_refresh<
    T,
    E,
    IsShutdown,
    Refresh,
    RefreshFuture,
    Reload,
    ReloadFuture,
>(
    cancel: CancellationToken,
    is_shutdown: IsShutdown,
    refresh: Refresh,
    reload: Reload,
) -> Result<Option<T>, E>
where
    IsShutdown: Fn() -> bool,
    Refresh: FnOnce() -> RefreshFuture,
    RefreshFuture: Future<Output = Result<T, E>>,
    Reload: FnOnce() -> ReloadFuture,
    ReloadFuture: Future<Output = Result<(), E>>,
{
    if cancel.is_cancelled() || is_shutdown() {
        return Ok(None);
    }

    let refreshed = refresh().await?;
    if cancel.is_cancelled() || is_shutdown() {
        return Ok(None);
    }

    reload().await?;
    Ok(Some(refreshed))
}
