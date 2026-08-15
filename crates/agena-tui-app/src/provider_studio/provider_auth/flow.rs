use super::super::provider_selection::*;

use super::super::{
    CredentialIssuer, I18n, ProviderConfigDraft, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind, ProviderStudioOverlay,
    join_inline_segments,
};
use super::{
    ProviderStudioSummaryLabel, provider_studio_action_with_summary,
    provider_studio_auth_is_configured, provider_studio_auth_login_kind,
    provider_studio_auth_status_summary, provider_studio_browser_continue_summary,
    provider_studio_labeled_summary, provider_studio_missing_continue_auth_field,
    provider_studio_missing_start_auth_field, provider_studio_required_field_summary,
};
use crate::ui_text;
use agena_application::provider_studio::{
    ProviderBrowserAuthSessionDraft, ProviderDeviceAuthSessionDraft, ProviderDraftAuthActionResult,
    ProviderDraftAuthError, ProviderDraftAuthField, ProviderDraftAuthMessage,
    ProviderOAuthTokensDraft,
};
use agena_runtime::{
    RuntimeDraftAuthKind, RuntimeDraftAuthToken, RuntimeDraftAuthenticationService,
    parse_oauth_callback_url,
};
use agena_tui::link::shorten_url_for_display;

pub(crate) fn provider_studio_start_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => {
            if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
                return provider_studio_required_field_summary(i18n, field);
            }
            match provider_studio_auth_login_kind(dialog) {
                Some(ProviderDraftInteractiveLoginKind::Browser) => {
                    if let Some(session) = dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .browser
                        .as_ref()
                    {
                        return provider_studio_action_with_summary(
                            i18n,
                            "provider-studio-summary-open-authorize",
                            Some(session.display_authorize_url().to_owned()),
                        );
                    }
                    provider_studio_action_with_summary(
                        i18n,
                        if provider_studio_auth_is_configured(dialog) {
                            "provider-studio-summary-restart-browser"
                        } else {
                            "provider-studio-summary-start-browser"
                        },
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            dialog
                                .draft
                                .credential_drafts
                                .openai_chatgpt
                                .redirect_uri
                                .as_str(),
                            36,
                        ),
                    )
                }
                Some(ProviderDraftInteractiveLoginKind::Device) => {
                    if let Some(device) = dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .device
                        .as_ref()
                    {
                        let mut parts = vec![device.display_verification_url().to_owned()];
                        if let Some(code) = provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Code,
                            device.user_code.as_str(),
                            18,
                        ) {
                            parts.push(code);
                        }
                        return provider_studio_action_with_summary(
                            i18n,
                            "provider-studio-summary-open-verify",
                            Some(join_inline_segments(parts)),
                        );
                    }
                    provider_studio_action_with_summary(
                        i18n,
                        if provider_studio_auth_is_configured(dialog) {
                            "provider-studio-summary-restart-device"
                        } else {
                            "provider-studio-summary-start-device"
                        },
                        None,
                    )
                }
                None => provider_studio_auth_status_summary(i18n, dialog),
            }
        }
        Some(CredentialIssuer::GithubCopilot) => {
            if let Some(device) = dialog
                .draft
                .credential_drafts
                .github_copilot
                .device
                .as_ref()
            {
                let mut parts = vec![device.display_verification_url().to_owned()];
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-open-verify",
                    Some(join_inline_segments(parts)),
                );
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-device"
                } else {
                    "provider-studio-summary-start-device"
                },
                provider_studio_summary_value(
                    dialog
                        .draft
                        .credential_drafts
                        .github_copilot
                        .enterprise_domain
                        .as_str(),
                    28,
                ),
            )
        }
        Some(CredentialIssuer::Gitlab) => {
            if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
                return provider_studio_required_field_summary(i18n, field);
            }
            if let Some(session) = dialog.draft.credential_drafts.gitlab.browser.as_ref() {
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-open-authorize",
                    Some(session.display_authorize_url().to_owned()),
                );
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-browser"
                } else {
                    "provider-studio-summary-start-browser"
                },
                provider_studio_summary_value(dialog.draft.auth.instance_url.as_str(), 40).or_else(
                    || {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            dialog.draft.credential_drafts.gitlab.redirect_uri.as_str(),
                            36,
                        )
                    },
                ),
            )
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
            provider_studio_auth_status_summary(i18n, dialog)
        }
    }
}

