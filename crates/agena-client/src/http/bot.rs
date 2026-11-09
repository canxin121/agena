use super::{AgenaClient, ClientError};
use agena_api::bot::*;
use uuid::Uuid;

impl AgenaClient {
    /// Resolve the server's oldest active Bot. Never selects by title or a local session ID.
    pub async fn ensure_bot_session(&self) -> Result<BotSession, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint("/api/v1/bot/session"),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn list_bots(&self, include_archived: bool) -> Result<BotList, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/bots?include_archived={include_archived}")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn create_bot(&self, request: &BotCreate) -> Result<BotProfile, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint("/api/v1/bots"),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn get_bot(&self, id: Uuid) -> Result<BotProfile, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/bots/{id}")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn update_bot(
        &self,
        id: Uuid,
        request: &BotUpdate,
    ) -> Result<BotProfile, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::PUT,
                self.endpoint(&format!("/api/v1/bots/{id}")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn archive_bot(
        &self,
        id: Uuid,
        request: &BotArchive,
    ) -> Result<BotProfile, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!("/api/v1/bots/{id}/archive")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn ensure_named_bot_session(&self, id: Uuid) -> Result<BotSession, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!("/api/v1/bots/{id}/session")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
}
