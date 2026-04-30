use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::provider::parse_sap_ai_core_service_key;

use super::{
    ConfigEnvironment, ConfigError, OpenAiApiModeConfig, StreamTransportMode,
    raw::{ProviderKind, RawProviderConfig},
};

const MODELS_DEV_PRESETS_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_PRESETS_CACHE_FILE: &str = "provider-presets.json";
const MODELS_DEV_PRESETS_PATH_ENV: &str = "AGENA_PROVIDER_PRESETS_PATH";
const PRESET_INTEGRATION_TITLE: &str = "agena";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderPresetRecord {
    id: String,
    npm: Option<String>,
    api: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    default_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevProviderRecord {
    npm: Option<String>,
    api: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    models: BTreeMap<String, ModelsDevModelRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevModelRecord {
    #[allow(dead_code)]
    id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresetFamily {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    GoogleVertex,
    AmazonBedrock,
    Copilot,
    Gitlab,
    CloudflareAiGateway,
    SapAiCore,
}

pub(super) fn apply_provider_preset(
    provider_id: &str,
    mut raw: RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<RawProviderConfig, ConfigError> {
    let preset = load_provider_presets(env)?
        .into_iter()
        .find(|preset| preset.id == provider_id)
        .ok_or_else(|| ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("unknown provider preset `{provider_id}`"),
        })?;

    let family = preset_family(&preset)?;

    match family {
        PresetFamily::OpenAi => apply_openai_preset(provider_id, &preset, &mut raw, env)?,
        PresetFamily::OpenAiCompatible => {
            apply_openai_compatible_preset(provider_id, &preset, &mut raw, env)?
        }
        PresetFamily::Anthropic => apply_anthropic_preset(provider_id, &preset, &mut raw),
        PresetFamily::Gemini => apply_gemini_preset(&preset, &mut raw),
        PresetFamily::GoogleVertex => {
            apply_google_vertex_preset(provider_id, &preset, &mut raw, env)?
        }
        PresetFamily::AmazonBedrock => apply_amazon_bedrock_preset(&preset, &mut raw, env),
        PresetFamily::Copilot => apply_copilot_preset(&preset, &mut raw),
        PresetFamily::Gitlab => apply_gitlab_preset(&preset, &mut raw),
        PresetFamily::CloudflareAiGateway => {
            apply_cloudflare_ai_gateway_preset(&preset, &mut raw, env)?
        }
        PresetFamily::SapAiCore => apply_sap_ai_core_preset(provider_id, &preset, &mut raw, env)?,
    }

    Ok(raw)
}

fn load_provider_presets(
    env: &dyn ConfigEnvironment,
) -> Result<Vec<ProviderPresetRecord>, ConfigError> {
    let path = presets_cache_path(env);

    let mut presets = if path.exists() {
        read_presets_file(path.as_path())?
    } else {
        let fetched = fetch_provider_presets()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::ReadFile {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let encoded = serde_json::to_vec(&fetched)?;
        fs::write(&path, encoded).map_err(|source| ConfigError::ReadFile {
            path: path.clone(),
            source,
        })?;
        fetched
    };

    inject_builtin_presets(&mut presets);
    Ok(presets)
}

/// Built-in presets for local model runtimes that ship with no API key and
/// run on a known localhost port. We inject these unconditionally so users
/// can name them in `[providers]` without depending on models.dev.
fn inject_builtin_presets(presets: &mut Vec<ProviderPresetRecord>) {
    for builtin in builtin_presets() {
        if !presets.iter().any(|p| p.id == builtin.id) {
            presets.push(builtin);
        }
    }
}

fn builtin_presets() -> Vec<ProviderPresetRecord> {
    vec![
        ProviderPresetRecord {
            id: "ollama".to_string(),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            api: Some("http://localhost:11434/v1".to_string()),
            env: Vec::new(),
            default_model: None,
        },
        ProviderPresetRecord {
            id: "lmstudio".to_string(),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            api: Some("http://localhost:1234/v1".to_string()),
            env: Vec::new(),
            default_model: None,
        },
    ]
}

fn read_presets_file(path: &Path) -> Result<Vec<ProviderPresetRecord>, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(text.as_str()).map_err(|err| {
        ConfigError::Validation(format!(
            "failed to parse provider preset file {}: {err}",
            path.display()
        ))
    })
}

fn fetch_provider_presets() -> Result<Vec<ProviderPresetRecord>, ConfigError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            ConfigError::Validation(format!(
                "failed to initialize runtime for provider preset fetch: {err}"
            ))
        })?;

    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|err| {
                ConfigError::Validation(format!(
                    "failed to build http client for provider preset fetch: {err}"
                ))
            })?;

        let response = client
            .get(MODELS_DEV_PRESETS_URL)
            .send()
            .await
            .map_err(|err| {
                ConfigError::Validation(format!(
                    "failed to fetch provider preset data from models.dev: {err}"
                ))
            })?;

        if !response.status().is_success() {
            return Err(ConfigError::Validation(format!(
                "models.dev preset fetch failed with status {}",
                response.status()
            )));
        }

        let payload = response
            .json::<BTreeMap<String, ModelsDevProviderRecord>>()
            .await
            .map_err(|err| {
                ConfigError::Validation(format!(
                    "failed to decode models.dev preset payload: {err}"
                ))
            })?;

        Ok(payload_to_presets(payload))
    })
}

