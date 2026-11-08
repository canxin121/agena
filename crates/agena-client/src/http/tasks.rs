//! Typed, authenticated task API. Mutations are never automatically retried;
//! callers retain the request key and revision when the receipt is uncertain.
use agena_api::tasks::{CreateTask, ResumeTask, Task, TaskQuery, TaskRevisionRequest};
use uuid::Uuid;

use super::{AgenaClient, ClientError};

impl AgenaClient {
    pub async fn list_tasks(&self, query: &TaskQuery) -> Result<Vec<Task>, ClientError> {
        let mut url = self.endpoint("/api/v1/tasks");
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(id) = query.source_session_id {
                pairs.append_pair("source_session_id", &id.to_string());
            }
            if let Some(id) = query.workspace_id {
                pairs.append_pair("workspace_id", &id.to_string());
            }
            if let Some(state) = query.state {
                pairs.append_pair("state", state.as_str());
            }
            pairs.append_pair("offset", &query.offset.to_string());
            if let Some(limit) = query.limit {
                pairs.append_pair("limit", &limit.to_string());
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn get_task(&self, id: Uuid) -> Result<Task, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::GET,
                self.endpoint(&format!("/api/v1/tasks/{id}")),
                None,
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    async fn task_mutation(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<Task, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(path),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    pub async fn create_task(&self, request: &CreateTask) -> Result<Task, ClientError> {
        self.task_mutation("/api/v1/tasks", serde_json::json!(request))
            .await
    }

    pub async fn cancel_task(&self, id: Uuid, expected_revision: i64) -> Result<Task, ClientError> {
        self.task_mutation(
            &format!("/api/v1/tasks/{id}/cancel"),
            serde_json::json!(TaskRevisionRequest { expected_revision }),
        )
        .await
    }

    pub async fn resume_task(&self, id: Uuid, request: &ResumeTask) -> Result<Task, ClientError> {
        self.task_mutation(
            &format!("/api/v1/tasks/{id}/resume"),
            serde_json::json!(request),
        )
        .await
    }

    pub async fn accept_task(&self, id: Uuid, expected_revision: i64) -> Result<Task, ClientError> {
        self.task_mutation(
            &format!("/api/v1/tasks/{id}/accept"),
            serde_json::json!(TaskRevisionRequest { expected_revision }),
        )
        .await
    }
}
