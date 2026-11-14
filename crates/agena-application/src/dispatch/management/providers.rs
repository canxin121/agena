//! Provider Studio, catalog and authentication operations use the same services
//! as HTTP, including validation failures and reload coordination.
use super::wire;
use crate::{Application, ApplicationError, AuthLoginKind};
use agena_api::management::provider::AuthLoginKindResource;
use agena_api::management::{AuthCommand, AuthQuery, ProviderCommand, ProviderQuery};
use serde_json::Value;

impl Application {
    pub(super) async fn query_provider_management(
        &self,
        query: ProviderQuery,
    ) -> Result<Value, ApplicationError> {
        match query {
            ProviderQuery::ConfiguredModels { provider_id } => wire(
                crate::provider_queries::list_configured_provider_models_response(
                    self,
                    provider_id,
                )?,
            ),
            ProviderQuery::ConfiguredAdapterModels { provider_id } => wire(
                crate::provider_queries::configured_provider_adapter_models_response(
                    self,
                    Some(&provider_id),
                ),
            ),
            ProviderQuery::AwsProfiles => {
                let profiles = self
                    .run_blocking(|_| crate::provider_queries::list_aws_profile_names())
                    .await?;
                Ok(serde_json::json!({ "profiles": profiles }))
            }
            ProviderQuery::Draft { provider_id } => wire(
                self.run_blocking(move |app| app.provider_config_draft(provider_id.as_deref()))
                    .await??,
            ),
            ProviderQuery::DraftModels { draft, adapter_ids } => wire(
                self.list_draft_provider_adapter_models(&draft, &adapter_ids)
                    .await?,
            ),
            ProviderQuery::ModelDraft {
                draft,
                adapter_id,
                model_id,
                provider_model,
            } => {
                let value = self
                    .run_blocking(move |app| {
                        app.provider_model_draft_value(
                            &draft,
                            &adapter_id,
                            &model_id,
                            provider_model.as_ref(),
                        )
                    })
                    .await??;
                Ok(serde_json::json!({ "value": value }))
            }
            ProviderQuery::CatalogTemplate {
                catalog_model_id,
                model_value,
                clear,
            } => {
                let value = self.provider_model_catalog_template_value(
                    &catalog_model_id,
                    model_value,
                    clear,
                )?;
                Ok(serde_json::json!({ "value": value }))
            }
        }
    }

