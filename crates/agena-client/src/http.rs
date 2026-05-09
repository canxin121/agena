//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the v2 endpoints.

use agena_api::{
    commands::{
        CancelTurnParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams, ImportSessionParams,
        ListRewindCheckpointsParams, ListSessionTreeParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, SubmitTurnParams, UnrewindSessionParams,
        UpdateSessionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    queries::{
        GetMessageParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
        ListEventsParams, ListMessagesParams, ListPermissionRulesParams, ListProviderModelsParams,
        ListSessionsParams, ListWorkspacesParams, PaginatedEvents, Query, QueryResult,
    },
    resource::{
        HealthResponse, PartLoadMode, PermissionRuleResource, RunOptions, SessionExecutionResource,
        SessionResource, WorkspaceResource,
    },
};

use crate::error::ClientError;

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

    async fn post_no_body_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ClientError> {
        let response = self.http.post(self.endpoint(path)).send().await?;
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

    // ─── high-level conveniences ───

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get_json("/api/v1/health").await
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

    pub async fn submit_turn(
        &self,
        params: SubmitTurnParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert("parts".to_string(), serde_json::to_value(params.parts)?);
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/turns", params.session_id),
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

    pub async fn cancel_turn(&self, session_id: i64) -> Result<(), ClientError> {
        let _: serde_json::Value = self
            .post_no_body_json(&format!("/api/v1/sessions/{session_id}/cancel"))
            .await?;
        Ok(())
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
        let mut url = self.endpoint("/api/v1/events");
        {
            let mut q = url.query_pairs_mut();
            if let Some(seq) = params.since_seq_global {
                q.append_pair("since_seq_global", &seq.to_string());
            }
            if let Some(limit) = params.limit {
                q.append_pair("limit", &limit.to_string());
            }
            match &params.scope {
                agena::event::Scope::Global => {}
                agena::event::Scope::Workspace { workspace_id } => {
                    q.append_pair("scope_kind", "workspace");
                    q.append_pair("workspace_id", &workspace_id.to_string());
                }
                agena::event::Scope::Session { session_id } => {
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
        let response = self.http.get(url).send().await?;
        self.parse_json(response).await
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
                session_id,
                title,
                parent_id,
                ..
            }) => Ok(CommandResult::Session(
                self.put_json(
                    &format!("/api/v1/sessions/{session_id}"),
                    serde_json::json!({
                        "title": title,
                        "parent_id": parent_id,
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
            Command::SubmitTurn(params) => {
                Ok(CommandResult::Execution(self.submit_turn(params).await?))
            }
            Command::ContinueRun(ContinueRunParams {
                session_id,
                options,
            }) => Ok(CommandResult::Execution(
                self.continue_run(session_id, options).await?,
            )),
            Command::CancelTurn(CancelTurnParams { session_id }) => {
                self.cancel_turn(session_id).await?;
                Ok(CommandResult::Ack)
            }
            Command::RewindSession(RewindSessionParams {
                session_id,
                message_id,
                ..
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/rewind"),
                    serde_json::json!({ "message_id": message_id }),
                )
                .await?,
            )),
            Command::UnrewindSession(UnrewindSessionParams {
                session_id,
                message_id,
                ..
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/unrewind"),
                    serde_json::json!({ "message_id": message_id }),
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
            Command::ListRewindCheckpoints(ListRewindCheckpointsParams { session_id }) => {
                Ok(CommandResult::RewindCheckpoints(
                    self.get_json(&format!("/api/v1/sessions/{session_id}/rewind-checkpoints"))
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
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor,
                limit,
                parts,
            }) => {
                let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/messages"));
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    q.append_pair(
                        "parts",
                        match parts {
                            PartLoadMode::None => "none",
                            PartLoadMode::Summary => "summary",
                            PartLoadMode::Full => "full",
                        },
                    );
                }
                Ok(QueryResult::Messages(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
            Query::GetMessage(GetMessageParams { message_id, parts }) => {
                let mut url = self.endpoint(&format!("/api/v1/messages/{message_id}"));
                url.query_pairs_mut().append_pair(
                    "parts",
                    match parts {
                        PartLoadMode::None => "none",
                        PartLoadMode::Summary => "summary",
                        PartLoadMode::Full => "full",
                    },
                );
                Ok(QueryResult::Message(
                    self.parse_json(self.http.get(url).send().await?).await?,
                ))
            }
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
