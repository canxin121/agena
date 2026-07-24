//! Runtime-owned plugin/provider composition policy.

use agena_plugin_host::{PluginHost, ProviderDescriptor, ProviderListInput, ProviderListPatch};

impl ProviderListPatchTarget for agena_runtime_provider::provider::ProviderRegistry {
    type Error = agena_runtime_provider::ProviderError;

    fn remove_provider(&mut self, provider_id: &str) {
        self.remove(provider_id);
    }

    fn add_provider(&mut self, descriptor: ProviderDescriptor) -> Result<(), Self::Error> {
        self.register_plugin_provider(descriptor)
    }
}

/// Concrete registry adapter required to apply plugin-provided provider-list
/// changes. The registry implementation remains outside Runtime; Runtime
/// owns the patch ordering and remove-before-add policy.
pub trait ProviderListPatchTarget {
    type Error;

    fn remove_provider(&mut self, provider_id: &str);
    fn add_provider(&mut self, descriptor: ProviderDescriptor) -> Result<(), Self::Error>;
}

/// Project concrete provider IDs into the host-facing descriptor shape used
/// by the plugin provider-list hook.
pub fn provider_descriptors_from_ids<I>(ids: I) -> Vec<ProviderDescriptor>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    ids.into_iter()
        .map(|id| {
            let id = id.into();
            ProviderDescriptor {
                display_name: id.clone(),
                id,
                models: Vec::new(),
                endpoint: None,
                kind: agena_plugin_host::ProviderKind::Custom,
            }
        })
        .collect()
}

/// Ask configured plugins to amend the provider list.
///
/// The concrete provider registry remains process-specific while it still
/// contains provider adapters. Runtime owns the host dispatch rule, including the
/// no-plugin fast path and the host-facing descriptor boundary.
pub async fn dispatch_provider_list_patch(
    plugins: &PluginHost,
    current: Vec<ProviderDescriptor>,
) -> Result<Option<ProviderListPatch>, agena_plugin_host::PluginError> {
    if plugins.is_empty() {
        return Ok(None);
    }
    plugins
        .dispatch_provider_list(ProviderListInput { current })
        .await
        .map(Some)
}

pub fn apply_provider_list_patch<T>(
    target: &mut T,
    patch: ProviderListPatch,
) -> Result<(), T::Error>
where
    T: ProviderListPatchTarget,
{
    for provider_id in patch.remove {
        target.remove_provider(provider_id.as_ref());
    }
    for descriptor in patch.add {
        target.add_provider(descriptor)?;
    }
    Ok(())
}
