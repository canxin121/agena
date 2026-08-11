//! Draft provider configuration edited in the studio, migrated from
//! `agena-tui-backend/src/backend_drafts/provider_draft_config.rs`.

use anyhow::{Result, anyhow};

use super::catalog::{credential_issuer_label, optional_non_empty, parse_oauth_expires_at_ms};
use super::draft_auth_data::{
    DEFAULT_GITLAB_INSTANCE_URL, ProviderCredentialDraftBundle, ProviderDraftAuthDetails,
    ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    ProviderOAuthTokensDraft, ProviderStudioSaveError, ProviderStudioSaveField,
    ProviderStudioSaveValidationError, provider_credential_drafts,
};
use super::save::{parse_credential_issuer, trimmed_owned};
use agena_provider::{
    AuthData, ProviderAdapterOverlay, ProviderApiSubtype, ProviderAuthMode, ProviderAuthOverlay,
    ProviderOverlay, ProviderSecretSourceOverlay,
};

#[derive(Debug, Clone)]
/// Draft provider configuration edited in the studio.
pub struct ProviderConfigDraft {
    pub source_provider_id: Option<String>,
    pub provider_id: String,
    pub auth_kind: ProviderDraftAuthKind,
    pub auth: ProviderDraftAuthDetails,
    pub credential_drafts: ProviderCredentialDraftBundle,
    pub default_adapter: String,
    pub default_model: String,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

impl ProviderConfigDraft {
    pub fn new_empty() -> Self {
        Self {
            source_provider_id: None,
            provider_id: String::new(),
            auth_kind: ProviderDraftAuthKind::Unset,
            auth: ProviderDraftAuthDetails::default(),
            credential_drafts: ProviderCredentialDraftBundle::default(),
            default_adapter: String::new(),
            default_model: String::new(),
            request_timeout_secs: agena_provider::ProviderNetworkConfig::default()
                .request_timeout_secs,
            connect_timeout_secs: agena_provider::ProviderNetworkConfig::default()
                .connect_timeout_secs,
        }
    }

