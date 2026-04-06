use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use futures_core::Stream;
use serde::Deserialize;

use crate::{
    error::AppError,
    model::{CapabilitySupport, Model, ModelCapabilities, ModelId, ModelLifecycle, ModelMetadata},
    provider::{
        AnthropicProvider, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        GeminiProvider, ManagedCredential, ModelProvider, OpenAiApiMode, OpenAiCompatibleProvider,
        OpenAiCompatibleStreamMode, OpenAiProvider, OpenAiStreamMode, PromptCacheShape,
        StreamResumePolicy, utils,
    },
};

#[derive(Clone)]
pub struct OpencodeProvider {
    id: String,
    default_model: ModelId,
    openai: OpenAiProvider,
    anthropic: AnthropicProvider,
    gemini: GeminiProvider,
    compatible: OpenAiCompatibleProvider,
    extra_headers: HashMap<String, String>,
    openai_responses_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpencodeBackend {
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
    OpenAiCompatible,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevProviderRecord {
    #[serde(default)]
    models: BTreeMap<String, ModelsDevModelRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevModelRecord {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    attachment: Option<bool>,
    #[serde(default)]
    tool_call: Option<bool>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    experimental: Option<bool>,
    #[serde(default)]
    limit: Option<ModelsDevLimitRecord>,
    #[serde(default)]
    modalities: Option<ModelsDevModalitiesRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevLimitRecord {
    #[serde(default)]
    context: Option<u32>,
    #[serde(default)]
    output: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevModalitiesRecord {
    #[serde(default)]
    input: Vec<String>,
}

impl OpencodeProvider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        client: reqwest::Client,
        api_key: ManagedCredential,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        _auth_header: impl Into<String>,
        _auth_scheme: Option<impl Into<String>>,
        extra_headers: HashMap<String, String>,
        stream_mode: OpenAiCompatibleStreamMode,
        realtime_ws_url: Option<String>,
    ) -> Self {
        let id = id.into();
        let default_model = ModelId::new(default_model);
        let base_url = base_url.into();
        let openai_responses_enabled = id != "opencode-go";
        let request_headers = extra_headers.clone();

        let openai_stream_mode = match stream_mode {
            OpenAiCompatibleStreamMode::Sse => OpenAiStreamMode::Sse,
            OpenAiCompatibleStreamMode::RealtimeWebSocket => OpenAiStreamMode::RealtimeWebSocket,
        };
        let openai = OpenAiProvider::new_managed(
            client.clone(),
            api_key.clone(),
            base_url.clone(),
            default_model.clone(),
        )
        .with_api_mode(OpenAiApiMode::Responses)
        .with_stream_mode(openai_stream_mode)
        .with_realtime_ws_url(realtime_ws_url.clone())
        .with_extra_headers(extra_headers.clone());

        let anthropic = AnthropicProvider::new_managed(
            client.clone(),
            api_key.clone(),
            base_url.clone(),
            default_model.clone(),
        )
        .with_extra_headers(extra_headers.clone());

        let gemini = GeminiProvider::new_managed(
            client.clone(),
            api_key.clone(),
            base_url.clone(),
            default_model.clone(),
        )
        .with_auth_header("x-goog-api-key", None::<String>)
        .with_extra_headers(extra_headers.clone());

        let compatible = OpenAiCompatibleProvider::new_managed(
            id.clone(),
            client,
            api_key,
            base_url,
            default_model.clone(),
        )
        .with_auth_header("authorization", Some("Bearer"))
        .with_top_level_prompt_cache(false)
        .with_extra_headers(extra_headers)
        .with_stream_mode(stream_mode)
        .with_realtime_ws_url(realtime_ws_url);

        Self {
            id,
            default_model,
            openai,
            anthropic,
            gemini,
            compatible,
            extra_headers: request_headers,
            openai_responses_enabled,
        }
    }

    fn backend_for_model(&self, model: &ModelId) -> OpencodeBackend {
        let normalized = Self::normalized_model(model.as_str());
        if self.openai_responses_enabled && Self::is_openai_responses_model(normalized.as_str()) {
            OpencodeBackend::OpenAiResponses
        } else if Self::is_anthropic_model(normalized.as_str()) {
            OpencodeBackend::AnthropicMessages
        } else if self.supports_gemini_route() && Self::is_gemini_model(normalized.as_str()) {
            OpencodeBackend::GeminiGenerateContent
        } else {
            OpencodeBackend::OpenAiCompatible
        }
    }

    fn supports_gemini_route(&self) -> bool {
        self.id == "opencode"
    }

    fn normalized_model(model: &str) -> String {
        model
            .trim()
            .trim_start_matches("models/")
            .to_ascii_lowercase()
    }

    fn is_openai_responses_model(normalized: &str) -> bool {
        normalized.starts_with("gpt-")
            || normalized.starts_with("o1")
            || normalized.starts_with("o3")
            || normalized.starts_with("o4")
            || normalized == "computer-use-preview"
            || normalized.starts_with("computer-use-preview")
            || normalized.contains("codex")
    }

    fn is_anthropic_model(normalized: &str) -> bool {
        normalized.starts_with("claude")
    }

    fn is_gemini_model(normalized: &str) -> bool {
        normalized.starts_with("gemini")
    }

    fn wrap_prompt_cache_shape(
        &self,
        backend: OpencodeBackend,
        inner: Option<PromptCacheShape>,
    ) -> Option<PromptCacheShape> {
        let inner = inner?;
        let backend_key = match backend {
            OpencodeBackend::OpenAiResponses => "openai_responses",
            OpencodeBackend::AnthropicMessages => "anthropic_messages",
            OpencodeBackend::GeminiGenerateContent => "gemini_generate_content",
            OpencodeBackend::OpenAiCompatible => "openai_compatible",
        };
        Some(
            PromptCacheShape::new(self.id.as_str())
                .with_string("backend", backend_key)
                .with_prefixed_shape("backend", &inner),
        )
    }

    fn request_extra_headers(&self, prompt_cache_key: Option<&str>) -> HashMap<String, String> {
        let mut headers = self.extra_headers.clone();
        if let Some(session_id) =
            utils::normalize_optional_text(prompt_cache_key.map(ToOwned::to_owned))
        {
            headers.insert("x-opencode-session".to_owned(), session_id);
        }
        headers
    }

    fn openai_for_request(&self, extra_headers: HashMap<String, String>) -> OpenAiProvider {
        self.openai.clone().with_extra_headers(extra_headers)
    }

    fn anthropic_for_request(&self, extra_headers: HashMap<String, String>) -> AnthropicProvider {
        self.anthropic.clone().with_extra_headers(extra_headers)
    }

    fn gemini_for_request(&self, extra_headers: HashMap<String, String>) -> GeminiProvider {
        self.gemini.clone().with_extra_headers(extra_headers)
    }

    fn compatible_for_request(
        &self,
        extra_headers: HashMap<String, String>,
    ) -> OpenAiCompatibleProvider {
        self.compatible.clone().with_extra_headers(extra_headers)
    }

    fn source_aligned_compatible_request(
        &self,
        mut request: CompletionRequest,
    ) -> CompletionRequest {
        request.prompt_cache_key = None;
        request
    }

    fn models_cache_path() -> Option<PathBuf> {
        if let Some(path) =
            utils::normalize_optional_text(std::env::var("OPENCODE_MODELS_PATH").ok())
        {
            return Some(PathBuf::from(path));
        }

        let home = utils::normalize_optional_text(std::env::var("HOME").ok())
            .or_else(|| utils::normalize_optional_text(std::env::var("USERPROFILE").ok()))?;
        Some(
            PathBuf::from(home)
                .join(".cache")
                .join("opencode")
                .join("models.json"),
        )
    }

    fn models_from_source_path(&self, path: &Path) -> Result<Option<Vec<Model>>, AppError> {
        let payload = fs::read_to_string(path)?;
        let providers: BTreeMap<String, ModelsDevProviderRecord> = serde_json::from_str(&payload)?;
        let Some(provider) = providers.get(self.id.as_str()) else {
            return Ok(None);
        };

        let models = provider
            .models
            .iter()
            .map(|(model_id, record)| self.model_from_source_record(model_id, record))
            .collect();
        Ok(Some(models))
    }

    fn model_from_source_record(&self, model_id: &str, record: &ModelsDevModelRecord) -> Model {
        let fallback_capabilities = self.model_capabilities(&ModelId::new(model_id));
        let fallback_metadata = self.model_metadata(&ModelId::new(model_id));
        let capabilities = self
            .model_capabilities_from_source(record)
            .with_fallbacks_from(&fallback_capabilities);
        let metadata = self.metadata_from_source(record, &fallback_metadata);

        let mut model = Model::new(self.id.clone(), model_id)
            .with_capabilities(capabilities)
            .with_metadata(metadata);
        if let Some(display_name) = utils::normalize_optional_text(record.name.clone()) {
            model = model.with_display_name(display_name);
        }
        model
    }

    fn model_capabilities_from_source(&self, record: &ModelsDevModelRecord) -> ModelCapabilities {
        let mut capabilities =
            ModelCapabilities::default().with_streaming(CapabilitySupport::Supported);

        if let Some(tool_call) = record.tool_call {
            capabilities = capabilities.with_tool_calling(if tool_call {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            });
        }

        if let Some(false) = record.attachment {
            capabilities = capabilities
                .with_image_input(CapabilitySupport::Unsupported)
                .with_document_input(CapabilitySupport::Unsupported)
                .with_audio_input(CapabilitySupport::Unsupported)
                .with_video_input(CapabilitySupport::Unsupported)
                .with_file_input(CapabilitySupport::Unsupported);
        }

        if let Some(modalities) = record.modalities.as_ref() {
            capabilities = capabilities
                .with_image_input(Self::support_for_modality(modalities, "image"))
                .with_document_input(Self::support_for_modality(modalities, "pdf"))
                .with_audio_input(Self::support_for_modality(modalities, "audio"))
                .with_video_input(Self::support_for_modality(modalities, "video"));
        }

        capabilities
    }

    fn metadata_from_source(
        &self,
        record: &ModelsDevModelRecord,
        fallback: &ModelMetadata,
    ) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();

        if let Some(family) = Self::map_models_dev_family(record.family.as_deref()) {
            metadata = metadata.with_family(family);
        } else if record.family.is_none() {
            metadata = metadata
                .with_fallbacks_from(&ModelMetadata::default().with_fallbacks_from(fallback));
        }

        match Self::map_models_dev_lifecycle(record.status.as_deref(), record.experimental) {
            Some(lifecycle) => {
                metadata = metadata.with_lifecycle(lifecycle);
            }
            None if record.status.is_none() && record.experimental != Some(true) => {
                if let Some(lifecycle) = fallback.lifecycle {
                    metadata = metadata.with_lifecycle(lifecycle);
                }
            }
            None => {}
        }

        if let Some(limit) = record.limit.as_ref() {
            if let Some(context) = limit.context {
                metadata = metadata.with_context_window_tokens(context);
            }
            if let Some(output) = limit.output {
                metadata = metadata.with_max_output_tokens(output);
            }
        }

        metadata.with_fallbacks_from(&ModelMetadata {
            family: None,
            lifecycle: None,
            ..fallback.clone()
        })
    }

    fn support_for_modality(
        modalities: &ModelsDevModalitiesRecord,
        expected: &str,
    ) -> CapabilitySupport {
        if modalities.input.iter().any(|item| item == expected) {
            CapabilitySupport::Supported
        } else {
            CapabilitySupport::Unsupported
        }
    }

    fn map_models_dev_family(family: Option<&str>) -> Option<crate::model::ModelFamily> {
        let family = family?.trim().to_ascii_lowercase();
        if family.contains("codex") {
            return Some(crate::model::ModelFamily::Codex);
        }
        if family.contains("claude") {
            return Some(crate::model::ModelFamily::Claude);
        }
        if family.contains("gemini") {
            return Some(crate::model::ModelFamily::Gemini);
        }
        if family.contains("llama") {
            return Some(crate::model::ModelFamily::Llama);
        }
        if family.contains("mistral") {
            return Some(crate::model::ModelFamily::Mistral);
        }
        if family.contains("deepseek") {
            return Some(crate::model::ModelFamily::Deepseek);
        }
        if family.contains("qwen") {
            return Some(crate::model::ModelFamily::Qwen);
        }
        if family.contains("nova") {
            return Some(crate::model::ModelFamily::Nova);
        }
        if family.starts_with("gpt") {
            return Some(crate::model::ModelFamily::Gpt);
        }
        None
    }

    fn map_models_dev_lifecycle(
        status: Option<&str>,
        experimental: Option<bool>,
    ) -> Option<ModelLifecycle> {
        if experimental == Some(true) {
            return Some(ModelLifecycle::Experimental);
        }

        match status?.trim().to_ascii_lowercase().as_str() {
            "alpha" => Some(ModelLifecycle::Alpha),
            "beta" => Some(ModelLifecycle::Beta),
            "deprecated" => Some(ModelLifecycle::Deprecated),
            _ => None,
        }
    }
}

#[async_trait]
impl ModelProvider for OpencodeProvider {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn model_capabilities(&self, model: &ModelId) -> crate::provider::ModelCapabilities {
        match self.backend_for_model(model) {
            OpencodeBackend::OpenAiResponses => self.openai.model_capabilities(model),
            OpencodeBackend::AnthropicMessages => self.anthropic.model_capabilities(model),
            OpencodeBackend::GeminiGenerateContent => self.gemini.model_capabilities(model),
            OpencodeBackend::OpenAiCompatible => self.compatible.model_capabilities(model),
        }
    }

    fn model_metadata(&self, model: &ModelId) -> crate::provider::ModelMetadata {
        match self.backend_for_model(model) {
            OpencodeBackend::OpenAiResponses => self.openai.model_metadata(model),
            OpencodeBackend::AnthropicMessages => self.anthropic.model_metadata(model),
            OpencodeBackend::GeminiGenerateContent => self.gemini.model_metadata(model),
            OpencodeBackend::OpenAiCompatible => self.compatible.model_metadata(model),
        }
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        // This wrapper fans out to multiple backend protocols with different replay safety.
        // Registry replay policy is provider-level today, so stay conservative here.
        StreamResumePolicy::Disabled
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        match self.backend_for_model(model) {
            OpencodeBackend::OpenAiResponses => self.openai.supports_prompt_continuation(model),
            OpencodeBackend::AnthropicMessages => {
                self.anthropic.supports_prompt_continuation(model)
            }
            OpencodeBackend::GeminiGenerateContent => {
                self.gemini.supports_prompt_continuation(model)
            }
            OpencodeBackend::OpenAiCompatible => {
                self.compatible.supports_prompt_continuation(model)
            }
        }
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        let backend = self.backend_for_model(model);
        let inner = match backend {
            OpencodeBackend::OpenAiResponses => self.openai.prompt_cache_shape(model),
            OpencodeBackend::AnthropicMessages => self.anthropic.prompt_cache_shape(model),
            OpencodeBackend::GeminiGenerateContent => self.gemini.prompt_cache_shape(model),
            OpencodeBackend::OpenAiCompatible => self.compatible.prompt_cache_shape(model),
        };
        self.wrap_prompt_cache_shape(backend, inner)
    }

    async fn list_models(&self) -> Result<Vec<Model>, AppError> {
        if let Some(path) = Self::models_cache_path() {
            match self.models_from_source_path(path.as_path()) {
                Ok(Some(models)) => return Ok(models),
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(
                        provider_id = %self.id,
                        path = %path.display(),
                        error = %err,
                        "failed to read opencode models cache; falling back"
                    );
                }
            }
        }

        if self.id == "opencode-go" {
            return Ok(vec![
                Model::new(self.id.clone(), self.default_model.clone())
                    .with_capabilities(self.model_capabilities(&self.default_model))
                    .with_metadata(self.model_metadata(&self.default_model)),
            ]);
        }

        self.compatible.list_models().await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let backend = self.backend_for_model(&request.model);
        let extra_headers = self.request_extra_headers(request.prompt_cache_key.as_deref());
        match backend {
            OpencodeBackend::OpenAiResponses => {
                self.openai_for_request(extra_headers)
                    .complete(request)
                    .await
            }
            OpencodeBackend::AnthropicMessages => {
                self.anthropic_for_request(extra_headers)
                    .complete(request)
                    .await
            }
            OpencodeBackend::GeminiGenerateContent => {
                self.gemini_for_request(extra_headers)
                    .complete(request)
                    .await
            }
            OpencodeBackend::OpenAiCompatible => {
                self.compatible_for_request(extra_headers)
                    .complete(self.source_aligned_compatible_request(request))
                    .await
            }
        }
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let backend = self.backend_for_model(&request.model);
        let extra_headers = self.request_extra_headers(request.prompt_cache_key.as_deref());
        match backend {
            OpencodeBackend::OpenAiResponses => {
                self.openai_for_request(extra_headers)
                    .complete_stream(request)
                    .await
            }
            OpencodeBackend::AnthropicMessages => {
                self.anthropic_for_request(extra_headers)
                    .complete_stream(request)
                    .await
            }
            OpencodeBackend::GeminiGenerateContent => {
                self.gemini_for_request(extra_headers)
                    .complete_stream(request)
                    .await
            }
            OpencodeBackend::OpenAiCompatible => {
                self.compatible_for_request(extra_headers)
                    .complete_stream(self.source_aligned_compatible_request(request))
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message::Message;
    use std::{
        ffi::OsStr,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    struct TempFileGuard {
        path: PathBuf,
    }

    impl TempFileGuard {
        fn new_json(value: serde_json::Value) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "agena-opencode-models-{}-{unique}.json",
                std::process::id()
            ));
            std::fs::write(
                &path,
                serde_json::to_vec(&value).expect("json payload should encode"),
            )
            .expect("temp models cache should be written");
            Self { path }
        }

        fn path(&self) -> &Path {
            self.path.as_path()
        }
    }

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn gpt_models_enable_prompt_continuation_through_openai_responses() {
        let provider = OpencodeProvider::new(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
            "authorization",
            Some("Bearer"),
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        assert!(provider.supports_prompt_continuation(&ModelId::new("gpt-5.3-codex")));
        assert!(!provider.supports_prompt_continuation(&ModelId::new("claude-sonnet-4-5")));
        assert!(!provider.supports_prompt_continuation(&ModelId::new("gemini-3-pro")));
    }

    #[test]
    fn prompt_cache_shape_tracks_selected_backend() {
        let provider = OpencodeProvider::new(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            "https://opencode.ai/zen/v1",
            "gemini-3-pro",
            "authorization",
            Some("Bearer"),
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        let gpt_shape = provider
            .prompt_cache_shape(&ModelId::new("gpt-5.3-codex"))
            .expect("shape should exist");
        let gemini_shape = provider
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");
        let claude_shape = provider
            .prompt_cache_shape(&ModelId::new("claude-sonnet-4-5"))
            .expect("shape should exist");

        assert_eq!(gpt_shape.provider_id(), "opencode");
        assert_eq!(
            gpt_shape.fields.get("backend").map(String::as_str),
            Some("openai_responses")
        );
        assert_eq!(
            claude_shape.fields.get("backend").map(String::as_str),
            Some("anthropic_messages")
        );
        assert_eq!(
            gemini_shape.fields.get("backend").map(String::as_str),
            Some("gemini_generate_content")
        );
        assert_ne!(gpt_shape.fingerprint(), claude_shape.fingerprint());
        assert_ne!(claude_shape.fingerprint(), gemini_shape.fingerprint());
        assert_ne!(gpt_shape.fingerprint(), gemini_shape.fingerprint());
    }

    #[tokio::test]
    async fn gpt_models_use_responses_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _responses = server
            .mock("POST", "/responses")
            .expect(1)
            .match_header("authorization", "Bearer sk-test")
            .match_header("x-opencode-session", "session-42")
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .match_body(mockito::Matcher::Regex(
                "\\\"previous_response_id\\\":\\\"resp_prev\\\"".to_owned(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "resp_next",
                    "model": "gpt-5.3-codex",
                    "output_text": "ok",
                    "stop_reason": "stop"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpencodeProvider::new(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            server.url(),
            "gemini-3-pro",
            "x-api-key",
            None::<String>,
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gpt-5.3-codex"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: Some("resp_prev".to_owned()),
                prompt_window_generation: Some(7),
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
        assert_eq!(
            response
                .provider_metadata
                .as_ref()
                .and_then(|value| value.get("response_id"))
                .and_then(|value| value.as_str()),
            Some("resp_next")
        );
    }

    #[tokio::test]
    async fn non_openai_models_keep_openai_compatible_transport() {
        let mut server = mockito::Server::new_async().await;
        let _messages = server
            .mock("POST", "/messages")
            .expect(1)
            .match_header("x-api-key", "sk-test")
            .match_header("anthropic-version", "2023-06-01")
            .match_header("x-opencode-session", "session-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "claude-sonnet-4-5",
                    "stop_reason": "end_turn",
                    "content": [{ "type": "text", "text": "ok" }],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _gemini = server
            .mock("POST", "/models/gemini-3-pro:generateContent")
            .expect(1)
            .match_header("x-goog-api-key", "sk-test")
            .match_header("x-opencode-route", "gemini-a")
            .match_header("x-opencode-session", "session-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "text": "ok" }]
                        },
                        "finishReason": "STOP"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex(
                "\\\"cache_control\\\":\\{\\\"type\\\":\\\"ephemeral\\\"\\}".to_owned(),
            ))
            .expect(0)
            .create_async()
            .await;
        let _chat_prompt_cache_key = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex(
                "\\\"prompt_cache_key\\\":\\\"session-42\\\"".to_owned(),
            ))
            .expect(0)
            .create_async()
            .await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .match_header("authorization", "Bearer sk-test")
            .match_header("x-opencode-session", "session-42")
            .match_header("x-session-affinity", mockito::Matcher::Missing)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "mistral-medium",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpencodeProvider::new(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            server.url(),
            "gemini-3-pro",
            "x-api-key",
            None::<String>,
            HashMap::from([("x-opencode-route".to_owned(), "gemini-a".to_owned())]),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        let claude = provider
            .complete(CompletionRequest {
                model: ModelId::new("claude-sonnet-4-5"),
                system: Some("system".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: Some("resp_prev".to_owned()),
                prompt_window_generation: Some(7),
            })
            .await
            .expect("anthropic-backed completion should succeed");
        assert_eq!(claude.text, "ok");

        let gemini = provider
            .complete(CompletionRequest {
                model: ModelId::new("gemini-3-pro"),
                system: Some("system".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: Some("resp_prev".to_owned()),
                prompt_window_generation: Some(7),
            })
            .await
            .expect("gemini-backed completion should succeed");
        assert_eq!(gemini.text, "ok");

        let compatible = provider
            .complete(CompletionRequest {
                model: ModelId::new("mistral-medium"),
                system: Some("system".to_owned()),
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: Some("resp_prev".to_owned()),
                prompt_window_generation: Some(7),
            })
            .await
            .expect("openai-compatible completion should succeed");

        assert_eq!(compatible.text, "ok");
    }

    #[tokio::test]
    async fn opencode_go_gemini_models_fall_back_to_openai_compatible_transport() {
        let mut server = mockito::Server::new_async().await;
        let _chat = server
            .mock("POST", "/chat/completions")
            .expect(1)
            .match_header("authorization", "Bearer sk-test")
            .match_header("x-opencode-session", "session-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "model": "gemini-3-pro",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;
        let _gemini = server
            .mock("POST", "/models/gemini-3-pro:generateContent")
            .expect(0)
            .create_async()
            .await;

        let provider = OpencodeProvider::new(
            "opencode-go",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            server.url(),
            "kimi-k2.5",
            "authorization",
            Some("Bearer"),
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        let shape = provider
            .prompt_cache_shape(&ModelId::new("gemini-3-pro"))
            .expect("shape should exist");
        assert_eq!(
            shape.fields.get("backend").map(String::as_str),
            Some("openai_compatible")
        );

        let response = provider
            .complete(CompletionRequest {
                model: ModelId::new("gemini-3-pro"),
                system: None,
                messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: Some(32),
                prompt_cache_key: Some("session-42".to_owned()),
                previous_response_id: None,
                prompt_window_generation: Some(1),
            })
            .await
            .expect("completion should succeed");

        assert_eq!(response.text, "ok");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_models_prefers_opencode_source_cache() {
        let cache = TempFileGuard::new_json(serde_json::json!({
            "opencode": {
                "models": {
                    "gpt-5.4": {
                        "name": "GPT-5.4",
                        "family": "gpt",
                        "attachment": false,
                        "tool_call": true,
                        "status": "beta",
                        "limit": { "context": 400000, "output": 32000 },
                        "modalities": { "input": ["text"] }
                    }
                }
            },
            "opencode-go": {
                "models": {
                    "kimi-k2.5": {
                        "name": "Kimi K2.5",
                        "family": "kimi",
                        "attachment": false,
                        "tool_call": true,
                        "status": "beta",
                        "limit": { "context": 262144, "output": 8192 },
                        "modalities": { "input": ["text"] }
                    },
                    "mimo-v2-pro": {
                        "name": "Mimo V2 Pro",
                        "family": "mimo-pro",
                        "attachment": true,
                        "tool_call": false,
                        "experimental": true,
                        "limit": { "context": 131072, "output": 4096 },
                        "modalities": { "input": ["text", "image"] }
                    }
                }
            }
        }));
        let _env = EnvVarGuard::set("OPENCODE_MODELS_PATH", cache.path());
        let mut server = mockito::Server::new_async().await;
        let _full_models = server.mock("GET", "/models").expect(0).create_async().await;

        let full = OpencodeProvider::new(
            "opencode",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            server.url(),
            "gpt-5.4",
            "authorization",
            Some("Bearer"),
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );
        let go = OpencodeProvider::new(
            "opencode-go",
            reqwest::Client::new(),
            ManagedCredential::static_value("opencode api key", "sk-test"),
            server.url(),
            "kimi-k2.5",
            "authorization",
            Some("Bearer"),
            HashMap::new(),
            OpenAiCompatibleStreamMode::Sse,
            None::<String>,
        );

        let full_models = full.list_models().await.expect("full models should load");
        let go_models = go.list_models().await.expect("go models should load");

        assert_eq!(full_models.len(), 1);
        assert_eq!(full_models[0].id.as_str(), "gpt-5.4");
        assert_eq!(full_models[0].display_name.as_deref(), Some("GPT-5.4"));
        assert_eq!(
            full_models[0].metadata.family,
            Some(crate::model::ModelFamily::Gpt)
        );
        assert_eq!(
            full_models[0].metadata.lifecycle,
            Some(ModelLifecycle::Beta)
        );
        assert_eq!(
            full_models[0].metadata.limits.context_window_tokens,
            Some(400000)
        );

        assert_eq!(go_models.len(), 2);
        let kimi = go_models
            .iter()
            .find(|model| model.id.as_str() == "kimi-k2.5")
            .expect("kimi model should exist");
        assert_eq!(kimi.display_name.as_deref(), Some("Kimi K2.5"));
        assert_eq!(kimi.metadata.family, None);
        assert_eq!(kimi.metadata.lifecycle, Some(ModelLifecycle::Beta));
        assert_eq!(kimi.metadata.limits.context_window_tokens, Some(262144));
        assert_eq!(kimi.metadata.limits.max_output_tokens, Some(8192));
        assert_eq!(
            kimi.capabilities.image_input,
            CapabilitySupport::Unsupported
        );
        assert_eq!(kimi.capabilities.tool_calling, CapabilitySupport::Supported);
        assert_eq!(kimi.capabilities.streaming, CapabilitySupport::Supported);

        let mimo = go_models
            .iter()
            .find(|model| model.id.as_str() == "mimo-v2-pro")
            .expect("mimo model should exist");
        assert_eq!(mimo.metadata.family, None);
        assert_eq!(mimo.metadata.lifecycle, Some(ModelLifecycle::Experimental));
        assert_eq!(mimo.capabilities.image_input, CapabilitySupport::Supported);
        assert_eq!(
            mimo.capabilities.tool_calling,
            CapabilitySupport::Unsupported
        );
    }
}
