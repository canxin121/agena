//! Platform-specific descriptor readiness and session termination.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(super) use unix::{PtyIo, signal_group, signal_session};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::{Job, PtyIo};
