//! Fixture documents and settings operations. Agena itself performs every merge
//! and reload; this module never reimplements its configuration precedence.

use std::{
    path::{Component, Path, PathBuf},
    time::Duration,
};

use agena_api::resource::{
    RuntimeBackgroundTaskKind, RuntimeBackgroundTaskStartResponse, RuntimeBackgroundTaskStatus,
    ServerIdentityResource,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{case::TestCase, http::TestHttpResponse};

/// The exact bytes of one production configuration source.
#[derive(Clone, Debug, Default)]
pub enum ConfigFile {
    Json(Value),
    Bytes(Vec<u8>),
    #[default]
    Missing,
}

impl From<Value> for ConfigFile {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl ConfigFile {
    pub(crate) async fn write(&self, path: &Path) -> Result<()> {
        match self {
            Self::Missing => match tokio::fs::remove_file(path).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
            Self::Json(value) => write_bytes(path, &serde_json::to_vec_pretty(value)?).await,
            Self::Bytes(bytes) => write_bytes(path, bytes).await,
        }
        .with_context(|| format!("writing configuration source {}", path.display()))
    }
}

pub(crate) async fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await?;
    Ok(())
}

/// All paths belong to one fixture. Neither the user's normal data directory nor
/// its discovery record is consulted.
#[derive(Clone, Debug)]
pub struct CasePaths {
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub workspace: PathBuf,
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub diagnostics: PathBuf,
    pub global_config: PathBuf,
    pub workspace_config: PathBuf,
    pub endpoint_record: PathBuf,
}

impl CasePaths {
    pub(crate) fn new(root: PathBuf, cwd: &Path) -> Result<Self> {
        let data_dir = root.join("data");
        let workspace = root.join("workspace");
        Ok(Self {
            cwd: owned_path(&root, cwd)?,
            home: root.join("home"),
            diagnostics: root.join("diagnostics"),
            global_config: data_dir.join("agena.json"),
            workspace_config: workspace.join(".agena/agena.json"),
            endpoint_record: data_dir.join("run/server.json"),
            root,
            data_dir,
            workspace,
        })
    }

    pub(crate) async fn create(&self) -> Result<()> {
        for path in [
            &self.data_dir,
            &self.workspace,
            &self.cwd,
            &self.home,
            &self.diagnostics,
            &self.root.join("tmp"),
            &self.root.join("tmux"),
        ] {
            tokio::fs::create_dir_all(path).await?;
        }
        Ok(())
    }
}

/// Reject traversal, absolute paths, and existing symlinks before fixture writes.
/// This is an ownership guard, not a security sandbox against racing processes.
pub(crate) fn owned_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        !relative.as_os_str().is_empty(),
        "fixture path must not be empty"
    );
    let mut path = root.to_owned();
    for component in relative.components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => continue,
            _ => bail!(
                "fixture path must stay inside its case directory: {}",
                relative.display()
            ),
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "fixture path crosses a symlink: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(
        path != root,
        "fixture path must name a file or subdirectory"
    );
    Ok(path)
}

/// A runnable local Responses provider. This is an explicit fixture document,
/// not a hidden override of RunOptions. Replacing the global document replaces
/// this whole preset; use configure_global to edit it before startup.
pub fn local_model_config(base_url: &str, model_id: &str) -> Result<Value> {
    let url = reqwest::Url::parse(base_url).context("invalid scripted model URL")?;
    ensure!(
        url.scheme() == "http"
            && url
                .host_str()
                .and_then(|host| host.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
            && url.username().is_empty()
            && url.password().is_none(),
        "scripted model must use an HTTP loopback URL"
    );
    let mut models = serde_json::Map::new();
    models.insert(
        model_id.to_owned(),
        json!({
            "native_compaction": false,
            "context_window_tokens": 131072,
            "max_output_tokens": 8192,
            "agena_tools": {"mode": "provider_protocol"}
        }),
    );
    Ok(json!({
        "providers": {
            "default_selection": {
                "provider": "testkit",
                "adapter": "openai_responses",
                "model": model_id
            },
            "testkit": {
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "base_url": base_url,
                    "api_key": {"kind": "inline", "value": "agena-testkit"}
                },
                "adapters": {
                    "openai_responses": {
                        "enabled": true,
                        "model_discovery": "configured_only",
                        "models": models
                    }
                }
            }
        },
        "runtime": {"providers": {"client_versions": {"auto_update": false}}},
        "session": {"auto_title": {"enabled": false}}
    }))
}