pub(crate) fn provider_studio_continue_auth_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    match dialog.draft.auth_kind.credential_issuer() {
        Some(CredentialIssuer::OpenaiChatgpt) => match provider_studio_auth_login_kind(dialog) {
            Some(ProviderDraftInteractiveLoginKind::Browser) => {
                if let Some(callback) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Callback,
                    dialog
                        .draft
                        .credential_drafts
                        .openai_chatgpt
                        .callback_url
                        .as_str(),
                    44,
                ) {
                    return provider_studio_action_with_summary(
                        i18n,
                        "provider-studio-summary-finish-callback",
                        Some(callback),
                    );
                }
                if let Some(session) = dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .browser
                    .as_ref()
                {
                    return provider_studio_browser_continue_summary(
                        i18n,
                        "provider-studio-summary-paste-callback",
                        session.state.as_str(),
                    );
                }
                provider_studio_missing_continue_auth_field(dialog)
                    .map(|field| provider_studio_required_field_summary(i18n, field))
                    .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-start-auth-first"))
            }
            Some(ProviderDraftInteractiveLoginKind::Device) => {
                if let Some(device) = dialog
                    .draft
                    .credential_drafts
                    .openai_chatgpt
                    .device
                    .as_ref()
                {
                    let mut parts = vec![
                        ui_text::t(i18n, "provider-studio-summary-poll-now"),
                        i18n.text_args(
                            "provider-studio-summary-poll-every",
                            &agena_tui::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                        ),
                    ];
                    if let Some(code) = provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Code,
                        device.user_code.as_str(),
                        18,
                    ) {
                        parts.push(code);
                    }
                    return join_inline_segments(parts);
                }
                provider_studio_action_with_summary(
                    i18n,
                    if provider_studio_auth_is_configured(dialog) {
                        "provider-studio-summary-restart-device"
                    } else {
                        "provider-studio-summary-start-device"
                    },
                    None,
                )
            }
            None => provider_studio_auth_status_summary(i18n, dialog),
        },
        Some(CredentialIssuer::GithubCopilot) => {
            if let Some(device) = dialog
                .draft
                .credential_drafts
                .github_copilot
                .device
                .as_ref()
            {
                let mut parts = vec![
                    ui_text::t(i18n, "provider-studio-summary-poll-now"),
                    i18n.text_args(
                        "provider-studio-summary-poll-every",
                        &agena_tui::fl_args!("seconds" => device.interval_seconds.max(1) as i64),
                    ),
                ];
                if let Some(code) = provider_studio_labeled_summary(
                    i18n,
                    ProviderStudioSummaryLabel::Code,
                    device.user_code.as_str(),
                    18,
                ) {
                    parts.push(code);
                }
                return join_inline_segments(parts);
            }
            provider_studio_action_with_summary(
                i18n,
                if provider_studio_auth_is_configured(dialog) {
                    "provider-studio-summary-restart-device"
                } else {
                    "provider-studio-summary-start-device"
                },
                None,
            )
        }
        Some(CredentialIssuer::Gitlab) => {
            if let Some(callback) = provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Callback,
                dialog.draft.credential_drafts.gitlab.callback_url.as_str(),
                44,
            ) {
                return provider_studio_action_with_summary(
                    i18n,
                    "provider-studio-summary-finish-callback",
                    Some(callback),
                );
            }
            if let Some(session) = dialog.draft.credential_drafts.gitlab.browser.as_ref() {
                return provider_studio_browser_continue_summary(
                    i18n,
                    "provider-studio-summary-paste-callback",
                    session.state.as_str(),
                );
            }
            provider_studio_missing_continue_auth_field(dialog)
                .map(|field| provider_studio_required_field_summary(i18n, field))
                .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-start-auth-first"))
        }
        Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
            provider_studio_auth_status_summary(i18n, dialog)
        }
    }
}

