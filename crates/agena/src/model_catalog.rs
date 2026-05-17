use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    AppError,
    model::{Model, ModelFamily, ModelId, ModelLifecycle},
    provider::{
        ConfiguredModelDefinition, ConfiguredModelVariant, ModelCapabilityPatch, ModelProvider,
    },
};

pub const DEFAULT_REMOTE_URL: &str =
    "https://raw.githubusercontent.com/canxin121/agena/main/catalog/model-catalog.json";
pub const DEFAULT_GITHUB_FALLBACK_URL: &str =
    "https://raw.githubusercontent.com/canxin121/agena/main/catalog/model-catalog.json";
pub const DEFAULT_CACHE_MAX_AGE_SECS: u64 = 60 * 60 * 24;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogEntrySourceKind {
    Remote,
    Fallback,
    Cache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CatalogModelDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ModelFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, ConfiguredModelVariant>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl CatalogModelDefinition {
    pub fn is_empty(&self) -> bool {
        self.family.is_none()
            && self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.description.is_none()
            && self.display_name.is_none()
            && self.variants.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn into_configured_definition(self) -> ConfiguredModelDefinition {
        ConfiguredModelDefinition {
            family: self.family,
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_output_tokens: self.max_output_tokens,
            display_name: self.display_name,
            description: self.description,
            variants: self.variants,
            capabilities: self.capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogProviderRecord {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogAdapterRecord {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
    /// Legacy adapter-rooted catalog shape. New catalog files should use
    /// `models`; this stays readable so existing remote caches and local
    /// custom catalogs can be migrated losslessly at runtime.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapters: BTreeMap<String, ModelCatalogAdapterRecord>,
    /// Legacy provider-rooted catalog shape. New catalog files should use
    /// `models`; this stays readable so existing remote caches and local
    /// custom catalogs can be migrated losslessly at runtime.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ModelCatalogProviderRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntryRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adapter_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_local_override: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ModelFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, ConfiguredModelVariant>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl ModelCatalogEntryRecord {
    pub fn definition(&self) -> CatalogModelDefinition {
        CatalogModelDefinition {
            family: self.family,
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_output_tokens: self.max_output_tokens,
            description: self.description.clone(),
            display_name: self.display_name.clone(),
            variants: self.variants.clone(),
            capabilities: self.capabilities.clone(),
        }
    }
}

impl ModelCatalogAdapterRecord {
    fn merge_from(&mut self, other: &ModelCatalogAdapterRecord) {
        for (model_id, model) in &other.models {
            self.models.insert(model_id.clone(), model.clone());
        }
    }
}

impl ModelCatalogDocument {
    pub(crate) fn model_ids(&self) -> BTreeSet<String> {
        let mut model_ids = BTreeSet::new();
        model_ids.extend(self.models.keys().cloned());
        for adapter in self.adapters.values() {
            model_ids.extend(adapter.models.keys().cloned());
        }
        for (legacy_provider_id, provider) in &self.providers {
            for model_id in provider.models.keys() {
                let (_, model_id) = split_catalog_adapter_model_ref(legacy_provider_id, model_id);
                if !model_id.is_empty() {
                    model_ids.insert(model_id);
                }
            }
        }
        model_ids
    }

    pub(crate) fn model_record(&self) -> ModelCatalogProviderRecord {
        let mut record = ModelCatalogProviderRecord {
            models: self.models.clone(),
        };

        for adapter in self.adapters.values() {
            for (model_id, definition) in &adapter.models {
                record
                    .models
                    .entry(model_id.clone())
                    .or_insert_with(|| definition.clone());
            }
        }

        for (legacy_provider_id, provider) in &self.providers {
            for (legacy_model_id, definition) in &provider.models {
                let (_, model_id) =
                    split_catalog_adapter_model_ref(legacy_provider_id, legacy_model_id);
                if model_id.is_empty() {
                    continue;
                }
                record
                    .models
                    .entry(model_id)
                    .or_insert_with(|| definition.clone());
            }
        }

        record
    }

    pub(crate) fn adapter_ids(&self) -> BTreeSet<String> {
        let mut adapter_ids = BTreeSet::new();
        adapter_ids.extend(self.adapters.keys().cloned());
        for (legacy_provider_id, provider) in &self.providers {
            for model_id in provider.models.keys() {
                let (adapter_id, _) = split_catalog_adapter_model_ref(legacy_provider_id, model_id);
                if !adapter_id.is_empty() {
                    adapter_ids.insert(adapter_id);
                }
            }
        }
        adapter_ids
    }

    pub(crate) fn adapter_record(&self, adapter_id: &str) -> Option<ModelCatalogAdapterRecord> {
        let mut record = self.adapters.get(adapter_id).cloned().unwrap_or_default();

        for (legacy_provider_id, provider) in &self.providers {
            for (legacy_model_id, definition) in &provider.models {
                let (model_adapter_id, model_id) =
                    split_catalog_adapter_model_ref(legacy_provider_id, legacy_model_id);
                if model_adapter_id != adapter_id || model_id.is_empty() {
                    continue;
                }
                record
                    .models
                    .entry(model_id)
                    .or_insert_with(|| definition.clone());
            }
        }

        (!record.models.is_empty()).then_some(record)
    }

    fn legacy_provider_record(&self, provider_id: &str) -> Option<ModelCatalogProviderRecord> {
        self.providers.get(provider_id).cloned()
    }
}

fn split_catalog_adapter_model_ref(fallback_adapter_id: &str, value: &str) -> (String, String) {
    let value = value.trim();
    if let Some((adapter_id, model_id)) = value.split_once('/') {
        let adapter_id = adapter_id.trim().to_owned();
        let model_id = model_id.trim().to_owned();
        if !adapter_id.is_empty() && !model_id.is_empty() {
            return (adapter_id, model_id);
        }
    }

    (fallback_adapter_id.trim().to_owned(), value.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogSnapshot {
    #[serde(default)]
    pub remote_url: String,
    #[serde(default)]
    pub fallback_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogEntrySourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub official: ModelCatalogDocument,
    #[serde(default)]
    pub custom: ModelCatalogDocument,
}

impl Default for ModelCatalogSnapshot {
    fn default() -> Self {
        Self {
            remote_url: DEFAULT_REMOTE_URL.to_owned(),
            fallback_url: DEFAULT_GITHUB_FALLBACK_URL.to_owned(),
            last_refresh_at: None,
            last_successful_source: None,
            last_error: None,
            official: ModelCatalogDocument::default(),
            custom: ModelCatalogDocument::default(),
        }
    }
}

impl ModelCatalogSnapshot {
    pub fn merged_models(&self) -> ModelCatalogProviderRecord {
        let mut merged = self.official.model_record();
        let custom = self.custom.model_record();
        for (model_id, definition) in custom.models {
            merged.models.insert(model_id, definition);
        }
        merged
    }

    pub fn merged_adapter(&self, adapter_id: &str) -> Option<ModelCatalogAdapterRecord> {
        let official = self.official.adapter_record(adapter_id);
        let custom = self.custom.adapter_record(adapter_id);
        if official.is_none() && custom.is_none() {
            return None;
        }

        let mut merged = official.unwrap_or_default();
        if let Some(custom) = &custom {
            merged.merge_from(custom);
        }
        Some(merged)
    }

    pub fn merged_provider_for_adapters(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Option<ModelCatalogProviderRecord> {
        let mut merged = ModelCatalogProviderRecord::default();
        let global_models = self.merged_models().models;

        for adapter_id in adapter_ids {
            for (model_id, definition) in &global_models {
                merged
                    .models
                    .insert(format!("{adapter_id}/{model_id}"), definition.clone());
            }
        }

        for adapter_id in adapter_ids {
            let Some(adapter_record) = self.merged_adapter(adapter_id.as_str()) else {
                continue;
            };
            for (model_id, definition) in adapter_record.models {
                merged
                    .models
                    .insert(format!("{adapter_id}/{model_id}"), definition);
            }
        }

        if let Some(legacy_provider) = self.legacy_merged_provider(provider_id) {
            for (model_id, definition) in legacy_provider.models {
                merged.models.insert(model_id, definition);
            }
        }

        (!merged.models.is_empty()).then_some(merged)
    }

    fn legacy_merged_provider(&self, provider_id: &str) -> Option<ModelCatalogProviderRecord> {
        let official = self.official.legacy_provider_record(provider_id);
        let custom = self.custom.legacy_provider_record(provider_id);
        if official.is_none() && custom.is_none() {
            return None;
        }

        let mut merged = official.unwrap_or_default();
        if let Some(custom) = custom {
            for (model_id, definition) in custom.models {
                merged.models.insert(model_id, definition);
            }
        }
        Some(merged)
    }

    pub fn entries(&self) -> Vec<ModelCatalogEntryRecord> {
        let mut entries = Vec::new();
        let official = self.official.model_record();
        let custom = self.custom.model_record();

        for (model_id, definition) in &official.models {
            entries.push(Self::entry_record(model_id.as_str(), definition, false));
        }

        for (model_id, definition) in &custom.models {
            entries.push(Self::entry_record(model_id.as_str(), definition, true));
        }

        entries.sort_by(|left, right| {
            left.model_id
                .cmp(&right.model_id)
                .then(left.has_local_override.cmp(&right.has_local_override))
        });
        entries
    }

    fn entry_record(
        model_id: &str,
        definition: &CatalogModelDefinition,
        has_local_override: bool,
    ) -> ModelCatalogEntryRecord {
        ModelCatalogEntryRecord {
            adapter_id: String::new(),
            model_id: model_id.to_owned(),
            display_name: definition.display_name.clone(),
            has_local_override,
            family: definition.family,
            lifecycle: definition.lifecycle,
            context_window_tokens: definition.context_window_tokens,
            max_output_tokens: definition.max_output_tokens,
            description: definition.description.clone(),
            variants: definition.variants.clone(),
            capabilities: definition.capabilities.clone(),
        }
    }

    pub fn adapter_ids(&self) -> Vec<String> {
        let mut adapter_ids = BTreeSet::new();
        adapter_ids.extend(self.official.adapter_ids());
        adapter_ids.extend(self.custom.adapter_ids());
        adapter_ids.into_iter().collect()
    }

    pub fn model_ids(&self) -> Vec<String> {
        let mut model_ids = BTreeSet::new();
        model_ids.extend(self.official.model_ids());
        model_ids.extend(self.custom.model_ids());
        model_ids.into_iter().collect()
    }

    pub fn to_response(&self) -> ModelCatalogResponse {
        ModelCatalogResponse {
            remote_url: self.remote_url.clone(),
            fallback_url: self.fallback_url.clone(),
            last_refresh_at: self.last_refresh_at,
            last_successful_source: self.last_successful_source,
            last_error: self.last_error.clone(),
            entries: self.entries(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogConfig {
    pub remote_url: String,
    pub fallback_url: String,
    pub cache_path: PathBuf,
    pub custom_path: PathBuf,
    pub cache_max_age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogResponse {
    pub remote_url: String,
    pub fallback_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogEntrySourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub entries: Vec<ModelCatalogEntryRecord>,
}

impl ModelCatalogConfig {
    pub fn for_workspace_root(workspace_root: &Path) -> Self {
        let root = workspace_root.join(".agena").join("catalog");
        Self {
            remote_url: DEFAULT_REMOTE_URL.to_owned(),
            fallback_url: DEFAULT_GITHUB_FALLBACK_URL.to_owned(),
            cache_path: root.join("model-catalog-cache.json"),
            custom_path: root.join("model-catalog-custom.json"),
            cache_max_age_secs: DEFAULT_CACHE_MAX_AGE_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedOfficialCatalog {
    #[serde(default)]
    remote_url: String,
    #[serde(default)]
    fallback_url: String,
    fetched_at_unix_ms: i64,
    source: ModelCatalogEntrySourceKind,
    document: ModelCatalogDocument,
}

#[derive(Clone)]
pub struct ModelCatalogStore {
    config: ModelCatalogConfig,
}

impl ModelCatalogStore {
    pub fn new(config: ModelCatalogConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ModelCatalogConfig {
        &self.config
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<Option<T>, AppError> {
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(path)?;
        let parsed = serde_json::from_str(&text)
            .map_err(|err| AppError::Config(format!("parse {}: {err}", path.display())))?;
        Ok(Some(parsed))
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(value).map_err(AppError::from)?;
        fs::write(path, format!("{text}\n"))?;
        Ok(())
    }

    pub fn read_custom(&self) -> Result<ModelCatalogDocument, AppError> {
        Ok(self
            .read_json(&self.config.custom_path)?
            .unwrap_or_default())
    }

    pub fn write_custom(&self, document: &ModelCatalogDocument) -> Result<(), AppError> {
        self.write_json(&self.config.custom_path, document)
    }

    fn read_cached_official(&self) -> Result<Option<CachedOfficialCatalog>, AppError> {
        self.read_json(&self.config.cache_path)
    }

    fn write_cached_official(&self, cached: &CachedOfficialCatalog) -> Result<(), AppError> {
        self.write_json(&self.config.cache_path, cached)
    }
}

#[derive(Clone)]
pub struct ModelCatalogService {
    client: reqwest::Client,
    store: ModelCatalogStore,
    state: Arc<RwLock<ModelCatalogSnapshot>>,
}

impl ModelCatalogService {
    pub fn new(client: reqwest::Client, store: ModelCatalogStore) -> Result<Self, AppError> {
        let custom = store.read_custom()?;
        let mut snapshot = ModelCatalogSnapshot {
            remote_url: store.config.remote_url.clone(),
            fallback_url: store.config.fallback_url.clone(),
            custom,
            ..ModelCatalogSnapshot::default()
        };

        if let Some(cached) = store.read_cached_official()? {
            snapshot.remote_url = cached.remote_url;
            snapshot.fallback_url = cached.fallback_url;
            snapshot.last_successful_source = Some(cached.source);
            snapshot.last_refresh_at =
                DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms);
            snapshot.official = cached.document;
        }

        Ok(Self {
            client,
            store,
            state: Arc::new(RwLock::new(snapshot)),
        })
    }

    pub fn snapshot(&self) -> ModelCatalogSnapshot {
        self.state
            .read()
            .expect("catalog state lock poisoned")
            .clone()
    }

    pub fn effective_provider_record(
        &self,
        provider_id: &str,
        adapter_ids: &[String],
    ) -> Option<ModelCatalogProviderRecord> {
        self.snapshot()
            .merged_provider_for_adapters(provider_id, adapter_ids)
    }

    pub async fn refresh_if_stale_on_startup(&self) -> Result<ModelCatalogSnapshot, AppError> {
        let snapshot = self.snapshot();
        if self.snapshot_needs_startup_refresh(&snapshot) {
            self.refresh().await
        } else {
            Ok(snapshot)
        }
    }

    pub async fn refresh(&self) -> Result<ModelCatalogSnapshot, AppError> {
        let config = self.store.config().clone();
        let remote_result = self.fetch_document(config.remote_url.as_str()).await;
        let (source, document) = match remote_result {
            Ok(document) => (ModelCatalogEntrySourceKind::Remote, document),
            Err(remote_error) => {
                let fallback_result = self.fetch_document(config.fallback_url.as_str()).await;
                match fallback_result {
                    Ok(document) => (ModelCatalogEntrySourceKind::Fallback, document),
                    Err(fallback_error) => {
                        if let Some(cached) = self.store.read_cached_official()? {
                            if self.cache_is_fresh(&cached) {
                                let mut snapshot = self.snapshot();
                                snapshot.last_error = Some(format!(
                                    "remote refresh failed: {remote_error}; fallback failed: {fallback_error}; using cache"
                                ));
                                snapshot.last_successful_source =
                                    Some(ModelCatalogEntrySourceKind::Cache);
                                snapshot.official = cached.document;
                                snapshot.last_refresh_at = DateTime::<Utc>::from_timestamp_millis(
                                    cached.fetched_at_unix_ms,
                                );
                                self.replace_snapshot(snapshot.clone());
                                return Ok(snapshot);
                            }
                        }
                        if let Ok(document) = bundled_catalog_document() {
                            let fetched_at_unix_ms = now_unix_ms();
                            self.store.write_cached_official(&CachedOfficialCatalog {
                                remote_url: config.remote_url.clone(),
                                fallback_url: config.fallback_url.clone(),
                                fetched_at_unix_ms,
                                source: ModelCatalogEntrySourceKind::Fallback,
                                document: document.clone(),
                            })?;

                            let mut snapshot = self.snapshot();
                            snapshot.remote_url = config.remote_url;
                            snapshot.fallback_url = config.fallback_url;
                            snapshot.official = document;
                            snapshot.last_successful_source =
                                Some(ModelCatalogEntrySourceKind::Fallback);
                            snapshot.last_refresh_at =
                                DateTime::<Utc>::from_timestamp_millis(fetched_at_unix_ms);
                            snapshot.last_error = Some(format!(
                                "remote refresh failed: {remote_error}; fallback failed: {fallback_error}; using bundled catalog"
                            ));
                            self.replace_snapshot(snapshot.clone());
                            return Ok(snapshot);
                        }
                        return Err(AppError::Config(format!(
                            "model catalog refresh failed: remote: {remote_error}; fallback: {fallback_error}"
                        )));
                    }
                }
            }
        };

        let fetched_at_unix_ms = now_unix_ms();
        self.store.write_cached_official(&CachedOfficialCatalog {
            remote_url: config.remote_url.clone(),
            fallback_url: config.fallback_url.clone(),
            fetched_at_unix_ms,
            source,
            document: document.clone(),
        })?;

        let mut snapshot = self.snapshot();
        snapshot.remote_url = config.remote_url;
        snapshot.fallback_url = config.fallback_url;
        snapshot.official = document;
        snapshot.last_successful_source = Some(source);
        snapshot.last_refresh_at = DateTime::<Utc>::from_timestamp_millis(fetched_at_unix_ms);
        snapshot.last_error = None;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub fn upsert_custom_entry(
        &self,
        model_id: impl Into<String>,
        definition: CatalogModelDefinition,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let model_id = model_id.into();
        let mut snapshot = self.snapshot();
        snapshot.custom.models.insert(model_id, definition);
        self.store.write_custom(&snapshot.custom)?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub fn remove_custom_entry(&self, model_id: &str) -> Result<ModelCatalogSnapshot, AppError> {
        let mut snapshot = self.snapshot();
        snapshot.custom.models.remove(model_id);

        let mut empty_adapters = Vec::new();
        for (adapter_id, adapter) in &mut snapshot.custom.adapters {
            adapter.models.remove(model_id);
            if adapter.models.is_empty() {
                empty_adapters.push(adapter_id.clone());
            }
        }
        for adapter_id in empty_adapters {
            snapshot.custom.adapters.remove(adapter_id.as_str());
        }

        let mut empty_legacy_providers = Vec::new();
        for (legacy_provider_id, provider) in &mut snapshot.custom.providers {
            let legacy_keys = provider
                .models
                .keys()
                .filter(|legacy_model_id| {
                    let (_, legacy_model_name) =
                        split_catalog_adapter_model_ref(legacy_provider_id, legacy_model_id);
                    legacy_model_name == model_id
                })
                .cloned()
                .collect::<Vec<_>>();
            for legacy_key in legacy_keys {
                provider.models.remove(legacy_key.as_str());
            }
            if provider.models.is_empty() {
                empty_legacy_providers.push(legacy_provider_id.clone());
            }
        }
        for provider_id in empty_legacy_providers {
            snapshot.custom.providers.remove(provider_id.as_str());
        }
        self.store.write_custom(&snapshot.custom)?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    async fn fetch_document(&self, url: &str) -> Result<ModelCatalogDocument, AppError> {
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(AppError::Config(format!(
                "GET {url} returned {}",
                response.status()
            )));
        }
        let text = response.text().await?;
        serde_json::from_str::<ModelCatalogDocument>(&text)
            .map_err(|err| AppError::Config(format!("parse model catalog from {url}: {err}")))
    }

    fn cache_is_fresh(&self, cached: &CachedOfficialCatalog) -> bool {
        let fetched = UNIX_EPOCH + Duration::from_millis(cached.fetched_at_unix_ms.max(0) as u64);
        SystemTime::now()
            .duration_since(fetched)
            .map(|age| age.as_secs() <= self.store.config.cache_max_age_secs)
            .unwrap_or(false)
    }

    fn snapshot_needs_startup_refresh(&self, snapshot: &ModelCatalogSnapshot) -> bool {
        if snapshot.official.model_ids().is_empty() {
            return true;
        }

        let Some(last_refresh_at) = snapshot.last_refresh_at else {
            return true;
        };

        match SystemTime::now().duration_since(last_refresh_at.into()) {
            Ok(age) => age.as_secs() > self.store.config.cache_max_age_secs,
            Err(_) => false,
        }
    }

    fn replace_snapshot(&self, snapshot: ModelCatalogSnapshot) {
        let mut guard = self.state.write().expect("catalog state lock poisoned");
        *guard = snapshot;
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

pub fn catalog_definition_to_provider_definition(
    definition: &CatalogModelDefinition,
) -> ConfiguredModelDefinition {
    definition.clone().into_configured_definition()
}

pub fn bundled_catalog_document() -> Result<ModelCatalogDocument, AppError> {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../catalog/model-catalog.json"
    )))
    .map_err(|err| AppError::Config(format!("parse bundled model catalog: {err}")))
}

pub fn catalog_family_for_model(
    provider: &ModelCatalogProviderRecord,
    model: &ModelId,
) -> Option<ModelFamily> {
    provider
        .models
        .get(model.as_str())
        .and_then(|entry| entry.family)
}

pub fn decorate_provider_models(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    mut models: Vec<Model>,
) -> Vec<Model> {
    let mut listed = BTreeSet::new();

    for model in &mut models {
        listed.insert(model.id.to_string());
        *model =
            decorate_provider_model(provider, provider_record, model.id.clone(), model.clone());
    }

    for model_id in provider_record.models.keys() {
        if listed.contains(model_id.as_str()) {
            continue;
        }

        let model_id = ModelId::new(model_id.clone());
        let base = Model::new(provider.id(), model_id.as_str())
            .with_capabilities(provider.model_capabilities(&model_id))
            .with_metadata(provider_model_metadata(
                provider,
                provider_record,
                &model_id,
            ))
            .with_variants(provider_model_variants(
                provider,
                provider_record,
                &model_id,
            ));
        models.push(decorate_provider_model(
            provider,
            provider_record,
            model_id,
            base,
        ));
    }

    models
}

fn decorate_provider_model(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    model_id: ModelId,
    mut model: Model,
) -> Model {
    if let Some(display_name) = provider_record
        .models
        .get(model_id.as_str())
        .and_then(|definition| definition.display_name.clone())
    {
        model.display_name = Some(display_name);
    }

    if let Some(configured) = provider_record
        .models
        .get(model_id.as_str())
        .map(catalog_definition_to_provider_definition)
    {
        configured.apply_to_model(
            model,
            &provider.model_capabilities(&model_id),
            &provider_model_metadata(provider, provider_record, &model_id),
        )
    } else {
        model
    }
}

fn provider_model_metadata(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    model: &ModelId,
) -> crate::model::ModelMetadata {
    let mut metadata = provider.model_metadata(model);
    if let Some(family) = provider_record
        .models
        .get(model.as_str())
        .and_then(|definition| definition.family)
    {
        metadata.family = Some(family);
    }
    metadata
}

fn provider_model_variants(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelVariant> {
    let mut variants = provider.model_variants(model);
    if let Some(configured) = provider_record
        .models
        .get(model.as_str())
        .map(catalog_definition_to_provider_definition)
    {
        for (name, configured_variant) in &configured.variants {
            match configured_variant.apply_to_variant(variants.get(name)) {
                Some(variant) => {
                    variants.insert(name.clone(), variant);
                }
                None => {
                    variants.remove(name);
                }
            }
        }
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::CapabilitySupport, provider::ConfiguredModelVariant};
    use mockito::Server;
    use tempfile::tempdir;

    #[test]
    fn merged_adapter_prefers_custom_models() {
        let snapshot = ModelCatalogSnapshot {
            official: ModelCatalogDocument {
                adapters: BTreeMap::from([(
                    "openai".to_owned(),
                    ModelCatalogAdapterRecord {
                        models: BTreeMap::from([(
                            "gpt-5".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("GPT-5".to_owned()),
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
                ..ModelCatalogDocument::default()
            },
            custom: ModelCatalogDocument {
                adapters: BTreeMap::from([(
                    "openai".to_owned(),
                    ModelCatalogAdapterRecord {
                        models: BTreeMap::from([(
                            "gpt-5-custom".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("GPT-5 Custom".to_owned()),
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
                ..ModelCatalogDocument::default()
            },
            ..ModelCatalogSnapshot::default()
        };

        let merged = snapshot
            .merged_adapter("openai")
            .expect("adapter should exist");
        assert!(merged.models.contains_key("gpt-5"));
        assert!(merged.models.contains_key("gpt-5-custom"));
    }

    #[test]
    fn entries_keep_official_and_custom_records_separate() {
        let snapshot = ModelCatalogSnapshot {
            last_successful_source: Some(ModelCatalogEntrySourceKind::Remote),
            official: ModelCatalogDocument {
                adapters: BTreeMap::from([(
                    "anthropic".to_owned(),
                    ModelCatalogAdapterRecord {
                        models: BTreeMap::from([(
                            "claude-sonnet".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("Claude Sonnet".to_owned()),
                                capabilities: ModelCapabilityPatch {
                                    reasoning: Some(CapabilitySupport::Supported),
                                    ..ModelCapabilityPatch::default()
                                },
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
                ..ModelCatalogDocument::default()
            },
            custom: ModelCatalogDocument {
                adapters: BTreeMap::from([(
                    "anthropic".to_owned(),
                    ModelCatalogAdapterRecord {
                        models: BTreeMap::from([(
                            "claude-sonnet".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("Claude Sonnet Local".to_owned()),
                                variants: BTreeMap::from([(
                                    "deep".to_owned(),
                                    ConfiguredModelVariant {
                                        display_name: Some("Deep".to_owned()),
                                        description: None,
                                        thinking: None,
                                        disabled: false,
                                    },
                                )]),
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
                ..ModelCatalogDocument::default()
            },
            ..ModelCatalogSnapshot::default()
        };

        let entries = snapshot.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].adapter_id, "");
        assert_eq!(entries[0].model_id, "claude-sonnet");
        assert_eq!(entries[0].display_name.as_deref(), Some("Claude Sonnet"));
        assert!(!entries[0].has_local_override);
        assert_eq!(entries[1].adapter_id, "");
        assert_eq!(entries[1].model_id, "claude-sonnet");
        assert_eq!(
            entries[1].display_name.as_deref(),
            Some("Claude Sonnet Local")
        );
        assert!(entries[1].has_local_override);
        assert!(entries[1].variants.contains_key("deep"));
    }

    #[test]
    fn bundled_fallback_catalog_file_parses_and_seeds_known_models() {
        let document = bundled_catalog_document().expect("bundled fallback catalog should parse");

        let catalog = document.model_record();
        assert!(catalog.models.contains_key("gpt-5.5"));
        assert!(catalog.models.contains_key("gpt-5.5-pro"));
        assert!(catalog.models.contains_key("claude-opus-4-7"));
        assert!(catalog.models.contains_key("claude-opus-4-6"));
        assert!(catalog.models.contains_key("gemini-3.1-flash-lite"));
        assert!(catalog.models.contains_key("amazon.nova-pro-v1:0"));
        assert!(catalog.models.contains_key("duo-chat-sonnet-4-5"));
    }

    #[tokio::test]
    async fn startup_refresh_reuses_fresh_cached_catalog() {
        let dir = tempdir().expect("tempdir should create");
        let store = ModelCatalogStore::new(ModelCatalogConfig {
            remote_url: "https://example.invalid/catalog.json".to_owned(),
            fallback_url: "https://example.invalid/fallback.json".to_owned(),
            cache_path: dir.path().join("model-catalog-cache.json"),
            custom_path: dir.path().join("model-catalog-custom.json"),
            cache_max_age_secs: 60,
        });
        let cached_document = model_catalog_document("openai", "cached-model");
        store
            .write_cached_official(&CachedOfficialCatalog {
                remote_url: store.config().remote_url.clone(),
                fallback_url: store.config().fallback_url.clone(),
                fetched_at_unix_ms: now_unix_ms(),
                source: ModelCatalogEntrySourceKind::Remote,
                document: cached_document.clone(),
            })
            .expect("cache should be written");

        let service =
            ModelCatalogService::new(reqwest::Client::new(), store).expect("service should load");

        let snapshot = service
            .refresh_if_stale_on_startup()
            .await
            .expect("fresh startup snapshot should succeed");

        assert_eq!(snapshot.official, cached_document);
        assert_eq!(
            snapshot.last_successful_source,
            Some(ModelCatalogEntrySourceKind::Remote)
        );
    }

    #[tokio::test]
    async fn startup_refresh_updates_stale_cached_catalog() {
        let dir = tempdir().expect("tempdir should create");
        let mut server = Server::new_async().await;
        let remote_document = model_catalog_document("openai", "fresh-model");
        let remote_body =
            serde_json::to_string(&remote_document).expect("remote document should serialize");
        let remote_mock = server
            .mock("GET", "/catalog.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(remote_body)
            .create();

        let store = ModelCatalogStore::new(ModelCatalogConfig {
            remote_url: format!("{}/catalog.json", server.url()),
            fallback_url: format!("{}/fallback.json", server.url()),
            cache_path: dir.path().join("model-catalog-cache.json"),
            custom_path: dir.path().join("model-catalog-custom.json"),
            cache_max_age_secs: 1,
        });
        store
            .write_cached_official(&CachedOfficialCatalog {
                remote_url: store.config().remote_url.clone(),
                fallback_url: store.config().fallback_url.clone(),
                fetched_at_unix_ms: now_unix_ms() - 5_000,
                source: ModelCatalogEntrySourceKind::Remote,
                document: model_catalog_document("openai", "stale-model"),
            })
            .expect("stale cache should be written");

        let service =
            ModelCatalogService::new(reqwest::Client::new(), store).expect("service should load");

        let snapshot = service
            .refresh_if_stale_on_startup()
            .await
            .expect("stale startup refresh should succeed");

        remote_mock.assert();
        assert_eq!(snapshot.official, remote_document);
        assert_eq!(
            snapshot.last_successful_source,
            Some(ModelCatalogEntrySourceKind::Remote)
        );
    }

    fn model_catalog_document(adapter_id: &str, model_id: &str) -> ModelCatalogDocument {
        ModelCatalogDocument {
            adapters: BTreeMap::from([(
                adapter_id.to_owned(),
                ModelCatalogAdapterRecord {
                    models: BTreeMap::from([(
                        model_id.to_owned(),
                        CatalogModelDefinition {
                            display_name: Some(model_id.to_owned()),
                            ..CatalogModelDefinition::default()
                        },
                    )]),
                },
            )]),
            ..ModelCatalogDocument::default()
        }
    }
}
