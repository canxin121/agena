//! Provider Studio entry points, migrated from
//! `agena-tui-backend/src/backend_provider/selection.rs` and
//! `settings.rs`. These are thin `impl Application` delegations to the
//! migrated free functions in `provider_studio::save`.

use crate::provider_studio::save;
use crate::provider_studio::{
    ProviderBrowserAuthSessionDraft, ProviderConfigDraft, ProviderDeviceAuthSessionDraft,
    ProviderDraftAuthActionResult, ProviderDraftAuthError, ProviderDraftAuthField,
    ProviderDraftAuthKind, ProviderDraftAuthMessage, ProviderDraftInteractiveLoginKind,
    ProviderOAuthTokensDraft, ProviderStudioSaveError, ProviderStudioSaveResult,
};
use crate::{Application, ApplicationError};
use agena_provider::CredentialIssuer;
use agena_runtime::{RuntimeDraftAuthKind, RuntimeDraftAuthToken, parse_oauth_callback_url};

impl Application {
    pub fn provider_config_draft(
        &self,
        provider_id: Option<&str>,
    ) -> Result<ProviderConfigDraft, ApplicationError> {
        save::provider_config_draft(self, provider_id)
    }

    pub async fn save_provider_draft(
        &self,
        draft: ProviderConfigDraft,
        adapter_model_lists: &[agena_api::resource::ProviderAdapterModelsResource],
        selected_adapter_ids: &[String],
        selected_model_keys: &std::collections::BTreeSet<String>,
        model_config_values: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::save_provider_draft(
            self,
            draft,
            adapter_model_lists,
            selected_adapter_ids,
            selected_model_keys,
            model_config_values,
        )
        .await
    }

    pub async fn save_provider_adapter_matches(
        &self,
        draft: ProviderConfigDraft,
        adapter_models: agena_api::resource::ProviderAdapterModelsResource,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::save_provider_adapter_matches(self, draft, adapter_models).await
    }

    pub async fn list_draft_provider_adapter_models(
        &self,
        draft: &ProviderConfigDraft,
        adapter_ids: &[String],
    ) -> Result<agena_api::resource::ProviderAdapterModelsResponse, ApplicationError> {
        save::list_draft_provider_adapter_models(self, draft, adapter_ids).await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Result<agena_api::resource::ProviderAdapterModelsResponse, ApplicationError> {
        save::list_saved_provider_adapter_models(self, provider_id, adapter_ids).await
    }

    pub async fn save_provider_model_value(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        model_value: serde_json::Value,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::save_provider_model_value(self, draft, adapter_id, model_id, model_value).await
    }

    pub async fn delete_provider_model(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::delete_provider_model(self, draft, adapter_id, model_id).await
    }

    pub async fn delete_provider(
        &self,
        provider_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::delete_provider(self, provider_id).await
    }

    pub async fn delete_provider_adapter(
        &self,
        draft: ProviderConfigDraft,
        adapter_id: &str,
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::delete_provider_adapter(self, draft, adapter_id).await
    }

    pub fn provider_model_draft_value(
        &self,
        draft: &ProviderConfigDraft,
        adapter_id: &str,
        model_id: &str,
        provider_model: Option<&agena_api::resource::ProviderModelResource>,
    ) -> Result<serde_json::Value, ApplicationError> {
        save::provider_model_draft_value(self, draft, adapter_id, model_id, provider_model)
    }

    pub async fn set_provider_default_selection(
        &self,
        provider_id: &str,
        selection: serde_json::Value,
    ) -> Result<agena_runtime::ConfigSettingsEditResponse, ApplicationError> {
        save::set_provider_default_selection(self, provider_id, selection).await
    }

    pub async fn start_provider_draft_auth(
        &self,
        mut draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        draft.normalize_shape();
        let auth = self.runtime_draft_authentication().as_ref();
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
                        draft.credential_drafts.openai_chatgpt.clear_pending();
                        draft.credential_drafts.openai_chatgpt.browser =
                            Some(ProviderBrowserAuthSessionDraft {
                                authorize_url: start.authorize_url.clone(),
                                display_url: None,
                                state: start.state,
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
                        draft.credential_drafts.openai_chatgpt.clear_pending();
                        draft.credential_drafts.openai_chatgpt.device =
                            Some(ProviderDeviceAuthSessionDraft {
                                verification_url: start.verification_url.clone(),
                                display_url: None,
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
                draft.credential_drafts.github_copilot.device =
                    Some(ProviderDeviceAuthSessionDraft {
                        verification_url: start.verification_url.clone(),
                        display_url: None,
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
                draft.credential_drafts.gitlab.callback_url.clear();
                draft.credential_drafts.gitlab.browser = Some(ProviderBrowserAuthSessionDraft {
                    authorize_url: start.authorize_url.clone(),
                    display_url: None,
                    state: start.state,
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

    pub async fn continue_provider_draft_auth(
        &self,
        mut draft: ProviderConfigDraft,
    ) -> std::result::Result<ProviderDraftAuthActionResult, ProviderDraftAuthError> {
        draft.normalize_shape();
        let auth = self.runtime_draft_authentication().as_ref();
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
                update_oauth_tokens_from_response(
                    &mut draft.credential_drafts.gitlab.tokens,
                    &token,
                );
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
}

fn optional_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn required_provider_auth_field(
    value: &str,
    field: ProviderDraftAuthField,
) -> std::result::Result<&str, ProviderDraftAuthError> {
    optional_non_empty(value).ok_or(ProviderDraftAuthError::RequiredField(field))
}

fn update_oauth_tokens_from_response(
    tokens: &mut ProviderOAuthTokensDraft,
    response: &RuntimeDraftAuthToken,
) {
    tokens.refresh_token = response.refresh_token.clone();
    tokens.access_token = response.access_token.clone();
    tokens.expires_at_ms = response.expires_at_ms.to_string();
}
