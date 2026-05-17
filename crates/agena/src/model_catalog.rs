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
    model::{Model, ModelId, ModelLifecycle},
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
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.description.is_none()
            && self.display_name.is_none()
            && self.variants.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn into_configured_definition(self) -> ConfiguredModelDefinition {
        ConfiguredModelDefinition {
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
#[serde(deny_unknown_fields)]
pub struct ModelCatalogDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntryRecord {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_local_override: bool,
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

impl ModelCatalogDocument {
    pub(crate) fn model_ids(&self) -> BTreeSet<String> {
        self.models.keys().cloned().collect()
    }

    pub(crate) fn model_record(&self) -> ModelCatalogProviderRecord {
        ModelCatalogProviderRecord {
            models: self.models.clone(),
        }
    }
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
            model_id: model_id.to_owned(),
            display_name: definition.display_name.clone(),
            has_local_override,
            lifecycle: definition.lifecycle,
            context_window_tokens: definition.context_window_tokens,
            max_output_tokens: definition.max_output_tokens,
            description: definition.description.clone(),
            variants: definition.variants.clone(),
            capabilities: definition.capabilities.clone(),
        }
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
        _adapter_ids: &[String],
    ) -> Option<ModelCatalogProviderRecord> {
        let record = self.snapshot().merged_models();
        (!record.models.is_empty()).then_some(record)
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
            .with_capabilities(provider.model_capabilities_for_adapter(None, &model_id))
            .with_metadata(provider_model_metadata(provider, None, &model_id))
            .with_variants(provider_model_variants(
                provider,
                provider_record,
                None,
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
        let adapter_id = model.adapter_id.clone();
        configured.apply_to_model(
            model,
            &provider.model_capabilities_for_adapter(adapter_id.as_ref(), &model_id),
            &provider_model_metadata(provider, adapter_id.as_ref(), &model_id),
        )
    } else {
        model
    }
}

fn provider_model_metadata(
    provider: &dyn ModelProvider,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> crate::model::ModelMetadata {
    provider.model_metadata_for_adapter(adapter_id, model)
}

fn provider_model_variants(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelVariant> {
    let mut variants = provider.model_variants_for_adapter(adapter_id, model);
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
    fn merged_models_prefers_custom_models() {
        let snapshot = ModelCatalogSnapshot {
            official: ModelCatalogDocument {
                models: BTreeMap::from([(
                    "gpt-5".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("GPT-5".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                )]),
            },
            custom: ModelCatalogDocument {
                models: BTreeMap::from([(
                    "gpt-5-custom".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("GPT-5 Custom".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                )]),
            },
            ..ModelCatalogSnapshot::default()
        };

        let merged = snapshot.merged_models();
        assert!(merged.models.contains_key("gpt-5"));
        assert!(merged.models.contains_key("gpt-5-custom"));
    }

    #[test]
    fn entries_keep_official_and_custom_records_separate() {
        let snapshot = ModelCatalogSnapshot {
            last_successful_source: Some(ModelCatalogEntrySourceKind::Remote),
            official: ModelCatalogDocument {
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
            custom: ModelCatalogDocument {
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
            ..ModelCatalogSnapshot::default()
        };

        let entries = snapshot.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model_id, "claude-sonnet");
        assert_eq!(entries[0].display_name.as_deref(), Some("Claude Sonnet"));
        assert!(!entries[0].has_local_override);
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

        let mut lowered = BTreeSet::new();
        for model_id in catalog.models.keys() {
            assert!(
                !model_id.contains('/'),
                "bundled catalog model id should not contain '/': {model_id}"
            );
            assert!(
                lowered.insert(model_id.to_ascii_lowercase()),
                "bundled catalog should not contain case-insensitive duplicate model ids: {model_id}"
            );
        }
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
        let cached_document = model_catalog_document("cached-model");
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
        let remote_document = model_catalog_document("fresh-model");
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
                document: model_catalog_document("stale-model"),
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

    fn model_catalog_document(model_id: &str) -> ModelCatalogDocument {
        ModelCatalogDocument {
            models: BTreeMap::from([(
                model_id.to_owned(),
                CatalogModelDefinition {
                    display_name: Some(model_id.to_owned()),
                    ..CatalogModelDefinition::default()
                },
            )]),
        }
    }
}
