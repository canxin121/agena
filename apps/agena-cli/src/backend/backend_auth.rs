use anyhow::anyhow;

pub(super) fn required_trimmed<'a>(value: &'a str, field: &str) -> Result<&'a str> {
    optional_non_empty(value).ok_or_else(|| anyhow!("{field} is required"))
}

pub(super) fn provider_credential_drafts(
    issuer: CredentialIssuer,
    credential: Option<&AuthData>,
) -> ProviderCredentialDraftBundle {
    let Some(AuthData::OAuth {
        refresh,
        access,
        expires_at_ms,
        account_id,
        enterprise_url,
        ..
    }) = credential
    else {
        return ProviderCredentialDraftBundle::default();
    };

    let tokens = ProviderOAuthTokensDraft {
        refresh_token: refresh.clone(),
        access_token: access.clone(),
        expires_at_ms: (*expires_at_ms).to_string(),
    };
    match issuer {
        CredentialIssuer::OpenaiChatgpt => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft {
                tokens,
                account_id: account_id.clone().unwrap_or_default(),
                ..OpenAiChatgptCredentialDraft::default()
            },
            github_copilot: GithubCopilotCredentialDraft::default(),
            gitlab: GitlabCredentialDraft::default(),
        },
        CredentialIssuer::GithubCopilot => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft::default(),
            github_copilot: GithubCopilotCredentialDraft {
                enterprise_domain: enterprise_url.clone().unwrap_or_default(),
                tokens,
                device: None,
            },
            gitlab: GitlabCredentialDraft::default(),
        },
        CredentialIssuer::Gitlab => ProviderCredentialDraftBundle {
            openai_chatgpt: OpenAiChatgptCredentialDraft::default(),
            github_copilot: GithubCopilotCredentialDraft::default(),
            gitlab: GitlabCredentialDraft {
                tokens,
                ..GitlabCredentialDraft::default()
            },
        },
        CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore => {
            ProviderCredentialDraftBundle::default()
        }
    }
}

pub(super) fn update_oauth_tokens_from_response(
    tokens: &mut ProviderOAuthTokensDraft,
    response: &agena::provider::auth::OAuthTokenResponse,
) {
    tokens.refresh_token = response.refresh.clone();
    tokens.access_token = response.access.clone();
    tokens.expires_at_ms = response.expires_at_ms.to_string();
}

pub(super) async fn start_provider_draft_auth(
    mut draft: ProviderConfigDraft,
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
    draft.normalize_shape();
    match draft.auth_kind {
        ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
            match draft.credential_drafts.openai_chatgpt.login_kind {
                ProviderDraftInteractiveLoginKind::Browser => {
                    let redirect_uri = required_provider_auth_field(
                        draft.credential_drafts.openai_chatgpt.redirect_uri.as_str(),
                        ProviderDraftAuthField::RedirectUri,
                    )?;
                    let start = start_openai_browser_oauth(redirect_uri)
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
                    let start = start_openai_headless_device_code()
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
            let start = start_copilot_device_code(domain)
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
            let start = start_gitlab_oauth(instance_url, redirect_uri)
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

pub(super) fn required_provider_auth_field(
    value: &str,
    field: ProviderDraftAuthField,
) -> std::result::Result<&str, ProviderDraftAuthError> {
    optional_non_empty(value).ok_or(ProviderDraftAuthError::RequiredField(field))
}

pub(super) async fn continue_provider_draft_auth(
    mut draft: ProviderConfigDraft,
) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
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
                    let token = exchange_openai_oauth_code(
                        callback.code.as_str(),
                        session.pkce_verifier.as_str(),
                        redirect_uri,
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
                    let Some(token) = poll_openai_headless_device_code(
                        session.device_code.as_str(),
                        session.user_code.as_str(),
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
            let Some(token) = poll_copilot_device_code(domain, session.device_code.as_str())
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
            let token = exchange_gitlab_oauth_code(
                instance_url,
                callback.code.as_str(),
                session.pkce_verifier.as_str(),
                redirect_uri,
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
use crate::backend::Result;
use crate::backend::{
    AuthData, CredentialIssuer, GithubCopilotCredentialDraft, GitlabCredentialDraft,
    OpenAiChatgptCredentialDraft, ProviderBrowserAuthSessionDraft, ProviderConfigDraft,
    ProviderCredentialDraftBundle, ProviderDeviceAuthSessionDraft, ProviderDraftAuthActionResult,
    ProviderDraftAuthError, ProviderDraftAuthField, ProviderDraftAuthKind,
    ProviderDraftAuthMessage, ProviderDraftInteractiveLoginKind, ProviderOAuthTokensDraft,
    exchange_gitlab_oauth_code, exchange_openai_oauth_code, optional_non_empty,
    parse_oauth_callback_url, poll_copilot_device_code, poll_openai_headless_device_code,
    shorten_url_for_display, start_copilot_device_code, start_gitlab_oauth,
    start_openai_browser_oauth, start_openai_headless_device_code,
};
