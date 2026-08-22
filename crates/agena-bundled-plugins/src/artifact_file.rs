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
    let parent = path.parent().ok_or_else(|| {
        PluginError::internal(format!(
            "{purpose} path has no parent directory: {}",
            path.display()
        ))
    })?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            format!("cannot create {purpose} directory"),
            &error,
        ))
    })?;
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
        let path = agena_runtime_tools::canonicalize_mutation_path(&path);
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            if replace && path.exists() {
                agena_runtime_tools::atomic_replace_file(&path, &bytes)
            } else {
                agena_runtime_tools::atomic_create_file(&path, &bytes, None)
            }
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

#[cfg(test)]
mod tests {
    use super::{persist_new, persist_replace_or_create};

    #[tokio::test]
    async fn persistence_honours_create_and_replace_policies() {
        let workspace = tempfile::tempdir().expect("workspace");
        let path = workspace.path().join("artifact.json");

        persist_new(path.clone(), b"complete".to_vec(), "test artifact")
            .await
            .expect("persist artifact");
        let error = persist_new(path.clone(), b"stale".to_vec(), "test artifact")
            .await
            .expect_err("existing artifact must not be overwritten");
        assert!(error.to_string().contains("cannot persist test artifact"));
        assert_eq!(std::fs::read(&path).expect("read artifact"), b"complete");

        persist_replace_or_create(path.clone(), b"replacement".to_vec(), "test artifact")
            .await
            .expect("replace artifact");
        assert_eq!(
            std::fs::read(path).expect("read replacement"),
            b"replacement"
        );
    }
}
