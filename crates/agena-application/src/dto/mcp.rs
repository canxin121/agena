use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Durable backend used for a manual MCP bearer credential.
pub enum McpCredentialStoreResource {
    #[default]
    Keyring,
    File,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to store a manual MCP bearer credential in the center process.
pub struct McpBearerCredentialWriteRequest {
    pub token: String,
    #[serde(default)]
    pub store: McpCredentialStoreResource,
}

#[derive(Debug, Clone, Default, Deserialize)]
/// Query selecting the manual bearer credential backend to clear.
pub struct McpBearerCredentialDeleteQuery {
    #[serde(default)]
    pub store: McpCredentialStoreResource,
}

#[derive(Debug, Clone, Deserialize)]
/// Begin a center-owned MCP OAuth authorization-code flow.
pub struct McpOAuthStartRequest {
    pub server: String,
    pub url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Redacted browser authorization state returned to a thin client.
pub struct McpOAuthStartResource {
    pub flow_id: uuid::Uuid,
    pub server: String,
    pub authorization_url: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Verified loopback callback forwarded to the center-owned OAuth flow.
pub struct McpOAuthFinishRequest {
    pub flow_id: uuid::Uuid,
    pub code: String,
    pub state: String,
    #[serde(default)]
    pub issuer: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
/// Options for removing a center-owned MCP OAuth credential.
pub struct McpOAuthDeleteQuery {
    #[serde(default)]
    pub revoke: bool,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Kind of MCP credential changed by a center operation.
pub enum McpCredentialKindResource {
    Bearer,
    OAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Secret-free result of one MCP credential mutation.
pub struct McpCredentialMutationResource {
    pub server: String,
    pub credential_kind: McpCredentialKindResource,
    pub store: McpCredentialStoreResource,
    pub action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_resource_has_no_place_for_credential_material() {
        let value = serde_json::to_value(McpCredentialMutationResource {
            server: "example".to_owned(),
            credential_kind: McpCredentialKindResource::Bearer,
            store: McpCredentialStoreResource::Keyring,
            action: "stored".to_owned(),
        })
        .expect("serialize MCP credential result");

        assert_eq!(value["server"], "example");
        assert!(value.get("token").is_none());
        assert!(value.get("credential").is_none());
    }
}
