//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the v2 endpoints.

use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams, ImportSessionParams,
        ListSessionTreeParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, ResolveWorkspaceParams, RevokePermissionRuleParams,
        RewindSessionParams, SubmitMessageParams, UpdateSessionParams, UpdateWorkspaceParams,
        UpsertPermissionRuleParams,
    },
    notifications::Notification,
    queries::{
        GetOperationDetailParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
        ListEventsParams, ListPermissionRulesParams, ListProviderAdapterModelsParams,
        ListProviderModelsParams, ListSavedProviderAdapterModelsParams, ListSessionsParams,
        ListWorkspacesParams,
        PaginatedEvents, Query, QueryResult,
    },
    resource::{
        HealthResponse, PermissionRuleResource, ProviderAdapterModelsRequest,
        ProviderAdapterModelsResponse, RunOptions, SavedProviderAdapterModelsRequest,
        SessionExecutionResource, SessionResource, WorkspaceResource,
    },
};
use futures_util::StreamExt;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::error::ClientError;
use crate::ws::SubscriptionEvent;

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

#[derive(Debug, Default)]
struct ParsedSseEvent {
    event: String,
    data: String,
}

/// Stateless REST client. Holds a `reqwest::Client` and the base URL like
/// `http://localhost:7878`.
#[derive(Debug, Clone)]
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
            http: reqwest::Client::new(),
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

    fn append_event_query(url: &mut url::Url, params: &ListEventsParams) {
        let mut q = url.query_pairs_mut();
        if let Some(seq) = params.since_seq_global {
            q.append_pair("since_seq_global", &seq.to_string());
        }
        if let Some(limit) = params.limit {
            q.append_pair("limit", &limit.to_string());
        }
        match &params.scope {
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
        if let Some(kinds) = &params.kinds {
            let csv = kinds
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(",");
            q.append_pair("kinds", &csv);
        }
    }

    fn events_url(&self, params: &ListEventsParams) -> url::Url {
        let mut url = self.endpoint("/api/v1/events");
        Self::append_event_query(&mut url, params);
        url
    }

    fn events_stream_url(&self, params: &ListEventsParams) -> url::Url {
        let mut url = self.endpoint("/api/v1/events/stream");
        Self::append_event_query(&mut url, params);
        url
    }

    async fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
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
        let text = response.text().await?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_str(&text)?;
            return Err(ClientError::Api(api));
        }
        Ok(text)
    }

    fn normalize_sse_buffer(mut buffer: String) -> String {
        if buffer.contains('\r') {
            buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
        }
        buffer
    }

    fn parse_sse_event_block(block: &str) -> ParsedSseEvent {
        let mut event = String::from("message");
        let mut data = Vec::new();
        for raw_line in block.lines() {
            if raw_line.is_empty() || raw_line.starts_with(':') {
                continue;
            }
            let (field, value) = match raw_line.split_once(':') {
                Some((field, value)) => (field, value.trim_start()),
                None => (raw_line, ""),
            };
            match field {
                "event" => {
                    if !value.is_empty() {
                        event.clear();
                        event.push_str(value);
                    }
                }
                "data" => data.push(value.to_string()),
                _ => {}
            }
        }
        ParsedSseEvent {
            event,
            data: data.join("\n"),
        }
    }

    async fn send_notification_frame(
        tx: &mpsc::Sender<Result<SubscriptionEvent, ClientError>>,
        notification: Notification,
    ) -> bool {
        let item = match notification {
            Notification::Event { event, .. } => Ok(SubscriptionEvent::Event(*event)),
            Notification::Lagged { skipped, .. } => Ok(SubscriptionEvent::Lagged(skipped)),
            Notification::Resumed { .. } => return true,
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
        params: SubmitMessageParams,
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

    pub async fn list_events(
        &self,
        params: ListEventsParams,
    ) -> Result<PaginatedEvents, ClientError> {
        let url = self.events_url(&params);
        let response = self.http.get(url).send().await?;
        self.parse_json(response).await
    }

    pub async fn stream_notifications(
        &self,
        params: ListEventsParams,
    ) -> Result<NotificationSubscription, ClientError> {
        let url = self.events_stream_url(&params);
        let response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            if let Ok(api) = serde_json::from_str::<agena_api::error::ApiError>(&body) {
                return Err(ClientError::Api(api));
            }
            return Err(ClientError::Transport(format!(
                "notification stream request failed ({status}): {}",
                body.trim()
            )));
        }

        let (tx, rx) = mpsc::channel(256);
        let mut stream = response.bytes_stream();
        let task = tokio::spawn(async move {
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let _ = tx.send(Err(ClientError::Transport(err.to_string()))).await;
                        return;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                buffer = Self::normalize_sse_buffer(buffer);

                while let Some(boundary) = buffer.find("\n\n") {
                    let block = buffer[..boundary].trim().to_string();
                    buffer = buffer[boundary + 2..].to_string();
                    if block.is_empty() {
                        continue;
                    }
                    let parsed = Self::parse_sse_event_block(&block);
                    if parsed.event != "notification" || parsed.data.trim().is_empty() {
                        continue;
                    }
                    let notification: Notification = match serde_json::from_str(&parsed.data) {
                        Ok(notification) => notification,
                        Err(err) => {
                            let _ = tx.send(Err(ClientError::Decode(err))).await;
                            return;
                        }
                    };
                    if !Self::send_notification_frame(&tx, notification).await {
                        return;
                    }
                }
            }

            let trailing = buffer.trim();
            if trailing.is_empty() {
                return;
            }
            let parsed = Self::parse_sse_event_block(trailing);
            if parsed.event != "notification" || parsed.data.trim().is_empty() {
                return;
            }
            match serde_json::from_str::<Notification>(&parsed.data) {
                Ok(notification) => {
                    let _ = Self::send_notification_frame(&tx, notification).await;
                }
                Err(err) => {
                    let _ = tx.send(Err(ClientError::Decode(err))).await;
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
            Query::ListEvents(p) => Ok(QueryResult::Events(self.list_events(p).await?)),
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
        }
    }
}

#[cfg(test)]
mod sse_contract_tests {
    use super::AgenaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("checked-in client fixture must be readable")
    }

    #[test]
    fn sse_parser_normalizes_crlf_and_multiline_data() {
        let normalized = AgenaClient::normalize_sse_buffer(
            "event: notification\r\ndata: {\"seq\":1}\r\ndata: {\"kind\":\"x\"}\r\n\r\n".to_owned(),
        );
        let parsed = AgenaClient::parse_sse_event_block(normalized.trim());
        assert_eq!(parsed.event, "notification");
        assert_eq!(parsed.data, "{\"seq\":1}\n{\"kind\":\"x\"}");
    }

    #[test]
    fn sse_parser_ignores_comments_and_defaults_event_name() {
        let parsed = AgenaClient::parse_sse_event_block(": keep-alive\ndata: payload");
        assert_eq!(parsed.event, "message");
        assert_eq!(parsed.data, "payload");
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