fn payload_to_presets(
    payload: BTreeMap<String, ModelsDevProviderRecord>,
) -> Vec<ProviderPresetRecord> {
    payload
        .into_iter()
        .map(|(id, provider)| ProviderPresetRecord {
            id,
            npm: provider.npm,
            api: provider.api,
            env: provider.env,
            default_model: default_model_from_models(provider.models),
        })
        .collect()
}

fn default_model_from_models(models: BTreeMap<String, ModelsDevModelRecord>) -> Option<String> {
    let mut ids = models.into_keys().collect::<Vec<_>>();
    ids.sort_by(|left, right| compare_model_priority(left.as_str(), right.as_str()));
    ids.into_iter().next()
}

fn compare_model_priority(left: &str, right: &str) -> std::cmp::Ordering {
    const PRIORITY: &[&str] = &["gpt-5", "claude-sonnet-4", "big-pickle", "gemini-3-pro"];

    let left_priority = PRIORITY
        .iter()
        .position(|needle| left.contains(needle))
        .unwrap_or(usize::MAX);
    let right_priority = PRIORITY
        .iter()
        .position(|needle| right.contains(needle))
        .unwrap_or(usize::MAX);

    left_priority
        .cmp(&right_priority)
        .then_with(|| {
            let left_latest = if left.contains("latest") { 0 } else { 1 };
            let right_latest = if right.contains("latest") { 0 } else { 1 };
            left_latest.cmp(&right_latest)
        })
        .then_with(|| right.cmp(left))
}

