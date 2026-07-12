use super::super::provider_selection::*;

use super::super::{
    CredentialIssuer, I18n, ProviderConfigDraft, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind, ProviderStudioField,
    ProviderStudioOverlay, provider_studio_field_value, truncate_display_width, ui_text,
};
use super::{
    provider_studio_auth_details_summary, provider_studio_auth_login_kind,
    provider_studio_available_login_kinds, provider_studio_continue_auth_summary,
    provider_studio_start_auth_summary,
};

pub(in crate::app) fn provider_studio_main_field_value(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    match field {
        ProviderStudioField::AuthLoginMethod => dialog
            .draft
            .interactive_login_kind()
            .map(|kind| kind.token().to_owned())
            .unwrap_or_default(),
        ProviderStudioField::StartAuthAction => provider_studio_start_auth_summary(i18n, dialog),
        ProviderStudioField::ContinueAuthAction => {
            provider_studio_continue_auth_summary(i18n, dialog)
        }
        ProviderStudioField::EditAuthDetailsAction => {
            provider_studio_auth_details_summary(i18n, dialog)
        }
        _ => provider_studio_field_value(&dialog.draft, field),
    }
}

pub(in crate::app) fn provider_studio_has_pending_auth_state(
    dialog: &ProviderStudioOverlay,
) -> bool {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .is_some(),
            Some(ProviderDraftInteractiveLoginKind::Device) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .is_some(),
            None => false,
        },
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .is_some(),
        Some(CredentialIssuer::Gitlab) => dialog.draft.credential_drafts.gitlab.browser.is_some(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => false,
    }
}

