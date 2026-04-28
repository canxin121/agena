//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the v2 endpoints.

use agena_api::{
    commands::{Command, CommandResult, ContinueRunParams, ReplyPermissionParams,
        ReplyUserInputParams, SubmitTurnParams},
    queries::{ListEventsParams, PaginatedEvents, Query, QueryResult},
    resource::{HealthResponse, RunOptions, SessionResource},
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

    async fn parse_query(&self, response: reqwest::Response) -> Result<QueryResult, ClientError> {
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_value(value)?;
            return Err(ClientError::Api(api));
        }
        Ok(serde_json::from_value(value)?)
    }

    async fn parse_command(
        &self,
        response: reqwest::Response,
    ) -> Result<CommandResult, ClientError> {
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_value(value)?;
            return Err(ClientError::Api(api));
        }
        Ok(serde_json::from_value(value)?)
    }

    // ─── high-level conveniences ───

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let response = self.http.get(self.endpoint("/api/v1/health")).send().await?;
        match self.parse_query(response).await? {
            QueryResult::Health(h) => Ok(h),
            other => Err(ClientError::Protocol(format!(
                "expected Health, got {other:?}"
            ))),
        }
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
        let response = self
            .http
            .post(self.endpoint("/api/v1/sessions"))
            .json(&body)
            .send()
            .await?;
        match self.parse_command(response).await? {
            CommandResult::Session(s) => Ok(s),
            other => Err(ClientError::Protocol(format!(
                "expected Session result, got {other:?}"
            ))),
        }
    }

    pub async fn submit_turn(
        &self,
        params: SubmitTurnParams,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "options": params.options,
            "parts": params.parts,
        });
        let response = self
            .http
            .post(self.endpoint(&format!("/api/v1/sessions/{}/turns", params.session_id)))
            .json(&body)
            .send()
            .await?;
        match self.parse_command(response).await? {
            CommandResult::Session(s) => Ok(s),
            other => Err(ClientError::Protocol(format!("expected Session, got {other:?}"))),
        }
    }

    pub async fn continue_run(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({ "options": options });
        let response = self
            .http
            .post(self.endpoint(&format!("/api/v1/sessions/{session_id}/continue")))
            .json(&body)
            .send()
            .await?;
        let _ = ContinueRunParams { session_id, options };
        match self.parse_command(response).await? {
            CommandResult::Session(s) => Ok(s),
            other => Err(ClientError::Protocol(format!("expected Session, got {other:?}"))),
        }
    }

    pub async fn cancel_turn(&self, session_id: i64) -> Result<(), ClientError> {
        let response = self
            .http
            .post(self.endpoint(&format!("/api/v1/sessions/{session_id}/cancel")))
            .send()
            .await?;
        let _ = self.parse_command(response).await?;
        Ok(())
    }

    pub async fn reply_permission(
        &self,
        params: ReplyPermissionParams,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "options": params.options,
            "reply": params.reply,
        });
        let response = self
            .http
            .post(self.endpoint(&format!(
                "/api/v1/sessions/{}/permission-replies",
                params.session_id
            )))
            .json(&body)
            .send()
            .await?;
        match self.parse_command(response).await? {
            CommandResult::Session(s) => Ok(s),
            other => Err(ClientError::Protocol(format!("expected Session, got {other:?}"))),
        }
    }

    pub async fn reply_user_input(
        &self,
        params: ReplyUserInputParams,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "options": params.options,
            "reply": params.reply,
        });
        let response = self
            .http
            .post(self.endpoint(&format!(
                "/api/v1/sessions/{}/user-input-replies",
                params.session_id
            )))
            .json(&body)
            .send()
            .await?;
        match self.parse_command(response).await? {
            CommandResult::Session(s) => Ok(s),
            other => Err(ClientError::Protocol(format!("expected Session, got {other:?}"))),
        }
    }

    pub async fn list_events(
        &self,
        params: ListEventsParams,
    ) -> Result<PaginatedEvents, ClientError> {
        let mut url = self.endpoint("/api/v1/events");
        // Encode each param manually so we don't rely on serde_urlencoded
        // for nested types (Scope is tagged, kinds is a HashSet).
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
        match self.parse_query(response).await? {
            QueryResult::Events(p) => Ok(p),
            other => Err(ClientError::Protocol(format!("expected Events, got {other:?}"))),
        }
    }

    /// Escape hatch: run any [`Command`] over REST. Falls back to
    /// `submit_turn` etc. when you'd rather use the typed conveniences.
    pub async fn command(&self, _cmd: Command) -> Result<CommandResult, ClientError> {
        // The REST surface is route-per-command; this helper exists as a
        // future hook for a generic `/api/v1/commands` endpoint. For now
        // callers should use the typed methods.
        Err(ClientError::Protocol(
            "generic command dispatch over REST is not implemented; use typed helpers or WS".into(),
        ))
    }

    /// Generic query escape hatch. Only `Query::Health` and
    /// `Query::ListEvents` are routed for now; use typed helpers for the
    /// rest.
    pub async fn query(&self, q: Query) -> Result<QueryResult, ClientError> {
        match q {
            Query::Health => Ok(QueryResult::Health(self.health().await?)),
            Query::ListEvents(p) => Ok(QueryResult::Events(self.list_events(p).await?)),
            _ => Err(ClientError::Protocol(
                "generic query dispatch over REST is not implemented; use typed helpers or WS".into(),
            )),
        }
    }
}
