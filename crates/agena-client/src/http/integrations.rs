use super::{AgenaClient, ClientError};
use agena_api::integrations::*;
use uuid::Uuid;
impl AgenaClient {
    pub async fn update_channel_access(
        &self,
        source: i64,
        id: Uuid,
        request: &ChannelPolicyUpdate,
    ) -> Result<IntegrationBinding, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::PUT,
                self.endpoint(&format!(
                    "/api/v1/sessions/{source}/integrations/{id}/access"
                )),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    pub async fn integrations(&self, source: i64) -> Result<IntegrationsSnapshot, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{source}/integrations")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn create_integration(
        &self,
        source: i64,
        request: &CreateIntegration,
    ) -> Result<IntegrationBinding, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!("/api/v1/sessions/{source}/integrations")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn integration_detail(
        &self,
        source: i64,
        id: Uuid,
    ) -> Result<IntegrationDetail, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{source}/integrations/{id}")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }
    pub async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: &IntegrationControl,
    ) -> Result<IntegrationBinding, ClientError> {
        let body = serde_json::json!(request);
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!(
                    "/api/v1/sessions/{source}/integrations/{id}/control"
                )),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }
}
