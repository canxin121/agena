//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the v2 endpoints.

use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, DismissActivityParams, ExportSessionParams, ForkSessionParams,
        ImportSessionParams, ListSessionTreeParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, StopActivityParams, SubmitRunParams,
        UpdateSessionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    notifications::Notification,
    queries::{
        ActivityLogsParams, GetActivityParams, GetOperationDetailParams, GetPermissionRuleParams,
        GetSessionParams, GetWorkspaceParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
        QueryResult,
    },
    resource::{
        BackgroundActivityResource, HealthResponse, PermissionRuleResource,
        ProviderAdapterModelsRequest, ProviderAdapterModelsResponse, RunOptions,
        SavedProviderAdapterModelsRequest, SessionExecutionResource, SessionResource,
        WorkspaceResource,
    },
};
use futures_util::{StreamExt, TryStreamExt as _};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_sse_codec::{Frame as SseFrame, SseDecoder};
use tokio_util::{codec::FramedRead, io::StreamReader};

use crate::error::ClientError;
use crate::ws::SubscriptionEvent;

const MAX_JSON_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEXT_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;

async fn read_response_text_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
    context: &str,
) -> Result<String, ClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ClientError::Protocol(format!(
            "{context} exceeds the {max_bytes}-byte limit"
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ClientError::Protocol(format!(
                "{context} exceeds the {max_bytes}-byte limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ClientError::Protocol(format!("{context} is not UTF-8 text")))
}

/// Handle to an active HTTP notification subscription.
pub struct NotificationSubscription {
    rx: mpsc::Receiver<Result<SubscriptionEvent, ClientError>>,
    task: Option<JoinHandle<()>>,
}

impl NotificationSubscription {
    pub async fn recv(&mut self) -> Option<Result<SubscriptionEvent, ClientError>> {
        self.rx.recv().await
    }

    pub fn close(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for NotificationSubscription {
    fn drop(&mut self) {
        self.close();
    }
}

/// Stateless REST client. Holds a `reqwest::Client` and the base URL like
/// `http://localhost:7878`.
#[derive(Debug, Clone)]
/// HTTP client for the Agena runtime API.
pub struct AgenaClient {
    base_url: url::Url,
    http: reqwest::Client,
}

impl AgenaClient {
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, ClientError> {
        let base_url = url::Url::parse(base_url.as_ref())
            .map_err(|e| ClientError::Transport(format!("invalid base url: {e}")))?;
        Ok(Self {
            base_url,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()?,
        })
    }

    pub fn base_url(&self) -> &url::Url {
        &self.base_url
    }

    fn endpoint(&self, path: &str) -> url::Url {
        self.base_url
            .join(path.trim_start_matches('/'))
            .expect("valid endpoint")
    }

    fn append_live_scope(url: &mut url::Url, scope: &agena_api::Scope) {
        let mut q = url.query_pairs_mut();
        match scope {
            agena_api::Scope::Global => {}
            agena_api::Scope::Workspace { workspace_id } => {
                q.append_pair("scope_kind", "workspace");
                q.append_pair("workspace_id", &workspace_id.to_string());
            }
            agena_api::Scope::Session { session_id } => {
                q.append_pair("scope_kind", "session");
                q.append_pair("session_id", &session_id.to_string());
            }
        }
    }

    fn changes_stream_url(&self, scope: &agena_api::Scope) -> url::Url {
        let mut url = self.endpoint("/api/v1/changes/stream");
        Self::append_live_scope(&mut url, scope);
        url
    }

    async fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::Protocol(format!(
                "JSON response exceeds the {} MiB limit",
                MAX_JSON_RESPONSE_BYTES / 1024 / 1024
            )));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_JSON_RESPONSE_BYTES {
                return Err(ClientError::Protocol(format!(
                    "JSON response exceeds the {} MiB limit",
                    MAX_JSON_RESPONSE_BYTES / 1024 / 1024
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_value(value)?;
            return Err(ClientError::Api(api));
        }
        Ok(serde_json::from_value(value)?)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let response = self.http.get(self.endpoint(path)).send().await?;
        self.parse_json(response).await
    }

    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .post(self.endpoint(path))
            .json(&body)
            .send()
            .await?;
        self.parse_json(response).await
    }

    async fn put_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .put(self.endpoint(path))
            .json(&body)
            .send()
            .await?;
        self.parse_json(response).await
    }

    async fn delete_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ClientError> {
        let response = self.http.delete(self.endpoint(path)).send().await?;
        self.parse_json(response).await
    }

    async fn get_text(&self, path: &str) -> Result<String, ClientError> {
        let response = self.http.get(self.endpoint(path)).send().await?;
        let status = response.status();
        let text =
            read_response_text_bounded(response, MAX_TEXT_RESPONSE_BYTES, "text response").await?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_str(&text)?;
            return Err(ClientError::Api(api));
        }
        Ok(text)
    }

    async fn send_notification_frame(
        tx: &mpsc::Sender<Result<SubscriptionEvent, ClientError>>,
        notification: Notification,
    ) -> bool {
        let item = match notification {
            Notification::SessionChanged { change, .. } => {
                Ok(SubscriptionEvent::SessionChanged(*change))
            }
            Notification::RuntimeSignal { signal, .. } => {
                Ok(SubscriptionEvent::RuntimeSignal(*signal))
            }
            Notification::Lagged { skipped, .. } => Ok(SubscriptionEvent::Lagged(skipped)),
            Notification::SubscriptionClosed { reason, .. } => Err(ClientError::Protocol(format!(
                "sse subscription closed: {reason}"
            ))),
        };
        tx.send(item).await.is_ok()
    }

    // ─── high-level conveniences ───

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get_json("/api/v1/health").await
    }

    pub async fn list_provider_adapter_models(
        &self,
        request: ProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsResponse, ClientError> {
        self.post_json("/api/v1/providers/models", serde_json::to_value(request)?)
            .await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        request: SavedProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsResponse, ClientError> {
        self.post_json(
            &format!("/api/v1/providers/{provider_id}/models"),
            serde_json::to_value(request)?,
        )
        .await
    }

    pub async fn create_session(
        &self,
        workspace_id: i64,
        title: impl Into<String>,
        parent_id: Option<i64>,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "workspace_id": workspace_id,
            "title": title.into(),
            "parent_id": parent_id,
        });
        self.post_json("/api/v1/sessions", body).await
    }

    pub async fn submit_message(
        &self,
        params: SubmitRunParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert(
                "document".to_string(),
                serde_json::to_value(params.document)?,
            );
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/messages", params.session_id),
            body,
        )
        .await
    }

    pub async fn continue_run(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ClientError> {
        let body = serde_json::to_value(options.clone())?;
        let _ = ContinueRunParams {
            session_id,
            options,
        };
        self.post_json(&format!("/api/v1/sessions/{session_id}/continue"), body)
            .await
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/cancel"),
            serde_json::json!({ "execution_id": execution_id }),
        )
        .await
    }

    pub async fn reply_permission(
        &self,
        params: ReplyPermissionParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert("reply".to_string(), serde_json::to_value(params.reply)?);
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/permission-replies", params.session_id),
            body,
        )
        .await
    }

    pub async fn reply_user_input(
        &self,
        params: ReplyUserInputParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert("reply".to_string(), serde_json::to_value(params.reply)?);
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/user-input-replies", params.session_id),
            body,
        )
        .await
    }

    pub async fn stream_changes(
        &self,
        scope: agena_api::Scope,
    ) -> Result<NotificationSubscription, ClientError> {
        let url = self.changes_stream_url(&scope);
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_response_text_bounded(
                response,
                MAX_ERROR_RESPONSE_BYTES,
                "notification stream error response",
            )
            .await?;
            if let Ok(api) = serde_json::from_str::<agena_api::error::ApiError>(&body) {
                return Err(ClientError::Api(api));
            }
            return Err(ClientError::Transport(format!(
                "notification stream request failed ({status}): {}",
                body.trim()
            )));
        }

        let (tx, rx) = mpsc::channel(256);
        let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
        let mut frames = FramedRead::new(
            reader,
            SseDecoder::<String>::with_max_size(MAX_SSE_EVENT_BYTES),
        );
        let task = tokio::spawn(async move {
            while let Some(frame) = frames.next().await {
                let event = match frame {
                    Ok(SseFrame::Event(event)) => event,
                    Ok(SseFrame::Comment(_) | SseFrame::Retry(_)) => continue,
                    Err(error) => {
                        let _ = tx
                            .send(Err(ClientError::Protocol(format!(
                                "invalid or oversized notification event: {error}"
                            ))))
                            .await;
                        return;
                    }
                };
                if event.name != "notification" || event.data.trim().is_empty() {
                    continue;
                }
                let notification = match serde_json::from_str::<Notification>(&event.data) {
                    Ok(notification) => notification,
                    Err(error) => {
                        let _ = tx.send(Err(ClientError::Decode(error))).await;
                        return;
                    }
                };
                if !Self::send_notification_frame(&tx, notification).await {
                    return;
                }
            }
        });

        Ok(NotificationSubscription {
            rx,
            task: Some(task),
        })
    }

    /// Escape hatch: run any [`Command`] over REST where a dedicated route
    /// already exists.
    pub async fn command(&self, cmd: Command) -> Result<CommandResult, ClientError> {
        match cmd {
            Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
                Ok(CommandResult::Workspace(
                    self.post_json("/api/v1/workspaces", serde_json::json!({ "path": path }))
                        .await?,
                ))
            }
            Command::UpdateWorkspace(UpdateWorkspaceParams {
                workspace_id, path, ..
            }) => Ok(CommandResult::Workspace(
                self.put_json(
                    &format!("/api/v1/workspaces/{workspace_id}"),
                    serde_json::json!({ "path": path }),
                )
                .await?,
            )),
            Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
                let _: WorkspaceResource = self
                    .delete_json(&format!("/api/v1/workspaces/{workspace_id}"))
                    .await?;
                Ok(CommandResult::WorkspaceDeleted { id: workspace_id })
            }
            Command::ResolveWorkspace(ResolveWorkspaceParams {
                path,
                create_if_missing,
            }) => Ok(CommandResult::Workspace(
                self.post_json(
                    "/api/v1/workspaces/resolve",
                    serde_json::json!({
                        "path": path,
                        "create_if_missing": create_if_missing,
                    }),
                )
                .await?,
            )),
            Command::CreateSession(CreateSessionParams {
                workspace_id,
                title,
                parent_id,
            }) => Ok(CommandResult::Session(
                self.create_session(workspace_id, title, parent_id).await?,
            )),
            Command::UpdateSession(UpdateSessionParams {
                session_id, title, ..
            }) => Ok(CommandResult::Session(
                self.put_json(
                    &format!("/api/v1/sessions/{session_id}"),
                    serde_json::json!({
                        "title": title,
                    }),
                )
                .await?,
            )),
            Command::DeleteSession(DeleteSessionParams { session_id, .. }) => {
                let _: SessionResource = self
                    .delete_json(&format!("/api/v1/sessions/{session_id}"))
                    .await?;
                Ok(CommandResult::SessionDeleted { id: session_id })
            }
            Command::SubmitMessage(params) => {
                Ok(CommandResult::Execution(self.submit_message(params).await?))
            }
            Command::ContinueRun(ContinueRunParams {
                session_id,
                options,
            }) => Ok(CommandResult::Execution(
                self.continue_run(session_id, options).await?,
            )),
            Command::CancelRun(CancelRunParams {
                session_id,
                execution_id,
            }) => {
                let result = self.cancel_run(session_id, execution_id).await?;
                Ok(CommandResult::Cancellation(result))
            }
            Command::RewindSession(RewindSessionParams {
                session_id,
                turn_id,
                ..
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/rewind"),
                    serde_json::json!({ "turn_id": turn_id }),
                )
                .await?,
            )),
            Command::ForkSession(ForkSessionParams {
                session_id,
                at_message_id,
                title,
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/fork"),
                    serde_json::json!({
                        "at_message_id": at_message_id,
                        "title": title,
                    }),
                )
                .await?,
            )),
            Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
                Ok(CommandResult::SessionTree(
                    self.get_json(&format!("/api/v1/sessions/tree/{root_id}"))
                        .await?,
                ))
            }
            Command::ExportSession(ExportSessionParams { session_id }) => {
                Ok(CommandResult::SessionExport {
                    jsonl: self
                        .get_text(&format!("/api/v1/sessions/{session_id}/export"))
                        .await?,
                })
            }
            Command::ImportSession(ImportSessionParams { jsonl }) => Ok(CommandResult::Execution(
                self.post_json(
                    "/api/v1/sessions/import",
                    serde_json::json!({ "jsonl": jsonl }),
                )
                .await?,
            )),
            Command::ReplyPermission(params) => Ok(CommandResult::Execution(
                self.reply_permission(params).await?,
            )),
            Command::ReplyUserInput(params) => Ok(CommandResult::Execution(
                self.reply_user_input(params).await?,
            )),
            Command::UpsertPermissionRule(UpsertPermissionRuleParams {
                action_key,
                subject_kind,
                tool_name,
                qualifier,
                path_access_kind,
                workspace_root,
                target_path,
                network_target,
                network_host,
                network_port,
                scope,
                session_id,
                mode,
            }) => {
                let mut body = serde_json::Map::new();
                if let Some(action_key) = action_key {
                    body.insert("action_key".to_string(), serde_json::json!(action_key));
                }
                if let Some(subject_kind) = subject_kind {
                    body.insert("subject_kind".to_string(), serde_json::json!(subject_kind));
                }
                if let Some(tool_name) = tool_name {
                    body.insert("tool_name".to_string(), serde_json::json!(tool_name));
                }
                if let Some(qualifier) = qualifier {
                    body.insert("qualifier".to_string(), serde_json::json!(qualifier));
                }
                if let Some(path_access_kind) = path_access_kind {
                    body.insert(
                        "path_access_kind".to_string(),
                        serde_json::json!(path_access_kind),
                    );
                }
                if let Some(workspace_root) = workspace_root {
                    body.insert(
                        "workspace_root".to_string(),
                        serde_json::json!(workspace_root),
                    );
                }
                if let Some(target_path) = target_path {
                    body.insert("target_path".to_string(), serde_json::json!(target_path));
                }
                if let Some(network_target) = network_target {
                    body.insert(
                        "network_target".to_string(),
                        serde_json::json!(network_target),
                    );
                }
                if let Some(network_host) = network_host {
                    body.insert("network_host".to_string(), serde_json::json!(network_host));
                }
                if let Some(network_port) = network_port {
                    body.insert("network_port".to_string(), serde_json::json!(network_port));
                }
                if let Some(scope) = scope {
                    body.insert("scope".to_string(), serde_json::json!(scope));
                }
                if let Some(session_id) = session_id {
                    body.insert("session_id".to_string(), serde_json::json!(session_id));
                }
                body.insert("mode".to_string(), serde_json::json!(mode));
                Ok(CommandResult::PermissionRule(
                    self.post_json("/api/v1/permission-rules", serde_json::Value::Object(body))
                        .await?,
                ))
            }
            Command::ReplacePermissionRule(ReplacePermissionRuleParams { rule_id, rule }) => {
                let UpsertPermissionRuleParams {
                    action_key,
                    subject_kind,
                    tool_name,
                    qualifier,
                    path_access_kind,
                    workspace_root,
                    target_path,
                    network_target,
                    network_host,
                    network_port,
                    scope,
                    session_id,
                    mode,
                } = rule;
                let mut body = serde_json::Map::new();
                if let Some(action_key) = action_key {
                    body.insert("action_key".to_string(), serde_json::json!(action_key));
                }
                if let Some(subject_kind) = subject_kind {
                    body.insert("subject_kind".to_string(), serde_json::json!(subject_kind));
                }
                if let Some(tool_name) = tool_name {
                    body.insert("tool_name".to_string(), serde_json::json!(tool_name));
                }
                if let Some(qualifier) = qualifier {
                    body.insert("qualifier".to_string(), serde_json::json!(qualifier));
                }
                if let Some(path_access_kind) = path_access_kind {
                    body.insert(
                        "path_access_kind".to_string(),
                        serde_json::json!(path_access_kind),
                    );
                }
                if let Some(workspace_root) = workspace_root {
                    body.insert(
                        "workspace_root".to_string(),
                        serde_json::json!(workspace_root),
                    );
                }
                if let Some(target_path) = target_path {
                    body.insert("target_path".to_string(), serde_json::json!(target_path));
                }
                if let Some(network_target) = network_target {
                    body.insert(
                        "network_target".to_string(),
                        serde_json::json!(network_target),
                    );
                }
                if let Some(network_host) = network_host {
                    body.insert("network_host".to_string(), serde_json::json!(network_host));
                }
                if let Some(network_port) = network_port {
                    body.insert("network_port".to_string(), serde_json::json!(network_port));
                }
                if let Some(scope) = scope {
                    body.insert("scope".to_string(), serde_json::json!(scope));
                }
                if let Some(session_id) = session_id {
                    body.insert("session_id".to_string(), serde_json::json!(session_id));
                }
                body.insert("mode".to_string(), serde_json::json!(mode));
                Ok(CommandResult::PermissionRule(
                    self.put_json(
                        &format!("/api/v1/permission-rules/{rule_id}"),
                        serde_json::Value::Object(body),
                    )
                    .await?,
                ))
            }
            Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
                Ok(CommandResult::PermissionRule(
                    self.post_json(
                        &format!("/api/v1/permission-rules/{rule_id}/revoke"),
                        serde_json::json!({ "reason": reason }),
                    )
                    .await?,
                ))
            }
            Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
                let _: PermissionRuleResource = self
                    .delete_json(&format!("/api/v1/permission-rules/{rule_id}"))
                    .await?;
                Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
            }
            Command::StopActivity(StopActivityParams { activity_id }) => {
                Ok(CommandResult::Activity(
                    self.post_json(
                        &format!("/api/v1/activities/{activity_id}/stop"),
                        serde_json::json!({}),
                    )
                    .await?,
                ))
            }
            Command::DismissActivity(DismissActivityParams { activity_id }) => {
                let _: BackgroundActivityResource = self
                    .post_json(
                        &format!("/api/v1/activities/{activity_id}/dismiss"),
                        serde_json::json!({}),
                    )
                    .await?;
                Ok(CommandResult::ActivityDeleted { id: activity_id })
            }
            Command::ClearFinishedActivities => {
                let count: usize = self
                    .post_json("/api/v1/activities/clear-finished", serde_json::json!({}))
                    .await?;
                Ok(CommandResult::ActivitiesCleared { count })
            }
            _ => Err(ClientError::Protocol(
                "unsupported command in generic HTTP client".to_string(),
            )),
        }
    }

    /// Generic query escape hatch over the existing REST surface.
    pub async fn query(&self, q: Query) -> Result<QueryResult, ClientError> {
        match q {
            Query::Health => Ok(QueryResult::Health(self.health().await?)),
            Query::Runtime => Ok(QueryResult::Runtime(
                self.get_json("/api/v1/runtime").await?,
            )),
            Query::ListProviders => Ok(QueryResult::Providers(
                self.get_json("/api/v1/providers").await?,
            )),
            Query::ListProviderModels(ListProviderModelsParams { provider_id }) => {
                Ok(QueryResult::ProviderModels(
                    self.get_json(&format!("/api/v1/providers/{provider_id}/models"))
                        .await?,
                ))
            }
            Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
                provider_id,
                base_url,
                protocol_paths,
                api_key,
                adapter_ids,
            }) => Ok(QueryResult::ProviderAdapterModels(
                self.list_provider_adapter_models(ProviderAdapterModelsRequest {
                    provider_id,
                    base_url,
                    protocol_paths,
                    api_key,
                    adapter_ids,
                })
                .await?,
            )),
            Query::ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams {
                provider_id,
                adapter_ids,
            }) => Ok(QueryResult::ProviderAdapterModels(
                self.list_saved_provider_adapter_models(
                    provider_id.as_str(),
                    SavedProviderAdapterModelsRequest { adapter_ids },
                )
                .await?,
            )),
            Query::ListWorkspaces(ListWorkspacesParams {
                cursor,
                limit,
                search,
                include_session_count,
            }) => {
                let mut url = self.endpoint("/api/v1/workspaces");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                    if include_session_count {
                        q.append_pair("include_session_count", "true");
                    }
                }
                Ok(QueryResult::Workspaces(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
            Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => Ok(QueryResult::Workspace(
                self.get_json(&format!("/api/v1/workspaces/{workspace_id}"))
                    .await?,
            )),
            Query::ListSessions(ListSessionsParams {
                cursor,
                limit,
                workspace_id,
                parent_id,
                roots,
                search,
            }) => {
                let mut url = self.endpoint("/api/v1/sessions");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(workspace_id) = workspace_id {
                        q.append_pair("workspace_id", &workspace_id.to_string());
                    }
                    if let Some(parent_id) = parent_id {
                        q.append_pair("parent_id", &parent_id.to_string());
                    }
                    if roots {
                        q.append_pair("roots", "true");
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                }
                Ok(QueryResult::Sessions(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
            Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
                self.get_json(&format!("/api/v1/sessions/{session_id}"))
                    .await?,
            )),
            Query::GetSessionState(GetSessionParams { session_id }) => {
                Ok(QueryResult::SessionState(
                    self.get_json(&format!("/api/v1/sessions/{session_id}/state"))
                        .await?,
                ))
            }
            Query::GetOperationDetail(GetOperationDetailParams {
                session_id,
                activity_id,
            }) => Ok(QueryResult::OperationDetail(
                self.get_json(&format!(
                    "/api/v1/sessions/{session_id}/operations/{activity_id}/detail"
                ))
                .await?,
            )),
            Query::ListPermissionRules(ListPermissionRulesParams {
                cursor,
                limit,
                search,
            }) => {
                let mut url = self.endpoint("/api/v1/permission-rules");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                }
                Ok(QueryResult::PermissionRules(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
            Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
                Ok(QueryResult::PermissionRule(
                    self.get_json(&format!("/api/v1/permission-rules/{rule_id}"))
                        .await?,
                ))
            }
            Query::ListActivities(params) => {
                let mut url = self.endpoint("/api/v1/activities");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(kinds) = params.kinds.filter(|kinds| !kinds.is_empty()) {
                        q.append_pair("kinds", &kinds);
                    }
                    if let Some(statuses) = params.statuses.filter(|statuses| !statuses.is_empty())
                    {
                        q.append_pair("statuses", &statuses);
                    }
                    if let Some(session_id) = params.session_id {
                        q.append_pair("session_id", &session_id.to_string());
                    }
                    if params.active_only {
                        q.append_pair("active_only", "true");
                    }
                }
                Ok(QueryResult::Activities(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
            Query::GetActivity(GetActivityParams { activity_id }) => Ok(QueryResult::Activity(
                self.get_json(&format!("/api/v1/activities/{activity_id}"))
                    .await?,
            )),
            Query::ActivityLogs(ActivityLogsParams {
                activity_id,
                since_seq,
                limit,
                wait_ms,
            }) => {
                let mut url = self.endpoint(&format!("/api/v1/activities/{activity_id}/logs"));
                {
                    let mut q = url.query_pairs_mut();
                    if since_seq > 0 {
                        q.append_pair("since_seq", &since_seq.to_string());
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if wait_ms > 0 {
                        q.append_pair("wait_ms", &wait_ms.to_string());
                    }
                }
                Ok(QueryResult::ActivityLogs(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod sse_contract_tests {
    use super::AgenaClient;
    use futures_util::StreamExt as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_sse_codec::{Frame as SseFrame, SseDecoder};
    use tokio_util::codec::FramedRead;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("checked-in client fixture must be readable")
    }

    #[tokio::test]
    async fn bounded_sse_codec_normalizes_crlf_and_multiline_data() {
        let source =
            &b"event: notification\r\ndata: {\"seq\":1}\r\ndata: {\"kind\":\"x\"}\r\n\r\n"[..];
        let mut frames = FramedRead::new(source, SseDecoder::<String>::with_max_size(1024));
        let frame = frames
            .next()
            .await
            .expect("one SSE frame")
            .expect("valid SSE frame");
        let SseFrame::Event(event) = frame else {
            panic!("expected event frame");
        };
        assert_eq!(event.name, "notification");
        assert_eq!(event.data, "{\"seq\":1}\n{\"kind\":\"x\"}");
    }

    #[tokio::test]
    async fn bounded_sse_codec_rejects_an_oversized_event() {
        let source = format!("event: notification\ndata: {}\n\n", "x".repeat(2_048));
        let mut frames = FramedRead::new(
            source.as_bytes(),
            SseDecoder::<String>::with_max_size(1_024),
        );

        assert!(
            frames
                .next()
                .await
                .expect("oversized event must produce a decoder result")
                .is_err()
        );
    }

    #[tokio::test]
    async fn health_round_trips_through_a_real_http_transport() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = fixture("health-response.json");
        let fixture: agena_api::resource::HealthResponse =
            serde_json::from_str(&response_body).expect("health fixture matches API resource");
        assert_eq!(fixture.status, "ok");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /api/v1/health HTTP/1.1"));
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let health = client.health().await.unwrap();
        assert_eq!(health.status, "ok");
        assert_eq!(health.generation, 7);
        assert_eq!(health.loaded_at.to_rfc3339(), "2026-01-02T03:04:05+00:00");
        assert!(health.database_connected);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn non_success_json_maps_to_the_shared_api_error() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = fixture("api-error-not-found.json");
        let fixture: agena_api::ApiError =
            serde_json::from_str(&response_body).expect("error fixture matches API error");
        assert_eq!(
            fixture.problem.category,
            agena_failure::FailureCategory::NotFound
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let error = client.health().await.expect_err("404 must be an API error");
        match error {
            crate::ClientError::Api(api) => {
                assert_eq!(
                    api.problem.category,
                    agena_failure::FailureCategory::NotFound
                );
                assert_eq!(api.problem.user.fallback, "workspace missing");
            }
            other => panic!("expected shared API error, got {other:?}"),
        }
        server.await.unwrap();
    }
}
