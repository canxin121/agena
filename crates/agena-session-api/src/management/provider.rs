//! Provider and authentication wire values shared by every transport.

use agena_domain::{ModelLifecycle, ModelPricing};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to store an API key credential.
pub struct AuthApiKeyWriteRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request referencing a provider.
pub struct AuthProviderRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

impl AuthProviderRequest {
    pub fn normalized_provider_id(&self, default: &str) -> String {
        self.provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default)
            .to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to start an OAuth redirect flow.
pub struct AuthRedirectRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to exchange an OAuth code for credentials.
pub struct AuthCodeExchangeRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to poll a device-code flow.
pub struct AuthUserCodeDevicePollRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to start an enterprise device flow.
pub struct AuthEnterpriseDeviceRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to poll an enterprise device flow.
pub struct AuthEnterpriseDevicePollRequest {
    #[serde(flatten)]
    pub provider: AuthProviderRequest,
    pub device_code: String,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Type of stored provider credential.
pub enum AuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Issuer of a provider credential.
pub enum AuthCredentialIssuerResource {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
    GoogleAdc,
    SapAiCore,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Kind of provider login flow.
pub enum AuthLoginKindResource {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Authentication state of a provider.
pub struct AuthProviderResource {
    pub provider_id: String,
    pub configured: bool,
    pub credential_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<AuthCredentialType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_issuer: Option<AuthCredentialIssuerResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub expired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub api_key_write_supported: bool,
    pub refresh_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_login_kind: Option<AuthLoginKindResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_login_instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_login_kind: Option<AuthLoginKindResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Browser start details of an OAuth flow.
pub struct AuthBrowserStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Device start details of a device flow.
pub struct AuthDeviceStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_domain: Option<String>,
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a provider login flow.
pub struct AuthLoginResultResource {
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AuthProviderResource>,
}

/// AWS profile names available to the server's provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsProfileNamesResponse {
    #[serde(default)]
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of one provider adapter.
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of a provider and its adapters.
pub struct ProviderSummaryResource {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Models of a provider.
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModelResource>,
}

/// A provider/model route projected into the public protocol. Runtime model
/// selection, provider clients, and capability evaluation stay outside this
/// DTO; clients receive only serializable route metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderModelResource {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_model_id: Option<String>,
    /// Catalog entry this route is explicitly matched to, if one was chosen.
    /// Unlike `catalog_model_id`, which reports what the catalog resolved to
    /// for this listing, this is stored configuration: the studio can show the
    /// route's own match after a reload and tell it apart from a suggestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_catalog_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default = "default_true")]
    pub native_compaction: bool,
    #[serde(default)]
    pub capabilities: ProviderModelCapabilitiesResource,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelMetadataResource::is_empty"
    )]
    pub metadata: ProviderModelMetadataResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_modes: Vec<ProviderModelThinkingModeResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speed_modes: BTreeMap<String, ProviderModelSpeedModeResource>,
}

impl ProviderModelResource {
    /// Construct the minimal public projection for a configured route when
    /// discovery has not supplied capability metadata yet.
    pub fn configured(adapter_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider_id: String::new(),
            adapter_id: Some(adapter_id.into()),
            id: id.into(),
            catalog_model_id: None,
            pinned_catalog_model_id: None,
            display_name: None,
            native_compaction: true,
            capabilities: ProviderModelCapabilitiesResource::default(),
            metadata: ProviderModelMetadataResource::default(),
            thinking_modes: Vec::new(),
            speed_modes: BTreeMap::new(),
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Whether a model capability is supported.
pub enum CapabilitySupportResource {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Capabilities of a provider model (input modalities and features).
pub struct ProviderModelCapabilitiesResource {
    #[serde(default = "capability_supported")]
    pub text_input: CapabilitySupportResource,
    #[serde(default)]
    pub image_input: CapabilitySupportResource,
    #[serde(default)]
    pub document_input: CapabilitySupportResource,
    #[serde(default)]
    pub audio_input: CapabilitySupportResource,
    #[serde(default)]
    pub video_input: CapabilitySupportResource,
    #[serde(default)]
    pub file_input: CapabilitySupportResource,
    #[serde(default)]
    pub tool_calling: CapabilitySupportResource,
    #[serde(default)]
    pub streaming: CapabilitySupportResource,
    #[serde(default)]
    pub reasoning: CapabilitySupportResource,
    #[serde(default)]
    pub structured_output: CapabilitySupportResource,
    #[serde(default = "capability_supported")]
    pub temperature_supported: CapabilitySupportResource,
}

const fn capability_supported() -> CapabilitySupportResource {
    CapabilitySupportResource::Supported
}

impl Default for ProviderModelCapabilitiesResource {
    fn default() -> Self {
        Self {
            text_input: CapabilitySupportResource::Supported,
            image_input: CapabilitySupportResource::Unknown,
            document_input: CapabilitySupportResource::Unknown,
            audio_input: CapabilitySupportResource::Unknown,
            video_input: CapabilitySupportResource::Unknown,
            file_input: CapabilitySupportResource::Unknown,
            tool_calling: CapabilitySupportResource::Unknown,
            streaming: CapabilitySupportResource::Unknown,
            reasoning: CapabilitySupportResource::Unknown,
            structured_output: CapabilitySupportResource::Unknown,
            temperature_supported: CapabilitySupportResource::Supported,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Metadata of a provider model (lifecycle, context window, pricing).
pub struct ProviderModelMetadataResource {
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
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbosity_levels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

impl ProviderModelMetadataResource {
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
            && self.supports_parallel_tool_calls.is_none()
            && self.supports_verbosity.is_none()
            && self.default_verbosity.is_none()
            && self.verbosity_levels.is_empty()
            && self.provider_default_verbosity.is_none()
            && self.default_temperature.is_none()
            && self.default_top_p.is_none()
            && self.default_top_k.is_none()
            && self.assistant_reasoning_interleaved.is_none()
            && self.assistant_reasoning_field.is_none()
            && self.output_modalities.is_empty()
            && self.pricing.is_none()
    }
}

// These protocol values are shared with the domain; no runtime behavior or
// credential storage is part of the types.
pub use agena_domain::{
    ModelSpeedModeRequestOverride as ProviderModelRequestOverrideResource,
    ReasoningEffort as ReasoningEffortResource, ThinkingDisplay as ThinkingDisplayResource,
    ThinkingRequest as ThinkingRequestResource,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A thinking mode offered by a model.
pub struct ProviderModelThinkingModeResource {
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_default: bool,
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequestResource>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
}

impl ProviderModelThinkingModeResource {
    /// The stable selector exposed to users and persisted in execution
    /// preferences. Mirrors `agena_domain::ModelThinkingMode::selector`: a
    /// non-empty `preset` wins; otherwise the selector derives from the
    /// request shape (effort name or `off` for a disabled mode).
    pub fn selector(&self) -> Option<std::borrow::Cow<'_, str>> {
        if let Some(name) = self
            .preset
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return Some(std::borrow::Cow::Borrowed(name));
        }
        match self.thinking.as_ref() {
            Some(ThinkingRequestResource::Disabled) => Some(std::borrow::Cow::Borrowed("off")),
            Some(ThinkingRequestResource::Effort { effort })
            | Some(ThinkingRequestResource::Adaptive {
                effort: Some(effort),
                ..
            }) => Some(std::borrow::Cow::Borrowed(effort.as_ref())),
            Some(ThinkingRequestResource::Budget { .. })
            | Some(ThinkingRequestResource::Adaptive { effort: None, .. })
            | None => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A speed mode offered by a model.
pub struct ProviderModelSpeedModeResource {
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_default: bool,
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
}

#[cfg(test)]
mod authentication_contract_tests {
    use super::*;
    use serde_json::json;

    fn provider_wire() -> serde_json::Value {
        json!({
            "provider_id": "fixture", "configured": true, "credential_present": false,
            "api_key_write_supported": true, "refresh_supported": false
        })
    }

    #[test]
    fn authentication_provider_keeps_omitted_false_expiry_and_optional_fields() {
        let wire = provider_wire();
        let provider: AuthProviderResource = serde_json::from_value(wire.clone()).unwrap();
        assert!(!provider.expired);
        assert!(provider.credential_type.is_none());
        assert!(provider.expires_at.is_none());
        assert_eq!(serde_json::to_value(&provider).unwrap(), wire);

        let mut expired = wire;
        expired["expired"] = json!(true);
        expired["expires_at"] = json!("1970-01-01T00:00:01.234Z");
        expired["credential_type"] = json!("oauth");
        expired["credential_issuer"] = json!("github_copilot");
        let provider: AuthProviderResource = serde_json::from_value(expired.clone()).unwrap();
        assert!(provider.expired);
        assert_eq!(provider.expires_at.unwrap().timestamp_millis(), 1234);
        assert!(matches!(
            provider.credential_type,
            Some(AuthCredentialType::Oauth)
        ));
        assert_eq!(serde_json::to_value(provider).unwrap(), expired);
    }

    #[test]
    fn authentication_resources_keep_handshake_fields_without_copying_private_credentials() {
        let mut input = provider_wire();
        input["key_preview"] = json!("key...last");
        input["api_key"] = json!("fixture-private-key");
        input["access_token"] = json!("fixture-access-token");
        input["refresh_token"] = json!("fixture-refresh-token");
        let provider: AuthProviderResource = serde_json::from_value(input).unwrap();
        let published = serde_json::to_value(&provider).unwrap();
        assert_eq!(published["key_preview"], "key...last");
        for field in ["api_key", "access_token", "refresh_token"] {
            assert!(published.get(field).is_none());
        }

        let browser = json!({
            "provider_id": "fixture", "authorize_url": "https://example.test/authorize",
            "state": "fixture-state", "pkce_verifier": "fixture-verifier"
        });
        let decoded: AuthBrowserStartResource = serde_json::from_value(browser.clone()).unwrap();
        assert!(decoded.instance_url.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), browser);
        let device = json!({
            "provider_id": "fixture", "verification_url": "https://example.test/device",
            "user_code": "USER-CODE", "device_code": "fixture-device", "interval_seconds": 0
        });
        let decoded: AuthDeviceStartResource = serde_json::from_value(device.clone()).unwrap();
        assert!(decoded.enterprise_domain.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), device);

        let pending = json!({"completed": false});
        let decoded: AuthLoginResultResource = serde_json::from_value(pending.clone()).unwrap();
        assert!(!decoded.completed && decoded.provider.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), pending);
        let complete = json!({"completed": true, "provider": published});
        let decoded: AuthLoginResultResource = serde_json::from_value(complete.clone()).unwrap();
        assert!(decoded.completed && decoded.provider.is_some());
        assert_eq!(serde_json::to_value(decoded).unwrap(), complete);
    }
}

#[cfg(test)]
mod authentication_request_tests {
    use super::*;
    use serde_json::json;

    fn assert_wire_roundtrip<T: serde::de::DeserializeOwned + serde::Serialize>(
        wire: serde_json::Value,
    ) {
        let request: T = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), wire);
    }

    #[test]
    fn auth_requests_keep_flat_wire_fields_and_explicit_nulls() {
        assert_wire_roundtrip::<AuthApiKeyWriteRequest>(json!({"api_key":"fixture-key"}));
        assert_wire_roundtrip::<AuthProviderRequest>(json!({"provider_id":"fixture"}));
        assert_wire_roundtrip::<AuthRedirectRequest>(json!({
            "provider_id":"fixture", "redirect_uri":"http://localhost:9876/auth/callback"
        }));
        assert_wire_roundtrip::<AuthCodeExchangeRequest>(json!({
            "provider_id":"fixture", "code":"fixture-code", "pkce_verifier":"fixture-verifier",
            "redirect_uri":"http://localhost:9876/auth/callback"
        }));
        assert_wire_roundtrip::<AuthUserCodeDevicePollRequest>(json!({
            "provider_id":"fixture", "device_code":"fixture-device", "user_code":"USER-CODE"
        }));
        assert_wire_roundtrip::<AuthEnterpriseDeviceRequest>(json!({
            "provider_id":null, "enterprise_domain":null
        }));
        assert_wire_roundtrip::<AuthEnterpriseDevicePollRequest>(json!({
            "provider_id":"fixture", "device_code":"fixture-device", "enterprise_domain":null
        }));
        let enterprise: AuthEnterpriseDeviceRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            serde_json::to_value(enterprise).unwrap(),
            json!({
                "provider_id":null, "enterprise_domain":null
            })
        );
    }

    #[test]
    fn auth_request_provider_defaults_and_required_fields_keep_existing_validation() {
        for wire in [
            json!({}),
            json!({"provider_id":null}),
            json!({"provider_id":""}),
            json!({"provider_id":" \t"}),
        ] {
            let request: AuthProviderRequest = serde_json::from_value(wire).unwrap();
            assert_eq!(request.normalized_provider_id("fallback"), "fallback");
        }
        let request: AuthProviderRequest =
            serde_json::from_value(json!({"provider_id":" custom / "})).unwrap();
        assert_eq!(request.normalized_provider_id("fallback"), "custom /");
        let redirect: AuthRedirectRequest = serde_json::from_value(json!({
            "redirect_uri":"http://localhost:9876/auth/callback", "extra":true
        }))
        .unwrap();
        assert_eq!(redirect.provider.normalized_provider_id("openai"), "openai");
        assert_eq!(
            serde_json::to_value(redirect).unwrap(),
            json!({
                "provider_id":null, "redirect_uri":"http://localhost:9876/auth/callback"
            })
        );
        assert!(serde_json::from_value::<AuthApiKeyWriteRequest>(json!({})).is_err());
        assert!(serde_json::from_value::<AuthCodeExchangeRequest>(json!({
            "provider_id":"fixture", "code":"fixture-code", "redirect_uri":"http://localhost:9876/auth/callback"
        })).is_err());
        assert!(
            serde_json::from_value::<AuthUserCodeDevicePollRequest>(json!({
                "device_code":"fixture-device"
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<AuthEnterpriseDevicePollRequest>(json!({})).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Adapter model probe result for one adapter.
pub struct ProviderAdapterModelsResource {
    pub adapter_id: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base_url: Option<String>,
    pub models: Vec<ProviderModelResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response containing provider adapter model probes.
pub struct ProviderAdapterModelsResponse {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterModelsResource>,
}