pub(in crate::app) fn provider_studio_auth_state_lines(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> Vec<String> {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .browser
                .as_ref()
                .map(|session| {
                    vec![
                        ui_text::t(i18n, "provider-studio-auth-openai-ready"),
                        i18n.text_args(
                            "provider-studio-auth-authorize",
                            &crate::fl_args!("url" => session.display_authorize_url().to_owned()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-redirect",
                            &crate::fl_args!(
                                "url" => dialog
                                    .draft
                                    .credential_drafts
                                    .openai_chatgpt
                                    .redirect_uri
                                    .clone()
                            ),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-paste-callback",
                            &crate::fl_args!(
                                "state" => truncate_display_width(session.state.as_str(), 24)
                            ),
                        ),
                    ]
                })
                .unwrap_or_default(),
            Some(ProviderDraftInteractiveLoginKind::Device) => dialog
                .draft
                .credential_drafts
                .openai_chatgpt
                .device
                .as_ref()
                .map(|device| {
                    vec![
                        i18n.text_args(
                            "provider-studio-auth-openai-device-ready",
                            &crate::fl_args!("code" => device.user_code.clone()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-verify",
                            &crate::fl_args!("url" => device.display_verification_url().to_owned()),
                        ),
                        i18n.text_args(
                            "provider-studio-auth-poll",
                            &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                        ),
                    ]
                })
                .unwrap_or_default(),
            None => Vec::new(),
        },
        Some(CredentialIssuer::GithubCopilot) => dialog
            .draft
            .credential_drafts
            .github_copilot
            .device
            .as_ref()
            .map(|device| {
                vec![
                    i18n.text_args(
                        "provider-studio-auth-copilot-ready",
                        &crate::fl_args!("code" => device.user_code.clone()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-verify",
                        &crate::fl_args!("url" => device.display_verification_url().to_owned()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-poll",
                        &crate::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::Gitlab) => dialog
            .draft
            .credential_drafts
            .gitlab
            .browser
            .as_ref()
            .map(|session| {
                vec![
                    ui_text::t(i18n, "provider-studio-auth-gitlab-ready"),
                    i18n.text_args(
                        "provider-studio-auth-authorize",
                        &crate::fl_args!("url" => session.display_authorize_url().to_owned()),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-redirect",
                        &crate::fl_args!(
                            "url" => dialog.draft.credential_drafts.gitlab.redirect_uri.clone()
                        ),
                    ),
                    i18n.text_args(
                        "provider-studio-auth-paste-callback",
                        &crate::fl_args!(
                            "state" => truncate_display_width(session.state.as_str(), 24)
                        ),
                    ),
                ]
            })
            .unwrap_or_default(),
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => Vec::new(),
    }
}

pub(in crate::app) fn provider_studio_auth_status(
    dialog: &ProviderStudioOverlay,
) -> ProviderStudioAuthStatus {
    if provider_studio_has_pending_auth_state(dialog) {
        return ProviderStudioAuthStatus::Pending;
    }
    let detail_fields = provider_studio_detail_fields(dialog);
    if detail_fields.is_empty() {
        return match dialog.draft.auth_kind {
            ProviderDraftAuthKind::Unset => ProviderStudioAuthStatus::Unset,
            ProviderDraftAuthKind::None => ProviderStudioAuthStatus::None,
            ProviderDraftAuthKind::ApiPending => ProviderStudioAuthStatus::SelectSubtype,
            ProviderDraftAuthKind::Credential(None) => ProviderStudioAuthStatus::SelectIssuer,
            ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::Credential(Some(_))
            | ProviderDraftAuthKind::BedrockSigv4 => ProviderStudioAuthStatus::Unset,
        };
    }
    if provider_studio_auth_is_configured(dialog) {
        ProviderStudioAuthStatus::Configured
    } else if provider_studio_has_any_auth_detail_value(&dialog.draft, &detail_fields) {
        ProviderStudioAuthStatus::Partial
    } else {
        ProviderStudioAuthStatus::Unset
    }
}

pub(in crate::app) fn provider_studio_detail_fields(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderStudioField> {
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending => Vec::new(),
        ProviderDraftAuthKind::Api => {
            let mut fields = Vec::new();
            if provider_studio_base_url_visible(dialog) {
                fields.push(ProviderStudioField::BaseUrl);
            }
            fields.extend([
                ProviderStudioField::ApiKeySource,
                ProviderStudioField::ApiKeyValue,
            ]);
            fields
        }
        ProviderDraftAuthKind::ClineApi => {
            vec![
                ProviderStudioField::ApiKeySource,
                ProviderStudioField::ApiKeyValue,
            ]
        }
        ProviderDraftAuthKind::Gitlab => vec![
            ProviderStudioField::InstanceUrl,
            ProviderStudioField::ApiKeySource,
            ProviderStudioField::ApiKeyValue,
        ],
        ProviderDraftAuthKind::Credential(issuer) => match issuer {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                let mut fields = Vec::new();
                if provider_studio_auth_login_kind(dialog)
                    == Some(ProviderDraftInteractiveLoginKind::Browser)
                {
                    fields.push(ProviderStudioField::RedirectUri);
                    fields.push(ProviderStudioField::CallbackUrl);
                }
                fields.extend([
                    ProviderStudioField::RefreshToken,
                    ProviderStudioField::AccessToken,
                    ProviderStudioField::ExpiresAtMs,
                    ProviderStudioField::AccountId,
                ]);
                fields
            }
            Some(CredentialIssuer::GithubCopilot) => vec![
                ProviderStudioField::EnterpriseDomain,
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
            ],
            Some(CredentialIssuer::Gitlab) => vec![
                ProviderStudioField::InstanceUrl,
                ProviderStudioField::RedirectUri,
                ProviderStudioField::CallbackUrl,
                ProviderStudioField::RefreshToken,
                ProviderStudioField::AccessToken,
                ProviderStudioField::ExpiresAtMs,
            ],
            Some(CredentialIssuer::GoogleAdc) => {
                if provider_studio_base_url_visible(dialog) {
                    vec![ProviderStudioField::BaseUrl]
                } else {
                    Vec::new()
                }
            }
            Some(CredentialIssuer::SapAiCore) => {
                let mut fields = Vec::new();
                if provider_studio_base_url_visible(dialog) {
                    fields.push(ProviderStudioField::BaseUrl);
                }
                fields.push(ProviderStudioField::ServiceKeyEnv);
                fields
            }
            None => Vec::new(),
        },
        ProviderDraftAuthKind::BedrockSigv4 => vec![
            ProviderStudioField::BaseUrl,
            ProviderStudioField::Region,
            ProviderStudioField::Profile,
            ProviderStudioField::AccessKeyId,
            ProviderStudioField::SecretAccessKey,
            ProviderStudioField::SessionToken,
        ],
    }
}

pub(in crate::app) fn provider_studio_has_any_auth_detail_value(
    draft: &ProviderConfigDraft,
    fields: &[ProviderStudioField],
) -> bool {
    fields
        .iter()
        .any(|field| !provider_studio_field_value(draft, *field).trim().is_empty())
}

pub(in crate::app) fn provider_studio_auth_is_configured(dialog: &ProviderStudioOverlay) -> bool {
    match dialog.draft.auth_kind {
        ProviderDraftAuthKind::Unset => false,
        ProviderDraftAuthKind::None => true,
        ProviderDraftAuthKind::ApiPending => false,
        ProviderDraftAuthKind::Api => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::ClineApi => {
            dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::Gitlab => {
            !dialog.draft.auth.instance_url.trim().is_empty()
                && dialog.draft.auth.secret_source_kind != ProviderDraftSecretSourceKind::Unset
                && !dialog.draft.auth.secret_source_value.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot))
        | ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            dialog.draft.active_tokens().is_some_and(|tokens| {
                !tokens.refresh_token.trim().is_empty() || !tokens.access_token.trim().is_empty()
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)) => {
            !dialog.draft.auth.base_url.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore)) => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && !dialog.draft.auth.service_key_env.trim().is_empty()
        }
        ProviderDraftAuthKind::Credential(None) => false,
        ProviderDraftAuthKind::BedrockSigv4 => {
            !dialog.draft.auth.base_url.trim().is_empty()
                && !dialog.draft.auth.region.trim().is_empty()
                && ((!dialog.draft.auth.profile.trim().is_empty())
                    || (!dialog.draft.auth.access_key_id.trim().is_empty()
                        && !dialog.draft.auth.secret_access_key.trim().is_empty()))
        }
    }
}

pub(in crate::app) fn provider_studio_auth_status_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    ui_text::t(
        i18n,
        match provider_studio_auth_status(dialog) {
            ProviderStudioAuthStatus::Pending => "provider-studio-auth-status-pending",
            ProviderStudioAuthStatus::Unset => "provider-studio-auth-status-unset",
            ProviderStudioAuthStatus::None => "provider-studio-auth-status-none",
            ProviderStudioAuthStatus::SelectSubtype => "provider-studio-auth-status-select-subtype",
            ProviderStudioAuthStatus::SelectIssuer => "provider-studio-auth-status-select-issuer",
            ProviderStudioAuthStatus::Configured => "provider-studio-auth-status-configured",
            ProviderStudioAuthStatus::Partial => "provider-studio-auth-status-partial",
        },
    )
}

pub(in crate::app) fn provider_studio_visible_fields(
    dialog: &ProviderStudioOverlay,
) -> Vec<ProviderStudioField> {
    let mut fields = vec![
        ProviderStudioField::ProviderId,
        ProviderStudioField::AuthMode,
    ];
    if matches!(
        dialog.draft.auth_kind,
        ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::BedrockSigv4
            | ProviderDraftAuthKind::Credential(_)
    ) {
        fields.push(ProviderStudioField::AuthSubtype);
    }
    if !provider_studio_available_login_kinds(dialog).is_empty() {
        fields.push(ProviderStudioField::AuthLoginMethod);
    }
    if !provider_studio_detail_fields(dialog).is_empty() {
        if dialog.draft.supports_interactive_auth() {
            fields.push(ProviderStudioField::StartAuthAction);
            fields.push(ProviderStudioField::ContinueAuthAction);
        }
        fields.push(ProviderStudioField::EditAuthDetailsAction);
    }
    if !matches!(
        dialog.draft.auth_kind,
        ProviderDraftAuthKind::Unset
            | ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Credential(None)
    ) {
        fields.extend([
            ProviderStudioField::DefaultAdapter,
            ProviderStudioField::DefaultModel,
        ]);
    }
    fields
}
