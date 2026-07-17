use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio::sync::Mutex;

use super::copilot_models::CopilotModelExtension;

use crate::{
    config::{ProviderNativeToolKind, ProviderNativeToolRoute},
    error::AppError,
    message::{AttachmentItem, AttachmentKind, Message, MessageUsage},
    model::{ModelId, ProviderId},
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, CompletionUsage, ManagedCredential, ModelRuntime, ProviderModel,
        StreamResumePolicy, ThinkingDisplay, ThinkingRequest, auth::AuthData, prompt_cache, sse,
        utils, wire_message,
    },
    role::Role,
};

const PROVIDER_ID: &str = "anthropic";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const FIRST_PARTY_ANTHROPIC_HOSTS: &[&str] = &["api.anthropic.com", "api-staging.anthropic.com"];
const DEFAULT_ANTHROPIC_BETA_HEADER: &str = "claude-code-20250219,interleaved-thinking-2025-05-14,thinking-token-count-2026-05-13,effort-2025-11-24";
const DEFAULT_COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const DEFAULT_COPILOT_ANTHROPIC_BETA_HEADER: &str = "interleaved-thinking-2025-05-14";
const ADAPTER_KIND: &str = "anthropic";

#[derive(Clone)]
pub struct AnthropicAdapter {
    id: String,
    client: reqwest::Client,
    api_key: ManagedCredential,
    base_url: String,
    default_model: ModelId,
    auth_data: Option<Arc<Mutex<AuthData>>>,
    auth_header: String,
    auth_scheme: Option<String>,
    models_url: Option<String>,
    messages_url: Option<String>,
    profile: AnthropicProfile,
    extra_headers: HashMap<String, String>,
    eager_input_streaming_override: Option<bool>,
}

#[derive(Clone)]
pub struct AnthropicAdapterOptions {
    pub auth_data: Option<Arc<Mutex<AuthData>>>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub models_url: Option<String>,
    pub messages_url: Option<String>,
    pub profile: AnthropicProfile,
    pub extra_beta_header: Option<String>,
    pub override_beta_header: bool,
    pub extra_headers: HashMap<String, String>,
    pub eager_input_streaming_override: Option<bool>,
}

impl Default for AnthropicAdapterOptions {
    fn default() -> Self {
        Self {
            auth_data: None,
            auth_header: "x-api-key".to_owned(),
            auth_scheme: None,
            models_url: None,
            messages_url: None,
            profile: AnthropicProfile::Standard,
            extra_beta_header: None,
            override_beta_header: false,
            extra_headers: HashMap::new(),
            eager_input_streaming_override: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicProfile {
    Standard,
    GithubCopilot,
}

mod anthropic_requests;
mod anthropic_runtime;
mod anthropic_setup;
mod anthropic_thinking;
mod anthropic_transport;
mod anthropic_wire;

pub(crate) use self::anthropic_runtime::normalize_domain;
pub(crate) use self::anthropic_thinking::*;
pub(crate) use self::anthropic_wire::*;
