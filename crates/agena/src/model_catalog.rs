use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod curate;
mod sources;

use crate::{
    AppError,
    config::{ConfigResolution, ProviderAdapterDefinition, ProviderCapabilityFamilyConfig},
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

pub const DEFAULT_CACHE_MAX_AGE_SECS: u64 = 60 * 60 * 24;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogEntrySourceKind {
    Generated,
    Cache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CatalogModelDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub thinking_modes: BTreeMap<String, ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl CatalogModelDefinition {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_none()
            && self.context_window_tokens.is_none()
            && self.max_input_tokens.is_none()
            && self.max_output_tokens.is_none()
            && self.description.is_none()
            && self.knowledge_cutoff.is_none()
            && self.release_date.is_none()
            && self.last_updated.is_none()
            && self.open_weights.is_none()
            && self.default_thinking_mode.is_none()
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
            && self.display_name.is_none()
            && self.origin.is_none()
            && self.thinking_modes.is_empty()
            && self.speed_modes.is_empty()
            && self.capabilities.is_empty()
    }

    pub fn into_configured_definition(self) -> ConfiguredModelDefinition {
        ConfiguredModelDefinition {
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            display_name: self.display_name,
            description: self.description,
            knowledge_cutoff: self.knowledge_cutoff,
            release_date: self.release_date,
            last_updated: self.last_updated,
            open_weights: self.open_weights,
            default_thinking_mode: self.default_thinking_mode,
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.supports_verbosity,
            default_verbosity: self.default_verbosity,
            assistant_reasoning_field: self.assistant_reasoning_field,
            output_modalities: self.output_modalities,
            pricing: self.pricing,
            thinking_modes: self.thinking_modes,
            speed_modes: self.speed_modes,
            capabilities: self.capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCatalogProviderRecord {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, CatalogModelDefinition>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub appendable_model_ids: BTreeSet<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_local_override: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub thinking_modes: BTreeMap<String, ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatch,
}

impl ModelCatalogEntryRecord {
    pub fn definition(&self) -> CatalogModelDefinition {
        CatalogModelDefinition {
            lifecycle: self.lifecycle,
            context_window_tokens: self.context_window_tokens,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            description: self.description.clone(),
            knowledge_cutoff: self.knowledge_cutoff.clone(),
            release_date: self.release_date.clone(),
            last_updated: self.last_updated.clone(),
            open_weights: self.open_weights,
            default_thinking_mode: self.default_thinking_mode.clone(),
            supports_parallel_tool_calls: self.supports_parallel_tool_calls,
            supports_verbosity: self.supports_verbosity,
            default_verbosity: self.default_verbosity.clone(),
            assistant_reasoning_field: self.assistant_reasoning_field.clone(),
            output_modalities: self.output_modalities.clone(),
            pricing: self.pricing.clone(),
            display_name: self.display_name.clone(),
            origin: self.origin.clone(),
            thinking_modes: self.thinking_modes.clone(),
            speed_modes: self.speed_modes.clone(),
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
            appendable_model_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogSnapshot {
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
        merged.appendable_model_ids = custom.models.keys().cloned().collect();
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
            origin: definition.origin.clone(),
            has_local_override,
            lifecycle: definition.lifecycle,
            context_window_tokens: definition.context_window_tokens,
            max_input_tokens: definition.max_input_tokens,
            max_output_tokens: definition.max_output_tokens,
            description: definition.description.clone(),
            knowledge_cutoff: definition.knowledge_cutoff.clone(),
            release_date: definition.release_date.clone(),
            last_updated: definition.last_updated.clone(),
            open_weights: definition.open_weights,
            default_thinking_mode: definition.default_thinking_mode.clone(),
            supports_parallel_tool_calls: definition.supports_parallel_tool_calls,
            supports_verbosity: definition.supports_verbosity,
            default_verbosity: definition.default_verbosity.clone(),
            assistant_reasoning_field: definition.assistant_reasoning_field.clone(),
            output_modalities: definition.output_modalities.clone(),
            pricing: definition.pricing.clone(),
            thinking_modes: definition.thinking_modes.clone(),
            speed_modes: definition.speed_modes.clone(),
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
            last_refresh_at: self.last_refresh_at,
            last_successful_source: self.last_successful_source,
            last_error: self.last_error.clone(),
            entries: self.entries(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogConfig {
    pub cache_path: PathBuf,
    pub custom_path: PathBuf,
    pub cache_max_age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogResponse {
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
            cache_path: root.join("model-catalog-cache.json"),
            custom_path: root.join("model-catalog-custom.json"),
            cache_max_age_secs: DEFAULT_CACHE_MAX_AGE_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedOfficialCatalog {
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
    store: ModelCatalogStore,
    state: Arc<RwLock<ModelCatalogSnapshot>>,
    client: reqwest::Client,
    remote_sources: Vec<sources::ModelCatalogRemoteSource>,
}

impl ModelCatalogService {
    pub fn new(store: ModelCatalogStore) -> Result<Self, AppError> {
        Self::with_remote_sources(store, default_remote_sources())
    }

    fn with_remote_sources(
        store: ModelCatalogStore,
        remote_sources: Vec<sources::ModelCatalogRemoteSource>,
    ) -> Result<Self, AppError> {
        let custom = store.read_custom()?;
        let mut snapshot = ModelCatalogSnapshot {
            custom,
            ..ModelCatalogSnapshot::default()
        };

        if let Some(cached) = store.read_cached_official()? {
            snapshot.last_successful_source = Some(cached.source);
            snapshot.last_refresh_at =
                DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms);
            snapshot.official = cached.document;
        }

        Ok(Self {
            store,
            state: Arc::new(RwLock::new(snapshot)),
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .user_agent(format!("agena/{} model-catalog", env!("CARGO_PKG_VERSION")))
                .build()?,
            remote_sources,
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

    pub async fn refresh_if_stale_on_startup(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let snapshot = self.snapshot();
        if self.snapshot_needs_startup_refresh(&snapshot) {
            self.refresh_from_registry(providers, resolution).await
        } else {
            Ok(snapshot)
        }
    }

    pub async fn refresh_from_registry(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<ModelCatalogSnapshot, AppError> {
        let (document, warnings) = match self.build_catalog_document(providers, resolution).await {
            Ok(result) => result,
            Err(refresh_error) => {
                if let Some(cached) = self.store.read_cached_official()? {
                    let cache_is_fresh = self.cache_is_fresh(&cached);
                    let mut snapshot = self.snapshot();
                    snapshot.official = cached.document;
                    snapshot.last_successful_source = Some(ModelCatalogEntrySourceKind::Cache);
                    snapshot.last_refresh_at =
                        DateTime::<Utc>::from_timestamp_millis(cached.fetched_at_unix_ms);
                    snapshot.last_error = Some(format!(
                        "catalog refresh failed: {refresh_error}; using {}cache",
                        if cache_is_fresh { "" } else { "stale " }
                    ));
                    self.replace_snapshot(snapshot.clone());
                    return Ok(snapshot);
                }
                return Err(refresh_error);
            }
        };
        let fetched_at_unix_ms = now_unix_ms();
        self.store.write_cached_official(&CachedOfficialCatalog {
            fetched_at_unix_ms,
            source: ModelCatalogEntrySourceKind::Generated,
            document: document.clone(),
        })?;

        let mut snapshot = self.snapshot();
        snapshot.official = document;
        snapshot.last_successful_source = Some(ModelCatalogEntrySourceKind::Generated);
        snapshot.last_refresh_at = DateTime::<Utc>::from_timestamp_millis(fetched_at_unix_ms);
        snapshot.last_error = warnings;
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

    async fn build_catalog_document(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<(ModelCatalogDocument, Option<String>), AppError> {
        let mut merged = ModelCatalogDocument::default();
        let mut warnings = Vec::new();
        let mut succeeded = 0_usize;

        let (remote_documents, remote_warnings) =
            sources::fetch_documents(&self.client, &self.remote_sources).await;
        warnings.extend(remote_warnings);
        let mut remote_documents = remote_documents;
        remote_documents.sort_by(|left, right| {
            right
                .kind
                .priority()
                .cmp(&left.kind.priority())
                .then_with(|| left.name.cmp(&right.name))
        });
        for fetched in remote_documents {
            merge_public_source_catalog_document(&mut merged, fetched.document);
            succeeded += 1;
        }

        match self
            .build_live_provider_catalog_document(providers, resolution)
            .await
        {
            Ok((Some(document), warning)) => {
                merge_catalog_document(&mut merged, document);
                succeeded += 1;
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Ok((None, warning)) => {
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Err(error) => {
                warnings.push(format!("live discovery: {error}"));
                if succeeded == 0 {
                    return Err(error);
                }
            }
        }

        if succeeded == 0 {
            let detail = if warnings.is_empty() {
                "no public catalog sources or live provider model lists succeeded".to_owned()
            } else {
                warnings.join("; ")
            };
            return Err(AppError::Config(format!(
                "model catalog generation failed: {detail}"
            )));
        }

        let mut document = curate::curate_catalog_document(merged)
            .map_err(|err| AppError::Config(format!("curate generated model catalog: {err}")))?;
        sources::enrich_catalog_document_thinking_modes(&mut document);
        let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
        Ok((document, warning))
    }

    async fn build_live_provider_catalog_document(
        &self,
        providers: &ProviderRegistry,
        resolution: Option<&ConfigResolution>,
    ) -> Result<(Option<ModelCatalogDocument>, Option<String>), AppError> {
        let mut provider_ids = providers.provider_ids();
        provider_ids.sort_by(|left, right| {
            provider_priority(right.as_str(), resolution)
                .cmp(&provider_priority(left.as_str(), resolution))
                .then_with(|| left.cmp(right))
        });

        if provider_ids.is_empty() {
            return Ok((None, None));
        }

        let mut raw_models = BTreeMap::<String, CatalogModelDefinition>::new();
        let mut errors = Vec::new();
        let mut succeeded = 0_usize;

        for provider_id in provider_ids {
            match providers.list_models(provider_id.as_str()).await {
                Ok(models) => {
                    succeeded += 1;
                    for model in models {
                        if model.id.as_str().trim().is_empty() {
                            continue;
                        }
                        let definition = catalog_definition_from_model(&model);
                        raw_models
                            .entry(model.id.to_string())
                            .and_modify(|current| merge_catalog_definition(current, &definition))
                            .or_insert(definition);
                    }
                }
                Err(error) => errors.push(format!("{provider_id}: {error}")),
            }
        }

        if succeeded == 0 {
            let detail = if errors.is_empty() {
                return Ok((None, None));
            } else {
                errors.join("; ")
            };
            return Err(AppError::Config(format!(
                "live provider model discovery failed: {detail}"
            )));
        }

        let document = curate::curate_catalog_document(ModelCatalogDocument { models: raw_models })
            .map_err(|err| {
                AppError::Config(format!("curate live provider model catalog: {err}"))
            })?;
        let warning = (!errors.is_empty()).then(|| {
            format!(
                "live discovery generated catalog from {succeeded} provider(s); skipped {} provider(s): {}",
                errors.len(),
                errors.join("; ")
            )
        });
        Ok((Some(document), warning))
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

fn provider_priority(provider_id: &str, resolution: Option<&ConfigResolution>) -> i32 {
    let Some(resolution) = resolution else {
        return 0;
    };
    let Some(provider) = resolution.config.providers.get(provider_id) else {
        return 0;
    };
    provider
        .adapters
        .values()
        .filter(|adapter| adapter.enabled)
        .map(|adapter| match &adapter.definition {
            ProviderAdapterDefinition::Anthropic(_) => 500,
            ProviderAdapterDefinition::Gemini(_) => 500,
            ProviderAdapterDefinition::OpenAi(config) => match config.options.capability_family {
                Some(ProviderCapabilityFamilyConfig::OpenAi) | None => 450,
                Some(ProviderCapabilityFamilyConfig::Anthropic)
                | Some(ProviderCapabilityFamilyConfig::Gemini) => 350,
                Some(ProviderCapabilityFamilyConfig::Bedrock)
                | Some(ProviderCapabilityFamilyConfig::Gitlab) => 200,
            },
            ProviderAdapterDefinition::AmazonBedrock(_) => 200,
            ProviderAdapterDefinition::Gitlab(_) => 150,
            ProviderAdapterDefinition::Ollama(_) => 50,
        })
        .max()
        .unwrap_or_default()
}

fn catalog_definition_from_model(model: &Model) -> CatalogModelDefinition {
    CatalogModelDefinition {
        lifecycle: model.metadata.lifecycle,
        context_window_tokens: model.metadata.limits.context_window_tokens,
        max_input_tokens: model.metadata.limits.max_input_tokens,
        max_output_tokens: model.metadata.limits.max_output_tokens,
        description: model.metadata.description.clone(),
        knowledge_cutoff: model.metadata.knowledge_cutoff.clone(),
        release_date: model.metadata.release_date.clone(),
        last_updated: model.metadata.last_updated.clone(),
        open_weights: model.metadata.open_weights,
        default_thinking_mode: model.metadata.default_thinking_mode.clone(),
        supports_parallel_tool_calls: model.metadata.supports_parallel_tool_calls,
        supports_verbosity: model.metadata.supports_verbosity,
        default_verbosity: model.metadata.default_verbosity.clone(),
        assistant_reasoning_field: model.metadata.assistant_reasoning_field.clone(),
        output_modalities: model.metadata.output_modalities.clone(),
        pricing: model.metadata.pricing.clone(),
        display_name: model.display_name.clone(),
        origin: None,
        thinking_modes: model
            .thinking_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    ConfiguredModelThinkingMode {
                        display_name: mode.display_name.clone(),
                        description: mode.description.clone(),
                        thinking: mode.thinking.clone(),
                        request_override: mode.request_override.clone(),
                        adapter_overrides: mode.adapter_overrides.clone(),
                        disabled: false,
                    },
                )
            })
            .collect(),
        speed_modes: model
            .speed_modes
            .iter()
            .map(|(name, mode)| {
                (
                    name.clone(),
                    ConfiguredModelSpeedMode {
                        display_name: mode.display_name.clone(),
                        description: mode.description.clone(),
                        request_override: mode.request_override.clone(),
                        adapter_overrides: mode.adapter_overrides.clone(),
                        disabled: false,
                    },
                )
            })
            .collect(),
        capabilities: capability_patch_from_model(&model.capabilities),
    }
}

fn capability_patch_from_model(capabilities: &ModelCapabilities) -> ModelCapabilityPatch {
    let mut supported_inputs = Vec::new();
    let mut unsupported_inputs = Vec::new();
    for (modality, support) in [
        (ModelInputModality::Text, capabilities.text_input),
        (ModelInputModality::Image, capabilities.image_input),
        (ModelInputModality::Document, capabilities.document_input),
        (ModelInputModality::Audio, capabilities.audio_input),
        (ModelInputModality::Video, capabilities.video_input),
        (ModelInputModality::File, capabilities.file_input),
    ] {
        match support {
            CapabilitySupport::Supported if !matches!(modality, ModelInputModality::Text) => {
                supported_inputs.push(modality);
            }
            CapabilitySupport::Unsupported => unsupported_inputs.push(modality),
            _ => {}
        }
    }

    let mut supported_features = Vec::new();
    let mut unsupported_features = Vec::new();
    for (feature, support) in [
        (
            ModelCapabilityFeature::ToolCalling,
            capabilities.tool_calling,
        ),
        (ModelCapabilityFeature::Streaming, capabilities.streaming),
        (ModelCapabilityFeature::Reasoning, capabilities.reasoning),
        (
            ModelCapabilityFeature::StructuredOutput,
            capabilities.structured_output,
        ),
        (
            ModelCapabilityFeature::Temperature,
            capabilities.temperature_supported,
        ),
    ] {
        match support {
            CapabilitySupport::Supported
                if !matches!(feature, ModelCapabilityFeature::Temperature) =>
            {
                supported_features.push(feature);
            }
            CapabilitySupport::Unsupported => unsupported_features.push(feature),
            _ => {}
        }
    }

    ModelCapabilityPatch {
        input: (!supported_inputs.is_empty() || !unsupported_inputs.is_empty()).then_some(
            InputCapabilityPatch::Patch(InputCapabilityPatchBody {
                supported: supported_inputs,
                unsupported: unsupported_inputs,
            }),
        ),
        features: (!supported_features.is_empty() || !unsupported_features.is_empty()).then_some(
            FeatureCapabilityPatch::Patch(FeatureCapabilityPatchBody {
                supported: supported_features,
                unsupported: unsupported_features,
            }),
        ),
        ..ModelCapabilityPatch::default()
    }
}

fn merge_catalog_definition(current: &mut CatalogModelDefinition, next: &CatalogModelDefinition) {
    if current.lifecycle.is_none() {
        current.lifecycle = next.lifecycle;
    }
    if current.context_window_tokens.is_none() {
        current.context_window_tokens = next.context_window_tokens;
    }
    if current.max_input_tokens.is_none() {
        current.max_input_tokens = next.max_input_tokens;
    }
    if current.max_output_tokens.is_none() {
        current.max_output_tokens = next.max_output_tokens;
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.knowledge_cutoff.is_none() {
        current.knowledge_cutoff = next.knowledge_cutoff.clone();
    }
    if current.release_date.is_none() {
        current.release_date = next.release_date.clone();
    }
    if current.last_updated.is_none() {
        current.last_updated = next.last_updated.clone();
    }
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.default_thinking_mode.is_none() {
        current.default_thinking_mode = next.default_thinking_mode.clone();
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    if current.default_verbosity.is_none() {
        current.default_verbosity = next.default_verbosity.clone();
    }
    if current.assistant_reasoning_field.is_none() {
        current.assistant_reasoning_field = next.assistant_reasoning_field.clone();
    }
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    for (name, mode) in &next.thinking_modes {
        current
            .thinking_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_thinking_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    for (name, mode) in &next.speed_modes {
        current
            .speed_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_speed_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
}

fn merge_catalog_thinking_mode(
    current: &mut ConfiguredModelThinkingMode,
    next: &ConfiguredModelThinkingMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.thinking.is_none() {
        current.thinking = next.thinking.clone();
    }
    merge_speed_mode_request_override_fill_missing(
        &mut current.request_override,
        &next.request_override,
    );
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let current_patch = current
            .adapter_overrides
            .entry(adapter_id.clone())
            .or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
    }
    current.disabled |= next.disabled;
}

fn merge_catalog_speed_mode(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    current.request_override = current.request_override.merged_with(&next.request_override);
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let merged = current
            .adapter_overrides
            .get(adapter_id)
            .cloned()
            .unwrap_or_default()
            .merged_with(override_patch);
        current.adapter_overrides.insert(adapter_id.clone(), merged);
    }
    current.disabled |= next.disabled;
}

fn merge_catalog_speed_mode_fill_missing(
    current: &mut ConfiguredModelSpeedMode,
    next: &ConfiguredModelSpeedMode,
) {
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    merge_speed_mode_request_override_fill_missing(
        &mut current.request_override,
        &next.request_override,
    );
    for (adapter_id, override_patch) in &next.adapter_overrides {
        let current_patch = current
            .adapter_overrides
            .entry(adapter_id.clone())
            .or_default();
        merge_speed_mode_request_override_fill_missing(current_patch, override_patch);
    }
    current.disabled |= next.disabled;
}

fn merge_catalog_document(current: &mut ModelCatalogDocument, next: ModelCatalogDocument) {
    for (model_id, definition) in next.models {
        current
            .models
            .entry(model_id)
            .and_modify(|existing| merge_catalog_definition(existing, &definition))
            .or_insert(definition);
    }
}

fn merge_public_source_catalog_document(
    current: &mut ModelCatalogDocument,
    next: ModelCatalogDocument,
) {
    for (model_id, definition) in next.models {
        current
            .models
            .entry(model_id)
            .and_modify(|existing| merge_public_source_catalog_definition(existing, &definition))
            .or_insert(definition);
    }
}

fn merge_public_source_catalog_definition(
    current: &mut CatalogModelDefinition,
    next: &CatalogModelDefinition,
) {
    if current.lifecycle.is_none() {
        current.lifecycle = next.lifecycle;
    }
    if current.context_window_tokens.is_none() {
        current.context_window_tokens = next.context_window_tokens;
    }
    if current.max_input_tokens.is_none() {
        current.max_input_tokens = next.max_input_tokens;
    }
    if current.max_output_tokens.is_none() {
        current.max_output_tokens = next.max_output_tokens;
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.knowledge_cutoff.is_none() {
        current.knowledge_cutoff = next.knowledge_cutoff.clone();
    }
    if current.release_date.is_none() {
        current.release_date = next.release_date.clone();
    }
    if current.last_updated.is_none() {
        current.last_updated = next.last_updated.clone();
    }
    if current.open_weights.is_none() {
        current.open_weights = next.open_weights;
    }
    if current.default_thinking_mode.is_none() {
        current.default_thinking_mode = next.default_thinking_mode.clone();
    }
    if current.supports_parallel_tool_calls.is_none() {
        current.supports_parallel_tool_calls = next.supports_parallel_tool_calls;
    }
    if current.supports_verbosity.is_none() {
        current.supports_verbosity = next.supports_verbosity;
    }
    if current.default_verbosity.is_none() {
        current.default_verbosity = next.default_verbosity.clone();
    }
    if current.assistant_reasoning_field.is_none() {
        current.assistant_reasoning_field = next.assistant_reasoning_field.clone();
    }
    merge_unique(&mut current.output_modalities, &next.output_modalities);
    merge_model_pricing(&mut current.pricing, next.pricing.as_ref());
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    for (name, mode) in &next.thinking_modes {
        current
            .thinking_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_thinking_mode(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    for (name, mode) in &next.speed_modes {
        current
            .speed_modes
            .entry(name.clone())
            .and_modify(|existing| merge_catalog_speed_mode_fill_missing(existing, mode))
            .or_insert_with(|| mode.clone());
    }
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
}

fn merge_speed_mode_request_override_fill_missing(
    current: &mut crate::model::ModelSpeedModeRequestOverride,
    next: &crate::model::ModelSpeedModeRequestOverride,
) {
    for (key, value) in &next.headers {
        current
            .headers
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    merge_json_patch_maps_fill_missing(&mut current.body_patch, &next.body_patch);
}

fn merge_json_patch_maps_fill_missing(
    current: &mut BTreeMap<String, serde_json::Value>,
    next: &BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in next {
        match current.get_mut(key) {
            Some(existing) => merge_json_value_fill_missing(existing, value),
            None => {
                current.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_json_value_fill_missing(current: &mut serde_json::Value, next: &serde_json::Value) {
    match (current, next) {
        (serde_json::Value::Object(current_map), serde_json::Value::Object(next_map)) => {
            for (key, value) in next_map {
                match current_map.get_mut(key) {
                    Some(existing) => merge_json_value_fill_missing(existing, value),
                    None => {
                        current_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        _ => {}
    }
}

fn merge_capability_patch(current: &mut ModelCapabilityPatch, next: &ModelCapabilityPatch) {
    merge_input_patch(&mut current.input, next.input.as_ref());
    merge_feature_patch(&mut current.features, next.features.as_ref());
    if current.text_input.is_none() {
        current.text_input = next.text_input;
    }
    if current.image_input.is_none() {
        current.image_input = next.image_input;
    }
    if current.document_input.is_none() {
        current.document_input = next.document_input;
    }
    if current.audio_input.is_none() {
        current.audio_input = next.audio_input;
    }
    if current.video_input.is_none() {
        current.video_input = next.video_input;
    }
    if current.file_input.is_none() {
        current.file_input = next.file_input;
    }
    if current.tool_calling.is_none() {
        current.tool_calling = next.tool_calling;
    }
    if current.streaming.is_none() {
        current.streaming = next.streaming;
    }
    if current.reasoning.is_none() {
        current.reasoning = next.reasoning;
    }
    if current.structured_output.is_none() {
        current.structured_output = next.structured_output;
    }
    if current.temperature_supported.is_none() {
        current.temperature_supported = next.temperature_supported;
    }
}

fn merge_input_patch(
    current: &mut Option<InputCapabilityPatch>,
    next: Option<&InputCapabilityPatch>,
) {
    match (current.as_mut(), next) {
        (None, Some(next)) => *current = Some(next.clone()),
        (
            Some(InputCapabilityPatch::Supported(current_values)),
            Some(InputCapabilityPatch::Supported(next_values)),
        ) => {
            merge_unique(current_values, next_values);
        }
        (
            Some(InputCapabilityPatch::Patch(current_values)),
            Some(InputCapabilityPatch::Patch(next_values)),
        ) => {
            merge_unique(&mut current_values.supported, &next_values.supported);
            merge_unique(&mut current_values.unsupported, &next_values.unsupported);
        }
        (
            Some(InputCapabilityPatch::Supported(current_values)),
            Some(InputCapabilityPatch::Patch(next_values)),
        ) => {
            merge_unique(current_values, &next_values.supported);
        }
        (
            Some(InputCapabilityPatch::Patch(current_values)),
            Some(InputCapabilityPatch::Supported(next_values)),
        ) => {
            merge_unique(&mut current_values.supported, next_values);
        }
        _ => {}
    }
}

fn merge_feature_patch(
    current: &mut Option<FeatureCapabilityPatch>,
    next: Option<&FeatureCapabilityPatch>,
) {
    match (current.as_mut(), next) {
        (None, Some(next)) => *current = Some(next.clone()),
        (
            Some(FeatureCapabilityPatch::Supported(current_values)),
            Some(FeatureCapabilityPatch::Supported(next_values)),
        ) => merge_unique(current_values, next_values),
        (
            Some(FeatureCapabilityPatch::Patch(current_values)),
            Some(FeatureCapabilityPatch::Patch(next_values)),
        ) => {
            merge_unique(&mut current_values.supported, &next_values.supported);
            merge_unique(&mut current_values.unsupported, &next_values.unsupported);
        }
        (
            Some(FeatureCapabilityPatch::Supported(current_values)),
            Some(FeatureCapabilityPatch::Patch(next_values)),
        ) => merge_unique(current_values, &next_values.supported),
        (
            Some(FeatureCapabilityPatch::Patch(current_values)),
            Some(FeatureCapabilityPatch::Supported(next_values)),
        ) => merge_unique(&mut current_values.supported, next_values),
        _ => {}
    }
}

fn merge_unique<T: Clone + PartialEq>(current: &mut Vec<T>, next: &[T]) {
    for value in next {
        if !current.contains(value) {
            current.push(value.clone());
        }
    }
}

fn merge_model_pricing(current: &mut Option<ModelPricing>, next: Option<&ModelPricing>) {
    match (current.as_mut(), next) {
        (None, Some(next)) => *current = Some(next.clone()),
        (Some(current), Some(next)) => {
            if current.input_usd_per_million_tokens.is_none() {
                current.input_usd_per_million_tokens = next.input_usd_per_million_tokens.clone();
            }
            if current.output_usd_per_million_tokens.is_none() {
                current.output_usd_per_million_tokens = next.output_usd_per_million_tokens.clone();
            }
            if current.cache_read_usd_per_million_tokens.is_none() {
                current.cache_read_usd_per_million_tokens =
                    next.cache_read_usd_per_million_tokens.clone();
            }
            if current.cache_write_usd_per_million_tokens.is_none() {
                current.cache_write_usd_per_million_tokens =
                    next.cache_write_usd_per_million_tokens.clone();
            }
            for tier in &next.tiers {
                match current.tiers.iter_mut().find(|existing| {
                    existing.tier_type == tier.tier_type && existing.size_tokens == tier.size_tokens
                }) {
                    Some(existing) => {
                        if existing.input_usd_per_million_tokens.is_none() {
                            existing.input_usd_per_million_tokens =
                                tier.input_usd_per_million_tokens.clone();
                        }
                        if existing.output_usd_per_million_tokens.is_none() {
                            existing.output_usd_per_million_tokens =
                                tier.output_usd_per_million_tokens.clone();
                        }
                        if existing.cache_read_usd_per_million_tokens.is_none() {
                            existing.cache_read_usd_per_million_tokens =
                                tier.cache_read_usd_per_million_tokens.clone();
                        }
                        if existing.cache_write_usd_per_million_tokens.is_none() {
                            existing.cache_write_usd_per_million_tokens =
                                tier.cache_write_usd_per_million_tokens.clone();
                        }
                    }
                    None => current.tiers.push(tier.clone()),
                }
            }
        }
        _ => {}
    }
}

pub fn catalog_definition_to_provider_definition(
    definition: &CatalogModelDefinition,
) -> ConfiguredModelDefinition {
    definition.clone().into_configured_definition()
}

pub fn decorate_provider_models(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    mut models: Vec<Model>,
) -> Vec<Model> {
    let mut listed = BTreeSet::new();
    let mut listed_catalog_ids = BTreeSet::new();

    for model in &mut models {
        listed.insert(model.id.to_string());
        if let Some(catalog_model_id) = catalog_match_model_id_for_raw(model.id.as_str()) {
            listed_catalog_ids.insert(catalog_model_id.clone());
            model.catalog_model_id = Some(ModelId::new(catalog_model_id));
        }
        *model =
            decorate_provider_model(provider, provider_record, model.id.clone(), model.clone());
    }

    for model_id in &provider_record.appendable_model_ids {
        if listed.contains(model_id.as_str())
            || listed_catalog_ids.contains(model_id.as_str())
            || models.iter().any(|model| {
                model.id.as_str() == model_id.as_str()
                    || model.catalog_model_id.as_ref().map(ModelId::as_str)
                        == Some(model_id.as_str())
            })
        {
            continue;
        }

        let model_id = ModelId::new(model_id.clone());
        let base = Model::new(provider.id(), model_id.as_str())
            .with_catalog_model_id(model_id.as_str())
            .with_capabilities(provider.model_capabilities_for_adapter(None, &model_id))
            .with_metadata(provider_model_metadata(provider, None, &model_id))
            .with_thinking_modes(provider_model_thinking_modes(
                provider,
                provider_record,
                None,
                &model_id,
            ))
            .with_speed_modes(provider_model_speed_modes(
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
    let matched_catalog_id = catalog_match_model_id_for_raw(model_id.as_str());
    if let Some(catalog_model_id) = matched_catalog_id {
        model.catalog_model_id = Some(ModelId::new(catalog_model_id));
    }

    if let Some(display_name) = catalog_definition_for_model_id(provider_record, model_id.as_str())
        .and_then(|definition| definition.display_name.clone())
    {
        model.display_name = Some(display_name);
    }

    if let Some(configured) = catalog_definition_for_model_id(provider_record, model_id.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
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

fn provider_model_thinking_modes(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelThinkingMode> {
    let mut modes = provider.model_thinking_modes_for_adapter(adapter_id, model);
    if let Some(configured) = catalog_definition_for_model_id(provider_record, model.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
    {
        for (name, configured_mode) in &configured.thinking_modes {
            match configured_mode.apply_to_mode(modes.get(name)) {
                Some(mode) => {
                    modes.insert(name.clone(), mode);
                }
                None => {
                    modes.remove(name);
                }
            }
        }
    }
    modes
}

fn provider_model_speed_modes(
    provider: &dyn ModelProvider,
    provider_record: &ModelCatalogProviderRecord,
    adapter_id: Option<&crate::model::AdapterId>,
    model: &ModelId,
) -> BTreeMap<String, crate::model::ModelSpeedMode> {
    let mut modes = provider.model_speed_modes_for_adapter(adapter_id, model);
    if let Some(configured) = catalog_definition_for_model_id(provider_record, model.as_str())
        .cloned()
        .map(|definition| catalog_definition_to_provider_definition(&definition))
    {
        for (name, configured_mode) in &configured.speed_modes {
            match configured_mode.apply_to_mode(modes.get(name)) {
                Some(mode) => {
                    modes.insert(name.clone(), mode);
                }
                None => {
                    modes.remove(name);
                }
            }
        }
    }
    modes
}

fn catalog_definition_for_model_id<'a>(
    provider_record: &'a ModelCatalogProviderRecord,
    raw_model_id: &str,
) -> Option<&'a CatalogModelDefinition> {
    provider_record.models.get(raw_model_id).or_else(|| {
        catalog_match_model_id_for_raw(raw_model_id)
            .as_ref()
            .and_then(|catalog_model_id| provider_record.models.get(catalog_model_id))
    })
}

fn catalog_match_model_id_for_raw(raw_model_id: &str) -> Option<String> {
    let canonical = canonical_model_catalog_id(raw_model_id);
    let trimmed = canonical.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{CapabilitySupport, ModelId, ModelMetadata, ModelSpeedModeRequestOverride},
        provider::{
            ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ModelCapabilityFeature,
            ReasoningEffort, ThinkingRequest,
        },
    };
    use tempfile::tempdir;

    fn normalized_catalog_model_id(model_id: &str) -> String {
        curate::normalized_catalog_model_id(model_id)
    }

    struct StaticListProvider {
        provider_id: &'static str,
        default_model: ModelId,
        models: Vec<Model>,
    }

    impl StaticListProvider {
        fn new(provider_id: &'static str, default_model: &'static str, models: Vec<Model>) -> Self {
            Self {
                provider_id,
                default_model: ModelId::new(default_model),
                models,
            }
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for StaticListProvider {
        fn id(&self) -> &str {
            self.provider_id
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, AppError> {
            Ok(self.models.clone())
        }

        async fn complete(
            &self,
            _request: crate::provider::CompletionRequest,
        ) -> Result<crate::provider::CompletionResponse, AppError> {
            Err(AppError::Provider("not implemented".to_owned()))
        }
    }

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
            last_successful_source: Some(ModelCatalogEntrySourceKind::Generated),
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
                        thinking_modes: BTreeMap::from([(
                            "deep".to_owned(),
                            ConfiguredModelThinkingMode {
                                display_name: Some("Deep".to_owned()),
                                description: None,
                                thinking: None,
                                request_override: Default::default(),
                                adapter_overrides: BTreeMap::new(),
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
        assert!(entries[1].thinking_modes.contains_key("deep"));
    }

    #[test]
    fn curated_catalog_document_canonicalizes_aliases_and_seeds_origin_labels() {
        let document = curate::curate_catalog_document(ModelCatalogDocument {
            models: BTreeMap::from([
                (
                    "openai.gpt-5.4".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("OpenAI GPT-5.4".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "study_gpt-chatgpt-4o-latest".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("Study GPT ChatGPT-4o Latest".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "amazon.nova-pro-v1:0".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("Amazon Nova Pro".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "gpt-oss-120b:free".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("GPT OSS 120B".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
                (
                    "claude-opus-4-7".to_owned(),
                    CatalogModelDefinition {
                        display_name: Some("Claude Opus 4.7".to_owned()),
                        ..CatalogModelDefinition::default()
                    },
                ),
            ]),
        })
        .expect("seed document should curate");

        let catalog = document.model_record();
        assert!(catalog.models.contains_key("claude-opus-4-7"));
        assert!(catalog.models.contains_key("nova-pro-v1"));
        assert!(catalog.models.contains_key("gpt-5.4"));
        assert!(catalog.models.contains_key("gpt-4o"));
        assert!(catalog.models.contains_key("gpt-oss-120b"));
        assert_eq!(
            catalog
                .models
                .get("gpt-5.4")
                .and_then(|definition| definition.origin.as_deref()),
            Some("OpenAI")
        );
        assert_eq!(
            catalog
                .models
                .get("claude-opus-4-7")
                .and_then(|definition| definition.origin.as_deref()),
            Some("Anthropic")
        );
        assert!(!catalog.models.contains_key("openai.gpt-5.4"));
        assert!(!catalog.models.contains_key("study_gpt-chatgpt-4o-latest"));
        assert!(!catalog.models.contains_key("gpt-oss-120b:free"));
        assert!(!catalog.models.contains_key("amazon.nova-pro-v1:0"));

        let mut lowered = BTreeSet::new();
        let mut normalized = BTreeSet::new();
        for model_id in catalog.models.keys() {
            assert_eq!(
                model_id,
                &model_id.to_ascii_lowercase(),
                "curated model id should be lowercase canonical text: {model_id}"
            );
            assert!(
                !model_id.contains('/'),
                "curated model id should not contain '/': {model_id}"
            );
            assert!(
                !model_id.contains("@default"),
                "curated model id should not contain '@default': {model_id}"
            );
            assert!(
                !model_id.ends_with("-maas"),
                "curated model id should not contain provider route suffix '-maas': {model_id}"
            );
            assert!(
                !model_id.ends_with(":free"),
                "curated model id should not contain free-tier suffix ':free': {model_id}"
            );
            assert!(
                catalog
                    .models
                    .get(model_id)
                    .and_then(|definition| definition.origin.as_ref())
                    .is_some_and(|origin| !origin.trim().is_empty()),
                "curated model id should include a non-empty origin label: {model_id}"
            );
            assert!(
                lowered.insert(model_id.to_ascii_lowercase()),
                "curated catalog should not contain case-insensitive duplicate model ids: {model_id}"
            );
            let normalized_model_id = normalized_catalog_model_id(model_id);
            assert!(
                normalized.insert(normalized_model_id.clone()),
                "curated catalog should not contain normalized duplicate model ids: {model_id} -> {normalized_model_id}"
            );
        }
    }

    #[tokio::test]
    async fn startup_refresh_reuses_fresh_cached_catalog() {
        let dir = tempdir().expect("tempdir should create");
        let store = ModelCatalogStore::new(ModelCatalogConfig {
            cache_path: dir.path().join("model-catalog-cache.json"),
            custom_path: dir.path().join("model-catalog-custom.json"),
            cache_max_age_secs: 60,
        });
        let cached_document = model_catalog_document("cached-model");
        store
            .write_cached_official(&CachedOfficialCatalog {
                fetched_at_unix_ms: now_unix_ms(),
                source: ModelCatalogEntrySourceKind::Cache,
                document: cached_document.clone(),
            })
            .expect("cache should be written");

        let service = ModelCatalogService::with_remote_sources(store, Vec::new())
            .expect("service should load");
        let providers = ProviderRegistry::new();

        let snapshot = service
            .refresh_if_stale_on_startup(&providers, None)
            .await
            .expect("fresh startup snapshot should succeed");

        assert_eq!(snapshot.official, cached_document);
        assert_eq!(
            snapshot.last_successful_source,
            Some(ModelCatalogEntrySourceKind::Cache)
        );
    }

    #[tokio::test]
    async fn startup_refresh_updates_stale_cached_catalog_from_provider_registry() {
        let dir = tempdir().expect("tempdir should create");
        let store = ModelCatalogStore::new(ModelCatalogConfig {
            cache_path: dir.path().join("model-catalog-cache.json"),
            custom_path: dir.path().join("model-catalog-custom.json"),
            cache_max_age_secs: 1,
        });
        store
            .write_cached_official(&CachedOfficialCatalog {
                fetched_at_unix_ms: now_unix_ms() - 5_000,
                source: ModelCatalogEntrySourceKind::Cache,
                document: model_catalog_document("gpt-4o"),
            })
            .expect("stale cache should be written");

        let service = ModelCatalogService::with_remote_sources(store, Vec::new())
            .expect("service should load");
        let mut providers = ProviderRegistry::new();
        providers.register(StaticListProvider::new(
            "openai",
            "gpt-5.4",
            vec![
                Model::new("openai", "openai.gpt-5.4")
                    .with_display_name("GPT-5.4")
                    .with_metadata(ModelMetadata {
                        description: Some("Official OpenAI model".to_owned()),
                        ..ModelMetadata::default()
                    }),
            ],
        ));

        let snapshot = service
            .refresh_if_stale_on_startup(&providers, None)
            .await
            .expect("stale startup refresh should succeed");

        assert!(snapshot.official.models.contains_key("gpt-5.4"));
        assert_eq!(
            snapshot.last_successful_source,
            Some(ModelCatalogEntrySourceKind::Generated)
        );
        assert_eq!(
            snapshot
                .official
                .models
                .get("gpt-5.4")
                .and_then(|definition| definition.origin.as_deref()),
            Some("OpenAI")
        );
    }

    #[tokio::test]
    async fn refresh_from_registry_merges_public_sources_and_keeps_custom_appendable_only() {
        let mut server = mockito::Server::new_async().await;
        let _models_dev = server
            .mock("GET", "/models-dev.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "openai": {
                        "id": "openai",
                        "name": "OpenAI",
                        "models": {
                            "gpt-5": {
                                "id": "gpt-5",
                                "name": "GPT-5",
                                "description": "Models.dev GPT-5 description",
                                "knowledge": "2025-04",
                                "release_date": "2026-04-22",
                                "last_updated": "2026-04-24",
                                "open_weights": false,
                                "interleaved": {
                                    "field": "reasoning_content"
                                },
                                "reasoning": true,
                                "tool_call": true,
                                "structured_output": true,
                                "temperature": false,
                                "modalities": {
                                    "input": ["text", "image"],
                                    "output": ["text", "image"]
                                },
                                "cost": {
                                    "input": 1.25,
                                    "output": 10,
                                    "cache_read": 0.125,
                                    "tiers": [{
                                        "type": "context",
                                        "size": 200000,
                                        "input": 2.5,
                                        "output": 15
                                    }]
                                },
                                "limit": {
                                    "context": 400000,
                                    "input": 300000,
                                    "output": 128000
                                },
                                "experimental": {
                                    "modes": {
                                        "fast": {
                                            "provider": {
                                                "headers": {
                                                    "openai-beta": "fast-mode-2026-02-01"
                                                },
                                                "body": {
                                                    "service_tier": "priority"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _codex_models = server
            .mock("GET", "/openai-codex-models.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "models": [{
                        "slug": "gpt-5",
                        "display_name": "GPT-5",
                        "description": "Frontier coding model",
                        "default_reasoning_level": "medium",
                        "supports_parallel_tool_calls": true,
                        "support_verbosity": true,
                        "default_verbosity": "low",
                        "input_modalities": ["text", "image"],
                        "context_window": 400000,
                        "supported_reasoning_levels": [{
                            "effort": "low",
                            "description": "Fast responses with lighter reasoning"
                        }, {
                            "effort": "medium",
                            "description": "Balanced reasoning"
                        }, {
                            "effort": "high",
                            "description": "Deep reasoning for complex work"
                        }, {
                            "effort": "xhigh",
                            "description": "Maximum reasoning depth"
                        }],
                        "service_tiers": [{
                            "id": "turbo",
                            "name": "Fast",
                            "description": "Priority route"
                        }],
                        "additional_speed_tiers": ["fast"]
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _router = server
            .mock("GET", "/router.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "claude": [{
                        "id": "claude-opus-4-7",
                        "display_name": "Claude Opus 4.7",
                        "owned_by": "anthropic",
                        "description": "Anthropic route",
                        "context_length": 200000,
                        "max_completion_tokens": 64000,
                        "thinking": {
                            "zero_allowed": true,
                            "levels": ["low", "high", "max"]
                        }
                    }, {
                        "id": "gemini-2.5-pro",
                        "display_name": "Gemini 2.5 Pro",
                        "owned_by": "google",
                        "description": "Google route",
                        "inputTokenLimit": 1048576,
                        "outputTokenLimit": 65536,
                        "thinking": {
                            "min": 1024,
                            "max": 32768,
                            "dynamic_allowed": true
                        }
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let dir = tempdir().expect("tempdir should create");
        let store = ModelCatalogStore::new(ModelCatalogConfig {
            cache_path: dir.path().join("model-catalog-cache.json"),
            custom_path: dir.path().join("model-catalog-custom.json"),
            cache_max_age_secs: 60,
        });
        let service = ModelCatalogService::with_remote_sources(
            store,
            vec![
                sources::ModelCatalogRemoteSource::new(
                    "models.dev",
                    sources::ModelCatalogRemoteSourceKind::ModelsDev,
                    [format!("{}/models-dev.json", server.url())],
                ),
                sources::ModelCatalogRemoteSource::new(
                    "router-for-me",
                    sources::ModelCatalogRemoteSourceKind::RouterForMe,
                    [format!("{}/router.json", server.url())],
                ),
                sources::ModelCatalogRemoteSource::new(
                    "openai-codex-models",
                    sources::ModelCatalogRemoteSourceKind::OpenAiCodexModels,
                    [format!("{}/openai-codex-models.json", server.url())],
                ),
            ],
        )
        .expect("service should load");
        let mut providers = ProviderRegistry::new();
        providers.register(StaticListProvider::new(
            "gateway",
            "openai/gpt-5",
            vec![Model::new("gateway", "openai/gpt-5")],
        ));

        let snapshot = service
            .refresh_from_registry(&providers, None)
            .await
            .expect("refresh should succeed");

        let gpt5 = snapshot
            .official
            .models
            .get("gpt-5")
            .expect("gpt-5 should exist");
        assert_eq!(gpt5.display_name.as_deref(), Some("GPT-5"));
        assert_eq!(gpt5.origin.as_deref(), Some("OpenAI"));
        assert_eq!(gpt5.context_window_tokens, Some(400_000));
        assert_eq!(gpt5.max_input_tokens, Some(300_000));
        assert_eq!(gpt5.max_output_tokens, Some(128_000));
        assert_eq!(
            gpt5.description.as_deref(),
            Some("Models.dev GPT-5 description")
        );
        assert_eq!(gpt5.knowledge_cutoff.as_deref(), Some("2025-04"));
        assert_eq!(gpt5.release_date.as_deref(), Some("2026-04-22"));
        assert_eq!(gpt5.last_updated.as_deref(), Some("2026-04-24"));
        assert_eq!(gpt5.open_weights, Some(false));
        assert_eq!(
            gpt5.default_thinking_mode.as_deref(),
            Some("thinking-medium")
        );
        assert_eq!(gpt5.supports_parallel_tool_calls, Some(true));
        assert_eq!(gpt5.supports_verbosity, Some(true));
        assert_eq!(gpt5.default_verbosity.as_deref(), Some("low"));
        assert_eq!(
            gpt5.assistant_reasoning_field.as_deref(),
            Some("reasoning_content")
        );
        assert_eq!(gpt5.output_modalities, vec!["text", "image"]);
        assert_eq!(
            gpt5.pricing
                .as_ref()
                .and_then(|pricing| pricing.input_usd_per_million_tokens.as_deref()),
            Some("1.25")
        );
        assert_eq!(
            gpt5.pricing
                .as_ref()
                .and_then(|pricing| pricing.output_usd_per_million_tokens.as_deref()),
            Some("10")
        );
        assert_eq!(
            gpt5.pricing.as_ref().map(|pricing| pricing.tiers.len()),
            Some(1)
        );
        assert_eq!(
            gpt5.capabilities
                .feature_support(ModelCapabilityFeature::Reasoning),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            gpt5.capabilities
                .feature_support(ModelCapabilityFeature::StructuredOutput),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            gpt5.capabilities
                .feature_support(ModelCapabilityFeature::Temperature),
            Some(CapabilitySupport::Unsupported)
        );
        assert!(gpt5.speed_modes.contains_key("fast"));
        assert_eq!(
            gpt5.speed_modes
                .get("fast")
                .and_then(|mode| mode.adapter_overrides.get("openai")),
            Some(&ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([(
                    "openai-beta".to_owned(),
                    "fast-mode-2026-02-01".to_owned(),
                )]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("priority"),
                )]),
            })
        );
        assert_eq!(
            gpt5.speed_modes
                .get("fast")
                .and_then(|mode| mode.description.as_deref()),
            Some("Priority route")
        );
        assert_eq!(
            gpt5.thinking_modes
                .get("thinking-high")
                .and_then(|mode| mode.description.as_deref()),
            Some("Deep reasoning for complex work")
        );
        assert_eq!(
            gpt5.thinking_modes
                .get("thinking-xhigh")
                .and_then(|mode| mode.thinking.as_ref()),
            Some(&ThinkingRequest::Effort {
                effort: crate::provider::ReasoningEffort::Xhigh,
            })
        );

        let claude = snapshot
            .official
            .models
            .get("claude-opus-4-7")
            .expect("claude-opus-4-7 should exist");
        assert_eq!(claude.origin.as_deref(), Some("Anthropic"));
        assert_eq!(claude.description.as_deref(), Some("Anthropic route"));
        assert_eq!(claude.context_window_tokens, Some(200_000));
        assert_eq!(claude.max_input_tokens, None);
        assert_eq!(claude.max_output_tokens, Some(64_000));
        assert_eq!(
            claude
                .capabilities
                .feature_support(ModelCapabilityFeature::Reasoning),
            Some(CapabilitySupport::Supported)
        );
        assert!(claude.thinking_modes.contains_key("no-thinking"));
        assert!(claude.thinking_modes.contains_key("thinking-low"));
        assert!(claude.thinking_modes.contains_key("thinking-high"));
        assert!(claude.thinking_modes.contains_key("thinking-max"));

        let gemini = snapshot
            .official
            .models
            .get("gemini-2.5-pro")
            .expect("gemini-2.5-pro should exist");
        assert_eq!(gemini.origin.as_deref(), Some("Google"));
        assert_eq!(gemini.context_window_tokens, Some(1_048_576));
        assert_eq!(gemini.max_input_tokens, Some(1_048_576));
        assert_eq!(gemini.max_output_tokens, Some(65_536));
        assert_eq!(
            gemini
                .capabilities
                .feature_support(ModelCapabilityFeature::Reasoning),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            gemini
                .thinking_modes
                .get("thinking-high")
                .and_then(|mode| mode.thinking.as_ref()),
            Some(&ThinkingRequest::Budget {
                budget_tokens: 16_384,
            })
        );
        assert_eq!(
            gemini
                .thinking_modes
                .get("thinking-max")
                .and_then(|mode| mode.thinking.as_ref()),
            Some(&ThinkingRequest::Budget {
                budget_tokens: 32_768,
            })
        );

        let merged = snapshot.merged_models();
        assert!(
            merged.appendable_model_ids.is_empty(),
            "official public sources should not append every catalog model into provider /models"
        );
    }

    #[test]
    fn merge_catalog_speed_mode_merges_request_overrides_and_adapter_overrides() {
        let mut current = ConfiguredModelSpeedMode {
            display_name: Some("Fast".to_owned()),
            description: None,
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
                body_patch: BTreeMap::from([(
                    "response_format".to_owned(),
                    serde_json::json!({
                        "type": "json_object"
                    }),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::new(),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::json!("default"),
                    )]),
                },
            )]),
            disabled: false,
        };
        let next = ConfiguredModelSpeedMode {
            display_name: None,
            description: Some("Priority route".to_owned()),
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("x-extra".to_owned(), "two".to_owned())]),
                body_patch: BTreeMap::from([(
                    "response_format".to_owned(),
                    serde_json::json!({
                        "strict": true
                    }),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::from([("openai-beta".to_owned(), "fast".to_owned())]),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::json!("priority"),
                    )]),
                },
            )]),
            disabled: false,
        };

        merge_catalog_speed_mode(&mut current, &next);

        assert_eq!(current.display_name.as_deref(), Some("Fast"));
        assert_eq!(current.description.as_deref(), Some("Priority route"));
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-base")
                .map(String::as_str),
            Some("one")
        );
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-extra")
                .map(String::as_str),
            Some("two")
        );
        assert_eq!(
            current.request_override.body_patch.get("response_format"),
            Some(&serde_json::json!({
                "type": "json_object",
                "strict": true
            }))
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.headers.get("openai-beta"))
                .map(String::as_str),
            Some("fast")
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.body_patch.get("service_tier")),
            Some(&serde_json::json!("priority"))
        );
    }

    #[test]
    fn merge_catalog_thinking_mode_fill_missing_preserves_existing_override_values() {
        let mut current = ConfiguredModelThinkingMode {
            display_name: Some("Deep".to_owned()),
            description: None,
            thinking: Some(ThinkingRequest::Effort {
                effort: ReasoningEffort::High,
            }),
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
                body_patch: BTreeMap::from([(
                    "reasoning".to_owned(),
                    serde_json::json!({ "summary": "auto" }),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::from([("x-profile".to_owned(), "deep".to_owned())]),
                    body_patch: BTreeMap::new(),
                },
            )]),
            disabled: false,
        };
        let next = ConfiguredModelThinkingMode {
            display_name: None,
            description: Some("More reasoning".to_owned()),
            thinking: Some(ThinkingRequest::Effort {
                effort: ReasoningEffort::Low,
            }),
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([
                    ("x-base".to_owned(), "two".to_owned()),
                    ("x-extra".to_owned(), "three".to_owned()),
                ]),
                body_patch: BTreeMap::from([(
                    "reasoning".to_owned(),
                    serde_json::json!({ "summary": "concise" }),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::from([
                        ("x-profile".to_owned(), "light".to_owned()),
                        ("x-extra".to_owned(), "adapter".to_owned()),
                    ]),
                    body_patch: BTreeMap::new(),
                },
            )]),
            disabled: false,
        };

        merge_catalog_thinking_mode(&mut current, &next);

        assert_eq!(current.display_name.as_deref(), Some("Deep"));
        assert_eq!(current.description.as_deref(), Some("More reasoning"));
        assert_eq!(
            current.thinking,
            Some(ThinkingRequest::Effort {
                effort: ReasoningEffort::High,
            })
        );
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-base")
                .map(String::as_str),
            Some("one")
        );
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-extra")
                .map(String::as_str),
            Some("three")
        );
        assert_eq!(
            current.request_override.body_patch.get("reasoning"),
            Some(&serde_json::json!({ "summary": "auto" }))
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.headers.get("x-profile"))
                .map(String::as_str),
            Some("deep")
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.headers.get("x-extra"))
                .map(String::as_str),
            Some("adapter")
        );
    }

    #[test]
    fn merge_catalog_speed_mode_fill_missing_preserves_higher_priority_override_values() {
        let mut current = ConfiguredModelSpeedMode {
            display_name: Some("Fast".to_owned()),
            description: None,
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([("x-base".to_owned(), "one".to_owned())]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("priority"),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::from([("openai-beta".to_owned(), "fast".to_owned())]),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::json!("priority"),
                    )]),
                },
            )]),
            disabled: false,
        };
        let next = ConfiguredModelSpeedMode {
            display_name: None,
            description: Some("Priority route".to_owned()),
            request_override: ModelSpeedModeRequestOverride {
                headers: BTreeMap::from([
                    ("x-base".to_owned(), "two".to_owned()),
                    ("x-extra".to_owned(), "three".to_owned()),
                ]),
                body_patch: BTreeMap::from([(
                    "service_tier".to_owned(),
                    serde_json::json!("turbo"),
                )]),
            },
            adapter_overrides: BTreeMap::from([(
                "openai".to_owned(),
                ModelSpeedModeRequestOverride {
                    headers: BTreeMap::from([
                        ("openai-beta".to_owned(), "slow".to_owned()),
                        ("openai-extra".to_owned(), "tier".to_owned()),
                    ]),
                    body_patch: BTreeMap::from([(
                        "service_tier".to_owned(),
                        serde_json::json!("turbo"),
                    )]),
                },
            )]),
            disabled: false,
        };

        merge_catalog_speed_mode_fill_missing(&mut current, &next);

        assert_eq!(current.description.as_deref(), Some("Priority route"));
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-base")
                .map(String::as_str),
            Some("one")
        );
        assert_eq!(
            current
                .request_override
                .headers
                .get("x-extra")
                .map(String::as_str),
            Some("three")
        );
        assert_eq!(
            current.request_override.body_patch.get("service_tier"),
            Some(&serde_json::json!("priority"))
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.headers.get("openai-beta"))
                .map(String::as_str),
            Some("fast")
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.headers.get("openai-extra"))
                .map(String::as_str),
            Some("tier")
        );
        assert_eq!(
            current
                .adapter_overrides
                .get("openai")
                .and_then(|override_patch| override_patch.body_patch.get("service_tier")),
            Some(&serde_json::json!("priority"))
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
