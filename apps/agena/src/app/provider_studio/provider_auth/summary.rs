use super::super::provider_selection::*;

use super::super::{
    CredentialIssuer, Duration, I18n, ProviderDraftInteractiveLoginKind, ProviderStudioField,
    ProviderStudioOverlay, join_inline_segments, provider_studio_field_label, ui_text,
};
use super::provider_studio_detail_fields;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ProviderStudioSummaryLabel {
    Env,
    Callback,
    Redirect,
    Account,
    Profile,
    Region,
    Code,
}

pub(in crate::app) fn provider_studio_summary_label(
    i18n: &I18n,
    label: ProviderStudioSummaryLabel,
) -> String {
    ui_text::t(
        i18n,
        match label {
            ProviderStudioSummaryLabel::Env => "provider-studio-summary-env",
            ProviderStudioSummaryLabel::Callback => "provider-studio-summary-callback",
            ProviderStudioSummaryLabel::Redirect => "provider-studio-summary-redirect",
            ProviderStudioSummaryLabel::Account => "provider-studio-summary-account",
            ProviderStudioSummaryLabel::Profile => "provider-studio-summary-profile",
            ProviderStudioSummaryLabel::Region => "provider-studio-summary-region",
            ProviderStudioSummaryLabel::Code => "provider-studio-summary-code",
        },
    )
}

pub(in crate::app) fn provider_studio_labeled_summary(
    i18n: &I18n,
    label: ProviderStudioSummaryLabel,
    value: &str,
    max_width: usize,
) -> Option<String> {
    provider_studio_summary_value(value, max_width)
        .map(|value| format!("{} {value}", provider_studio_summary_label(i18n, label)))
}

pub(in crate::app) fn provider_studio_action_with_summary(
    i18n: &I18n,
    action_key: &str,
    summary: Option<String>,
) -> String {
    let mut parts = vec![ui_text::t(i18n, action_key)];
    if let Some(summary) = summary {
        parts.push(summary);
    }
    join_inline_segments(parts)
}

pub(in crate::app) fn provider_studio_state_summary(
    i18n: &I18n,
    state: &str,
    max_width: usize,
) -> Option<String> {
    provider_studio_summary_value(state, max_width).map(|state| {
        i18n.text_args(
            "provider-studio-summary-state",
            &agena_tui::fl_args!("state" => state),
        )
    })
}

pub(in crate::app) fn provider_studio_required_field_summary(
    i18n: &I18n,
    field: ProviderStudioField,
) -> String {
    i18n.text_args(
        "provider-studio-summary-set-field",
        &agena_tui::fl_args!("field" => provider_studio_field_label(i18n, field)),
    )
}

pub(in crate::app) fn provider_studio_auth_login_kind(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderDraftInteractiveLoginKind> {
    dialog.draft.interactive_login_kind()
}

pub(in crate::app) fn provider_studio_auth_poll_interval(
    dialog: &ProviderStudioOverlay,
) -> Option<Duration> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if provider_studio_auth_login_kind(dialog)
                != Some(ProviderDraftInteractiveLoginKind::Device)
            {
                return None;
            }
            dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .as_ref()
                .map(|device| Duration::from_secs(device.interval_seconds.max(1)))
        }
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| Duration::from_secs(device.interval_seconds.max(1))),
        Some(CredentialIssuer::Gitlab)
        | Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore)
        | None => None,
    }
}

pub(in crate::app) fn provider_studio_available_login_kinds(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderDraftInteractiveLoginKind> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => vec![
            ProviderDraftInteractiveLoginKind::Device,
            ProviderDraftInteractiveLoginKind::Browser,
        ],
        Some(CredentialIssuer::GithubCopilot) => vec![ProviderDraftInteractiveLoginKind::Device],
        Some(CredentialIssuer::Gitlab) => vec![ProviderDraftInteractiveLoginKind::Browser],
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => Vec::new(),
    }
}

pub(in crate::app) fn provider_studio_auth_login_kind_label(
    i18n: &I18n,
    kind: ProviderDraftInteractiveLoginKind,
) -> String {
    ui_text::t(
        i18n,
        match kind {
            ProviderDraftInteractiveLoginKind::Browser => "provider-auth-login-kind-browser-label",
            ProviderDraftInteractiveLoginKind::Device => "provider-auth-login-kind-device-label",
        },
    )
}

