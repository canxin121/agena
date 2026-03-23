use async_trait::async_trait;
use futures_core::Stream;
use std::collections::HashMap;

use crate::{
    auth::AuthData,
    error::AppError,
    provider::{
        CompletionRequest, CompletionResponse, CompletionStreamEvent, ModelProvider,
        OpenAiCompatibleProvider, ProviderModel,
    },
};

const PROVIDER_ID: &str = "cloudflare-ai-gateway";
const DEFAULT_MODEL: &str = "workers-ai/@cf/meta/llama-3.1-8b-instruct";

#[derive(Clone)]
pub struct CloudflareAiGatewayProvider {
    inner: OpenAiCompatibleProvider,
}

impl CloudflareAiGatewayProvider {
    pub fn from_env_and_auth(
        client: reqwest::Client,
        auth: Option<&AuthData>,
    ) -> Result<Option<Self>, AppError> {
        let account_id = env_non_empty("CLOUDFLARE_ACCOUNT_ID");
        let gateway_id = env_non_empty("CLOUDFLARE_GATEWAY_ID");

        let (Some(account_id), Some(gateway_id)) = (account_id, gateway_id) else {
            return Ok(None);
        };

        let api_token = env_non_empty("CLOUDFLARE_API_TOKEN")
            .or_else(|| env_non_empty("CF_AIG_TOKEN"))
            .or_else(|| auth.and_then(AuthData::api_key).map(ToOwned::to_owned));

        let Some(api_token) = api_token else {
            return Err(AppError::Config(
                "CLOUDFLARE_API_TOKEN (or CF_AIG_TOKEN) is required for cloudflare-ai-gateway"
                    .to_owned(),
            ));
        };

        let base_url = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_BASE_URL")
            .unwrap_or_else(|| {
                format!("https://gateway.ai.cloudflare.com/v1/{account_id}/{gateway_id}/compat")
            });

        let default_model = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_MODEL")
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());

        let mut inner =
            OpenAiCompatibleProvider::new(PROVIDER_ID, client, api_token, base_url, default_model);

        let mut extra_headers = HashMap::new();
        if let Some(raw_headers) =
            env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_HEADERS_JSON")
        {
            extra_headers.extend(parse_headers_json(raw_headers.as_str())?);
        }

        if let Some(metadata) = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_METADATA_JSON")
        {
            let metadata = normalize_json_value(metadata.as_str()).map_err(|err| {
                AppError::Config(format!(
                    "invalid cloudflare ai gateway metadata json: {err}"
                ))
            })?;
            extra_headers.insert("cf-aig-metadata".to_owned(), metadata);
        }

        if let Some(cache_ttl) = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_CACHE_TTL") {
            cache_ttl.parse::<u64>().map_err(|err| {
                AppError::Config(format!(
                    "invalid cloudflare ai gateway cache ttl `{cache_ttl}`: {err}"
                ))
            })?;
            extra_headers.insert("cf-aig-cache-ttl".to_owned(), cache_ttl);
        }

        if let Some(cache_key) = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_CACHE_KEY") {
            extra_headers.insert("cf-aig-cache-key".to_owned(), cache_key);
        }

        if let Some(skip_cache) = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_SKIP_CACHE") {
            extra_headers.insert(
                "cf-aig-skip-cache".to_owned(),
                parse_bool_flag(skip_cache.as_str())?.to_string(),
            );
        }

        if let Some(collect_log) = env_non_empty("AGENA_PROVIDER_CLOUDFLARE_AI_GATEWAY_COLLECT_LOG")
        {
            extra_headers.insert(
                "cf-aig-collect-log".to_owned(),
                parse_bool_flag(collect_log.as_str())?.to_string(),
            );
        }

        if !extra_headers.is_empty() {
            inner = inner.with_extra_headers(extra_headers);
        }

        Ok(Some(Self { inner }))
    }

    fn normalize_model(&self, model: &str) -> Result<String, AppError> {
        let normalized = if model.trim().is_empty() {
            self.inner.default_model().to_owned()
        } else {
            model.trim().to_owned()
        };

        if normalized.contains('/') && !normalized.starts_with('/') && !normalized.ends_with('/') {
            return Ok(normalized);
        }

        Err(AppError::Config(format!(
            "{PROVIDER_ID} requires unified model id in `provider/model` format"
        )))
    }
}

#[async_trait]
impl ModelProvider for CloudflareAiGatewayProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        self.inner.list_models().await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        let model = self.normalize_model(request.model.as_str())?;
        self.inner
            .complete(CompletionRequest { model, ..request })
            .await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let model = self.normalize_model(request.model.as_str())?;
        self.inner
            .complete_stream(CompletionRequest { model, ..request })
            .await
    }
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn parse_headers_json(value: &str) -> Result<HashMap<String, String>, AppError> {
    serde_json::from_str::<HashMap<String, String>>(value)
        .map_err(|e| AppError::Config(format!("invalid cloudflare ai gateway headers json: {e}")))
}

fn normalize_json_value(raw: &str) -> Result<String, serde_json::Error> {
    serde_json::from_str::<serde_json::Value>(raw).map(|v| v.to_string())
}

fn parse_bool_flag(raw: &str) -> Result<bool, AppError> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        _ => Err(AppError::Config(format!(
            "invalid cloudflare ai gateway boolean flag `{raw}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_model_requires_unified_format() {
        let provider = CloudflareAiGatewayProvider {
            inner: OpenAiCompatibleProvider::new(
                PROVIDER_ID,
                reqwest::Client::new(),
                "token",
                "https://example.com",
                "workers-ai/@cf/meta/llama-3.1-8b-instruct",
            ),
        };

        assert!(
            provider
                .normalize_model("workers-ai/@cf/meta/llama-3.1-8b-instruct")
                .is_ok()
        );
        assert!(provider.normalize_model("gpt-4.1-mini").is_err());
    }

    #[test]
    fn normalize_json_value_compacts_metadata_payload() {
        assert_eq!(
            normalize_json_value("{\"trace\":\"1\",\"nested\":{\"a\":1}}")
                .expect("metadata should parse"),
            "{\"nested\":{\"a\":1},\"trace\":\"1\"}"
        );
    }

    #[test]
    fn parse_bool_flag_accepts_common_values() {
        assert!(parse_bool_flag("true").expect("true should parse"));
        assert!(parse_bool_flag("1").expect("1 should parse"));
        assert!(!parse_bool_flag("false").expect("false should parse"));
        assert!(parse_bool_flag("not-bool").is_err());
    }
}
