use std::collections::{BTreeMap, HashSet};

use agena_domain::{ExternalAgentProvider, ExternalAgentProviderTarget, ExternalAgentProviders};
use axum::http::{HeaderName, HeaderValue};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use url::Url;

use super::*;

const ROUTES_FILE_ENV: &str = "AGENA_ACP_PROVIDERS_FILE";
const MAX_CONFIG_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Config {
    #[serde(default)]
    antigravity: Vec<Route>,
    #[serde(default)]
    claude_code: Vec<Route>,
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Route {
    Set {
        provider_id: String,
        api_type: String,
        base_url: String,
        #[serde(default)]
        headers: BTreeMap<String, HeaderSource>,
        #[serde(rename = "_meta")]
        meta: Option<serde_json::Map<String, Value>>,
    },
    Disable {
        provider_id: String,
    },
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum HeaderSource {
    Value(String),
    Environment { env: String },
}

#[derive(Deserialize)]
struct WireCatalog {
    providers: Vec<WireProvider>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProvider {
    provider_id: String,
    supported: Vec<String>,
    required: bool,
    current: Option<WireTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTarget {
    api_type: String,
    base_url: String,
}

impl AcpClient {
    pub(in crate::session::manager::external_agent) async fn prepare_provider_routes(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        if self.providers_prepared {
            return Ok(());
        }
        let routes = read_routes(self.agent, cancel).await?;
        if !routes.is_empty() {
            if !self.supports_provider_routes(cancel).await? {
                return Err(AppError::Config(format!(
                    "{} did not advertise ACP provider configuration.",
                    self.agent.label(),
                )));
            }
            let offered = self.read_providers(cancel).await?;
            let mut configured = HashSet::new();
            let mut requests = Vec::with_capacity(routes.len());
            for route in routes {
                let id = route.provider_id();
                if !configured.insert(id.to_owned()) {
                    return Err(AppError::Config(
                        "Configure each ACP provider ID only once.".into(),
                    ));
                }
                let provider = offered
                    .iter()
                    .find(|provider| provider.provider_id == id)
                    .ok_or_else(|| {
                        AppError::Config(
                            "Choose a provider ID advertised by the external agent.".into(),
                        )
                    })?;
                requests.push(route.request(provider)?);
            }
            for (method, params) in requests {
                self.call(method, params, cancel).await?;
            }
        }
        self.providers_prepared = true;
        Ok(())
    }

    pub(in crate::session::manager::external_agent) async fn provider_catalog(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<ExternalAgentProviders, AppError> {
        let supported = self.supports_provider_routes(cancel).await?;
        let providers = if supported {
            self.prepare_provider_routes(cancel).await?;
            self.read_providers(cancel).await?
        } else {
            Vec::new()
        };
        Ok(ExternalAgentProviders {
            supported,
            providers,
        })
    }

    async fn supports_provider_routes(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<bool, AppError> {
        let v2 = self.initialize(cancel).await?.version == Version::V2;
        let key = if v2 {
            "capabilities"
        } else {
            "agentCapabilities"
        };
        Ok(self
            .initialization
            .as_ref()
            .is_some_and(|value| value[key]["providers"].is_object()))
    }

    async fn read_providers(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Vec<ExternalAgentProvider>, AppError> {
        let value = self.call("providers/list", json!({}), cancel).await?;
        let catalog: WireCatalog = serde_json::from_value(value).map_err(|_| bad_catalog())?;
        let mut ids = HashSet::new();
        let mut providers = Vec::with_capacity(catalog.providers.len());
        for row in catalog.providers {
            if !valid_identifier(&row.provider_id)
                || !ids.insert(row.provider_id.clone())
                || row
                    .supported
                    .iter()
                    .any(|protocol| !valid_identifier(protocol))
            {
                return Err(bad_catalog());
            }
            let current = row.current.map(WireTarget::public).transpose()?;
            providers.push(ExternalAgentProvider {
                provider_id: row.provider_id,
                supported: row.supported,
                required: row.required,
                current,
            });
        }
        Ok(providers)
    }
}

impl Route {
    fn provider_id(&self) -> &str {
        match self {
            Self::Set { provider_id, .. } | Self::Disable { provider_id } => provider_id,
        }
    }

    fn request(self, provider: &ExternalAgentProvider) -> Result<(&'static str, Value), AppError> {
        match self {
            Self::Set {
                provider_id,
                api_type,
                base_url,
                headers,
                meta,
            } => {
                if !provider.supported.contains(&api_type) {
                    return Err(AppError::Config(
                        "Choose a protocol advertised for this ACP provider.".into(),
                    ));
                }
                let url = endpoint(&base_url).filter(|url| {
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                });
                if url.is_none() {
                    return Err(AppError::Config("An ACP provider endpoint must be an absolute HTTP(S) URL without credentials or a fragment.".into()));
                }
                let headers = resolve_headers(headers)?;
                let mut params = json!({
                    "providerId": provider_id, "apiType": api_type, "baseUrl": base_url, "headers": headers,
                });
                if let Some(meta) = meta {
                    params["_meta"] = Value::Object(meta);
                }
                Ok(("providers/set", params))
            }
            Self::Disable { provider_id } => {
                if provider.required {
                    return Err(AppError::Config(
                        "The external agent marked this ACP provider as required.".into(),
                    ));
                }
                Ok(("providers/disable", json!({"providerId": provider_id})))
            }
        }
    }
}

impl WireTarget {
    fn public(self) -> Result<ExternalAgentProviderTarget, AppError> {
        let mut url = endpoint(&self.base_url).ok_or_else(bad_catalog)?;
        if !valid_identifier(&self.api_type) {
            return Err(bad_catalog());
        }
        // The catalog is expected to contain only routing metadata. Strip URL
        // credentials and query parameters even if a peer returns them.
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        Ok(ExternalAgentProviderTarget {
            api_type: self.api_type,
            base_url: url.into(),
        })
    }
}

async fn read_routes(
    agent: ExternalAgent,
    cancel: &CancellationToken,
) -> Result<Vec<Route>, AppError> {
    let Some(path) = std::env::var_os(ROUTES_FILE_ENV) else {
        return Ok(Vec::new());
    };
    let read = async {
        let mut input = Vec::new();
        tokio::fs::File::open(path)
            .await?
            .take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut input)
            .await?;
        Ok::<_, std::io::Error>(input)
    };
    let input = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(AppError::Cancelled),
        input = tokio::time::timeout(STARTUP_TIMEOUT, read) => input,
    };
    let input = input
        .map_err(|_| AppError::Config("Reading the ACP provider configuration timed out.".into()))?
        .map_err(|error| {
            AppError::Config(format!(
                "Could not read the ACP provider configuration: {error}"
            ))
        })?;
    if input.len() > MAX_CONFIG_BYTES {
        return Err(AppError::Config(
            "The ACP provider configuration exceeds 256 KiB.".into(),
        ));
    }
    let config: Config = serde_json::from_slice(&input).map_err(|_| {
        AppError::Config(
            "Invalid ACP provider configuration. Check the JSON fields and action names.".into(),
        )
    })?;
    Ok(match agent {
        ExternalAgent::Antigravity => config.antigravity,
        ExternalAgent::ClaudeCode => config.claude_code,
    })
}

fn resolve_headers(
    headers: BTreeMap<String, HeaderSource>,
) -> Result<BTreeMap<String, String>, AppError> {
    let mut resolved = BTreeMap::new();
    for (name, source) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_headers())?;
        let value = match source {
            HeaderSource::Value(value) => value,
            HeaderSource::Environment { env } => std::env::var(env).map_err(|_| {
                AppError::Config(
                    "An environment variable used by an ACP provider header is unavailable.".into(),
                )
            })?,
        };
        HeaderValue::from_str(&value).map_err(|_| invalid_headers())?;
        if resolved.insert(name.as_str().to_owned(), value).is_some() {
            return Err(invalid_headers());
        }
    }
    Ok(resolved)
}

fn endpoint(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host().is_some()).then_some(url)
}

fn valid_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_CURSOR_BYTES
        && !value.chars().any(char::is_control)
}

fn bad_catalog() -> AppError {
    AppError::Config("The external agent returned an invalid ACP provider catalog.".into())
}

fn invalid_headers() -> AppError {
    AppError::Config(
        "The ACP provider configuration contains invalid or repeated HTTP headers.".into(),
    )
}
