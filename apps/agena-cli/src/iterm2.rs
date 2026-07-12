//! iTerm2 shell-integration helpers for user-selected local file uploads.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    helper_runner::{
        helper_is_executable, interrupted_status, run_interactive, run_interactive_guarded,
    },
    provider_error::ProviderError,
};

/// Locate the `it2ul` utility installed by iTerm2 shell integration.
///
/// The official installer stores utilities below `~/.iterm2` and generally
/// exposes them through shell aliases, which a process launched by Agena does
/// not inherit. Prefer that stable on-disk path and then fall back to PATH for
/// installations that provide a real executable.
pub(crate) fn upload_utility() -> Option<PathBuf> {
    utility_path("it2ul")
}

pub(crate) fn download_utility() -> Option<PathBuf> {
    utility_path("it2dl")
}

fn utility_path(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".iterm2").join(name));
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join(name)));
    }
    candidates
        .into_iter()
        .find(|path| helper_is_executable(path))
}

pub(crate) fn request_download(path: &Path) -> Result<(), ProviderError> {
    let utility = download_utility().ok_or_else(|| {
        ProviderError::DependencyMissing(
            "iTerm2 download utility `it2dl` is unavailable; install iTerm2 Shell Integration and Utilities on this remote account"
                .to_owned(),
        )
    })?;
    let mut command = Command::new(utility);
    command.arg(path);
    let status = run_interactive(&mut command, "iTerm2 file download")?;
    if status.success() {
        Ok(())
    } else if interrupted_status(status) {
        Err(ProviderError::Cancelled)
    } else {
        Err(ProviderError::Protocol(
            "iTerm2 file download failed or was cancelled".to_owned(),
        ))
    }
}

/// Ask iTerm2 to show its local file picker and unpack the returned upload
/// into `destination`. `it2ul` owns the terminal request/response protocol;
/// it must run while Agena has temporarily released raw terminal input.
pub(crate) fn request_upload(
    destination: &Path,
    guard: &mut dyn FnMut() -> Result<(), ProviderError>,
) -> Result<(), ProviderError> {
    let utility = upload_utility().ok_or_else(|| {
        ProviderError::DependencyMissing(
            "iTerm2 upload utility `it2ul` is unavailable; install iTerm2 Shell Integration and Utilities on this remote account"
                .to_owned(),
        )
    })?;
    let mut command = Command::new(utility);
    command.arg(destination);
    let status = run_interactive_guarded(&mut command, "iTerm2 file upload", guard)?;
    if status.success() {
        Ok(())
    } else if interrupted_status(status) {
        Err(ProviderError::Cancelled)
    } else {
        Err(ProviderError::Protocol(
            "iTerm2 file selection was cancelled or the upload failed".to_owned(),
        ))
    }
}
