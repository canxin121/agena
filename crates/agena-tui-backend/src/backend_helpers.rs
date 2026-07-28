/// Provider-native presets were removed from the active architecture. Keep the
/// transition helper until old TUI drafts are migrated, but never project a
/// provider-native declaration into model configuration.
pub fn provider_native_tools_config_for_preset(
    _preset: ProviderNativeToolsPreset,
    _custom: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsConfig {
    ProviderNativeToolsConfig::default()
}

/// Existing saved drafts are presented as disabled. Provider service
/// capabilities are ordinary plugins such as `agena.openai`.
pub fn provider_native_tools_preset_from_config(
    _config: &ProviderNativeToolsConfig,
) -> ProviderNativeToolsPreset {
    ProviderNativeToolsPreset::Disabled
}

use crate::{ProviderNativeToolsConfig, ProviderNativeToolsPreset};