pub(crate) fn provider_studio_auth_details_hint(
    i18n: &I18n,
    draft: &ProviderConfigDraft,
) -> Option<String> {
    match draft.auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Credential(None) => None,
        ProviderDraftAuthKind::Api => provider_studio_secret_source_hint(i18n, draft)
            .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48)),
        ProviderDraftAuthKind::ClineApi => provider_studio_secret_source_hint(i18n, draft),
        ProviderDraftAuthKind::Gitlab => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48)
                .or_else(|| provider_studio_secret_source_hint(i18n, draft))
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Account,
                draft.credential_drafts.openai_chatgpt.account_id.as_str(),
                24,
            )
            .or_else(|| {
                (draft.interactive_login_kind() == Some(ProviderDraftInteractiveLoginKind::Browser))
                    .then(|| {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Callback,
                            draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                            36,
                        )
                    })
                    .flatten()
            })
            .or_else(|| {
                (draft.interactive_login_kind() == Some(ProviderDraftInteractiveLoginKind::Browser))
                    .then(|| {
                        provider_studio_labeled_summary(
                            i18n,
                            ProviderStudioSummaryLabel::Redirect,
                            draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                            36,
                        )
                    })
                    .flatten()
            })
            .or_else(|| {
                draft
                    .tokens_present()
                    .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            provider_studio_summary_value(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
                32,
            )
            .or_else(|| {
                draft
                    .tokens_present()
                    .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            provider_studio_summary_value(draft.auth.instance_url.as_str(), 48)
                .or_else(|| {
                    provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Callback,
                        draft.credential_drafts.gitlab.callback_url.as_str(),
                        36,
                    )
                })
                .or_else(|| {
                    provider_studio_labeled_summary(
                        i18n,
                        ProviderStudioSummaryLabel::Redirect,
                        draft.credential_drafts.gitlab.redirect_uri.as_str(),
                        36,
                    )
                })
                .or_else(|| {
                    draft
                        .tokens_present()
                        .then(|| ui_text::t(i18n, "provider-studio-summary-tokens-set"))
                })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)) => {
            provider_studio_summary_value(draft.auth.base_url.as_str(), 48)
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore)) => {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Env,
                draft.auth.service_key_env.as_str(),
                28,
            )
            .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        }
        ProviderDraftAuthKind::BedrockSigv4 => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Profile,
            draft.auth.profile.as_str(),
            24,
        )
        .or_else(|| {
            provider_studio_labeled_summary(
                i18n,
                ProviderStudioSummaryLabel::Region,
                draft.auth.region.as_str(),
                24,
            )
        })
        .or_else(|| provider_studio_summary_value(draft.auth.base_url.as_str(), 48))
        .or_else(|| {
            (!draft.auth.access_key_id.trim().is_empty()
                && !draft.auth.secret_access_key.trim().is_empty())
            .then(|| ui_text::t(i18n, "provider-studio-summary-keys-set"))
        }),
    }
}

pub(crate) fn provider_studio_secret_source_hint(
    i18n: &I18n,
    draft: &ProviderConfigDraft,
) -> Option<String> {
    match draft.auth.secret_source_kind {
        ProviderDraftSecretSourceKind::Unset => None,
        ProviderDraftSecretSourceKind::Inline => {
            (!draft.auth.secret_source_value.trim().is_empty())
                .then(|| ui_text::t(i18n, "provider-studio-summary-keys-set"))
        }
        ProviderDraftSecretSourceKind::Env => provider_studio_labeled_summary(
            i18n,
            ProviderStudioSummaryLabel::Env,
            draft.auth.secret_source_value.as_str(),
            28,
        ),
    }
}

