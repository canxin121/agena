//! Provider Studio draft / save surface. Previously located in
//! `agena-tui-backend/src/backend_drafts/` (mod.rs + provider_draft_auth.rs +
//! provider_draft_config.rs + provider_draft_validation.rs) and the Provider
//! Studio save/delete helpers from `backend_provider/{selection,settings}.rs`,
//! `backend_config.rs`, `backend_util.rs`, `backend_catalog.rs`,
//! `backend_auth.rs`, and `backend_events.rs`.
//!
//! The interactive auth flow entry points (`start_provider_draft_auth` /
//! `continue_provider_draft_auth`) are exposed by [`crate::Application`]; the
//! transport-safe types they operate on are re-exported here.

// The submodules are `pub(crate)` so the crate-root `application_*.rs` modules
// can reach the helpers; nothing here is part of the public API beyond
// the `pub use` surface below.
pub(crate) mod catalog;
pub(crate) mod draft_auth_data;
pub(crate) mod draft_config;
pub(crate) mod draft_validation;
pub(crate) mod save;

pub use draft_auth_data::{
    GithubCopilotCredentialDraft, GitlabCredentialDraft, OpenAiChatgptCredentialDraft,
    ProviderBrowserAuthSessionDraft, ProviderCredentialDraftBundle, ProviderDeviceAuthSessionDraft,
    ProviderDraftAdapterRule, ProviderDraftAuthActionResult, ProviderDraftAuthDetails,
    ProviderDraftAuthError, ProviderDraftAuthField, ProviderDraftAuthKind,
    ProviderDraftAuthMessage, ProviderDraftInteractiveLoginKind, ProviderDraftSecretSourceKind,
    ProviderOAuthTokensDraft, ProviderStudioSaveError, ProviderStudioSaveField,
    ProviderStudioSaveResult, ProviderStudioSaveValidationError,
};
pub use draft_config::ProviderConfigDraft;

/// Build an editable model configuration from public provider metadata when
/// the model has no explicit file-level override yet.
pub fn provider_model_draft_value_from_resource(
    model_id: &str,
    provider_model: Option<&agena_api::resource::ProviderModelResource>,
) -> Result<serde_json::Value, serde_json::Error> {
    catalog::provider_model_json_for_model_id(&[], model_id, provider_model)
}

/// Copy one selected catalog definition into an editable provider-model value.
/// The selected catalog ID is UI state only; the returned value contains just
/// ordinary model fields and keeps the route's operational settings.
pub fn apply_catalog_template_to_model_value(
    model_value: serde_json::Value,
    catalog_model: &crate::dto::CatalogModelResource,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut configured: agena_provider::ResolvedProviderModelConfig =
        serde_json::from_value(model_value)?;
    configured.definition =
        catalog::catalog_model_to_catalog_definition(catalog_model).into_configured_definition();
    serde_json::to_value(configured)
}
