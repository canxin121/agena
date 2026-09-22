//! Cancellation belongs to an individual tool call, not to the lifetime of a
//! successfully returned terminal. A dropped launch kills only its new child;
//! a cancelled read leaves the terminal alive. Queued input is never delivered
//! after cancellation, and partially delivered input fails closed.

use super::{MonitorError, invalid};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(super) fn check(cancel: &CancellationToken) -> Result<(), MonitorError> {
    if cancel.is_cancelled() {
        Err(invalid("terminal interaction cancelled"))
    } else {
        Ok(())
    }
}

pub(super) fn interaction<'a>(
    mutex: &'a Mutex<()>,
    cancel: &CancellationToken,
) -> Result<MutexGuard<'a, ()>, MonitorError> {
    loop {
        check(cancel)?;
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}