fn presets_cache_path(env: &dyn ConfigEnvironment) -> PathBuf {
    if let Some(path) = env.var(MODELS_DEV_PRESETS_PATH_ENV) {
        return PathBuf::from(path);
    }

    let mut base = home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push(".agena");
    base.push(MODELS_DEV_PRESETS_CACHE_FILE);
    base
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

fn preset_family(preset: &ProviderPresetRecord) -> Result<PresetFamily, ConfigError> {
    if preset.id == "github-copilot" {
        return Ok(PresetFamily::Copilot);
    }

    match preset.npm.as_deref() {
        Some("@ai-sdk/openai") | Some("@ai-sdk/xai") | Some("@ai-sdk/azure") => {
            Ok(PresetFamily::OpenAi)
        }
        Some("@ai-sdk/openai-compatible")
        | Some("@openrouter/ai-sdk-provider")
        | Some("@ai-sdk/gateway")
        | Some("@ai-sdk/vercel")
        | Some("@ai-sdk/deepinfra")
        | Some("@ai-sdk/groq")
        | Some("@ai-sdk/mistral")
        | Some("@ai-sdk/perplexity")
        | Some("@ai-sdk/togetherai")
        | Some("venice-ai-sdk-provider")
        | Some("@aihubmix/ai-sdk-provider")
        | Some("@ai-sdk/cerebras")
        | Some("@ai-sdk/cohere") => Ok(PresetFamily::OpenAiCompatible),
        Some("@ai-sdk/anthropic") => Ok(PresetFamily::Anthropic),
        Some("@ai-sdk/google") => Ok(PresetFamily::Gemini),
        Some("@ai-sdk/google-vertex") | Some("@ai-sdk/google-vertex/anthropic") => {
            Ok(PresetFamily::GoogleVertex)
        }
        Some("@ai-sdk/amazon-bedrock") => Ok(PresetFamily::AmazonBedrock),
        Some("gitlab-ai-provider") => Ok(PresetFamily::Gitlab),
        Some("ai-gateway-provider") => Ok(PresetFamily::CloudflareAiGateway),
        Some("@jerome-benoit/sap-ai-provider-v2") => Ok(PresetFamily::SapAiCore),
        Some(other) => Err(ConfigError::InvalidProviderConfig {
            provider_id: preset.id.clone(),
            message: format!("unsupported provider preset package `{other}`"),
        }),
        None => Err(ConfigError::InvalidProviderConfig {
            provider_id: preset.id.clone(),
            message: "provider preset is missing npm metadata".to_owned(),
        }),
    }
}

fn apply_openai_preset(
    provider_id: &str,
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    raw.kind = Some(ProviderKind::OpenAi);
    set_default_model(raw, preset);
    set_openai_api_mode_default(raw);

    match provider_id {
        "azure" => {
            let resource = raw
                .base_url
                .clone()
                .or_else(|| {
                    env.var("AZURE_RESOURCE_NAME").and_then(|resource| {
                        normalize_text(resource).map(|resource| {
                            format!("https://{resource}.openai.azure.com/openai")
                        })
                    })
                })
                .ok_or_else(|| ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "azure preset requires `base_url` or environment variable `AZURE_RESOURCE_NAME`".to_owned(),
                })?;
            raw.base_url = Some(resource);
            raw.api_key_env
                .get_or_insert_with(|| "AZURE_API_KEY".to_owned());
        }
        "azure-cognitive-services" => {
            let resource = raw
                .base_url
                .clone()
                .or_else(|| {
                    env.var("AZURE_COGNITIVE_SERVICES_RESOURCE_NAME")
                        .and_then(|resource| {
                            normalize_text(resource).map(|resource| {
                                format!(
                                    "https://{resource}.cognitiveservices.azure.com/openai"
                                )
                            })
                        })
                })
                .ok_or_else(|| ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "azure-cognitive-services preset requires `base_url` or environment variable `AZURE_COGNITIVE_SERVICES_RESOURCE_NAME`".to_owned(),
                })?;
            raw.base_url = Some(resource);
            raw.api_key_env
                .get_or_insert_with(|| "AZURE_COGNITIVE_SERVICES_API_KEY".to_owned());
        }
        _ => {
            set_base_url(raw, preset, provider_id)?;
            set_first_api_key_env(raw, preset);
        }
    }

    Ok(())
}

