use anyhow::anyhow;

pub(super) fn parse_snapshot_payload(
    payload: Option<serde_json::Value>,
) -> Result<SnapshotCommandOutput> {
    let payload = payload.ok_or_else(|| anyhow!("snapshot tool returned no payload"))?;
    serde_json::from_value(payload).map_err(|error| anyhow!(error.to_string()))
}

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

use crate::Result;
use crate::{ProviderNativeToolsConfig, ProviderNativeToolsPreset, SnapshotCommandOutput};