pub(crate) fn provider_studio_auth_details_summary(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> String {
    provider_studio_auth_details_hint(i18n, &dialog.draft)
        .unwrap_or_else(|| ui_text::t(i18n, "provider-studio-summary-review-fields"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactive draft authentication flow.
//
// `start_provider_draft_auth` / `continue_provider_draft_auth` are NOT part of
// the down-moved Application surface (R7 brief §2 keeps interactive auth in the
// TUI), so these adapters drive the runtime draft authentication service
// directly from the in-process `Application`.

fn optional_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn update_oauth_tokens_from_response(
    tokens: &mut ProviderOAuthTokensDraft,
    response: &RuntimeDraftAuthToken,
) {
    tokens.refresh_token = response.refresh_token.clone();
    tokens.access_token = response.access_token.clone();
    tokens.expires_at_ms = response.expires_at_ms.to_string();
}

fn required_provider_auth_field(
    value: &str,
    field: ProviderDraftAuthField,
) -> std::result::Result<&str, ProviderDraftAuthError> {
    optional_non_empty(value).ok_or(ProviderDraftAuthError::RequiredField(field))
}

pub(crate) async fn start_provider_draft_auth(
    application: &crate::TuiBackend,
    mut draft: ProviderConfigDraft,
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
    let application = application
        .embedded_application()
        .map_err(ProviderDraftAuthError::other)?;
    let auth: &dyn RuntimeDraftAuthenticationService =
        application.runtime_draft_authentication().as_ref();
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            match draft.credential_drafts.openai_chatgpt.login_kind {
                ProviderDraftInteractiveLoginKind::Browser => {
                    let redirect_uri = required_provider_auth_field(
                        draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                        ProviderDraftAuthField::RedirectUri,
                    )?;
                    let start = auth
                        .start_draft_auth_browser(
                            RuntimeDraftAuthKind::OpenaiChatgpt,
                            None,
                            redirect_uri.to_owned(),
                        )
                        .map_err(ProviderDraftAuthError::other)?;
                    let display_url = shorten_url_for_display(start.authorize_url.as_str()).await;
                    draft.credential_drafts.openai_chatgpt.clear_pending();
                    draft.credential_drafts.openai_chatgpt.browser =
                        Some(ProviderBrowserAuthSessionDraft {
                            authorize_url: start.authorize_url.clone(),
                            display_url,
                            state: start.state.clone(),
                            pkce_verifier: start.pkce_verifier,
                        });
                    Ok(ProviderDraftAuthActionResult {
                        draft,
                        message: ProviderDraftAuthMessage::OpenaiBrowserStarted,
                        clipboard_text: Some(start.authorize_url),
                    })
                }
                ProviderDraftInteractiveLoginKind::Device => {
                    let start = auth
                        .start_draft_auth_device(RuntimeDraftAuthKind::OpenaiChatgpt, None)
                        .await
                        .map_err(ProviderDraftAuthError::other)?;
                    let display_url =
                        shorten_url_for_display(start.verification_url.as_str()).await;
                    draft.credential_drafts.openai_chatgpt.clear_pending();
                    draft.credential_drafts.openai_chatgpt.device =
                        Some(ProviderDeviceAuthSessionDraft {
                            verification_url: start.verification_url.clone(),
                            display_url,
                            user_code: start.user_code.clone(),
                            device_code: start.device_code,
                            interval_seconds: start.interval_seconds,
                        });
                    Ok(ProviderDraftAuthActionResult {
                        draft,
                        message: ProviderDraftAuthMessage::OpenaiDeviceStarted {
                            user_code: start.user_code,
                        },
                        clipboard_text: Some(start.verification_url),
                    })
                }
            }
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let domain = optional_non_empty(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
            )
            .unwrap_or("github.com");
            let start = auth
                .start_draft_auth_device(
                    RuntimeDraftAuthKind::GithubCopilot,
                    Some(domain.to_owned()),
                )
                .await
                .map_err(ProviderDraftAuthError::other)?;
            let display_url = shorten_url_for_display(start.verification_url.as_str()).await;
            draft.credential_drafts.github_copilot.device = Some(ProviderDeviceAuthSessionDraft {
                verification_url: start.verification_url.clone(),
                display_url,
                user_code: start.user_code.clone(),
                device_code: start.device_code,
                interval_seconds: start.interval_seconds,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::CopilotDeviceStarted {
                    user_code: start.user_code,
                },
                clipboard_text: Some(start.verification_url),
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let instance_url = required_provider_auth_field(
                draft.auth.instance_url.as_str(),
                ProviderDraftAuthField::InstanceUrl,
            )?;
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let start = auth
                .start_draft_auth_browser(
                    RuntimeDraftAuthKind::Gitlab,
                    Some(instance_url.to_owned()),
                    redirect_uri.to_owned(),
                )
                .map_err(ProviderDraftAuthError::other)?;
            let display_url = shorten_url_for_display(start.authorize_url.as_str()).await;
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = Some(ProviderBrowserAuthSessionDraft {
                authorize_url: start.authorize_url.clone(),
                display_url,
                state: start.state.clone(),
                pkce_verifier: start.pkce_verifier,
            });
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::GitlabBrowserStarted,
                clipboard_text: Some(start.authorize_url),
            })
        }
        _ => Err(ProviderDraftAuthError::UnsupportedInteractiveLogin),
    }
}

pub(crate) async fn continue_provider_draft_auth(
    application: &crate::TuiBackend,
    mut draft: ProviderConfigDraft,
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
    let application = application
        .embedded_application()
        .map_err(ProviderDraftAuthError::other)?;
    let auth: &dyn RuntimeDraftAuthenticationService =
        application.runtime_draft_authentication().as_ref();
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            match draft.credential_drafts.openai_chatgpt.login_kind {
                ProviderDraftInteractiveLoginKind::Browser => {
                    let session = draft
                        .credential_drafts
                        .openai_chatgpt
                        .browser
                        .clone()
                        .ok_or(ProviderDraftAuthError::StartBrowserAuthFirst)?;
                    let redirect_uri = required_provider_auth_field(
                        draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                        ProviderDraftAuthField::RedirectUri,
                    )?;
                    let callback_url = required_provider_auth_field(
                        draft.credential_drafts.openai_chatgpt.callback_url.as_str(),
                        ProviderDraftAuthField::CallbackUrl,
                    )?;
                    let callback =
                        parse_oauth_callback_url(callback_url, Some(session.state.as_str()))
                            .map_err(ProviderDraftAuthError::other)?;
                    let token = auth
                        .finish_draft_auth_browser(
                            RuntimeDraftAuthKind::OpenaiChatgpt,
                            None,
                            callback.code,
                            session.pkce_verifier,
                            redirect_uri.to_owned(),
                        )
                        .await
                        .map_err(ProviderDraftAuthError::other)?;
                    update_oauth_tokens_from_response(
                        &mut draft.credential_drafts.openai_chatgpt.tokens,
                        &token,
                    );
                    draft.credential_drafts.openai_chatgpt.account_id =
                        token.account_id.unwrap_or_default();
                    draft.credential_drafts.openai_chatgpt.clear_pending();
                    Ok(ProviderDraftAuthActionResult {
                        draft,
                        message: ProviderDraftAuthMessage::OpenaiCredentialCaptured,
                        clipboard_text: None,
                    })
                }
                ProviderDraftInteractiveLoginKind::Device => {
                    let session = draft
                        .credential_drafts
                        .openai_chatgpt
                        .device
                        .clone()
                        .ok_or(ProviderDraftAuthError::StartDeviceAuthFirst)?;
                    let Some(token) = auth
                        .poll_draft_auth_device(
                            RuntimeDraftAuthKind::OpenaiChatgpt,
                            None,
                            session.device_code,
                            Some(session.user_code),
                        )
                        .await
                        .map_err(ProviderDraftAuthError::other)?
                    else {
                        return Ok(ProviderDraftAuthActionResult {
                            draft,
                            message: ProviderDraftAuthMessage::OpenaiPending,
                            clipboard_text: None,
                        });
                    };
                    update_oauth_tokens_from_response(
                        &mut draft.credential_drafts.openai_chatgpt.tokens,
                        &token,
                    );
                    draft.credential_drafts.openai_chatgpt.account_id =
                        token.account_id.unwrap_or_default();
                    draft.credential_drafts.openai_chatgpt.clear_pending();
                    Ok(ProviderDraftAuthActionResult {
                        draft,
                        message: ProviderDraftAuthMessage::OpenaiCredentialCaptured,
                        clipboard_text: None,
                    })
                }
            }
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
            let session = draft
                .credential_drafts
                .github_copilot
                .device
                .clone()
                .ok_or(ProviderDraftAuthError::StartDeviceAuthFirst)?;
            let domain = optional_non_empty(
                draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain
                    .as_str(),
            )
            .unwrap_or("github.com");
            let Some(token) = auth
                .poll_draft_auth_device(
                    RuntimeDraftAuthKind::GithubCopilot,
                    Some(domain.to_owned()),
                    session.device_code,
                    None,
                )
                .await
                .map_err(ProviderDraftAuthError::other)?
            else {
                return Ok(ProviderDraftAuthActionResult {
                    draft,
                    message: ProviderDraftAuthMessage::CopilotPending,
                    clipboard_text: None,
                });
            };
            update_oauth_tokens_from_response(
                &mut draft.credential_drafts.github_copilot.tokens,
                &token,
            );
            draft.credential_drafts.github_copilot.device = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::CopilotCredentialCaptured,
                clipboard_text: None,
            })
        }
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
            let session = draft
                .credential_drafts
                .gitlab
                .browser
                .clone()
                .ok_or(ProviderDraftAuthError::StartBrowserAuthFirst)?;
            let instance_url = required_provider_auth_field(
                draft.auth.instance_url.as_str(),
                ProviderDraftAuthField::InstanceUrl,
            )?;
            let redirect_uri = required_provider_auth_field(
                draft.credential_drafts.gitlab.redirect_uri.as_str(),
                ProviderDraftAuthField::RedirectUri,
            )?;
            let callback_url = required_provider_auth_field(
                draft.credential_drafts.gitlab.callback_url.as_str(),
                ProviderDraftAuthField::CallbackUrl,
            )?;
            let callback = parse_oauth_callback_url(callback_url, Some(session.state.as_str()))
                .map_err(ProviderDraftAuthError::other)?;
            let token = auth
                .finish_draft_auth_browser(
                    RuntimeDraftAuthKind::Gitlab,
                    Some(instance_url.to_owned()),
                    callback.code,
                    session.pkce_verifier,
                    redirect_uri.to_owned(),
                )
                .await
                .map_err(ProviderDraftAuthError::other)?;
            update_oauth_tokens_from_response(&mut draft.credential_drafts.gitlab.tokens, &token);
            draft.credential_drafts.gitlab.callback_url.clear();
            draft.credential_drafts.gitlab.browser = None;
            Ok(ProviderDraftAuthActionResult {
                draft,
                message: ProviderDraftAuthMessage::GitlabCredentialCaptured,
                clipboard_text: None,
            })
        }
        _ => Err(ProviderDraftAuthError::UnsupportedInteractiveLogin),
    }
}