fn apply_openai_compatible_preset(
    provider_id: &str,
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    raw.kind = Some(ProviderKind::OpenAiCompatible);
    raw.auth_header
        .get_or_insert_with(|| "authorization".to_owned());
    if raw.auth_scheme.is_none() {
        raw.auth_scheme = Some("Bearer".to_owned());
    }
    raw.stream_mode.get_or_insert(StreamTransportMode::Sse);
    set_default_model(raw, preset);

    match provider_id {
        "opencode" | "opencode-go" => {
            set_base_url(raw, preset, provider_id)?;
            if raw.api_key.is_none() && raw.api_key_env.is_none() {
                if env_has_non_empty(env, "OPENCODE_API_KEY") {
                    raw.api_key_env = Some("OPENCODE_API_KEY".to_owned());
                } else {
                    raw.api_key = Some("public".to_owned());
                }
            }
        }
        "ollama" | "lmstudio" => {
            // Local runtimes accept any token, including none. Honor
            // OLLAMA_HOST / LMSTUDIO_HOST overrides for non-default ports.
            let host_env = if provider_id == "ollama" {
                "OLLAMA_HOST"
            } else {
                "LMSTUDIO_HOST"
            };
            if raw.base_url.is_none() {
                if let Some(host) = env.var(host_env).and_then(normalize_text) {
                    let normalized = if host.starts_with("http://") || host.starts_with("https://")
                    {
                        host
                    } else {
                        format!("http://{host}")
                    };
                    let trimmed = normalized.trim_end_matches('/').to_string();
                    let with_v1 = if trimmed.ends_with("/v1") {
                        trimmed
                    } else {
                        format!("{trimmed}/v1")
                    };
                    raw.base_url = Some(with_v1);
                } else {
                    set_base_url(raw, preset, provider_id)?;
                }
            }
            if raw.api_key.is_none() && raw.api_key_env.is_none() {
                raw.api_key = Some("local".to_owned());
            }
        }
        "cloudflare-workers-ai" => {
            let account_id = env
                .var("CLOUDFLARE_ACCOUNT_ID")
                .and_then(normalize_text)
                .ok_or_else(|| ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "cloudflare-workers-ai preset requires environment variable `CLOUDFLARE_ACCOUNT_ID` or an explicit `base_url`".to_owned(),
                })?;
            raw.base_url.get_or_insert_with(|| {
                format!("https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1")
            });
            raw.api_key_env
                .get_or_insert_with(|| "CLOUDFLARE_API_KEY".to_owned());
        }
        _ => {
            set_base_url(raw, preset, provider_id)?;
            set_first_api_key_env(raw, preset);
        }
    }

    match provider_id {
        "openrouter" | "zenmux" | "kilo" => {
            insert_header_if_missing(raw, "X-Title", PRESET_INTEGRATION_TITLE);
        }
        "vercel" => {
            insert_header_if_missing(raw, "x-title", PRESET_INTEGRATION_TITLE);
        }
        "cerebras" => {
            insert_header_if_missing(
                raw,
                "X-Cerebras-3rd-Party-Integration",
                PRESET_INTEGRATION_TITLE,
            );
        }
        _ => {}
    }

    Ok(())
}

fn apply_anthropic_preset(
    provider_id: &str,
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
) {
    let _ = provider_id;
    raw.kind = Some(ProviderKind::Anthropic);
    set_default_model(raw, preset);
    set_base_url(raw, preset, provider_id).expect("anthropic presets must have base url");
    raw.api_key_env.get_or_insert_with(|| {
        preset
            .env
            .first()
            .cloned()
            .unwrap_or_else(|| "ANTHROPIC_API_KEY".to_owned())
    });
    raw.auth_header
        .get_or_insert_with(|| "x-api-key".to_owned());
}

fn apply_gemini_preset(preset: &ProviderPresetRecord, raw: &mut RawProviderConfig) {
    raw.kind = Some(ProviderKind::Gemini);
    set_default_model(raw, preset);
    raw.base_url
        .get_or_insert_with(|| "https://generativelanguage.googleapis.com/v1beta".to_owned());
    if raw.api_key_env.is_none() {
        raw.api_key_env = preset
            .env
            .first()
            .cloned()
            .or_else(|| Some("GOOGLE_GENERATIVE_AI_API_KEY".to_owned()));
    }
}

fn apply_google_vertex_preset(
    provider_id: &str,
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    raw.kind = Some(ProviderKind::GoogleVertex);
    set_default_model(raw, preset);

    if raw.base_url.is_none() {
        let project = env
            .var("GOOGLE_VERTEX_PROJECT")
            .and_then(normalize_text)
            .or_else(|| env.var("GOOGLE_CLOUD_PROJECT").and_then(normalize_text))
            .or_else(|| env.var("GCP_PROJECT").and_then(normalize_text))
            .or_else(|| env.var("GCLOUD_PROJECT").and_then(normalize_text))
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: "google-vertex presets require `base_url` or one of `GOOGLE_VERTEX_PROJECT`, `GOOGLE_CLOUD_PROJECT`, `GCP_PROJECT`, `GCLOUD_PROJECT`".to_owned(),
            })?;

        let default_location = if provider_id == "google-vertex-anthropic" {
            "global"
        } else {
            "us-central1"
        };
        let location = env
            .var("GOOGLE_VERTEX_LOCATION")
            .and_then(normalize_text)
            .or_else(|| env.var("GOOGLE_CLOUD_LOCATION").and_then(normalize_text))
            .or_else(|| env.var("VERTEX_LOCATION").and_then(normalize_text))
            .unwrap_or_else(|| default_location.to_owned());

        let host = if location == "global" {
            "aiplatform.googleapis.com".to_owned()
        } else {
            format!("{location}-aiplatform.googleapis.com")
        };

        raw.base_url = Some(format!(
            "https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi"
        ));
    }

    Ok(())
}

