//! Provider Studio draft / save surface, migrated from
//! `agena-tui-backend/src/backend_drafts/` (mod.rs + provider_draft_auth.rs +
//! provider_draft_config.rs + provider_draft_validation.rs) and the Provider
//! Studio save/delete helpers from `backend_provider/{selection,settings}.rs`,
//! `backend_config.rs`, `backend_util.rs`, `backend_catalog.rs`,
//! `backend_auth.rs`, and `backend_events.rs`.
//!
//! The interactive auth flow entry points (`start_provider_draft_auth` /
//! `continue_provider_draft_auth`) remain in the TUI backend; the types they
//! operate on are re-exported here per the R7 brief §2.

// The submodules are `pub(crate)` so the crate-root `application_*.rs` modules
// can reach the migrated helpers; nothing here is part of the public API beyond
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
