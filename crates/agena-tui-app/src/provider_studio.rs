use agena_tui_backend::ProviderDraftSecretSourceKind;

/// Project the concrete backend draft into the provider feature contract.
/// Authentication polling and persistence stay in this app adapter.
pub(crate) fn provider_studio_snapshot(
    dialog: &ProviderStudioOverlay,
) -> agena_tui_provider_studio::ProviderDraft {
    let fields = [
        (ProviderStudioField::BaseUrl, false),
        (ProviderStudioField::InstanceUrl, false),
        (ProviderStudioField::ApiKeySource, false),
        (ProviderStudioField::ApiKeyValue, true),
        (ProviderStudioField::Region, false),
        (ProviderStudioField::Profile, false),
        (ProviderStudioField::RequestTimeoutSecs, false),
        (ProviderStudioField::ConnectTimeoutSecs, false),
    ]
    .into_iter()
    .map(|(field, secret)| agena_tui_provider_studio::ProviderField {
        key: provider_studio_field_label_key(field).to_owned(),
        label: provider_studio_field_label(&I18n::english(), field),
        value: provider_studio_field_value(&dialog.draft, field),
        secret,
    })
    .collect();

    agena_tui_provider_studio::ProviderDraft {
        provider_id: dialog.draft.provider_id.clone(),
        display_name: dialog.title.clone(),
        fields,
    }
}

pub(super) mod provider_auth;
pub(super) mod provider_fields;
pub(super) mod provider_selection;

pub(super) use self::provider_auth::*;
pub(super) use self::provider_fields::*;
use crate::{
    BTreeSet, CredentialIssuer, Duration, I18n, JsonValue, ProviderAdapterModelsResource,
    ProviderConfigDraft, ProviderDraftAdapterRule, ProviderDraftAuthKind,
    ProviderDraftInteractiveLoginKind, ProviderModelConfigDraft, ProviderModelConfigField,
    ProviderNativeToolsPreset, ProviderStudioField, ProviderStudioOverlay, join_inline_segments,
    provider_native_tools_config_for_preset, provider_native_tools_preset_from_config,
    truncate_display_width,
};
pub(super) use agena_tui_provider_studio::provider_model_helpers::*;
