//! Bridge the server-owned preview registry into the existing browser tools.
use agena_web::managed_preview::{
    ManagedPreviewInfo, ManagedPreviewLease, ManagedPreviewProvider, ManagedPreviewTarget,
};
use futures_util::future::BoxFuture;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

use super::{
    access::{PreviewAccess, run_generation},
    registry::{PreviewSessionRecord, WorkspacePreviewRegistry},
    runtime::WorkspacePreviewRuntime,
};

pub(crate) struct PreviewBrowserProvider {
    pub(crate) registry: Arc<WorkspacePreviewRegistry>,
    pub(crate) runtime: Arc<WorkspacePreviewRuntime>,
    pub(crate) access: Arc<PreviewAccess>,
    pub(crate) backend_origin: String,
}

async fn owned_by(record: &PreviewSessionRecord, workspace: &Path, session_id: &str) -> bool {
    if record.agena_session_id.as_deref() != Some(session_id) {
        return false;
    }
    let Ok(expected) = tokio::fs::canonicalize(workspace).await else {
        return false;
    };
    tokio::fs::canonicalize(&record.directory)
        .await
        .is_ok_and(|path| path == expected)
}

struct PreviewBrowserLease {
    registry: Arc<WorkspacePreviewRegistry>,
    runtime: Arc<WorkspacePreviewRuntime>,
    access: Arc<PreviewAccess>,
    preview_id: String,
    generation: String,
    host: String,
}

impl ManagedPreviewLease for PreviewBrowserLease {
    fn validate(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let grant = self
                .access
                .lookup(&self.host)
                .await
                .ok_or("preview access expired or was revoked")?;
            if grant.preview_id != self.preview_id || grant.generation != self.generation {
                return Err("preview grant no longer belongs to this run".into());
            }
            let record = self
                .registry
                .get_by_id(&self.preview_id)
                .await
                .ok_or("preview was deleted")?;
            if run_generation(&record) != self.generation
                || !self.runtime.is_owned_running(&record.id)
            {
                return Err("preview run changed or stopped; reopen it before interacting".into());
            }
            Ok(())
        })
    }
}

impl ManagedPreviewProvider for PreviewBrowserProvider {
    fn resolve(
        &self,
        workspace: PathBuf,
        session_id: String,
        preview_id: String,
    ) -> BoxFuture<'_, Result<Option<ManagedPreviewTarget>, String>> {
        Box::pin(async move {
            let Some(record) = self.registry.get_by_id(&preview_id).await else {
                return Ok(None);
            };
            if !owned_by(&record, &workspace, &session_id).await {
                return Ok(None);
            }
            // Agent access is limited to a live child owned by this server.
            // Manually attached targets are available only through the user's UI.
            if !self.runtime.is_owned_running(&record.id) {
                return Err("preview process is not owned by the current Agena server".into());
            }
            let access = self
                .access
                .issue_for_browser(&record, &self.backend_origin)
                .await
                .map_err(|error| error.to_string())?;
            let url = Url::parse(&access.url).map_err(|error| error.to_string())?;
            let lease = PreviewBrowserLease {
                registry: self.registry.clone(),
                runtime: self.runtime.clone(),
                access: self.access.clone(),
                preview_id: preview_id.clone(),
                generation: access.generation.clone(),
                host: url.host_str().ok_or("preview URL has no host")?.to_string(),
            };
            let upstream = self
                .access
                .validate_record(&record)
                .map_err(|error| error.to_string())?;
            Ok(Some(ManagedPreviewTarget::new(
                preview_id,
                access.generation,
                url,
                Arc::new(lease),
                upstream,
            )))
        })
    }

    fn list(
        &self,
        workspace: PathBuf,
        session_id: String,
    ) -> BoxFuture<'_, Result<Vec<ManagedPreviewInfo>, String>> {
        Box::pin(async move {
            let records = self
                .registry
                .list_all()
                .await
                .map_err(|error| error.to_string())?;
            let mut result = Vec::new();
            for record in records.sessions {
                if owned_by(&record, &workspace, &session_id).await {
                    result.push(ManagedPreviewInfo {
                        preview_id: record.id,
                        state: record.state,
                    });
                }
            }
            Ok(result)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn browser_preview_requires_both_workspace_and_session_ownership() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let record: PreviewSessionRecord = serde_json::from_value(serde_json::json!({
            "id":"demo", "directory":root.path().to_str().unwrap(), "runDirectory":root.path().to_str().unwrap(),
            "agenaSessionId":"7", "state":"stopped", "proxyBasePath":"/api/v1/workbench/preview/s/demo/",
            "targetUrl":"http://127.0.0.1:5173/", "command":"node", "args":[], "logsPath":"/tmp/preview.log", "updatedAt":0
        })).unwrap();
        assert!(owned_by(&record, root.path(), "7").await);
        assert!(!owned_by(&record, root.path(), "8").await);
        assert!(!owned_by(&record, other.path(), "7").await);
        let mut unowned = record;
        unowned.agena_session_id = None;
        assert!(!owned_by(&unowned, root.path(), "7").await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provider_requires_a_live_owned_run_and_revokes_old_browser_leases() {
        use super::super::{registry::PreviewSessionCreateRequest, runtime::tests::test_registry};
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (_, registry) = test_registry(root.path()).await;
        registry
            .create_server_session(PreviewSessionCreateRequest {
                id: "owned".into(),
                directory: root.path().to_string_lossy().into_owned(),
                run_directory: root.path().to_string_lossy().into_owned(),
                agena_session_id: Some("7".into()),
                command: "sh".into(),
                args: vec!["-c".into(), "exec sleep 30".into()],
                logs_path: root
                    .path()
                    .join("preview.log")
                    .to_string_lossy()
                    .into_owned(),
                target_url: Url::parse("http://127.0.0.1:39999/").unwrap(),
            })
            .await
            .unwrap();
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let access = Arc::new(PreviewAccess::new(None, 8080).unwrap());
        let provider = PreviewBrowserProvider {
            registry,
            runtime: runtime.clone(),
            access: access.clone(),
            backend_origin: "http://127.0.0.1:8080".into(),
        };
        // A matching database owner without a live handle is insufficient.
        assert!(
            provider
                .resolve(root.path().into(), "7".into(), "owned".into())
                .await
                .is_err()
        );
        runtime.start_by_id("owned").await.unwrap();
        assert!(
            provider
                .resolve(other.path().into(), "7".into(), "owned".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            provider
                .resolve(root.path().into(), "8".into(), "owned".into())
                .await
                .unwrap()
                .is_none()
        );
        let target = provider
            .resolve(root.path().into(), "7".into(), "owned".into())
            .await
            .unwrap()
            .unwrap();
        target.validate_url(target.url()).await.unwrap();
        assert!(
            access
                .lookup(target.url().host_str().unwrap())
                .await
                .unwrap()
                .browser_only
        );
        runtime.stop_by_id("owned").await.unwrap();
        assert!(target.validate().await.is_err());
        runtime.start_by_id("owned").await.unwrap();
        let replacement = provider
            .resolve(root.path().into(), "7".into(), "owned".into())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(target.generation(), replacement.generation());
        assert_ne!(target.url(), replacement.url());
        assert!(target.validate().await.is_err());
        replacement.validate().await.unwrap();
        access.revoke("owned").await;
        assert!(replacement.validate().await.is_err());
        runtime.shutdown().await.unwrap();
    }
}