    fn normalized_shape(
        auth_kind: &ProviderDraftAuthKind,
        mut auth: ProviderDraftAuthDetails,
        mut credential_drafts: ProviderCredentialDraftBundle,
        mut default_adapter: String,
        mut default_model: String,
    ) -> (
        ProviderDraftAuthDetails,
        ProviderCredentialDraftBundle,
        String,
        String,
    ) {
        credential_drafts.normalize_shape();
        auth.credential_issuer = auth_kind
            .credential_issuer()
            .map(credential_issuer_label)
            .unwrap_or_default()
            .to_owned();

        match auth_kind {
            ProviderDraftAuthKind::Unset => {
                auth.base_url.clear();
                auth.secret_source_kind = ProviderDraftSecretSourceKind::Unset;
                auth.secret_source_value.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::None => {
                auth.base_url.clear();
                auth.secret_source_kind = ProviderDraftSecretSourceKind::Unset;
                auth.secret_source_value.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::ApiPending => {
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Api => {
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::ClineApi => {
                auth.base_url.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Gitlab => {
                auth.base_url.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
                if auth.instance_url.trim().is_empty() {
                    auth.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
            }
            ProviderDraftAuthKind::Credential(None) => {
                auth.base_url.clear();
                auth.secret_source_kind = ProviderDraftSecretSourceKind::Unset;
                auth.secret_source_value.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                auth.service_key_env.clear();
            }
            ProviderDraftAuthKind::Credential(Some(issuer)) => {
                auth.secret_source_kind = ProviderDraftSecretSourceKind::Unset;
                auth.secret_source_value.clear();
                auth.region.clear();
                auth.profile.clear();
                auth.access_key_id.clear();
                auth.secret_access_key.clear();
                auth.session_token.clear();
                if !issuer.uses_http_endpoint() {
                    auth.base_url.clear();
                }
                if *issuer == CredentialIssuer::Gitlab && auth.instance_url.trim().is_empty() {
                    auth.instance_url = DEFAULT_GITLAB_INSTANCE_URL.to_owned();
                }
                if issuer.requires_service_key_env() {
                    if auth.service_key_env.trim().is_empty() {
                        auth.service_key_env = "AICORE_SERVICE_KEY".to_owned();
                    }
                } else {
                    auth.service_key_env.clear();
                }
            }
            ProviderDraftAuthKind::BedrockSigv4 => {
                auth.secret_source_kind = ProviderDraftSecretSourceKind::Unset;
                auth.secret_source_value.clear();
                auth.service_key_env.clear();
            }
        }

        if !default_adapter.trim().is_empty()
            && !auth_kind.supports_adapter(default_adapter.as_str())
        {
            default_adapter.clear();
        }
        if default_adapter.trim().is_empty() {
            default_model.clear();
        }
        (auth, credential_drafts, default_adapter, default_model)
    }

    pub fn normalize_shape(&mut self) {
        let (auth, credential_drafts, default_adapter, default_model) = Self::normalized_shape(
            &self.auth_kind,
            std::mem::take(&mut self.auth),
            std::mem::take(&mut self.credential_drafts),
            std::mem::take(&mut self.default_adapter),
            std::mem::take(&mut self.default_model),
        );
        self.auth = auth;
        self.credential_drafts = credential_drafts;
        self.default_adapter = default_adapter;
        self.default_model = default_model;
    }

    pub fn from_configured_editor(provider: agena_provider::ProviderConfiguredEditor) -> Self {
        let split_api_key = |api_key: Option<agena_provider::ProviderApiKeySource>| match api_key {
            Some(agena_provider::ProviderApiKeySource::Inline(value)) => {
                (ProviderDraftSecretSourceKind::Inline, value)
            }
            Some(agena_provider::ProviderApiKeySource::Environment(value)) => {
                (ProviderDraftSecretSourceKind::Env, value)
            }
            None => (ProviderDraftSecretSourceKind::Unset, String::new()),
        };
        let (
            auth_kind,
            base_url,
            instance_url,
            secret_source_kind,
            secret_source_value,
            credential_issuer,
            region,
            profile,
            access_key_id,
            secret_access_key,
            session_token,
            service_key_env,
            credential_drafts,
        ) = match provider.auth {
            agena_provider::ProviderConfiguredEditorAuth::None => (
                ProviderDraftAuthKind::None,
                String::new(),
                String::new(),
                ProviderDraftSecretSourceKind::Unset,
                String::new(),
                credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                ProviderCredentialDraftBundle::default(),
            ),
            agena_provider::ProviderConfiguredEditorAuth::Api { base_url, api_key } => {
                let (kind, value) = split_api_key(api_key);
                (
                    ProviderDraftAuthKind::Api,
                    base_url,
                    String::new(),
                    kind,
                    value,
                    credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ProviderCredentialDraftBundle::default(),
                )
            }
            agena_provider::ProviderConfiguredEditorAuth::ClineApi { api_key } => {
                let (kind, value) = split_api_key(api_key);
                (
                    ProviderDraftAuthKind::ClineApi,
                    String::new(),
                    String::new(),
                    kind,
                    value,
                    credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ProviderCredentialDraftBundle::default(),
                )
            }
            agena_provider::ProviderConfiguredEditorAuth::Gitlab {
                api_key,
                instance_url,
            } => {
                let (kind, value) = split_api_key(api_key);
                (
                    ProviderDraftAuthKind::Gitlab,
                    String::new(),
                    instance_url.unwrap_or_default(),
                    kind,
                    value,
                    credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ProviderCredentialDraftBundle::default(),
                )
            }
            agena_provider::ProviderConfiguredEditorAuth::Credential {
                issuer,
                credential,
                base_url,
                instance_url,
                service_key_env,
            } => (
                ProviderDraftAuthKind::Credential(Some(issuer)),
                base_url.unwrap_or_default(),
                instance_url.unwrap_or_default(),
                ProviderDraftSecretSourceKind::Unset,
                String::new(),
                credential_issuer_label(issuer).to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                service_key_env.unwrap_or_default(),
                provider_credential_drafts(issuer, credential.as_ref()),
            ),
            agena_provider::ProviderConfiguredEditorAuth::BedrockSigv4 {
                base_url,
                region,
                profile,
                access_key_id,
                secret_access_key,
                session_token,
            } => (
                ProviderDraftAuthKind::BedrockSigv4,
                base_url,
                String::new(),
                ProviderDraftSecretSourceKind::Unset,
                String::new(),
                credential_issuer_label(CredentialIssuer::OpenaiChatgpt).to_owned(),
                region,
                profile.unwrap_or_default(),
                access_key_id.unwrap_or_default(),
                secret_access_key.unwrap_or_default(),
                session_token.unwrap_or_default(),
                String::new(),
                ProviderCredentialDraftBundle::default(),
            ),
        };
        let (auth, credential_drafts, default_adapter, default_model) = Self::normalized_shape(
            &auth_kind,
            ProviderDraftAuthDetails {
                base_url,
                instance_url,
                secret_source_kind,
                secret_source_value,
                credential_issuer,
                region,
                profile,
                access_key_id,
                secret_access_key,
                session_token,
                service_key_env,
            },
            credential_drafts,
            provider.default_adapter.unwrap_or_default(),
            provider.default_model.unwrap_or_default(),
        );
        Self {
            source_provider_id: Some(provider.provider_id.clone()),
            provider_id: provider.provider_id,
            auth_kind,
            auth,
            credential_drafts,
            default_adapter,
            default_model,
            request_timeout_secs: provider.request_timeout_secs,
            connect_timeout_secs: provider.connect_timeout_secs,
        }
    }

    pub(crate) fn to_provider_overlay_for_save(
        &self,
        default_adapter: &str,
        default_model: Option<&str>,
        adapters: std::collections::BTreeMap<String, ProviderAdapterOverlay>,
        include_defaults: bool,
    ) -> std::result::Result<ProviderOverlay, ProviderStudioSaveError> {
        Ok(ProviderOverlay {
            enabled: Some(true),
            defaults: include_defaults.then(|| agena_provider::ProviderDefaultsOverlay {
                adapter: Some(default_adapter.to_owned()),
                model: default_model.map(ToOwned::to_owned),
                ..Default::default()
            }),
            auth: Some(self.to_auth_overlay_for_save()?),
            adapters,
            network: Some(agena_provider::ProviderNetworkOverlay {
                request_timeout_secs: Some(self.request_timeout_secs),
                connect_timeout_secs: Some(self.connect_timeout_secs),
            }),
        })
    }

    pub(crate) fn to_auth_overlay_for_save(
        &self,
    ) -> std::result::Result<ProviderAuthOverlay, ProviderStudioSaveError> {
        let credential = self
            .oauth_auth_data()
            .map_err(ProviderStudioSaveError::other)?;
        let mode = Some(self.to_provider_auth_mode_for_save()?);

        let (
            subtype,
            base_url,
            api_key,
            access,
            instance_url,
            issuer,
            credential,
            profile,
            access_key_id,
            secret_access_key,
            session_token,
            region,
            service_key_env,
        ) = match self.auth_kind {
            ProviderDraftAuthKind::Unset => {
                return Err(ProviderStudioSaveError::Validation(
                    ProviderStudioSaveValidationError::FieldRequired(
                        ProviderStudioSaveField::AuthMode,
                    ),
                ));
            }
            ProviderDraftAuthKind::None => (
                None, None, None, None, None, None, None, None, None, None, None, None, None,
            ),
            ProviderDraftAuthKind::ApiPending => {
                return Err(ProviderStudioSaveError::Validation(
                    ProviderStudioSaveValidationError::FieldRequired(
                        ProviderStudioSaveField::AuthSubtype,
                    ),
                ));
            }
            ProviderDraftAuthKind::Api => (
                Some(ProviderApiSubtype::Custom),
                trimmed_owned(self.auth.base_url.as_str()),
                self.secret_source_overlay(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            ProviderDraftAuthKind::ClineApi => (
                Some(ProviderApiSubtype::ClineApi),
                None,
                self.secret_source_overlay(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            ProviderDraftAuthKind::Gitlab => (
                Some(ProviderApiSubtype::Gitlab),
                None,
                None,
                self.secret_source_overlay().map(|source| {
                    agena_provider::ProviderGitlabApiAccessOverlay::ApiKey { source }
                }),
                trimmed_owned(self.auth.instance_url.as_str()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            ProviderDraftAuthKind::Credential(None) => {
                return Err(ProviderStudioSaveError::Validation(
                    ProviderStudioSaveValidationError::FieldRequired(
                        ProviderStudioSaveField::CredentialIssuer,
                    ),
                ));
            }
            ProviderDraftAuthKind::Credential(Some(_)) => {
                let issuer = parse_credential_issuer(self.auth.credential_issuer.as_str())
                    .map_err(ProviderStudioSaveError::other)?;
                (
                    None,
                    issuer
                        .uses_http_endpoint()
                        .then(|| trimmed_owned(self.auth.base_url.as_str()))
                        .flatten(),
                    None,
                    None,
                    (issuer == CredentialIssuer::Gitlab)
                        .then(|| trimmed_owned(self.auth.instance_url.as_str()))
                        .flatten(),
                    Some(issuer),
                    credential,
                    None,
                    None,
                    None,
                    None,
                    None,
                    issuer
                        .requires_service_key_env()
                        .then(|| trimmed_owned(self.auth.service_key_env.as_str()))
                        .flatten(),
                )
            }
            ProviderDraftAuthKind::BedrockSigv4 => (
                Some(ProviderApiSubtype::BedrockSigv4),
                trimmed_owned(self.auth.base_url.as_str()),
                None,
                None,
                None,
                None,
                None,
                trimmed_owned(self.auth.profile.as_str()),
                trimmed_owned(self.auth.access_key_id.as_str()),
                trimmed_owned(self.auth.secret_access_key.as_str()),
                trimmed_owned(self.auth.session_token.as_str()),
                trimmed_owned(self.auth.region.as_str()),
                None,
            ),
        };

        Ok(ProviderAuthOverlay {
            mode,
            subtype,
            base_url,
            protocol_paths: None,
            api_key,
            access,
            instance_url,
            ai_gateway_url: None,
            ai_gateway_headers: std::collections::BTreeMap::new(),
            feature_flags: std::collections::BTreeMap::new(),
            issuer,
            credential,
            profile,
            access_key_id,
            secret_access_key,
            session_token,
            region,
            service_key_env,
        })
    }

    pub(crate) fn to_provider_auth_mode_for_save(
        &self,
    ) -> std::result::Result<ProviderAuthMode, ProviderStudioSaveError> {
        match self.auth_kind {
            ProviderDraftAuthKind::Unset => Err(ProviderStudioSaveError::Validation(
                ProviderStudioSaveValidationError::FieldRequired(ProviderStudioSaveField::AuthMode),
            )),
            ProviderDraftAuthKind::None => Ok(ProviderAuthMode::None),
            ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi => Ok(ProviderAuthMode::Api),
            ProviderDraftAuthKind::Gitlab => Ok(ProviderAuthMode::Api),
            ProviderDraftAuthKind::Credential(_) => Ok(ProviderAuthMode::Credential),
            ProviderDraftAuthKind::BedrockSigv4 => Ok(ProviderAuthMode::Api),
        }
    }

    pub(crate) fn secret_source_overlay(&self) -> Option<ProviderSecretSourceOverlay> {
        let value = optional_non_empty(self.auth.secret_source_value.as_str())?.to_owned();
        match self.auth.secret_source_kind {
            ProviderDraftSecretSourceKind::Unset => None,
            ProviderDraftSecretSourceKind::Inline => {
                Some(ProviderSecretSourceOverlay::Inline(value))
            }
            ProviderDraftSecretSourceKind::Env => Some(ProviderSecretSourceOverlay::Env(value)),
        }
    }

    pub(crate) fn secret_source_inline_value(&self) -> Option<&str> {
        matches!(
            self.auth.secret_source_kind,
            ProviderDraftSecretSourceKind::Inline
        )
        .then_some(self.auth.secret_source_value.as_str())
    }

    pub(crate) fn secret_source_env_value(&self) -> Option<&str> {
        matches!(
            self.auth.secret_source_kind,
            ProviderDraftSecretSourceKind::Env
        )
        .then_some(self.auth.secret_source_value.as_str())
    }

    pub(crate) fn oauth_auth_data(&self) -> Result<Option<AuthData>> {
        match self.auth_kind {
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)) => {
                let tokens = &self.credential_drafts.openai_chatgpt.tokens;
                if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                    && optional_non_empty(tokens.access_token.as_str()).is_none()
                {
                    return Ok(None);
                }
                Ok(Some(AuthData::OAuth {
                    issuer: Some(CredentialIssuer::OpenaiChatgpt),
                    refresh: tokens.refresh_token.clone(),
                    access: tokens.access_token.clone(),
                    id_token: None,
                    expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                    account_id: optional_non_empty(
                        self.credential_drafts.openai_chatgpt.account_id.as_str(),
                    )
                    .map(ToOwned::to_owned),
                    chatgpt_account_is_fedramp: false,
                    enterprise_url: None,
                    user: None,
                }))
            }
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot)) => {
                let tokens = &self.credential_drafts.github_copilot.tokens;
                if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                    && optional_non_empty(tokens.access_token.as_str()).is_none()
                {
                    return Ok(None);
                }
                Ok(Some(AuthData::OAuth {
                    issuer: Some(CredentialIssuer::GithubCopilot),
                    refresh: tokens.refresh_token.clone(),
                    access: tokens.access_token.clone(),
                    id_token: None,
                    expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                    account_id: None,
                    chatgpt_account_is_fedramp: false,
                    enterprise_url: optional_non_empty(
                        self.credential_drafts
                            .github_copilot
                            .enterprise_domain
                            .as_str(),
                    )
                    .map(ToOwned::to_owned),
                    user: None,
                }))
            }
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)) => {
                let tokens = &self.credential_drafts.gitlab.tokens;
                if optional_non_empty(tokens.refresh_token.as_str()).is_none()
                    && optional_non_empty(tokens.access_token.as_str()).is_none()
                {
                    return Ok(None);
                }
                Ok(Some(AuthData::OAuth {
                    issuer: Some(CredentialIssuer::Gitlab),
                    refresh: tokens.refresh_token.clone(),
                    access: tokens.access_token.clone(),
                    id_token: None,
                    expires_at_ms: parse_oauth_expires_at_ms(tokens.expires_at_ms.as_str())?,
                    account_id: None,
                    chatgpt_account_is_fedramp: false,
                    enterprise_url: None,
                    user: None,
                }))
            }
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore,
            ))
            | ProviderDraftAuthKind::Unset
            | ProviderDraftAuthKind::None
            | ProviderDraftAuthKind::ApiPending
            | ProviderDraftAuthKind::Api
            | ProviderDraftAuthKind::ClineApi
            | ProviderDraftAuthKind::Gitlab
            | ProviderDraftAuthKind::Credential(None)
            | ProviderDraftAuthKind::BedrockSigv4 => Ok(None),
        }
    }

    pub(crate) fn credential_auth_data_for_listing(
        &self,
        issuer: CredentialIssuer,
    ) -> Result<Option<AuthData>> {
        match issuer {
            CredentialIssuer::OpenaiChatgpt
            | CredentialIssuer::GithubCopilot
            | CredentialIssuer::Gitlab => {
                let credential = self.oauth_auth_data()?.ok_or_else(|| {
                    anyhow!(
                        "draft {} model listing requires OAuth tokens; start auth first or enter tokens manually",
                        credential_issuer_label(issuer)
                    )
                })?;
                Ok(Some(credential))
            }
            CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore => Ok(None),
        }
    }

    pub fn active_credential_issuer(&self) -> Option<CredentialIssuer> {
        self.auth_kind.credential_issuer()
    }

    pub fn active_tokens(&self) -> Option<&ProviderOAuthTokensDraft> {
        self.credential_drafts
            .active_tokens(self.active_credential_issuer())
    }

    pub fn active_tokens_mut(&mut self) -> Option<&mut ProviderOAuthTokensDraft> {
        self.credential_drafts
            .active_tokens_mut(self.active_credential_issuer())
    }

    pub fn redirect_uri(&self) -> Option<&str> {
        self.credential_drafts
            .redirect_uri(self.active_credential_issuer())
    }

    pub fn callback_url(&self) -> Option<&str> {
        self.credential_drafts
            .callback_url(self.active_credential_issuer())
    }

    pub fn account_id(&self) -> Option<&str> {
        self.credential_drafts
            .account_id(self.active_credential_issuer())
    }

    pub fn set_redirect_uri(&mut self, value: String) {
        self.credential_drafts
            .set_redirect_uri(self.active_credential_issuer(), value);
    }

    pub fn set_callback_url(&mut self, value: String) {
        self.credential_drafts
            .set_callback_url(self.active_credential_issuer(), value);
    }

    pub fn set_refresh_token(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.refresh_token = value;
        }
    }

    pub fn set_access_token(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.access_token = value;
        }
    }

    pub fn set_expires_at_ms(&mut self, value: String) {
        if let Some(tokens) = self.active_tokens_mut() {
            tokens.expires_at_ms = value;
        }
    }

    pub fn set_account_id(&mut self, value: String) {
        self.credential_drafts
            .set_account_id(self.active_credential_issuer(), value);
    }

    pub fn interactive_login_kind(&self) -> Option<ProviderDraftInteractiveLoginKind> {
        match self.active_credential_issuer() {
            Some(CredentialIssuer::OpenaiChatgpt) => {
                Some(self.credential_drafts.openai_chatgpt.login_kind)
            }
            Some(CredentialIssuer::GithubCopilot) => {
                Some(ProviderDraftInteractiveLoginKind::Device)
            }
            Some(CredentialIssuer::Gitlab) => Some(ProviderDraftInteractiveLoginKind::Browser),
            Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => None,
        }
    }

    pub fn set_interactive_login_kind(&mut self, kind: ProviderDraftInteractiveLoginKind) {
        if self.active_credential_issuer() == Some(CredentialIssuer::OpenaiChatgpt)
            && self.credential_drafts.openai_chatgpt.login_kind != kind
        {
            self.credential_drafts.openai_chatgpt.login_kind = kind;
            self.credential_drafts.openai_chatgpt.clear_pending();
        }
    }

    pub fn supports_interactive_auth(&self) -> bool {
        matches!(
            self.auth_kind,
            ProviderDraftAuthKind::Credential(Some(
                CredentialIssuer::OpenaiChatgpt
                    | CredentialIssuer::GithubCopilot
                    | CredentialIssuer::Gitlab
            ))
        )
    }

    pub fn supports_saved_model_listing(&self) -> bool {
        self.auth_kind.supports_draft_model_listing()
    }

    pub fn tokens_present(&self) -> bool {
        self.active_tokens().is_some_and(|tokens| {
            !tokens.refresh_token.trim().is_empty() || !tokens.access_token.trim().is_empty()
        })
    }
}

use agena_provider::CredentialIssuer;
