//! Host-owned resolution of declared plugin service seams.
//!
//! A service import is not ambient discovery. The host resolves it exactly once
//! from immutable prefetched manifests, records the provider binding, feeds the
//! binding into activation ordering, and rejects every invocation that does not
//! match that binding.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::sdk::{PluginManifest, PluginServiceImport, PluginServiceMethod};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PluginServiceBindingKey {
    pub consumer: String,
    pub service: String,
    pub api_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginServiceBinding {
    pub consumer: String,
    pub provider: String,
    pub service: String,
    pub api_version: u32,
    pub optional: bool,
    pub methods: BTreeMap<String, PluginServiceMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginServiceResolutionBlock {
    pub plugin_id: String,
    pub code: &'static str,
    pub message: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginServiceResolutionPlan {
    pub bindings: BTreeMap<PluginServiceBindingKey, PluginServiceBinding>,
    pub activation_dependencies: BTreeMap<String, BTreeSet<String>>,
    pub blocked: BTreeMap<String, PluginServiceResolutionBlock>,
}

pub fn resolve_plugin_services(
    manifests: &BTreeMap<String, PluginManifest>,
) -> PluginServiceResolutionPlan {
    let mut exporters = BTreeMap::<(String, u32), BTreeSet<String>>::new();
    for (plugin_id, manifest) in manifests {
        for export in &manifest.services.exports {
            exporters
                .entry((export.id.clone(), export.api_version))
                .or_default()
                .insert(plugin_id.clone());
        }
    }

    let mut plan = PluginServiceResolutionPlan::default();
    for (consumer, manifest) in manifests {
        for import in &manifest.services.imports {
            resolve_import(consumer, import, manifests, &exporters, &mut plan);
        }
    }
    plan
}

fn resolve_import(
    consumer: &str,
    import: &PluginServiceImport,
    manifests: &BTreeMap<String, PluginManifest>,
    exporters: &BTreeMap<(String, u32), BTreeSet<String>>,
    plan: &mut PluginServiceResolutionPlan,
) {
    let candidates = exporters
        .get(&(import.id.clone(), import.api_version))
        .cloned()
        .unwrap_or_default();

    let provider = if let Some(explicit) = &import.provider {
        if explicit == consumer {
            block(
                plan,
                consumer,
                "service_self_dependency",
                format!(
                    "service import `{}` API v{} cannot bind plugin `{consumer}` to itself",
                    import.id, import.api_version
                ),
                vec![explicit.clone()],
            );
            return;
        }
        match manifests.get(explicit) {
            None if import.optional => return,
            None => {
                block(
                    plan,
                    consumer,
                    "service_provider_missing",
                    format!(
                        "required service `{}` API v{} pins missing provider `{explicit}`",
                        import.id, import.api_version
                    ),
                    vec![explicit.clone()],
                );
                return;
            }
            Some(_) if !candidates.contains(explicit) => {
                block(
                    plan,
                    consumer,
                    "service_provider_incompatible",
                    format!(
                        "provider `{explicit}` does not export service `{}` API v{}",
                        import.id, import.api_version
                    ),
                    vec![explicit.clone()],
                );
                return;
            }
            Some(_) => explicit.clone(),
        }
    } else {
        let usable = candidates
            .into_iter()
            .filter(|candidate| candidate != consumer)
            .collect::<Vec<_>>();
        match usable.as_slice() {
            [] if import.optional => return,
            [] => {
                block(
                    plan,
                    consumer,
                    "service_provider_missing",
                    format!(
                        "required service `{}` API v{} has no provider",
                        import.id, import.api_version
                    ),
                    Vec::new(),
                );
                return;
            }
            [provider] => provider.clone(),
            providers => {
                block(
                    plan,
                    consumer,
                    "service_provider_ambiguous",
                    format!(
                        "service `{}` API v{} has multiple providers: {}; pin one with `provider`",
                        import.id,
                        import.api_version,
                        providers
                            .iter()
                            .map(|provider| format!("`{provider}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    providers.to_vec(),
                );
                return;
            }
        }
    };

    let key = PluginServiceBindingKey {
        consumer: consumer.to_string(),
        service: import.id.clone(),
        api_version: import.api_version,
    };
    let methods =
        manifests
            .get(&provider)
            .and_then(|manifest| {
                manifest.services.exports.iter().find(|export| {
                    export.id == import.id && export.api_version == import.api_version
                })
            })
            .map(|export| {
                export
                    .methods
                    .iter()
                    .cloned()
                    .map(|method| (method.id.clone(), method))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
    plan.bindings.insert(
        key,
        PluginServiceBinding {
            consumer: consumer.to_string(),
            provider: provider.clone(),
            service: import.id.clone(),
            api_version: import.api_version,
            optional: import.optional,
            methods,
        },
    );
    // A resolved optional service is optional only with respect to absence.
    // Once bound, its provider must initialize first so the consumer can call
    // it safely from meta/init.
    plan.activation_dependencies
        .entry(consumer.to_string())
        .or_default()
        .insert(provider);
}

fn block(
    plan: &mut PluginServiceResolutionPlan,
    plugin_id: &str,
    code: &'static str,
    message: String,
    dependencies: Vec<String>,
) {
    // Keep the first declaration-order diagnostic stable. The full manifest is
    // still available in inspect for any additional imports.
    plan.blocked
        .entry(plugin_id.to_string())
        .or_insert_with(|| PluginServiceResolutionBlock {
            plugin_id: plugin_id.to_string(),
            code,
            message,
            dependencies,
        });
}
