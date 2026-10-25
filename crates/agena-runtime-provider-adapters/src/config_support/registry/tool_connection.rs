//! Connection resolution for bundled protocol tools. Protocol selection does
//! not select a different provider or invent a vendor endpoint.
use std::collections::BTreeMap;

use agena_provider::{AuthRefreshStrategy, AuthSecretSelector};

use super::*;

/// Ephemeral, sensitive connection data; never serialize or log this value.
pub struct ProviderToolConnection {
    pub base_url: String,
    pub messages_url: Option<String>,
    pub realtime_ws_url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub model: Option<String>,
    pub receipt_key: String,
}

fn compatible(definition: &ProviderAdapterDefinition, protocol: &str) -> bool {
    matches!(
        (definition, protocol),
        (ProviderAdapterDefinition::OpenAiResponses(_), "chatgpt")
            | (
                ProviderAdapterDefinition::OpenAiChatCompletions(_),
                "chatgpt"
            )
            | (ProviderAdapterDefinition::Anthropic(_), "claude")
            | (ProviderAdapterDefinition::Gemini(_), "gemini")
    )
}

pub(super) fn tool_base_url(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    kind: HttpAdapterKind,
) -> Result<String, ConfigError> {
    let (base, paths) = provider_endpoint_root(auth, provider_id)?;
    let base = normalize_base_url(base)?;
    let path = http_adapter_protocol_path(paths, kind).trim();
    if path.is_empty() {
        return Ok(base);
    }
    let path = format!("/{}", path.trim_matches('/'));
    if base.ends_with(&path) {
        return Ok(base);
    }
    // SDK-style provider URLs often already contain another protocol prefix,
    // e.g. a gateway configured as /v1 while this tool needs /v1beta.
    // Only replace a suffix explicitly known to this provider's protocol paths.
    let prefix = [&paths.openai, &paths.anthropic, &paths.gemini]
        .into_iter()
        .map(|value| format!("/{}", value.trim().trim_matches('/')))
        .filter(|value| value != "/" && base.ends_with(value))
        .max_by_key(String::len);
    let root = prefix
        .as_ref()
        .map_or(base.as_str(), |prefix| &base[..base.len() - prefix.len()]);
    Ok(format!("{root}{path}"))
}

/// Reuses provider endpoint normalization, managed credential precedence and
/// adapter headers. A gateway may serve a protocol without a configured chat
/// adapter for it; such requests still use this provider's protocol paths.
pub async fn resolve_provider_tool_connection(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    protocol: &str,
    selected_adapter: Option<&str>,
    selected_model: &str,
    env: &dyn ConfigEnvironment,
    identity: &crate::ProviderClientIdentity,
) -> Result<ProviderToolConnection, ConfigError> {
    let kind = match protocol {
        "chatgpt" => HttpAdapterKind::OpenAi,
        "claude" => HttpAdapterKind::Anthropic,
        "gemini" => HttpAdapterKind::Gemini,
        _ => {
            return Err(ConfigError::Validation(
                "unknown bundled tool protocol".into(),
            ));
        }
    };
    if !resolved.enabled {
        return Err(ConfigError::Validation(
            "the current provider is disabled".into(),
        ));
    }
    // Dedicated OAuth backends (Codex, Copilot, GitLab, etc.) are not generic
    // vendor APIs. Never forward their credentials to an invented endpoint.
    let base_url = tool_base_url(provider_id, &resolved.auth, kind)?;
    let credential = api_auth_managed_credential(
        provider_id,
        "api_key",
        &resolved.auth,
        AuthSecretSelector::AccessOrApiKey,
        AuthRefreshStrategy::ReloadFromStore,
        env,
        true,
    )?;
    let key = credential
        .credential
        .with_client_identity(identity.clone())
        .resolve()
        .await
        .map_err(|error| ConfigError::Validation(error.to_string()))?;

    let selected = selected_adapter
        .and_then(|id| resolved.adapters.get_key_value(id))
        .filter(|(_, adapter)| adapter.enabled && compatible(&adapter.definition, protocol));
    let adapter = selected.or_else(|| {
        resolved
            .adapters
            .iter()
            .find(|(_, adapter)| adapter.enabled && compatible(&adapter.definition, protocol))
    });
    // The tool's protocol chooses connection details, not the request model.
    // Callers may override this session default with input.model; vendor and
    // operation suitability are documented rather than guessed from an adapter.
    let model = Some(selected_model.trim().to_owned()).filter(|model| !model.is_empty());
    let mut headers = BTreeMap::new();
    let mut messages_url = None;
    let mut realtime_ws_url = None;
    let (mut auth_header, mut auth_scheme) = match protocol {
        "chatgpt" => ("authorization".to_owned(), Some("Bearer".to_owned())),
        "claude" => ("x-api-key".to_owned(), None),
        _ => ("x-goog-api-key".to_owned(), None),
    };
    let user_agent = http_adapter_default_user_agent(
        identity,
        &resolved.auth,
        kind,
        model.as_deref().unwrap_or(selected_model),
    );
    headers.insert("user-agent".into(), user_agent.clone());
    if let Some((_, adapter)) = adapter {
        macro_rules! adapter_headers {
            ($config:expr) => {
                for (name, value) in http_adapter_extra_headers($config, Some(user_agent)) {
                    headers.insert(name.to_ascii_lowercase(), value);
                }
            };
        }
        match &adapter.definition {
            ProviderAdapterDefinition::OpenAiResponses(config) => {
                auth_header = config.options.auth_header.clone();
                auth_scheme = config.options.auth_scheme.clone();
                adapter_headers!(config);
            }
            ProviderAdapterDefinition::OpenAiChatCompletions(config) => {
                auth_header = config.options.auth_header.clone();
                auth_scheme = config.options.auth_scheme.clone();
                adapter_headers!(config);
            }
            ProviderAdapterDefinition::Anthropic(config) => {
                messages_url = config.options.messages_url.clone();
                auth_header = config.options.auth_header.clone();
                auth_scheme = config.options.auth_scheme.clone();
                adapter_headers!(config);
                if let Some(beta) = &config.options.extra_beta_header {
                    headers.insert("anthropic-beta".into(), beta.clone());
                }
            }
            ProviderAdapterDefinition::Gemini(config) => {
                realtime_ws_url = config.options.realtime_ws_url.clone();
                if let Some(header) = &config.options.auth_header {
                    auth_header = header.clone();
                    auth_scheme = config.options.auth_scheme.clone();
                }
                adapter_headers!(config);
            }
            _ => unreachable!("adapter was filtered by protocol"),
        }
    }
    headers
        .entry(auth_header.to_ascii_lowercase())
        .or_insert_with(|| match auth_scheme {
            Some(scheme) if !scheme.trim().is_empty() => format!("{scheme} {key}"),
            _ => key.clone(),
        });
    for (name, value) in &headers {
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || reqwest::header::HeaderValue::from_str(value).is_err()
        {
            return Err(ConfigError::Validation(
                "invalid current provider HTTP header".into(),
            ));
        }
    }
    Ok(ProviderToolConnection {
        realtime_ws_url,
        base_url,
        messages_url,
        headers,
        model,
        receipt_key: key,
    })
}
