//! Provider Studio entry points, migrated from
//! `agena-tui-backend/src/backend_provider/selection.rs` and
//! `settings.rs`. These are thin `impl Application` delegations to the
//! migrated free functions in `provider_studio::save`.

use crate::provider_studio::save;
use crate::provider_studio::{
    ProviderConfigDraft, ProviderStudioSaveError, ProviderStudioSaveResult,
};
use crate::{Application, ApplicationError};

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
    ) -> std::result::Result<ProviderStudioSaveResult, ProviderStudioSaveError> {
        save::save_provider_draft(
            self,
            draft,
            adapter_model_lists,
            selected_adapter_ids,
            selected_model_keys,
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
}
