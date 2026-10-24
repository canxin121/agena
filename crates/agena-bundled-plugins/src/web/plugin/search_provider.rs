use super::*;
use agena_web::{SearchApiOptions, SearchApiProvider, search_api};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchBackend {
    #[default]
    Html,
    Brave,
    Tavily,
    Exa,
    Searxng,
}

impl WebSearchBackend {
    fn api(self) -> Option<SearchApiProvider> {
        match self {
            Self::Html => None,
            Self::Brave => Some(SearchApiProvider::Brave),
            Self::Tavily => Some(SearchApiProvider::Tavily),
            Self::Exa => Some(SearchApiProvider::Exa),
            Self::Searxng => Some(SearchApiProvider::Searxng),
        }
    }
}

fn endpoint(config: &WebSearchConfig) -> SdkResult<url::Url> {
    let provider = config
        .provider
        .api()
        .ok_or_else(|| PluginError::invalid_params("HTML search has no API endpoint"))?;
    let value = config.endpoint.as_deref().or(provider.default_endpoint())
        .ok_or_else(|| PluginError::configuration_required("web.search", "Set search.endpoint to the SearXNG /search URL and enable JSON search on the server."))?;
    let url = url::Url::parse(value).map_err(|_| {
        PluginError::invalid_params("search.endpoint must be an absolute HTTP(S) URL")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(PluginError::invalid_params(
            "search.endpoint must be HTTP(S), without credentials, query parameters or a fragment",
        ));
    }
    if provider != SearchApiProvider::Searxng && url.scheme() != "https" {
        return Err(PluginError::invalid_params(
            "authenticated search providers require an HTTPS endpoint",
        ));
    }
    Ok(url)
}

pub(super) fn validate_config(config: &WebSearchConfig) -> SdkResult<()> {
    if config.max_limit > 50 {
        return Err(PluginError::invalid_params(
            "search.max_limit must not exceed 50 (API providers cap each response at 20)",
        ));
    }
    for (default_name, default, max_name, maximum, ceiling) in [
        (
            "default_results_per_engine",
            config.default_results_per_engine,
            "max_results_per_engine",
            config.max_results_per_engine,
            50,
        ),
        (
            "default_pages_per_engine",
            config.default_pages_per_engine,
            "max_pages_per_engine",
            config.max_pages_per_engine,
            5,
        ),
    ] {
        if default == 0 || default > maximum || !(1..=ceiling).contains(&maximum) {
            return Err(PluginError::invalid_params(format!(
                "search.{default_name} and search.{max_name} require 1 <= default <= maximum <= {ceiling}",
            )));
        }
    }
    if config.provider == WebSearchBackend::Html {
        if config.endpoint.is_some()
            || config.api_key_env.is_some()
            || config.allow_private_endpoint
        {
            return Err(PluginError::invalid_params(
                "search endpoint and credential settings require an API provider",
            ));
        }
        return Ok(());
    }
    endpoint(config)?;
    if config.allow_private_endpoint && config.provider != WebSearchBackend::Searxng {
        return Err(PluginError::invalid_params(
            "allow_private_endpoint is supported only for an explicitly configured SearXNG service",
        ));
    }
    if let Some(name) = config.api_key_env.as_deref() {
        if config.provider == WebSearchBackend::Searxng {
            return Err(PluginError::invalid_params(
                "the SearXNG adapter does not use an API key",
            ));
        }
        if name.is_empty()
            || name.len() > 128
            || !name.starts_with(|c: char| c == '_' || c.is_ascii_alphabetic())
            || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            return Err(PluginError::invalid_params(
                "search.api_key_env must be an environment variable name",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_input(input: &CrawlWebSearchInput) -> SdkResult<()> {
    if input.query.trim().is_empty() || input.query.len() > 8192 {
        return Err(PluginError::invalid_params(
            "search query must contain 1–8192 bytes",
        ));
    }
    for (name, value, ceiling) in [
        ("max_results", input.max_results, 50),
        ("max_results_per_engine", input.max_results_per_engine, 50),
        ("max_pages_per_engine", input.max_pages_per_engine, 5),
    ] {
        if value.is_some_and(|value| !(1..=ceiling).contains(&value)) {
            return Err(PluginError::invalid_params(format!(
                "{name} must be between 1 and {ceiling}",
            )));
        }
    }
    if input.allowed_domains.len() > 64 || input.blocked_domains.len() > 64 {
        return Err(PluginError::invalid_params(
            "at most 64 included and 64 excluded domains are supported",
        ));
    }
    for domain in input.allowed_domains.iter().chain(&input.blocked_domains) {
        let domain = domain.trim().trim_end_matches('.');
        if domain.is_empty()
            || domain.len() > 253
            || domain.contains(['/', ':', '@', '*', '?', '#'])
            || url::Host::parse(domain).is_err()
        {
            return Err(PluginError::invalid_params(
                "domain filters must be bare hostnames, without schemes, paths, ports or wildcards",
            ));
        }
    }
    Ok(())
}

async fn resolve_endpoint(
    config: &WebSearchConfig,
    url: &url::Url,
    timeout: Duration,
) -> SdkResult<Vec<std::net::SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::invalid_params("search endpoint has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PluginError::invalid_params("search endpoint has no port"))?;
    let addresses = tokio::time::timeout(timeout, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| PluginError::internal("search endpoint DNS lookup timed out"))?
        .map_err(|error| plugin_internal_error_with_context("resolve search endpoint", &error))?
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(PluginError::internal(
            "search endpoint DNS lookup returned no addresses",
        ));
    }
    if !(config.provider == WebSearchBackend::Searxng && config.allow_private_endpoint)
        && addresses
            .iter()
            .any(|address| !is_public_address(address.ip()))
    {
        return Err(PluginError::invalid_params(
            "search endpoint resolves to a non-public address; private SearXNG requires search.allow_private_endpoint",
        ));
    }
    Ok(addresses)
}

impl WebPlugin {
    pub(super) async fn search_with_provider(
        &self,
        input: &CrawlWebSearchInput,
        limit: usize,
    ) -> SdkResult<ToolInvokeOutput> {
        let state = self.state()?;
        let config = &state.config;
        let provider = config
            .search
            .provider
            .api()
            .ok_or_else(|| PluginError::internal("search API backend was not selected"))?;
        let url = endpoint(&config.search)?;
        state
            .host
            .require_network_permission(url.to_string())
            .await?;
        let timeout = Duration::from_secs(config.fetch.request.timeout_secs);
        let key_env = config
            .search
            .api_key_env
            .as_deref()
            .or(provider.default_key_env());
        let key = key_env
            .map(|name| {
                crate::plugins::provided::official_service::env_secret(name, provider.label())
                    .map_err(|_| {
                        PluginError::configuration_required(
                            "web.search",
                            format!(
                                "Set the server environment variable {name} for {} search.",
                                provider.label()
                            ),
                        )
                    })
            })
            .transpose()?;
        state.fetch_coordinator.wait_for_url_host(&url).await;
        let addresses = resolve_endpoint(&config.search, &url, timeout).await?;
        let result = search_api(
            &input.query,
            &SearchApiOptions {
                provider,
                endpoint: &url,
                resolved_addrs: &addresses,
                api_key: key.as_deref(),
                limit,
                timeout,
                user_agent: &self.user_agent,
                allowed_domains: &input.allowed_domains,
                blocked_domains: &input.blocked_domains,
            },
        )
        .await
        .map_err(crawl_error_to_plugin)?;
        let mut payload =
            serde_json::to_value(&result).map_err(|error| PluginError::internal_error(&error))?;
        payload["query"] = serde_json::json!(input.query);
        payload["engine"] = serde_json::json!(provider.label());
        payload["attempted_engines"] = serde_json::json!([provider.label()]);
        payload["engine_errors"] = serde_json::json!([]);
        let mut text = format!(
            "Found {} result(s) via {}. Fetch relevant URLs before using snippets as evidence.\n\n{}",
            result.results.len(),
            provider.label(),
            results_to_text(&result.results)
        );
        if result.truncated {
            text.push_str(
                "\nSome additional results or snippet text were omitted by the output budget.",
            );
        }
        if !result.warnings.is_empty() {
            text.push_str(&format!("\n{}", result.warnings.join("\n")));
        }
        Ok(ToolInvokeOutput::from_parts(
            format!("web search {}", input.query),
            format!("{} results · {}", result.results.len(), provider.label()),
            text,
            Some(payload),
            BTreeMap::new(),
            Vec::new(),
        ))
    }
}
