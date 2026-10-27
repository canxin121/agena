//! Typed client for the shared skill-management API.

use agena_api::skills::{
    SkillCatalog, SkillDeleteResult, SkillDetail, SkillInstallPreview, SkillInstallResult,
    SkillInstallSource, SkillResource, SkillScope, SkillValidation,
};
use serde_json::{Value, json};

use super::{AgenaClient, ClientError};

impl AgenaClient {
    async fn skill_request<T: serde::de::DeserializeOwned>(
        &self,
        workspace_id: i64,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let mut url = self.endpoint(path);
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("workspace_id", &workspace_id.to_string());
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let response = self.send_request(method, url, body.as_ref(), None).await?;
        self.parse_json(response).await
    }

    pub async fn list_skills(&self, workspace_id: i64) -> Result<SkillCatalog, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::GET,
            "/api/v1/skills",
            None,
            &[],
        )
        .await
    }

    pub async fn get_skill(&self, workspace_id: i64, id: &str) -> Result<SkillDetail, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::GET,
            &format!("/api/v1/skills/{id}"),
            None,
            &[],
        )
        .await
    }

    pub async fn create_skill(
        &self,
        workspace_id: i64,
        scope: SkillScope,
        document: &str,
    ) -> Result<SkillDetail, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::POST,
            "/api/v1/skills",
            Some(json!({ "scope": scope, "document": document })),
            &[],
        )
        .await
    }

    pub async fn update_skill(
        &self,
        workspace_id: i64,
        id: &str,
        document: &str,
        expected_revision: &str,
    ) -> Result<SkillDetail, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::PUT,
            &format!("/api/v1/skills/{id}"),
            Some(json!({ "document": document, "expected_revision": expected_revision })),
            &[],
        )
        .await
    }

    pub async fn delete_skill(
        &self,
        workspace_id: i64,
        id: &str,
        expected_revision: &str,
    ) -> Result<SkillDeleteResult, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::DELETE,
            &format!("/api/v1/skills/{id}"),
            None,
            &[("expected_revision", expected_revision)],
        )
        .await
    }

    pub async fn set_skill_enabled(
        &self,
        workspace_id: i64,
        id: &str,
        enabled: bool,
        expected_revision: &str,
    ) -> Result<SkillDetail, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::PUT,
            &format!("/api/v1/skills/{id}/enabled"),
            Some(json!({ "enabled": enabled, "expected_revision": expected_revision })),
            &[],
        )
        .await
    }

    pub async fn get_skill_resource(
        &self,
        workspace_id: i64,
        id: &str,
        path: &str,
    ) -> Result<SkillResource, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::GET,
            &format!("/api/v1/skills/{id}/resource"),
            None,
            &[("path", path)],
        )
        .await
    }

    /// Omitting the revision creates a new resource; an existing resource needs
    /// its last-read revision so concurrent edits cannot be overwritten.
    pub async fn write_skill_resource(
        &self,
        workspace_id: i64,
        id: &str,
        path: &str,
        content: &str,
        expected_revision: Option<&str>,
    ) -> Result<SkillResource, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::PUT,
            &format!("/api/v1/skills/{id}/resource"),
            Some(json!({
                "path": path,
                "content": content,
                "expected_revision": expected_revision,
            })),
            &[],
        )
        .await
    }

    pub async fn delete_skill_resource(
        &self,
        workspace_id: i64,
        id: &str,
        path: &str,
        expected_revision: &str,
    ) -> Result<SkillDetail, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::DELETE,
            &format!("/api/v1/skills/{id}/resource"),
            None,
            &[("path", path), ("expected_revision", expected_revision)],
        )
        .await
    }

    pub async fn validate_skill(
        &self,
        workspace_id: i64,
        document: &str,
        name: Option<&str>,
    ) -> Result<SkillValidation, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::POST,
            "/api/v1/skills/validate",
            Some(json!({ "document": document, "name": name })),
            &[],
        )
        .await
    }

    pub async fn preview_skill_install(
        &self,
        workspace_id: i64,
        scope: SkillScope,
        source: &SkillInstallSource,
    ) -> Result<SkillInstallPreview, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::POST,
            "/api/v1/skills/install/preview",
            Some(json!({ "scope": scope, "source": source })),
            &[],
        )
        .await
    }

    pub async fn install_skills(
        &self,
        workspace_id: i64,
        scope: SkillScope,
        source: &SkillInstallSource,
        selected: &[String],
        replace: bool,
        expected_source_revision: &str,
    ) -> Result<SkillInstallResult, ClientError> {
        self.skill_request(
            workspace_id,
            reqwest::Method::POST,
            "/api/v1/skills/install",
            Some(json!({
                "scope": scope,
                "source": source,
                "selected": selected,
                "replace": replace,
                "expected_source_revision": expected_source_revision,
            })),
            &[],
        )
        .await
    }
}
