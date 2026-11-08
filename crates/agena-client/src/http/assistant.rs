use super::{AgenaClient, ClientError};
use agena_api::assistant::*;
use uuid::Uuid;
impl AgenaClient {
    pub async fn assistant_snapshot(&self, source: i64) -> Result<AssistantSnapshot, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{source}/assistant")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn update_assistant(
        &self,
        source: i64,
        request: &AssistantUpdate,
    ) -> Result<AssistantProfile, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::PUT,
                self.endpoint(&format!("/api/v1/sessions/{source}/assistant")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn control_assistant_reminder(
        &self,
        source: i64,
        id: Uuid,
        request: &ReminderControl,
    ) -> Result<Reminder, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!(
                    "/api/v1/sessions/{source}/assistant/reminders/{id}"
                )),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn edit_assistant_memory(
        &self,
        source: i64,
        request: &AssistantMemoryUpdate,
    ) -> Result<Option<AssistantMemory>, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!("/api/v1/sessions/{source}/assistant/memory")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
}