    pub(super) async fn command_provider_management(
        &self,
        command: ProviderCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            ProviderCommand::RefreshClientVersions => {
                let versions = self.refresh_provider_client_versions().await?;
                Ok(
                    serde_json::json!({ "codex": versions.codex, "claude": versions.claude, "gemini": versions.gemini }),
                )
            }
            ProviderCommand::SaveDraft {
                draft,
                adapter_models,
                selected_adapter_ids,
                selected_model_keys,
                model_config_values,
            } => wire(
                self.save_provider_draft(
                    draft,
                    &adapter_models,
                    &selected_adapter_ids,
                    &selected_model_keys,
                    &model_config_values,
                )
                .await
                .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::SaveAdapter { draft, adapter } => wire(
                self.save_provider_adapter_matches(draft, adapter)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::SaveModel {
                draft,
                adapter_id,
                model_id,
                value,
            } => wire(
                self.save_provider_model_value(draft, &adapter_id, &model_id, value)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::DeleteProvider { provider_id } => wire(
                self.delete_provider(&provider_id)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::DeleteAdapter { draft, adapter_id } => wire(
                self.delete_provider_adapter(draft, &adapter_id)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::DeleteModel {
                draft,
                adapter_id,
                model_id,
            } => wire(
                self.delete_provider_model(draft, &adapter_id, &model_id)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::StartDraftAuth { draft } => wire(
                self.start_provider_draft_auth(draft)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
            ProviderCommand::ContinueDraftAuth { draft } => wire(
                self.continue_provider_draft_auth(draft)
                    .await
                    .map_err(ApplicationError::from)?,
            ),
        }
    }

    pub(super) async fn query_auth_management(
        &self,
        query: AuthQuery,
    ) -> Result<Value, ApplicationError> {
        self.run_blocking(move |app| match query {
            AuthQuery::List => wire(app.auth_providers()?),
            AuthQuery::Get { provider_id } => wire(app.auth_provider(&provider_id)?),
        })
        .await?
    }

    pub(super) async fn command_auth_management(
        &self,
        command: AuthCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            AuthCommand::SetApiKey {
                provider_id,
                api_key,
            } => wire(self.set_auth_api_key(&provider_id, api_key).await?),
            AuthCommand::Remove { provider_id } => {
                wire(self.remove_auth_provider(&provider_id).await?)
            }
            AuthCommand::Refresh { provider_id } => {
                wire(self.refresh_auth_provider(&provider_id).await?)
            }
            AuthCommand::StartBrowser {
                provider_id,
                kind,
                redirect_uri,
            } => wire(
                self.start_auth_browser(provider_id, login_kind(kind), redirect_uri)
                    .await?,
            ),
            AuthCommand::FinishBrowser {
                provider_id,
                kind,
                code,
                pkce_verifier,
                redirect_uri,
            } => wire(
                self.finish_auth_browser(
                    &provider_id,
                    login_kind(kind),
                    code,
                    pkce_verifier,
                    redirect_uri,
                )
                .await?,
            ),
            AuthCommand::StartDevice {
                provider_id,
                kind,
                enterprise_domain,
            } => wire(
                self.start_auth_device(provider_id, login_kind(kind), enterprise_domain)
                    .await?,
            ),
            AuthCommand::PollDevice {
                provider_id,
                kind,
                device_code,
                user_code,
                enterprise_domain,
            } => wire(
                self.poll_auth_device(
                    &provider_id,
                    login_kind(kind),
                    device_code,
                    user_code,
                    enterprise_domain,
                )
                .await?,
            ),
        }
    }
}

fn login_kind(kind: AuthLoginKindResource) -> AuthLoginKind {
    match kind {
        AuthLoginKindResource::OpenaiChatgpt => AuthLoginKind::OpenaiChatgpt,
        AuthLoginKindResource::GithubCopilot => AuthLoginKind::GithubCopilot,
        AuthLoginKindResource::Gitlab => AuthLoginKind::Gitlab,
    }
}

impl From<crate::provider_studio::ProviderStudioSaveError> for ApplicationError {
    fn from(error: crate::provider_studio::ProviderStudioSaveError) -> Self {
        provider_studio_save_error(error)
    }
}
impl From<crate::provider_studio::ProviderDraftAuthError> for ApplicationError {
    fn from(error: crate::provider_studio::ProviderDraftAuthError) -> Self {
        provider_studio_auth_error(error)
    }
}

fn provider_studio_save_error(
    error: crate::provider_studio::ProviderStudioSaveError,
) -> ApplicationError {
    match error {
        crate::provider_studio::ProviderStudioSaveError::Other(problem) => {
            ApplicationError::from_failure(problem.into())
        }
        crate::provider_studio::ProviderStudioSaveError::Validation(validation) => {
            ApplicationError::bad_request(provider_studio_validation_message(&validation))
        }
        crate::provider_studio::ProviderStudioSaveError::ExistingProviderSettingsMustBeObject => {
            ApplicationError::bad_request("The existing provider settings must be a JSON object.")
        }
        crate::provider_studio::ProviderStudioSaveError::ProviderAdapterMustBeObject { .. } => {
            ApplicationError::bad_request("The provider adapter settings must be a JSON object.")
        }
        crate::provider_studio::ProviderStudioSaveError::ProviderModelConfigMustBeObject => {
            ApplicationError::bad_request("The provider model configuration must be a JSON object.")
        }
        crate::provider_studio::ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject => {
            ApplicationError::bad_request("The configured provider adapters must be a JSON object.")
        }
        crate::provider_studio::ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject => {
            ApplicationError::bad_request("The configured provider models must be a JSON object.")
        }
    }
}

fn provider_studio_validation_message(
    error: &crate::provider_studio::ProviderStudioSaveValidationError,
) -> &'static str {
    use crate::provider_studio::{ProviderStudioSaveField, ProviderStudioSaveValidationError};

    match error {
        ProviderStudioSaveValidationError::FieldRequired(field) => match field {
            ProviderStudioSaveField::ProviderId => "Provider ID is required.",
            ProviderStudioSaveField::AdapterId => "Adapter ID is required.",
            ProviderStudioSaveField::ModelId => "Model ID is required.",
            ProviderStudioSaveField::AuthMode => "Choose an authentication mode.",
            ProviderStudioSaveField::AuthSubtype => "Choose an authentication subtype.",
            ProviderStudioSaveField::CredentialIssuer => "Choose a credential issuer.",
        },
        ProviderStudioSaveValidationError::UnsupportedAdapters { .. } => {
            "One or more selected adapters are not supported by this authentication mode."
        }
        ProviderStudioSaveValidationError::ApiBaseUrlRequired => {
            "A provider base URL is required for this API adapter."
        }
        ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired => {
            "A GitLab API key or API-key environment variable is required."
        }
        ProviderStudioSaveValidationError::CredentialBaseUrlRequired { .. } => {
            "A provider base URL is required for this credential issuer."
        }
        ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired { .. } => {
            "A service-key environment variable is required for this credential issuer."
        }
        ProviderStudioSaveValidationError::BedrockKeyPairRequired => {
            "An AWS profile or both Bedrock access-key fields are required."
        }
    }
}

fn provider_studio_auth_error(
    error: crate::provider_studio::ProviderDraftAuthError,
) -> ApplicationError {
    match error {
        crate::provider_studio::ProviderDraftAuthError::Other(problem) => {
            ApplicationError::from_failure(problem.into())
        }
        crate::provider_studio::ProviderDraftAuthError::UnsupportedInteractiveLogin => {
            ApplicationError::bad_request(
                "This provider does not support interactive authentication.",
            )
        }
        crate::provider_studio::ProviderDraftAuthError::StartBrowserAuthFirst => {
            ApplicationError::bad_request("Start browser authorization before continuing.")
        }
        crate::provider_studio::ProviderDraftAuthError::StartDeviceAuthFirst => {
            ApplicationError::bad_request("Start device authorization before continuing.")
        }
        crate::provider_studio::ProviderDraftAuthError::RequiredField(field) => {
            let message = match field {
                crate::provider_studio::ProviderDraftAuthField::RedirectUri => {
                    "Redirect URI is required."
                }
                crate::provider_studio::ProviderDraftAuthField::InstanceUrl => {
                    "Provider instance URL is required."
                }
                crate::provider_studio::ProviderDraftAuthField::CallbackUrl => {
                    "OAuth callback URL is required."
                }
            };
            ApplicationError::bad_request(message)
        }
    }
}
