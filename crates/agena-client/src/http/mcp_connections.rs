//! Management of external MCP connections owned by the Agena server.

use agena_api::mcp_connections::{
    McpCapabilityPage, McpConnectionCatalog, McpConnectionDetail, McpConnectionScope,
    McpImportPreview, McpImportResult, McpMutationResult, McpOAuthStart, McpTestResult,
};
use serde_json::{Value, json};

use super::{AgenaClient, ClientError};

impl AgenaClient {
    async fn mcp_connection_request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        segments: &[&str],
        body: Option<Value>,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let mut url = self.endpoint("/api/v1/mcp-connections");
        {
            let mut path = url
                .path_segments_mut()
                .expect("HTTP endpoint supports path segments");
            for segment in segments {
                path.push(segment);
            }
        }
        for (key, value) in query {
            url.query_pairs_mut().append_pair(key, value);
        }
        let response = self.send_request(method, url, body.as_ref(), None).await?;
        self.parse_json(response).await
    }

    pub async fn list_mcp_connections(&self) -> Result<McpConnectionCatalog, ClientError> {
        self.mcp_connection_request(reqwest::Method::GET, &[], None, &[])
            .await
    }

    pub async fn set_mcp_bridge_enabled(
        &self,
        enabled: bool,
        scope: McpConnectionScope,
    ) -> Result<McpConnectionCatalog, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::PUT,
            &["bridge", "enabled"],
            Some(json!({"enabled":enabled,"scope":scope})),
            &[],
        )
        .await
    }

    pub async fn get_mcp_connection(&self, id: &str) -> Result<McpConnectionDetail, ClientError> {
        self.mcp_connection_request(reqwest::Method::GET, &[id], None, &[])
            .await
    }

    pub async fn create_mcp_connection(
        &self,
        name: &str,
        scope: McpConnectionScope,
        config: &Value,
    ) -> Result<McpMutationResult, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[],
            Some(json!({"name":name,"scope":scope,"config":config})),
            &[],
        )
        .await
    }

    pub async fn update_mcp_connection(
        &self,
        id: &str,
        config: &Value,
        preserve_secrets: &[String],
        expected_revision: &str,
    ) -> Result<McpMutationResult, ClientError> {
        self.mcp_connection_request(reqwest::Method::PUT, &[id], Some(json!({
            "config":config,"preserve_secrets":preserve_secrets,"expected_revision":expected_revision
        })), &[]).await
    }

    pub async fn delete_mcp_connection(
        &self,
        id: &str,
        expected_revision: &str,
    ) -> Result<McpMutationResult, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::DELETE,
            &[id],
            None,
            &[("expected_revision", expected_revision)],
        )
        .await
    }

    pub async fn set_mcp_connection_enabled(
        &self,
        id: &str,
        enabled: bool,
        expected_revision: &str,
    ) -> Result<McpMutationResult, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::PUT,
            &[id, "enabled"],
            Some(json!({"enabled":enabled,"expected_revision":expected_revision})),
            &[],
        )
        .await
    }

    pub async fn reconnect_mcp_connection(
        &self,
        id: &str,
    ) -> Result<McpConnectionDetail, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[id, "reconnect"],
            Some(json!({})),
            &[],
        )
        .await
    }

    pub async fn test_mcp_connection(&self, id: &str) -> Result<McpTestResult, ClientError> {
        self.mcp_connection_request(reqwest::Method::POST, &[id, "test"], Some(json!({})), &[])
            .await
    }

    pub async fn mcp_connection_capabilities(
        &self,
        id: &str,
        kind: &str,
        cursor: Option<&str>,
    ) -> Result<McpCapabilityPage, ClientError> {
        let mut query = vec![("kind", kind)];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        self.mcp_connection_request(reqwest::Method::GET, &[id, "capabilities"], None, &query)
            .await
    }

    pub async fn read_mcp_connection_resource(
        &self,
        id: &str,
        uri: &str,
    ) -> Result<Value, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[id, "resource"],
            Some(json!({"uri":uri})),
            &[],
        )
        .await
    }

    pub async fn get_mcp_connection_prompt(
        &self,
        id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<Value, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[id, "prompt"],
            Some(json!({"name":name,"arguments":arguments})),
            &[],
        )
        .await
    }

    pub async fn preview_mcp_connection_import(
        &self,
        scope: McpConnectionScope,
        document: &Value,
    ) -> Result<McpImportPreview, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &["import", "preview"],
            Some(json!({"scope":scope,"document":document})),
            &[],
        )
        .await
    }

    pub async fn import_mcp_connections(
        &self,
        scope: McpConnectionScope,
        document: &Value,
        selected: &[String],
        replace: bool,
        expected_revision: &str,
    ) -> Result<McpImportResult, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &["import"],
            Some(json!({
                "scope":scope,"document":document,"selected":selected,"replace":replace,
                "expected_revision":expected_revision
            })),
            &[],
        )
        .await
    }

    pub async fn set_mcp_connection_credential(
        &self,
        id: &str,
        token: &str,
    ) -> Result<McpConnectionDetail, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::PUT,
            &[id, "credential"],
            Some(json!({"token":token})),
            &[],
        )
        .await
    }

    pub async fn delete_mcp_connection_credential(
        &self,
        id: &str,
    ) -> Result<McpConnectionDetail, ClientError> {
        self.mcp_connection_request(reqwest::Method::DELETE, &[id, "credential"], None, &[])
            .await
    }

    pub fn mcp_authorization_redirect_uri(&self) -> String {
        self.endpoint("/settings/mcp-connections").to_string()
    }

    pub async fn start_mcp_authorization(
        &self,
        id: &str,
        redirect_uri: &str,
    ) -> Result<McpOAuthStart, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[id, "auth", "start"],
            Some(json!({"redirect_uri":redirect_uri})),
            &[],
        )
        .await
    }

    pub async fn complete_mcp_authorization(
        &self,
        id: &str,
        flow_id: &str,
        callback_url: &str,
    ) -> Result<McpConnectionDetail, ClientError> {
        self.mcp_connection_request(
            reqwest::Method::POST,
            &[id, "auth", "complete"],
            Some(json!({"flow_id":flow_id,"callback_url":callback_url})),
            &[],
        )
        .await
    }

    pub async fn cancel_mcp_authorization(
        &self,
        id: &str,
        flow_id: Option<&str>,
    ) -> Result<McpConnectionDetail, ClientError> {
        let query = flow_id
            .map(|flow| vec![("flow_id", flow)])
            .unwrap_or_default();
        self.mcp_connection_request(reqwest::Method::DELETE, &[id, "auth"], None, &query)
            .await
    }
}
