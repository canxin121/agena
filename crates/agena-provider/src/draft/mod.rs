//! Shared editable provider contracts. These values never read or save credentials.

use crate::CredentialIssuer;
use anyhow::anyhow;
mod draft_auth_data;
mod draft_config;
mod draft_validation;
pub use draft_auth_data::*;
pub use draft_config::ProviderConfigDraft;

pub fn credential_issuer_label(issuer: CredentialIssuer) -> &'static str {
    match issuer {
        CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        CredentialIssuer::GithubCopilot => "github_copilot",
        CredentialIssuer::Gitlab => "gitlab",
        CredentialIssuer::GoogleAdc => "google_adc",
        CredentialIssuer::SapAiCore => "sap_ai_core",
    }
}

pub fn optional_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub fn parse_oauth_expires_at_ms(value: &str) -> anyhow::Result<i64> {
    optional_non_empty(value)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| anyhow!("expires_at_ms must be an integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

pub fn parse_credential_issuer(value: &str) -> anyhow::Result<crate::CredentialIssuer> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_chatgpt" => Ok(crate::CredentialIssuer::OpenaiChatgpt),
        "github_copilot" => Ok(crate::CredentialIssuer::GithubCopilot),
        "gitlab" => Ok(crate::CredentialIssuer::Gitlab),
        "google_adc" => Ok(crate::CredentialIssuer::GoogleAdc),
        "sap_ai_core" => Ok(crate::CredentialIssuer::SapAiCore),
        _ => Err(anyhow!(
            "unsupported credential issuer `{}`; expected openai_chatgpt, github_copilot, gitlab, google_adc, or sap_ai_core",
            value.trim()
        )),
    }
}

pub fn trimmed_owned(value: &str) -> Option<String> {
    optional_non_empty(value).map(ToOwned::to_owned)
}

pub fn supported_provider_draft_adapter_list(auth_kind: &ProviderDraftAuthKind) -> String {
    let supported = auth_kind
        .adapter_rules()
        .iter()
        .map(|rule| rule.adapter_id)
        .collect::<Vec<_>>()
        .join(", ");
    if supported.is_empty() {
        "no adapters until auth details are selected".to_owned()
    } else {
        supported
    }
}
