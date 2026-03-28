mod command_runner;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::Path;

use crate::sandbox::{SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxPolicy};

pub(crate) fn execute(
    request: &SandboxCommandRequest,
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> Result<SandboxExecOutput, SandboxError> {
    #[cfg(target_os = "windows")]
    {
        windows::execute(request, policy, workspace_root)
    }

    #[cfg(target_os = "linux")]
    {
        linux::execute(request, policy, workspace_root)
    }

    #[cfg(target_os = "macos")]
    {
        macos::execute(request, policy, workspace_root)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        fallback::execute(request, policy, workspace_root)
    }
}
