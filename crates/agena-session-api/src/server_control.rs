//! Listener-owned server controls exposed through the shared operations API.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ServerQuery {
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ServerCommand {
    UpdateMcp(UpdateMcpServerControlRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerControlResponse {
    pub enabled: bool,
    pub server_url: Option<String>,
    pub public_url: Option<String>,
    pub oauth_issuer_url: Option<String>,
    pub resource_url: String,
    pub ready: bool,
    pub warnings: Vec<String>,
    pub oauth: McpOAuthControlStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthControlStatus {
    pub configured: bool,
    pub password_configured: bool,
    pub password_source: String,
    pub ready: bool,
    pub authorization_server_kind: String,
    pub registration_methods: Vec<String>,
    pub token_endpoint_auth_methods: Vec<String>,
    pub pkce_methods: Vec<String>,
    pub oidc_supported: bool,
    pub warnings: Vec<String>,
    pub scope: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub protected_resource_metadata: String,
    pub authorization_server_metadata: String,
    pub jwks_uri: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMcpServerControlRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// One origin derives both /mcp and the OAuth issuer, clearing overrides.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_nullable_update"
    )]
    pub server_url: Option<Option<String>>,
    /// Omitted preserves the value; null restores the listener-local fallback.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_nullable_update"
    )]
    pub public_url: Option<Option<String>>,
    /// Omitted preserves the value; null restores the resource-derived issuer.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_nullable_update"
    )]
    pub oauth_issuer_url: Option<Option<String>>,
}

fn deserialize_nullable_update<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_updates_preserve_omitted_and_explicit_null_across_transports() {
        for value in [
            json!({}),
            json!({"enabled": false}),
            json!({"serverUrl": null}),
            json!({"publicUrl": "https://example.test/mcp", "oauthIssuerUrl": null}),
        ] {
            let request: UpdateMcpServerControlRequest =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), value);
        }
        let keep: UpdateMcpServerControlRequest = serde_json::from_value(json!({})).unwrap();
        let clear: UpdateMcpServerControlRequest =
            serde_json::from_value(json!({"serverUrl": null})).unwrap();
        assert_eq!(keep.server_url, None);
        assert_eq!(clear.server_url, Some(None));
    }
}
