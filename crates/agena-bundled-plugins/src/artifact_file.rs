//! Atomic persistence for binary and JSON artifacts produced by bundled tools.
//!
//! `tempfile` owns staging and atomic rename semantics in
//! `agena-runtime-tools`; this module only supplies the async/bounded plugin
//! boundary and the two overwrite policies needed by artifact-producing tools.

use std::path::PathBuf;

use agena_plugin_host::{PluginError, sdk::Result as SdkResult};

pub(crate) async fn persist_new(
    path: PathBuf,
    bytes: Vec<u8>,
    purpose: &'static str,
) -> SdkResult<()> {
    persist(path, bytes, purpose, false).await
}

pub(crate) async fn persist_replace_or_create(
    path: PathBuf,
    bytes: Vec<u8>,
    purpose: &'static str,
) -> SdkResult<()> {
    persist(path, bytes, purpose, true).await
}

async fn persist(
    path: PathBuf,
    bytes: Vec<u8>,
    purpose: &'static str,
    replace: bool,
) -> SdkResult<()> {
    let display_path = path.display().to_string();
    let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                format!("acquire a {purpose} artifact worker"),
                &error,
            ))
        })?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "artifact path has no parent directory",
            )
        })?;
        let managed = path.starts_with(agena_storage_layout::StorageLayout::current().root());
        if managed {
            agena_storage_layout::secure_directory(parent)?;
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "managed artifact cannot be a symbolic link",
                ));
            }
        } else {
            std::fs::create_dir_all(parent)?;
        }
        let path = agena_runtime_tools::canonicalize_mutation_path(&path);
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            let result = if replace && path.exists() {
                agena_runtime_tools::atomic_replace_file(&path, &bytes)
            } else {
                agena_runtime_tools::atomic_create_file(&path, &bytes, private_permissions(managed))
            };
            result?;
            if managed {
                agena_storage_layout::secure_file(&path)?;
            }
            Ok(())
        })?
    })
    .await
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            format!("{purpose} worker failed"),
            &error,
        ))
    })?
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            format!("cannot persist {purpose} '{display_path}'"),
            &error,
        ))
    })
}

#[cfg(unix)]
fn private_permissions(managed: bool) -> Option<std::fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;
    managed.then(|| std::fs::Permissions::from_mode(0o600))
}
#[cfg(not(unix))]
fn private_permissions(_managed: bool) -> Option<std::fs::Permissions> {
    None
}