#[derive(Clone, Copy, Debug)]
pub enum SettingsLayer {
    Global,
    Workspace,
}

impl SettingsLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}

/// Every production edit option is exposed, including invalid combinations that
/// a negative test deliberately sends. Raw HTTP remains available for malformed
/// documents and options not yet represented here.
#[derive(Clone, Debug, Serialize)]
pub struct EditOptions {
    pub dry_run: bool,
    pub validate: bool,
    pub reload: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            validate: true,
            reload: true,
            expected_revision: None,
        }
    }
}

/// The last successfully loaded runtime configuration, independent of whether
/// its current source files still parse. All fields belong to one server and
/// runtime generation observed before and after the public API reads.
#[derive(Clone, Debug)]
pub struct LoadedConfigSnapshot {
    pub generation: u64,
    pub server: agena_api::resource::ServerIdentityResource,
    pub runtime: Value,
    pub loaded: Value,
}

/// Loaded state observed within one server identity and runtime generation.
/// Source files carry their own revisions and are not an atomic multi-file
/// snapshot. A failed reload may leave them different from the loaded state.
#[derive(Clone, Debug)]
pub struct ConfigSnapshot {
    pub generation: u64,
    pub server: agena_api::resource::ServerIdentityResource,
    pub runtime: Value,
    pub loaded: Value,
    pub global: Value,
    pub workspace: Value,
}

#[derive(Clone)]
pub struct SettingsDriver {
    case: TestCase,
    wait_timeout: Duration,
}

impl SettingsDriver {
    pub(crate) fn new(case: TestCase) -> Self {
        let wait_timeout = case.timeout();
        Self { case, wait_timeout }
    }

