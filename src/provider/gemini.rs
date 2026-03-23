use async_trait::async_trait;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    message::MessageUsage,
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        ModelProvider, ProviderModel, sse, utils,
    },
    role::Role,
};

const PROVIDER_ID: &str = "google";

#[derive(Clone)]
pub struct GeminiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model: String,
}

impl GeminiProvider {
    pub fn new(
        client: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            base_url: utils::normalize_base_url(base_url.into().as_str()),
            default_model: default_model.into(),
        }
    }

    pub fn from_env(client: reqwest::Client) -> Result<Self, AppError> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_API_KEY"))
            .map_err(|_| AppError::Config("GEMINI_API_KEY or GOOGLE_API_KEY is not set".into()))?;
        let base_url = std::env::var("GEMINI_BASE_URL")
            .unwrap_or_else(|_| "https://generativelanguage.googleapis.com/v1beta".to_owned());
        let default_model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_owned());

        Ok(Self::new(client, api_key, base_url, default_model))
    }

    fn list_models_endpoint(&self) -> String {
        format!("{}/models?key={}", self.base_url, self.api_key)
    }

    fn generate_endpoint(&self, model: &str) -> String {
        let model_name = if model.starts_with("models/") {
            model.to_owned()
        } else {
            format!("models/{model}")
        };
        format!(
            "{}/{}:generateContent?key={}",
            self.base_url, model_name, self.api_key
        )
    }

    fn stream_generate_endpoint(&self, model: &str) -> String {
        let model_name = if model.starts_with("models/") {
            model.to_owned()
        } else {
            format!("models/{model}")
        };
        format!(
            "{}/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, model_name, self.api_key
        )
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        let response = self.client.get(self.list_models_endpoint()).send().await?;

        let payload: GeminiModelListResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        Ok(payload
            .models
            .into_iter()
            .map(|m| ProviderModel {
                provider_id: PROVIDER_ID.to_owned(),
                id: m.name.trim_start_matches("models/").to_owned(),
                display_name: m.display_name,
            })
            .collect())
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system);
        }

        let mut contents = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
                Role::Assistant => contents.push(GeminiContent {
                    role: Some("model".to_owned()),
                    parts: vec![GeminiPart {
                        text: msg.as_text_lossy(),
                    }],
                }),
                Role::User | Role::Tool => contents.push(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: vec![GeminiPart {
                        text: msg.as_text_lossy(),
                    }],
                }),
            }
        }

        let body = GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart {
                    text: system_chunks.join("\n\n"),
                }],
            }),
            contents,
            generation_config: GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
            },
            stream: None,
        };

        let response = self
            .client
            .post(self.generate_endpoint(&model))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        let payload: GeminiGenerateResponse =
            utils::parse_json_response(PROVIDER_ID, response).await?;
        let text = payload
            .candidates
            .first()
            .map(GeminiCandidate::text)
            .unwrap_or_default();

        let finish_reason = payload
            .candidates
            .first()
            .and_then(|c| c.finish_reason.clone());
        let usage = payload.usage_metadata.map(map_gemini_usage);

        Ok(CompletionResponse {
            provider_id: PROVIDER_ID.to_owned(),
            model,
            text,
            finish_reason: CompletionFinishReason::from_provider(finish_reason.as_deref()),
            tool_calls: Vec::new(),
            usage,
            provider_metadata: payload
                .candidates
                .first()
                .and_then(GeminiCandidate::provider_metadata),
        })
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = if request.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let mut system_chunks = Vec::new();
        if let Some(system) = request.system.filter(|s| !s.trim().is_empty()) {
            system_chunks.push(system);
        }

        let mut contents = Vec::new();
        for msg in request.messages {
            match msg.role {
                Role::System => system_chunks.push(msg.as_text_lossy()),
                Role::Assistant => contents.push(GeminiContent {
                    role: Some("model".to_owned()),
                    parts: vec![GeminiPart {
                        text: msg.as_text_lossy(),
                    }],
                }),
                Role::User | Role::Tool => contents.push(GeminiContent {
                    role: Some("user".to_owned()),
                    parts: vec![GeminiPart {
                        text: msg.as_text_lossy(),
                    }],
                }),
            }
        }

        let body = GeminiGenerateRequest {
            system_instruction: (!system_chunks.is_empty()).then(|| GeminiInstruction {
                parts: vec![GeminiPart {
                    text: system_chunks.join("\n\n"),
                }],
            }),
            contents,
            generation_config: GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
            },
            stream: Some(true),
        };

        let response = self
            .client
            .post(self.stream_generate_endpoint(&model))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(utils::http_status_error_from_response(PROVIDER_ID, response).await);
        }

        let mut events = sse::json_events(response);
        let provider_id = PROVIDER_ID.to_owned();
        let model_name = model;

        let stream = async_stream::try_stream! {
            let mut emitted = String::new();

            while let Some(event) = events.next().await {
                let event = event?;

                let chunk: GeminiGenerateResponse =
                    utils::parse_json_value(provider_id.as_str(), "stream chunk", event)?;
                let mut done = false;

                for stream_event in GeminiStreamEvent::from_chunk(chunk, &mut emitted) {
                    match stream_event {
                        GeminiStreamEvent::TextDelta(delta) => {
                            yield CompletionStreamEvent::TextDelta {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                delta,
                            };
                        }
                        GeminiStreamEvent::Completed {
                            finish_reason,
                            usage,
                            provider_metadata,
                        } => {
                            yield CompletionStreamEvent::Completed {
                                provider_id: provider_id.clone(),
                                model: model_name.clone(),
                                finish_reason: CompletionFinishReason::from_provider(
                                    Some(finish_reason.as_str()),
                                ),
                                usage,
                                provider_metadata,
                            };
                            done = true;
                            break;
                        }
                    }
                }

                if done {
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct GeminiGenerateRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiInstruction>,
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct GeminiInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelListResponse {
    #[serde(default)]
    models: Vec<GeminiModel>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiGenerateResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
    #[serde(default, rename = "safetyRatings")]
    safety_ratings: Option<serde_json::Value>,
    #[serde(default, rename = "groundingMetadata")]
    grounding_metadata: Option<serde_json::Value>,
}

impl GeminiCandidate {
    fn text(&self) -> String {
        self.content
            .as_ref()
            .map(|content| {
                content
                    .parts
                    .iter()
                    .map(|part| part.text.clone())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    }

    fn provider_metadata(&self) -> Option<serde_json::Value> {
        let mut map = serde_json::Map::new();
        if let Some(s) = self.safety_ratings.clone() {
            map.insert("safety_ratings".to_owned(), s);
        }
        if let Some(g) = self.grounding_metadata.clone() {
            map.insert("grounding_metadata".to_owned(), g);
        }
        (!map.is_empty()).then_some(serde_json::Value::Object(map))
    }
}

fn map_gemini_usage(u: GeminiUsageMetadata) -> crate::provider::CompletionUsage {
    MessageUsage {
        input_tokens: u.prompt_token_count.unwrap_or_default(),
        output_tokens: u.candidates_token_count.unwrap_or_default(),
        reasoning_tokens: u.thoughts_token_count.unwrap_or_default(),
        cache_write_tokens: 0,
        cache_read_tokens: u.cached_content_token_count.unwrap_or_default(),
        total_cost: 0.0,
    }
    .into()
}

#[derive(Debug)]
enum GeminiStreamEvent {
    TextDelta(String),
    Completed {
        finish_reason: String,
        usage: Option<crate::provider::CompletionUsage>,
        provider_metadata: Option<serde_json::Value>,
    },
}

impl GeminiStreamEvent {
    fn from_chunk(chunk: GeminiGenerateResponse, emitted: &mut String) -> Vec<Self> {
        let mut events = Vec::new();
        let candidate = chunk.candidates.first();

        if let Some(candidate) = candidate {
            let full_text = candidate.text();
            if full_text.starts_with(emitted.as_str()) {
                let delta = full_text[emitted.len()..].to_owned();
                if !delta.is_empty() {
                    *emitted = full_text;
                    events.push(Self::TextDelta(delta));
                }
            } else if !full_text.is_empty() {
                *emitted = full_text.clone();
                events.push(Self::TextDelta(full_text));
            }

            if let Some(finish_reason) = candidate.finish_reason.clone() {
                events.push(Self::Completed {
                    finish_reason,
                    usage: chunk.usage_metadata.map(map_gemini_usage),
                    provider_metadata: candidate.provider_metadata(),
                });
            }
        }

        events
    }
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
    #[serde(default, rename = "thoughtsTokenCount")]
    thoughts_token_count: Option<u64>,
    #[serde(default, rename = "cachedContentTokenCount")]
    cached_content_token_count: Option<u64>,
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    use crate::provider::{CompletionRequest, ProviderMessage};

    #[tokio::test]
    async fn complete_stream_parses_typed_gemini_chunks() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"finishReason\":\"STOP\",\"content\":{\"parts\":[{\"text\":\"Hello\"}]},\"safetyRatings\":[{\"category\":\"SAFE\"}]}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"thoughtsTokenCount\":1,\"cachedContentTokenCount\":1}}\n\n",
            "data: [DONE]\n\n"
        );

        let _mock = server
            .mock("POST", "/models/gemini-2.5-flash:streamGenerateContent")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("alt".to_owned(), "sse".to_owned()),
                mockito::Matcher::UrlEncoded("key".to_owned(), "test-key".to_owned()),
            ]))
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            "test-key",
            server.url(),
            "gemini-2.5-flash",
        );

        let mut stream = provider
            .complete_stream(CompletionRequest {
                model: "gemini-2.5-flash".to_owned(),
                system: None,
                messages: vec![ProviderMessage::new(crate::role::Role::User, "hello")],
                temperature: None,
                max_output_tokens: Some(64),
            })
            .await
            .expect("stream should start");

        let mut text = String::new();
        let mut done = false;

        while let Some(item) = stream.next().await {
            match item.expect("stream item should parse") {
                CompletionStreamEvent::TextDelta { delta, .. } => text.push_str(delta.as_str()),
                CompletionStreamEvent::Completed {
                    finish_reason,
                    usage,
                    provider_metadata,
                    ..
                } => {
                    assert!(matches!(finish_reason, Some(CompletionFinishReason::Stop)));
                    let usage = usage.expect("usage should be present");
                    assert_eq!(usage.input_tokens, 4);
                    assert_eq!(usage.output_tokens, 2);
                    assert_eq!(usage.reasoning_tokens, 1);
                    assert_eq!(usage.cache_read_tokens, 1);
                    let metadata = provider_metadata.expect("provider metadata should be present");
                    assert!(metadata.get("safety_ratings").is_some());
                    done = true;
                }
                _ => {}
            }
        }

        assert_eq!(text, "Hello");
        assert!(done);
    }
}
