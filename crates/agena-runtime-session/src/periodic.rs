use std::{future::Future, sync::Arc, time::Duration};

use crate::TaskControl;

/// Minimal shutdown/notification port required by periodic runtime loops.
/// Composition owners can implement this for their process-level task
/// controller without making the session crate depend on that controller.
pub trait PeriodicControl: Send + Sync {
    fn is_shutdown(&self) -> bool;
    fn notify(&self) -> &tokio::sync::Notify;
}

impl PeriodicControl for TaskControl {
    fn is_shutdown(&self) -> bool {
        self.is_shutdown()
    }

    fn notify(&self) -> &tokio::sync::Notify {
        self.notify()
    }
}

pub async fn run_periodic<C, I, F, Fut>(control: Arc<C>, mut interval: I, mut tick: F)
where
    C: PeriodicControl + ?Sized,
    I: FnMut() -> Duration + Send,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    loop {
        if control.is_shutdown() {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(interval()) => {}
            _ = control.notify().notified() => {}
        }
        if control.is_shutdown() {
            break;
        }
        tick().await;
    }
}
