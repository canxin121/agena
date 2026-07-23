//! Provider-owned Bedrock SigV4 credential settings.

use std::fmt;

use serde::Serialize;

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BedrockSigv4AuthConfig {
    pub base_url: String,
    pub region: String,
    pub profile: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
}

impl fmt::Debug for BedrockSigv4AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BedrockSigv4AuthConfig")
            .field("base_url", &self.base_url)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .field("access_key_id", &redacted(self.access_key_id.as_deref()))
            .field(
                "secret_access_key",
                &redacted(self.secret_access_key.as_deref()),
            )
            .field("session_token", &redacted(self.session_token.as_deref()))
            .finish()
    }
}

fn redacted(value: Option<&str>) -> &'static str {
    match value {
        Some(value) if !value.is_empty() => "***redacted***",
        _ => "<none>",
    }
}

#[cfg(test)]
mod tests {
    use super::BedrockSigv4AuthConfig;

    #[test]
    fn debug_output_redacts_bedrock_secrets() {
        let config = BedrockSigv4AuthConfig {
            base_url: "https://bedrock.example".to_owned(),
            region: "us-east-1".to_owned(),
            profile: None,
            access_key_id: Some("key".to_owned()),
            secret_access_key: Some("secret".to_owned()),
            session_token: None,
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("***redacted***"));
        assert!(!debug.contains("\"secret\""));
    }
}