fn apply_amazon_bedrock_preset(
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) {
    raw.kind = Some(ProviderKind::AmazonBedrock);
    set_default_model(raw, preset);

    let region = raw
        .region
        .clone()
        .or_else(|| env.var("AWS_REGION").and_then(normalize_text))
        .unwrap_or_else(|| "us-east-1".to_owned());
    raw.region = Some(region.clone());
    raw.base_url
        .get_or_insert_with(|| format!("https://bedrock-runtime.{region}.amazonaws.com/openai/v1"));

    if raw.api_key.is_none()
        && raw.api_key_env.is_none()
        && env_has_non_empty(env, "AWS_BEARER_TOKEN_BEDROCK")
    {
        raw.api_key_env = Some("AWS_BEARER_TOKEN_BEDROCK".to_owned());
    }
}

fn apply_copilot_preset(preset: &ProviderPresetRecord, raw: &mut RawProviderConfig) {
    raw.kind = Some(ProviderKind::Copilot);
    set_default_model(raw, preset);
    raw.base_url
        .get_or_insert_with(|| "https://api.githubcopilot.com".to_owned());
    raw.auth_provider_id
        .get_or_insert_with(|| "github-copilot".to_owned());
}

fn apply_gitlab_preset(preset: &ProviderPresetRecord, raw: &mut RawProviderConfig) {
    raw.kind = Some(ProviderKind::Gitlab);
    set_default_model(raw, preset);
    raw.instance_url
        .get_or_insert_with(|| "https://gitlab.com".to_owned());
    raw.ai_gateway_url
        .get_or_insert_with(|| "https://cloud.gitlab.com".to_owned());
    raw.auth_provider_id
        .get_or_insert_with(|| "gitlab".to_owned());
    if raw.api_key_env.is_none() {
        raw.api_key_env = preset.env.first().cloned();
    }
    raw.ai_gateway_headers
        .entry("anthropic-beta".to_owned())
        .or_insert_with(|| "context-1m-2025-08-07".to_owned());
    raw.feature_flags
        .entry("duo_agent_platform_agentic_chat".to_owned())
        .or_insert(true);
    raw.feature_flags
        .entry("duo_agent_platform".to_owned())
        .or_insert(true);
}

fn apply_cloudflare_ai_gateway_preset(
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    raw.kind = Some(ProviderKind::CloudflareAiGateway);
    set_default_model(raw, preset);
    if raw.base_url.is_none() {
        let account = env
            .var("CLOUDFLARE_ACCOUNT_ID")
            .and_then(normalize_text)
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: preset.id.clone(),
                message: "cloudflare-ai-gateway preset requires `base_url` or environment variable `CLOUDFLARE_ACCOUNT_ID`".to_owned(),
            })?;
        let gateway = env
            .var("CLOUDFLARE_GATEWAY_ID")
            .and_then(normalize_text)
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: preset.id.clone(),
                message: "cloudflare-ai-gateway preset requires `base_url` or environment variable `CLOUDFLARE_GATEWAY_ID`".to_owned(),
            })?;
        raw.base_url = Some(format!(
            "https://gateway.ai.cloudflare.com/v1/{account}/{gateway}/compat"
        ));
    }
    raw.api_key_env
        .get_or_insert_with(|| "CLOUDFLARE_API_TOKEN".to_owned());
    Ok(())
}

