impl ProviderConfigDraft {
    fn validate_for_adapters(
        &self,
        adapter_ids: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let default_adapter = required_trimmed(self.default_adapter.as_str(), "defaults.adapter")?;
        if !self.auth_kind.supports_adapter(default_adapter) {
            return Err(anyhow!(
                "auth {} does not support defaults.adapter `{default_adapter}`; expected one of {}",
                self.auth_kind.label(),
                supported_provider_draft_adapter_list(&self.auth_kind),
            ));
        }

        let incompatible = adapter_ids
            .iter()
            .filter(|adapter_id| !self.auth_kind.supports_adapter(adapter_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !incompatible.is_empty() {
            return Err(anyhow!(
                "auth {} does not support adapter(s): {}; expected one of {}",
                self.auth_kind.label(),
                incompatible.join(", "),
                supported_provider_draft_adapter_list(&self.auth_kind),
            ));
        }

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(anyhow!("provider auth_mode is required"));
            }
            ProviderDraftAuthKind::None => {}
            ProviderDraftAuthKind::ApiPending => {
                return Err(anyhow!("api auth requires auth_subtype"));
            }
            ProviderDraftAuthKind::Api => {
                let requires_base_url = adapter_ids.iter().any(|adapter_id| {
                    self.auth_kind
                        .adapter_rule(adapter_id.as_str())
                        .map(|rule| rule.requires_base_url)
                        .unwrap_or(false)
                });
                if requires_base_url && optional_non_empty(self.auth.base_url.as_str()).is_none() {
                    return Err(anyhow!(
                        "api auth requires base_url when using openai, anthropic, or gemini adapters"
                    ));
                }
            }
            ProviderDraftAuthKind::ClineApi => {}
            ProviderDraftAuthKind::Gitlab => {
                if self.secret_source_overlay().is_none() {
                    return Err(anyhow!("gitlab_api auth requires an api key source"));
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                return Err(anyhow!("credential auth requires credential_issuer"));
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                if issuer.uses_http_endpoint()
                    && optional_non_empty(self.auth.base_url.as_str()).is_none()
                {
                    return Err(anyhow!(
                        "credential issuer `{}` requires base_url",
                        credential_issuer_label(issuer)
                    ));
                }
                if issuer.requires_service_key_env()
                    && optional_non_empty(self.auth.service_key_env.as_str()).is_none()
                {
                    return Err(anyhow!(
                        "credential issuer `{}` requires service_key_env",
                        credential_issuer_label(issuer)
                    ));
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                let has_access_key_id =
                    optional_non_empty(self.auth.access_key_id.as_str()).is_some();
                let has_secret_access_key =
                    optional_non_empty(self.auth.secret_access_key.as_str()).is_some();
                if has_access_key_id ^ has_secret_access_key {
                    return Err(anyhow!(
                        "bedrock_sigv4 requires access_key_id and secret_access_key together"
                    ));
                }
            }
        }

        Ok(())
    }

    pub(crate) fn validate_for_adapters_for_save(
        &self,
        adapter_ids: &std::collections::BTreeSet<String>,
    ) -> std::result::Result<(), ProviderStudioSaveValidationError> {
        let default_adapter = optional_non_empty(self.default_adapter.as_str()).ok_or(
            ProviderStudioSaveValidationError::FieldRequired(
                ProviderStudioSaveField::DefaultAdapter,
            ),
        )?;
        if !self.auth_kind.supports_adapter(default_adapter) {
            return Err(
                ProviderStudioSaveValidationError::UnsupportedDefaultAdapter {
                    auth_kind: self.auth_kind.clone(),
                    adapter: default_adapter.to_owned(),
                    supported: supported_provider_draft_adapter_list(&self.auth_kind),
                },
            );
        }

        let incompatible = adapter_ids
            .iter()
            .filter(|adapter_id| !self.auth_kind.supports_adapter(adapter_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !incompatible.is_empty() {
            return Err(ProviderStudioSaveValidationError::UnsupportedAdapters {
                auth_kind: self.auth_kind.clone(),
                adapters: incompatible,
                supported: supported_provider_draft_adapter_list(&self.auth_kind),
            });
        }

        match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(ProviderStudioSaveValidationError::FieldRequired(
                    ProviderStudioSaveField::AuthMode,
                ));
            }
            ProviderDraftAuthKind::None => {}
            ProviderDraftAuthKind::ApiPending => {
                return Err(ProviderStudioSaveValidationError::FieldRequired(
                    ProviderStudioSaveField::AuthSubtype,
                ));
            }
            ProviderDraftAuthKind::Api => {
                let requires_base_url = adapter_ids.iter().any(|adapter_id| {
                    self.auth_kind
                        .adapter_rule(adapter_id.as_str())
                        .map(|rule| rule.requires_base_url)
                        .unwrap_or(false)
                });
                if requires_base_url && optional_non_empty(self.auth.base_url.as_str()).is_none() {
                    return Err(ProviderStudioSaveValidationError::ApiBaseUrlRequired);
                }
            }
            ProviderDraftAuthKind::ClineApi => {}
            ProviderDraftAuthKind::Gitlab => {
                if self.secret_source_overlay().is_none() {
                    return Err(ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired);
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                return Err(ProviderStudioSaveValidationError::FieldRequired(
                    ProviderStudioSaveField::CredentialIssuer,
                ));
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                if issuer.uses_http_endpoint()
                    && optional_non_empty(self.auth.base_url.as_str()).is_none()
                {
                    return Err(
                        ProviderStudioSaveValidationError::CredentialBaseUrlRequired { issuer },
                    );
                }
                if issuer.requires_service_key_env()
                    && optional_non_empty(self.auth.service_key_env.as_str()).is_none()
                {
                    return Err(
                        ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
                            issuer,
                        },
                    );
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                let has_access_key_id =
                    optional_non_empty(self.auth.access_key_id.as_str()).is_some();
                let has_secret_access_key =
                    optional_non_empty(self.auth.secret_access_key.as_str()).is_some();
                if has_access_key_id ^ has_secret_access_key {
                    return Err(ProviderStudioSaveValidationError::BedrockKeyPairRequired);
                }
            }
        }

        Ok(())
    }

    pub(super) fn validate_listing_request(&self, adapter_ids: &[String]) -> Result<()> {
        let selected = adapter_ids
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(anyhow!(
                "draft adapter model listing requires at least one explicit adapter"
            ));
        }
        let unsupported = selected
            .iter()
            .filter(|adapter_id| {
                self.auth_kind
                    .adapter_rule(adapter_id)
                    .map(|rule| !rule.supports_draft_model_listing)
                    .unwrap_or(true)
            })
            .copied()
            .collect::<Vec<_>>();
        if !unsupported.is_empty() {
            return Err(anyhow!(
                "draft adapter model listing only supports adapters with live model discovery for the current auth; unsupported: {}",
                unsupported.join(", ")
            ));
        }
        self.validate_for_adapters(
            &selected
                .into_iter()
                .map(ToOwned::to_owned)
                .collect::<std::collections::BTreeSet<_>>(),
        )
    }

    pub(crate) fn build_listing_target(
        &self,
        adapter_ids: &[String],
    ) -> Result<agena::config::ProviderAdapterModelsTarget> {
        if !self.auth_kind.supports_draft_model_listing() {
            return Err(anyhow!(
                "draft adapter model listing requires an auth/adapter combination with live model discovery; current auth is {}",
                self.auth_kind.label()
            ));
        }
        self.validate_listing_request(adapter_ids)?;
        let mut target = match self.auth_kind {
            ProviderDraftAuthKind::None => draft_none_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::ApiPending => unreachable!("validated auth subtype first"),
            ProviderDraftAuthKind::Api => draft_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                self.auth.base_url.as_str(),
                provider_draft_protocol_paths_for_listing(self),
                self.secret_source_inline_value(),
                self.secret_source_env_value(),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::ClineApi => draft_cline_api_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                self.secret_source_inline_value(),
                self.secret_source_env_value(),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::Gitlab => draft_gitlab_provider_adapter_models_target(
                Some(self.provider_id.as_str()),
                self.secret_source_inline_value(),
                self.secret_source_env_value(),
                adapter_ids,
            )
            .map_err(map_provider_adapter_models_config_error),
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                let credential = self.credential_auth_data_for_listing(issuer)?;
                draft_credential_provider_adapter_models_target(
                    Some(self.provider_id.as_str()),
                    issuer,
                    credential,
                    Some(self.auth.base_url.as_str()),
                    provider_draft_protocol_paths_for_listing(self),
                    Some(self.auth.service_key_env.as_str()),
                    Some(self.auth.instance_url.as_str()),
                    adapter_ids,
                )
                .map_err(map_provider_adapter_models_config_error)
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                draft_bedrock_sigv4_provider_adapter_models_target(
                    Some(self.provider_id.as_str()),
                    Some(self.auth.base_url.as_str()),
                    Some(self.auth.region.as_str()),
                    Some(self.auth.profile.as_str()),
                    Some(self.auth.access_key_id.as_str()),
                    Some(self.auth.secret_access_key.as_str()),
                    Some(self.auth.session_token.as_str()),
                    adapter_ids,
                )
                .map_err(map_provider_adapter_models_config_error)
            }
            ProviderDraftAuthKind::Unset | ProviderDraftAuthKind::Credential(None) => {
                unreachable!("listing guard ensures only supported draft auth kinds reach here")
            }
        }?;
        apply_known_provider_listing_defaults(self, &mut target);
        Ok(target)
    }

    pub(crate) fn request_fingerprint(&self, adapter_ids: &[String]) -> String {
        let mut hasher = DefaultHasher::new();
        self.source_provider_id
            .as_deref()
            .unwrap_or("<new>")
            .trim()
            .hash(&mut hasher);
        self.provider_id.trim().hash(&mut hasher);
        self.auth_kind.label().hash(&mut hasher);
        self.auth.base_url.trim().hash(&mut hasher);
        self.auth.instance_url.trim().hash(&mut hasher);
        self.auth.secret_source_kind.token().hash(&mut hasher);
        self.auth.secret_source_value.trim().hash(&mut hasher);
        self.auth.credential_issuer.trim().hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .login_kind
            .token()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .redirect_uri
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .callback_url
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .openai_chatgpt
            .account_id
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .enterprise_domain
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .github_copilot
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .redirect_uri
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .callback_url
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .refresh_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .access_token
            .trim()
            .hash(&mut hasher);
        self.credential_drafts
            .gitlab
            .tokens
            .expires_at_ms
            .trim()
            .hash(&mut hasher);
        self.auth.region.trim().hash(&mut hasher);
        self.auth.profile.trim().hash(&mut hasher);
        self.auth.access_key_id.trim().hash(&mut hasher);
        self.auth.secret_access_key.trim().hash(&mut hasher);
        self.auth.session_token.trim().hash(&mut hasher);
        self.auth.service_key_env.trim().hash(&mut hasher);
        self.default_adapter.trim().hash(&mut hasher);
        self.default_model.trim().hash(&mut hasher);
        let mut normalized_adapter_ids = adapter_ids
            .iter()
            .map(|adapter_id| adapter_id.trim())
            .filter(|adapter_id| !adapter_id.is_empty())
            .collect::<Vec<_>>();
        normalized_adapter_ids.sort_unstable();
        normalized_adapter_ids.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}
use crate::backend::Result;
use crate::backend::{
    DefaultHasher, ProviderConfigDraft, ProviderDraftAuthKind, ProviderStudioSaveField,
    ProviderStudioSaveValidationError, anyhow, apply_known_provider_listing_defaults,
    credential_issuer_label, draft_bedrock_sigv4_provider_adapter_models_target,
    draft_cline_api_provider_adapter_models_target,
    draft_credential_provider_adapter_models_target, draft_gitlab_provider_adapter_models_target,
    draft_none_provider_adapter_models_target, draft_provider_adapter_models_target,
    map_provider_adapter_models_config_error, optional_non_empty,
    provider_draft_protocol_paths_for_listing, required_trimmed,
    supported_provider_draft_adapter_list,
};
use std::hash::{Hash, Hasher};
