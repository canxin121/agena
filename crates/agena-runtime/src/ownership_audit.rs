//! Opt-in process audit for the concrete Runtime ownership boundary.
//!
//! Production processes do not write an audit file by default. Process-level
//! integration tests set [`RUNTIME_OWNERSHIP_AUDIT_PATH_ENV`] on the center and
//! every candidate thin client; every successful concrete Runtime composition
//! then appends one bounded JSON record. This makes an accidental client-local
//! Runtime observable without exposing Runtime handles through a public API.

use std::{
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::AppError;

pub(crate) const RUNTIME_OWNERSHIP_AUDIT_PATH_ENV: &str = "AGENA_RUNTIME_OWNERSHIP_AUDIT_PATH";
pub(crate) const RUNTIME_BOOTSTRAP_FORBIDDEN_ENV: &str = "AGENA_RUNTIME_BOOTSTRAP_FORBIDDEN";

const OWNED_COMPONENTS: &[&str] = &[
    "runtime",
    "provider_clients",
    "scheduler",
    "plugin_host",
    "execution_registry",
    "session_database",
];

#[derive(Serialize)]
struct RuntimeOwnershipAuditRecord<'a> {
    schema: u32,
    pid: u32,
    recorded_at_ms: u128,
    workspace_root: &'a Path,
    components: &'static [&'static str],
}

/// Fail before configuration or database composition when a process has been
/// launched under an explicit thin-client-only contract.
pub(crate) fn ensure_runtime_bootstrap_allowed() -> Result<(), crate::RuntimeBootstrapError> {
    if std::env::var_os(RUNTIME_BOOTSTRAP_FORBIDDEN_ENV).is_some() {
        return Err(crate::RuntimeBootstrapError::configuration(format!(
            "Runtime bootstrap is forbidden in this thin-client process by {RUNTIME_BOOTSTRAP_FORBIDDEN_ENV}"
        )));
    }
    Ok(())
}

/// Append one record after a concrete Runtime has been fully composed.
///
/// The environment variable is intentionally an explicit file path rather
/// than a boolean so tests cannot leak records into a user's normal state
/// directory. When it is set, failure to record is fatal: silently continuing
/// would turn the process-level ownership assertion into a false negative.
pub(crate) fn record_runtime_ownership(workspace_root: &Path) -> Result<(), AppError> {
    let Some(path) = std::env::var_os(RUNTIME_OWNERSHIP_AUDIT_PATH_ENV).map(PathBuf::from) else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Internal(format!(
                "failed to create Runtime ownership audit directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    let workspace_root = std::fs::canonicalize(workspace_root).map_err(|error| {
        AppError::Internal(format!(
            "failed to canonicalize Runtime ownership audit workspace {}: {error}",
            workspace_root.display()
        ))
    })?;
    let record = RuntimeOwnershipAuditRecord {
        schema: 1,
        pid: std::process::id(),
        recorded_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        workspace_root: workspace_root.as_path(),
        components: OWNED_COMPONENTS,
    };
    let mut bytes = serde_json::to_vec(&record).map_err(|error| {
        AppError::Internal(format!(
            "failed to encode Runtime ownership audit record: {error}"
        ))
    })?;
    bytes.push(b'\n');

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|error| {
        AppError::Internal(format!(
            "failed to open Runtime ownership audit file {}: {error}",
            path.display()
        ))
    })?;
    file.write_all(&bytes).map_err(|error| {
        AppError::Internal(format!(
            "failed to append Runtime ownership audit file {}: {error}",
            path.display()
        ))
    })
}