fn apply_sap_ai_core_preset(
    provider_id: &str,
    preset: &ProviderPresetRecord,
    raw: &mut RawProviderConfig,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    raw.kind = Some(ProviderKind::SapAiCore);
    raw.auth_header
        .get_or_insert_with(|| "authorization".to_owned());
    if raw.auth_scheme.is_none() {
        raw.auth_scheme = Some("Bearer".to_owned());
    }
    raw.stream_mode.get_or_insert(StreamTransportMode::Sse);
    set_default_model(raw, preset);

    if let Some(resource_group) = env.var("AICORE_RESOURCE_GROUP").and_then(normalize_text) {
        raw.extra_headers
            .entry("AI-Resource-Group".to_owned())
            .or_insert(resource_group);
    }

    if raw.api_key.is_some() && raw.base_url.is_some() {
        return Ok(());
    }

    let service_key_raw = env
        .var("AICORE_SERVICE_KEY")
        .and_then(normalize_text)
        .ok_or_else(|| ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "sap-ai-core preset requires `AICORE_SERVICE_KEY` when `api_key` and `base_url` are not provided".to_owned(),
        })?;

    let service_key = parse_sap_ai_core_service_key(service_key_raw.as_str()).map_err(|err| {
        ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("failed to parse `AICORE_SERVICE_KEY`: {err}"),
        }
    })?;

    raw.base_url
        .get_or_insert_with(|| service_key.serviceurls.ai_api_url.clone());

    Ok(())
}

fn set_default_model(raw: &mut RawProviderConfig, preset: &ProviderPresetRecord) {
    if raw.default_model.is_none() {
        raw.default_model = preset.default_model.clone();
    }
}

fn set_openai_api_mode_default(raw: &mut RawProviderConfig) {
    raw.api_mode.get_or_insert(OpenAiApiModeConfig::Responses);
    raw.stream_mode.get_or_insert(StreamTransportMode::Sse);
}

fn set_base_url(
    raw: &mut RawProviderConfig,
    preset: &ProviderPresetRecord,
    provider_id: &str,
) -> Result<(), ConfigError> {
    if raw.base_url.is_some() {
        return Ok(());
    }

    raw.base_url = preset
        .api
        .clone()
        .or_else(|| manual_base_url(provider_id).map(str::to_owned));

    if raw.base_url.is_none() {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "preset could not determine default base_url; set `base_url` explicitly"
                .to_owned(),
        });
    }

    Ok(())
}

fn set_first_api_key_env(raw: &mut RawProviderConfig, preset: &ProviderPresetRecord) {
    if raw.api_key.is_none() && raw.api_key_env.is_none() {
        raw.api_key_env = preset.env.first().cloned();
    }
}

fn insert_header_if_missing(raw: &mut RawProviderConfig, key: &str, value: &str) {
    raw.extra_headers
        .entry(key.to_owned())
        .or_insert_with(|| value.to_owned());
}

fn env_has_non_empty(env: &dyn ConfigEnvironment, key: &str) -> bool {
    env.var(key).and_then(normalize_text).is_some()
}

fn normalize_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn manual_base_url(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "aihubmix" => Some("https://api.aihubmix.com/v1"),
        "anthropic" => Some("https://api.anthropic.com/v1"),
        "cerebras" => Some("https://api.cerebras.ai/v1"),
        "cohere" => Some("https://api.cohere.com/compatibility/v1"),
        "deepinfra" => Some("https://api.deepinfra.com/v1/openai"),
        "google" => Some("https://generativelanguage.googleapis.com/v1beta"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        "mistral" => Some("https://api.mistral.ai/v1"),
        "openai" => Some("https://api.openai.com/v1"),
        "perplexity" => Some("https://api.perplexity.ai"),
        "togetherai" => Some("https://api.together.xyz/v1"),
        "v0" => Some("https://api.v0.dev/v1"),
        "venice" => Some("https://api.venice.ai/api/v1"),
        "vercel" => Some("https://ai-gateway.vercel.sh/v1"),
        "xai" => Some("https://api.x.ai/v1"),
        _ => None,
    }
}