pub(in crate::app) fn provider_studio_browser_continue_summary(
    i18n: &I18n,
    prefix_key: &str,
    state: &str,
) -> String {
    let mut parts = vec![ui_text::t(i18n, prefix_key)];
    if let Some(state) = provider_studio_state_summary(i18n, state, 20) {
        parts.push(state);
    }
    join_inline_segments(parts)
}

pub(in crate::app) fn provider_studio_detail_field_index(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> Option<usize> {
    provider_studio_detail_fields(dialog)
        .iter()
        .position(|candidate| *candidate == field)
}

pub(in crate::app) fn provider_studio_missing_start_auth_field(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderStudioField> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => (provider_studio_auth_login_kind(dialog)
            == Some(ProviderDraftInteractiveLoginKind::Browser)
            && dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .redirect_uri
                .trim()
                .is_empty())
        .then_some(ProviderStudioField::RedirectUri),
        Some(CredentialIssuer::GithubCopilot) => None,
        Some(CredentialIssuer::Gitlab) => {
            if dialog.draft.auth.instance_url.trim().is_empty() {
                Some(ProviderStudioField::InstanceUrl)
            } else {
                dialog
                    .draft
                    .credential_drafts
                    .gitlab
                    .redirect_uri
                    .trim()
                    .is_empty()
                    .then_some(ProviderStudioField::RedirectUri)
            }
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
    }
}

pub(in crate::app) fn provider_studio_missing_continue_auth_field(
    dialog: &ProviderStudioOverlay,
) -> Option<ProviderStudioField> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if provider_studio_auth_login_kind(dialog)
                == Some(ProviderDraftInteractiveLoginKind::Browser)
            {
                if dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .redirect_uri
                    .trim()
                    .is_empty()
                {
                    Some(ProviderStudioField::RedirectUri)
                } else {
                    dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .browser
                        .as_ref()
                        .and_then(|_| {
                            dialog
                                .draft
                                .credential_drafts
                                .openai_chatgpt
                                .callback_url
                                .trim()
                                .is_empty()
                                .then_some(ProviderStudioField::CallbackUrl)
                        })
                }
            } else {
                None
            }
        }
        Some(CredentialIssuer::GithubCopilot) => None,
        Some(CredentialIssuer::Gitlab) => {
            if dialog.draft.auth.instance_url.trim().is_empty() {
                Some(ProviderStudioField::InstanceUrl)
            } else if dialog
                .draft
                .credential_drafts
                .gitlab
                .redirect_uri
                .trim()
                .is_empty()
            {
                Some(ProviderStudioField::RedirectUri)
            } else {
                dialog
                    .draft
                    .credential_drafts
                    .gitlab
                    .browser
                    .as_ref()
                    .and_then(|_| {
                        dialog
                            .draft
                            .credential_drafts
                            .gitlab
                            .callback_url
                            .trim()
                            .is_empty()
                            .then_some(ProviderStudioField::CallbackUrl)
                    })
            }
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
    }
}

pub(in crate::app) fn provider_studio_preferred_detail_field_index(
    dialog: &ProviderStudioOverlay,
) -> usize {
    provider_studio_missing_continue_auth_field(dialog)
        .and_then(|field| provider_studio_detail_field_index(dialog, field))
        .or_else(|| match dialog.draft.auth_kind.credential_issuer() {
            Some(CredentialIssuer::OpenaiChatgpt)
                if provider_studio_auth_login_kind(dialog)
                    == Some(ProviderDraftInteractiveLoginKind::Browser) =>
            {
                dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .browser
                    .as_ref()
                    .and_then(|_| {
                        provider_studio_detail_field_index(dialog, ProviderStudioField::CallbackUrl)
                    })
            }
            Some(CredentialIssuer::OpenaiChatgpt) => None,
            Some(CredentialIssuer::Gitlab) => dialog
                .draft
                .credential_drafts
                .gitlab
                .browser
                .as_ref()
                .and_then(|_| {
                    provider_studio_detail_field_index(dialog, ProviderStudioField::CallbackUrl)
                }),
            Some(CredentialIssuer::GithubCopilot)
            | Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore)
            | None => None,
        })
        .or_else(|| {
            provider_studio_missing_start_auth_field(dialog)
                .and_then(|field| provider_studio_detail_field_index(dialog, field))
        })
        .unwrap_or(0)
}
