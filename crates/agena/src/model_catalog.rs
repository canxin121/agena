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

use crate::{
    AppError,
    config::{ConfigResolution, ProviderAdapterDefinition, ProviderCapabilityFamilyConfig},
    model::{
        CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality, ModelLifecycle,
    },
    provider::{
        ConfiguredModelDefinition, ConfiguredModelVariant, FeatureCapabilityPatch,
        FeatureCapabilityPatchBody, InputCapabilityPatch, InputCapabilityPatchBody,
        ModelCapabilityFeature, ModelCapabilityPatch, ModelProvider, ProviderRegistry,
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
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
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
            && self.origin.is_none()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
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
            origin: self.origin.clone(),
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
}

impl ModelCatalogService {
    pub fn new(store: ModelCatalogStore) -> Result<Self, AppError> {
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
        let mut provider_ids = providers.provider_ids();
        provider_ids.sort_by(|left, right| {
            provider_priority(right.as_str(), resolution)
                .cmp(&provider_priority(left.as_str(), resolution))
                .then_with(|| left.cmp(right))
        });

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
                "no enabled providers exposed model lists".to_owned()
            } else {
                errors.join("; ")
            };
            return Err(AppError::Config(format!(
                "model catalog generation failed: {detail}"
            )));
        }

        let document = curate::curate_catalog_document(ModelCatalogDocument { models: raw_models })
            .map_err(|err| AppError::Config(format!("curate generated model catalog: {err}")))?;
        let warning = (!errors.is_empty()).then(|| {
            format!(
                "catalog generated from {succeeded} provider(s); skipped {} provider(s): {}",
                errors.len(),
                errors.join("; ")
            )
        });
        Ok((document, warning))
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

pub(crate) fn canonical_model_catalog_id(model_id: &str) -> String {
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
        max_output_tokens: model.metadata.limits.max_output_tokens,
        description: model.metadata.description.clone(),
        display_name: model.display_name.clone(),
        origin: None,
        variants: model
            .variants
            .iter()
            .map(|(name, variant)| {
                (
                    name.clone(),
                    ConfiguredModelVariant {
                        display_name: variant.display_name.clone(),
                        description: variant.description.clone(),
                        thinking: variant.thinking.clone(),
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
    if current.max_output_tokens.is_none() {
        current.max_output_tokens = next.max_output_tokens;
    }
    if current.description.is_none() {
        current.description = next.description.clone();
    }
    if current.display_name.is_none() {
        current.display_name = next.display_name.clone();
    }
    if current.origin.is_none() {
        current.origin = next.origin.clone();
    }
    for (name, variant) in &next.variants {
        current
            .variants
            .entry(name.clone())
            .or_insert_with(|| variant.clone());
    }
    merge_capability_patch(&mut current.capabilities, &next.capabilities);
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
    use crate::{
        model::{CapabilitySupport, ModelId, ModelMetadata},
        provider::ConfiguredModelVariant,
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

        let service = ModelCatalogService::new(store).expect("service should load");
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

        let service = ModelCatalogService::new(store).expect("service should load");
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
