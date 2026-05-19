use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};

mod curate;
mod decorate;
mod merge;
mod service;
mod sources;
mod store;
mod types;

pub use decorate::{catalog_definition_to_provider_definition, decorate_provider_models};
pub use service::ModelCatalogService;
pub use store::ModelCatalogStore;
pub use types::{
    CatalogModelDefinition, ModelCatalogConfig, ModelCatalogDocument, ModelCatalogEntryRecord,
    ModelCatalogEntrySourceKind, ModelCatalogProviderRecord, ModelCatalogResponse,
    ModelCatalogSnapshot,
};

#[cfg(test)]
use store::CachedOfficialCatalog;

use crate::{
    AppError,
    config::{ConfigResolution, ProviderAdapterDefinition, ProviderCapabilityFamilyConfig},
    db::entities::{model_catalog_entry, model_catalog_state},
    model::{
        CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality, ModelLifecycle,
        ModelPricing,
    },
    provider::{
        ConfiguredModelDefinition, ConfiguredModelSpeedMode, ConfiguredModelThinkingMode,
        FeatureCapabilityPatch, FeatureCapabilityPatchBody, InputCapabilityPatch,
        InputCapabilityPatchBody, ModelCapabilityFeature, ModelCapabilityPatch, ModelProvider,
        ProviderRegistry,
    },
};

pub const DEFAULT_CACHE_MAX_AGE_SECS: u64 = 60 * 60 * 24 * 7;

const CATALOG_KIND_OFFICIAL: &str = "official";
const CATALOG_KIND_CUSTOM: &str = "custom";
const CATALOG_STATE_ID: i32 = 1;

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn format_catalog_source(source: ModelCatalogEntrySourceKind) -> String {
    match source {
        ModelCatalogEntrySourceKind::Generated => "generated",
        ModelCatalogEntrySourceKind::Cache => "cache",
    }
    .to_owned()
}

fn parse_catalog_source(value: &str) -> Result<ModelCatalogEntrySourceKind, AppError> {
    match value {
        "generated" => Ok(ModelCatalogEntrySourceKind::Generated),
        "cache" => Ok(ModelCatalogEntrySourceKind::Cache),
        other => Err(AppError::Config(format!(
            "invalid model catalog cache source `{other}`"
        ))),
    }
}

fn model_catalog_definition_search_text(
    model_id: &str,
    definition: &CatalogModelDefinition,
) -> String {
    let definition_text = serde_json::to_string(definition).unwrap_or_default();
    [
        model_id,
        definition.display_name.as_deref().unwrap_or_default(),
        definition.origin.as_deref().unwrap_or_default(),
        definition.description.as_deref().unwrap_or_default(),
        definition_text.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

fn default_remote_sources() -> Vec<sources::ModelCatalogRemoteSource> {
    if public_catalog_sources_disabled() {
        Vec::new()
    } else {
        sources::default_public_sources()
    }
}

fn public_catalog_sources_disabled() -> bool {
    std::env::var_os("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES")
        .map(|value| {
            matches!(
                value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn canonical_model_catalog_id(model_id: &str) -> String {
    curate::normalized_catalog_model_id(model_id)
}
use merge::{
    catalog_definition_from_model, merge_catalog_definition, merge_catalog_document,
    merge_public_source_catalog_document, provider_priority,
};

#[cfg(test)]
use merge::{
    merge_catalog_speed_mode, merge_catalog_speed_mode_fill_missing, merge_catalog_thinking_mode,
};

#[cfg(test)]
mod tests;
