use anyhow::anyhow;

pub(super) fn parse_credential_issuer(value: &str) -> Result<CredentialIssuer> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_chatgpt" => Ok(CredentialIssuer::OpenaiChatgpt),
        "github_copilot" => Ok(CredentialIssuer::GithubCopilot),
        "gitlab" => Ok(CredentialIssuer::Gitlab),
        "google_adc" => Ok(CredentialIssuer::GoogleAdc),
        "sap_ai_core" => Ok(CredentialIssuer::SapAiCore),
        _ => Err(anyhow!(
            "unsupported credential issuer `{}`; expected openai_chatgpt, github_copilot, gitlab, google_adc, or sap_ai_core",
            value.trim()
        )),
    }
}

pub(super) fn summarize_named_mode(
    display_name: Option<&str>,
    description: Option<&str>,
) -> String {
    match (
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        description.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(display_name), Some(description)) => format!("{display_name} · {description}"),
        (Some(display_name), None) => display_name.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => "configured mode".to_owned(),
    }
}

pub(super) fn parse_aws_profile_names(text: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') || !line.ends_with(']') {
            continue;
        }
        let section = line.trim_start_matches('[').trim_end_matches(']').trim();
        if section.eq_ignore_ascii_case("default") {
            names.insert("default".to_owned());
            continue;
        }
        if let Some(profile) = section.strip_prefix("profile ") {
            let profile = profile.trim();
            if !profile.is_empty() {
                names.insert(profile.to_owned());
            }
            continue;
        }
        if !section.contains(' ') && !section.contains('.') {
            names.insert(section.to_owned());
        }
    }
    names.into_iter().collect()
}

pub(super) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

pub(super) fn trimmed_owned(value: &str) -> Option<String> {
    non_empty(Some(value)).map(ToOwned::to_owned)
}

use crate::CredentialIssuer;
use crate::Result;
