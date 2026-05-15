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
    "https://raw.githubusercontent.com/agena-ai/model-catalog/main/catalog.json";
pub const DEFAULT_GITHUB_FALLBACK_URL: &str =
    "https://raw.githubusercontent.com/agena-ai/agena/main/catalog/model-catalog.json";
const DEFAULT_CACHE_MAX_AGE_SECS: u64 = 60 * 60 * 24;

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
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_output_tokens: self.max_output_tokens,
            description: self.description,
            variants: self.variants,
            capabilities: self.capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogProviderRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ModelCatalogProviderRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogEntryRecord {
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_for_provider: Option<String>,
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
    pub fn merged_provider(&self, provider_id: &str) -> Option<ModelCatalogProviderRecord> {
        let official = self.official.providers.get(provider_id);
        let custom = self.custom.providers.get(provider_id);
        if official.is_none() && custom.is_none() {
            return None;
        }

        let mut merged = official.cloned().unwrap_or_default();
        if let Some(custom) = custom {
            if let Some(default_model) = custom.default_model.clone() {
                merged.default_model = Some(default_model);
            }
            for (model_id, model) in &custom.models {
                merged.models.insert(model_id.clone(), model.clone());
            }
        }
        Some(merged)
    }

    pub fn entries(&self) -> Vec<ModelCatalogEntryRecord> {
        let mut entries = Vec::new();
        let mut provider_ids = BTreeSet::new();
        provider_ids.extend(self.official.providers.keys().cloned());
        provider_ids.extend(self.custom.providers.keys().cloned());

        for provider_id in provider_ids {
            let Some(provider) = self.merged_provider(provider_id.as_str()) else {
                continue;
            };
            let merged_default = provider.default_model.clone();
            let custom_provider = self.custom.providers.get(provider_id.as_str());

            for (model_id, definition) in provider.models {
                entries.push(ModelCatalogEntryRecord {
                    provider_id: provider_id.clone(),
                    model_id: model_id.clone(),
                    default_model_for_provider: merged_default.clone(),
                    display_name: definition.display_name,
                    has_local_override: custom_provider
                        .and_then(|provider| provider.models.get(model_id.as_str()))
                        .is_some(),
                    family: definition.family,
                    lifecycle: definition.lifecycle,
                    context_window_tokens: definition.context_window_tokens,
                    max_output_tokens: definition.max_output_tokens,
                    description: definition.description,
                    variants: definition.variants,
                    capabilities: definition.capabilities,
                });
            }
        }

        entries.sort_by(|left, right| {
            left.provider_id
                .cmp(&right.provider_id)
                .then(left.model_id.cmp(&right.model_id))
        });
        entries
    }

    pub fn provider_ids(&self) -> Vec<String> {
        let mut provider_ids = BTreeSet::new();
        provider_ids.extend(self.official.providers.keys().cloned());
        provider_ids.extend(self.custom.providers.keys().cloned());
        provider_ids.into_iter().collect()
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
    ) -> Option<ModelCatalogProviderRecord> {
        self.snapshot().merged_provider(provider_id)
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
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        definition: CatalogModelDefinition,
        set_default_for_provider: bool,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        let mut snapshot = self.snapshot();
        let provider = snapshot.custom.providers.entry(provider_id).or_default();
        provider.models.insert(model_id.clone(), definition);
        if set_default_for_provider {
            provider.default_model = Some(model_id);
        }
        self.store.write_custom(&snapshot.custom)?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub fn set_provider_default_model(
        &self,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        let mut snapshot = self.snapshot();
        let provider = snapshot.custom.providers.entry(provider_id).or_default();
        provider.default_model = Some(model_id);
        self.store.write_custom(&snapshot.custom)?;
        self.replace_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    pub fn remove_custom_entry(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let mut snapshot = self.snapshot();
        if let Some(provider) = snapshot.custom.providers.get_mut(provider_id) {
            provider.models.remove(model_id);
            if provider.default_model.as_deref() == Some(model_id) {
                provider.default_model = None;
            }
            if provider.default_model.is_none() && provider.models.is_empty() {
                snapshot.custom.providers.remove(provider_id);
            }
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
        *model = decorate_provider_model(provider, provider_record, model.id.clone(), model.clone());
    }

    for model_id in provider_record.models.keys() {
        if listed.contains(model_id.as_str()) {
            continue;
        }

        let model_id = ModelId::new(model_id.clone());
        let base = Model::new(provider.id(), model_id.as_str())
            .with_capabilities(provider.model_capabilities(&model_id))
            .with_metadata(provider_model_metadata(provider, provider_record, &model_id))
            .with_variants(provider_model_variants(provider, provider_record, &model_id));
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

    #[test]
    fn merged_provider_prefers_custom_default_and_models() {
        let snapshot = ModelCatalogSnapshot {
            official: ModelCatalogDocument {
                providers: BTreeMap::from([(
                    "openai".to_owned(),
                    ModelCatalogProviderRecord {
                        default_model: Some("gpt-5".to_owned()),
                        models: BTreeMap::from([(
                            "gpt-5".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("GPT-5".to_owned()),
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
            },
            custom: ModelCatalogDocument {
                providers: BTreeMap::from([(
                    "openai".to_owned(),
                    ModelCatalogProviderRecord {
                        default_model: Some("gpt-5-custom".to_owned()),
                        models: BTreeMap::from([(
                            "gpt-5-custom".to_owned(),
                            CatalogModelDefinition {
                                display_name: Some("GPT-5 Custom".to_owned()),
                                ..CatalogModelDefinition::default()
                            },
                        )]),
                    },
                )]),
            },
            ..ModelCatalogSnapshot::default()
        };

        let merged = snapshot
            .merged_provider("openai")
            .expect("provider should exist");
        assert_eq!(merged.default_model.as_deref(), Some("gpt-5-custom"));
        assert!(merged.models.contains_key("gpt-5"));
        assert!(merged.models.contains_key("gpt-5-custom"));
    }

    #[test]
    fn entries_merge_overrides_into_single_record() {
        let snapshot = ModelCatalogSnapshot {
            last_successful_source: Some(ModelCatalogEntrySourceKind::Remote),
            official: ModelCatalogDocument {
                providers: BTreeMap::from([(
                    "anthropic".to_owned(),
                    ModelCatalogProviderRecord {
                        default_model: Some("claude-sonnet".to_owned()),
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
            },
            custom: ModelCatalogDocument {
                providers: BTreeMap::from([(
                    "anthropic".to_owned(),
                    ModelCatalogProviderRecord {
                        default_model: None,
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
            },
            ..ModelCatalogSnapshot::default()
        };

        let entries = snapshot.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider_id, "anthropic");
        assert_eq!(entries[0].model_id, "claude-sonnet");
        assert_eq!(
            entries[0].display_name.as_deref(),
            Some("Claude Sonnet Local")
        );
        assert!(entries[0].has_local_override);
        assert_eq!(
            entries[0].default_model_for_provider.as_deref(),
            Some("claude-sonnet")
        );
        assert!(entries[0].variants.contains_key("deep"));
    }

    #[test]
    fn bundled_fallback_catalog_file_parses_and_seeds_known_providers() {
        let document = bundled_catalog_document().expect("bundled fallback catalog should parse");

        let provider_ids = document.providers.keys().cloned().collect::<Vec<_>>();
        assert!(provider_ids.iter().any(|id| id == "openai"));
        assert!(provider_ids.iter().any(|id| id == "anthropic"));
        assert!(provider_ids.iter().any(|id| id == "gemini"));
        assert!(provider_ids.iter().any(|id| id == "gitlab"));

        let gitlab = document
            .providers
            .get("gitlab")
            .expect("gitlab provider should exist");
        assert!(gitlab.models.contains_key("gitlab/duo-chat-sonnet-4-5"));
    }
}