    /// Set the deadline for reload and loaded-value waits. Individual raw HTTP
    /// operations keep the HTTP client's own request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.wait_timeout = timeout;
        self
    }

    pub async fn layer(
        &self,
        layer: SettingsLayer,
        path: Option<&str>,
    ) -> Result<TestHttpResponse> {
        let http = self.case.http().await;
        let mut request = http.request(
            Method::GET,
            &format!("/api/v1/settings/layers/{}", layer.as_str()),
        )?;
        if let Some(path) = path {
            request = request.query(&[("path", path)]);
        }
        http.execute(request).await
    }

    pub async fn set(
        &self,
        layer: SettingsLayer,
        path: &str,
        value: Value,
        options: EditOptions,
    ) -> Result<TestHttpResponse> {
        let mut body = serde_json::to_value(options)?;
        body["path"] = json!(path);
        body["value"] = value;
        self.case
            .http()
            .await
            .send_json(
                Method::PUT,
                &format!("/api/v1/settings/layers/{}", layer.as_str()),
                &body,
            )
            .await
    }

    pub async fn patch(
        &self,
        path: Option<&str>,
        changes: Value,
        options: EditOptions,
    ) -> Result<TestHttpResponse> {
        let mut body = serde_json::to_value(options)?;
        if let Some(path) = path {
            body["path"] = json!(path);
        }
        body["changes"] = changes;
        self.case
            .http()
            .await
            .send_json(Method::PATCH, "/api/v1/settings", &body)
            .await
    }

    pub async fn delete(
        &self,
        layer: SettingsLayer,
        path: &str,
        options: EditOptions,
    ) -> Result<TestHttpResponse> {
        let http = self.case.http().await;
        let request = http
            .request(
                Method::DELETE,
                &format!("/api/v1/settings/layers/{}", layer.as_str()),
            )?
            .query(&options)
            .query(&[("path", path)]);
        http.execute(request).await
    }

    pub async fn validate(&self) -> Result<TestHttpResponse> {
        self.case
            .http()
            .await
            .send_json(Method::POST, "/api/v1/settings/validate", &json!({}))
            .await
    }

    pub async fn reload(&self) -> Result<TestHttpResponse> {
        self.case
            .http()
            .await
            .send_json(Method::POST, "/api/v1/runtime/reload", &json!({}))
            .await
    }

    /// Start the real asynchronous reload and bind its receipt to this server
    /// identity. A different task or a restart cannot satisfy the returned wait.
    pub async fn start_reload(&self) -> Result<ReloadHandle> {
        let cancel = self.case.cancellation_token();
        let model = self.case.model();
        let operation = async {
            let client = self.case.client().await;
            let server = client.server_identity().await?;
            let before = client.runtime_status().await?;
            let response: RuntimeBackgroundTaskStartResponse =
                self.reload().await?.require_success()?.json()?;
            ensure!(
                response.task.kind == RuntimeBackgroundTaskKind::RuntimeReload,
                "reload returned a different task kind: {:?}",
                response.task.kind
            );
            ensure!(
                !response.task.id.is_empty(),
                "reload returned an empty task receipt"
            );
            ensure!(
                client.server_identity().await? == server,
                "server identity changed while starting runtime reload"
            );
            Ok(ReloadHandle {
                case: self.case.clone(),
                task_id: response.task.id,
                previous_generation: before.generation,
                server,
                timeout: self.wait_timeout,
                require_new_generation: response.started,
            })
        };
        tokio::select! {
            () = cancel.cancelled() => bail!("case ended while starting runtime reload"),
            error = model.failed() => Err(error),
            result = tokio::time::timeout(self.wait_timeout, operation) =>
                result.context("starting runtime reload exceeded its deadline")?,
        }
    }

    /// Bind a plugin's `reload_task_id` to a snapshot captured before its edit.
    /// This does not start another reload or reinterpret the plugin's result.
    pub fn track_reload(
        &self,
        task_id: impl Into<String>,
        before: &ConfigSnapshot,
    ) -> ReloadHandle {
        ReloadHandle {
            case: self.case.clone(),
            task_id: task_id.into(),
            previous_generation: before.generation,
            server: before.server.clone(),
            timeout: self.wait_timeout,
            require_new_generation: true,
        }
    }

    pub async fn reload_and_wait(&self) -> Result<ConfigSnapshot> {
        let cancel = self.case.cancellation_token();
        let model = self.case.model();
        let operation = async {
            let client = self.case.client().await;
            let server = client.server_identity().await?;
            let before = client.runtime_status().await?.generation;
            loop {
                let snapshot = self.start_reload().await?.wait().await?;
                ensure!(
                    snapshot.server == server,
                    "server identity changed while requesting a fresh runtime reload"
                );
                if snapshot.generation > before {
                    return Ok(snapshot);
                }
                // The request joined a task whose generation was already visible.
                // Its exact receipt has now completed; request a fresh task.
            }
        };
        tokio::select! {
            () = cancel.cancelled() => bail!("case ended while waiting for a fresh runtime reload"),
            error = model.failed() => Err(error),
            result = tokio::time::timeout(self.wait_timeout, operation) =>
                result.context("runtime reload did not publish a fresh snapshot before the deadline")?,
        }
    }

    /// Wait for a value in the public resolved-config document, for example
    /// `/config/session/max_turns`. This observes eventual activation; it does
    /// not claim that any particular reload task succeeded. Use ReloadHandle
    /// when the task receipt itself is part of the assertion.
    pub async fn wait_loaded(
        &self,
        pointer: &str,
        expected: Value,
    ) -> Result<LoadedConfigSnapshot> {
        ensure!(
            pointer.is_empty() || pointer.starts_with('/'),
            "loaded configuration path must be a JSON pointer: {pointer:?}"
        );
        let cancel = self.case.cancellation_token();
        let model = self.case.model();
        let mut observed = String::from("no snapshot observed");
        let wait = async {
            let client = self.case.client().await;
            let server = client.server_identity().await?;
            loop {
                let snapshot = self.loaded_snapshot().await?;
                ensure!(
                    snapshot.server == server,
                    "server identity changed while waiting for loaded configuration {pointer}"
                );
                let actual = snapshot.loaded.pointer(pointer);
                if actual == Some(&expected) {
                    return Ok(snapshot);
                }
                observed = format!(
                    "generation {}: {}",
                    snapshot.generation,
                    actual
                        .map(bounded_value)
                        .unwrap_or_else(|| "<missing>".into())
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        let result = tokio::select! {
            () = cancel.cancelled() => bail!("case ended while waiting for loaded configuration {pointer}"),
            error = model.failed() => return Err(error),
            result = tokio::time::timeout(self.wait_timeout, wait) => result,
        };
        match result {
            Ok(result) => result,
            Err(_) => bail!(
                "loaded configuration {pointer} did not become {} within {:?}; last observed {observed}",
                bounded_value(&expected),
                self.wait_timeout
            ),
        }
    }

    /// Read the published configuration without parsing global/workspace source
    /// files. This remains useful after a malformed edit causes reload failure.
    pub async fn loaded_snapshot(&self) -> Result<LoadedConfigSnapshot> {
        let cancel = self.case.cancellation_token();
        let model = self.case.model();
        let read = async {
            loop {
                let client = self.case.client().await;
                let identity_before = client.server_identity().await?;
                let before = client.runtime_status().await?;
                let loaded = client.resolved_config().await?;
                let after = client.runtime_status().await?;
                let identity_after = client.server_identity().await?;
                if before.generation == after.generation && identity_before == identity_after {
                    return Ok(LoadedConfigSnapshot {
                        generation: after.generation,
                        server: identity_after,
                        runtime: serde_json::to_value(after)?,
                        loaded,
                    });
                }
                tokio::task::yield_now().await;
            }
        };
        tokio::select! {
            () = cancel.cancelled() => bail!("case ended while reading loaded configuration"),
            error = model.failed() => Err(error),
            result = tokio::time::timeout(self.wait_timeout, read) =>
                result.context("loaded configuration snapshot did not stabilize before the deadline")?,
        }
    }

    pub async fn snapshot(&self) -> Result<ConfigSnapshot> {
        tokio::time::timeout(self.case.timeout(), async {
            loop {
                let client = self.case.client().await;
                let identity_before = client.server_identity().await?;
                let before = client.runtime_status().await?;
                let loaded = client.resolved_config().await?;
                let global = self
                    .layer(SettingsLayer::Global, None)
                    .await?
                    .require_success()?
                    .json()?;
                let workspace = self
                    .layer(SettingsLayer::Workspace, None)
                    .await?
                    .require_success()?
                    .json()?;
                let after = client.runtime_status().await?;
                let identity_after = client.server_identity().await?;
                if before.generation == after.generation && identity_before == identity_after {
                    return Ok(ConfigSnapshot {
                        generation: after.generation,
                        server: identity_after,
                        runtime: serde_json::to_value(after)?,
                        loaded,
                        global,
                        workspace,
                    });
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .context("configuration kept changing while reading its snapshot")?
    }
}

/// One accepted runtime reload on one server epoch. A newly started task and a
/// plugin receipt require a generation newer than their starting observation.
/// A coalesced request can join a task whose generation was already published;
/// `reload_and_wait` additionally guarantees a fresh generation in that case.
#[derive(Clone)]
pub struct ReloadHandle {
    case: TestCase,
    task_id: String,
    previous_generation: u64,
    server: ServerIdentityResource,
    timeout: Duration,
    require_new_generation: bool,
}

impl ReloadHandle {
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Wait for success; failed/cancelled tasks report their actual public
    /// problem. Unknown receipts time out rather than following another reload.
    pub async fn wait(&self) -> Result<ConfigSnapshot> {
        ensure!(
            !self.task_id.is_empty(),
            "reload task receipt must not be empty"
        );
        let cancel = self.case.cancellation_token();
        let model = self.case.model();
        let mut observed = String::from("task has not been observed");
        let wait = async {
            let client = self.case.client().await;
            loop {
                ensure!(
                    client.server_identity().await? == self.server,
                    "reload {} belongs to a different server identity; reattach after restart",
                    self.task_id
                );
                let runtime = client.runtime_status().await?;
                if let Some(task) = runtime
                    .background_tasks
                    .iter()
                    .find(|task| task.id == self.task_id)
                {
                    ensure!(
                        task.kind == RuntimeBackgroundTaskKind::RuntimeReload,
                        "task {} is {:?}, not a runtime reload",
                        self.task_id,
                        task.kind
                    );
                    observed = format!(
                        "task {:?}, generation {} (requires > {})",
                        task.status, runtime.generation, self.previous_generation
                    );
                    match task.status {
                        RuntimeBackgroundTaskStatus::Failed
                        | RuntimeBackgroundTaskStatus::Cancelled => {
                            let detail = if let Some(problem) = &task.failure {
                                bounded_value(&serde_json::to_value(problem)?)
                            } else {
                                bounded_value(&json!(
                                    task.message
                                        .as_deref()
                                        .unwrap_or("no public failure detail")
                                ))
                            };
                            bail!(
                                "runtime reload {} ended as {:?}: {detail}",
                                self.task_id,
                                task.status
                            );
                        }
                        RuntimeBackgroundTaskStatus::Succeeded
                            if !self.require_new_generation
                                || runtime.generation > self.previous_generation =>
                        {
                            let snapshot = self.case.settings().snapshot().await?;
                            ensure!(
                                snapshot.server == self.server,
                                "reload {} belongs to a different server identity; reattach after restart",
                                self.task_id
                            );
                            ensure!(
                                !self.require_new_generation
                                    || snapshot.generation > self.previous_generation,
                                "reload {} completed without a fresh runtime snapshot",
                                self.task_id
                            );
                            return Ok(snapshot);
                        }
                        _ => {}
                    }
                } else {
                    observed = format!(
                        "task absent, generation {} (requires > {})",
                        runtime.generation, self.previous_generation
                    );
                }
                // Status reads the snapshot and task registry separately. A
                // completed task can briefly accompany the prior generation.
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        let result = tokio::select! {
            () = cancel.cancelled() => bail!("case ended while waiting for runtime reload {}", self.task_id),
            error = model.failed() => return Err(error),
            result = tokio::time::timeout(self.timeout, wait) => result,
        };
        match result {
            Ok(result) => result,
            Err(_) => bail!(
                "runtime reload {} did not complete within {:?}; last observed {observed}",
                self.task_id,
                self.timeout
            ),
        }
    }
}

fn bounded_value(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= 1024 {
        return text;
    }
    format!(
        "{}…[truncated]",
        text.chars().take(1024).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn raw_and_missing_sources_reach_disk_without_interpretation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workspace/.agena/agena.json");
        let bytes = vec![b'{', 0xff, b'\n'];
        ConfigFile::Bytes(bytes.clone()).write(&path).await?;
        assert_eq!(tokio::fs::read(&path).await?, bytes);
        ConfigFile::Missing.write(&path).await?;
        assert!(!path.exists());
        ConfigFile::Missing.write(&path).await?;
        Ok(())
    }

    #[test]
    fn seed_paths_cannot_escape_owned_directory() -> Result<()> {
        let directory = tempfile::tempdir()?;
        for relative in ["../outside", "workspace/../../outside", "/absolute", ""] {
            assert!(
                owned_path(directory.path(), Path::new(relative)).is_err(),
                "{relative}"
            );
        }
        assert_eq!(
            owned_path(directory.path(), Path::new("workspace/prompts/main.md"))?,
            directory.path().join("workspace/prompts/main.md")
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn seed_paths_reject_existing_symlink_ancestors() -> Result<()> {
        let owned = tempfile::tempdir()?;
        let unrelated = tempfile::tempdir()?;
        std::os::unix::fs::symlink(unrelated.path(), owned.path().join("link"))?;
        assert!(owned_path(owned.path(), Path::new("link/file")).is_err());
        assert!(!unrelated.path().join("file").exists());
        Ok(())
    }
}
