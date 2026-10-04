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

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_host::sdk::host_api::{
        AskUserRequest, AskUserResponse, EventSubscription, LogLevel, PermissionQuery,
    };
    use agena_plugin_host::sdk::{EventEnvelope, EventFilter};

    struct PolicyHost(bool);
    #[async_trait]
    impl HostClient for PolicyHost {
        async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
        async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }
        async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
            Err(PluginError::internal("unused"))
        }
        async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
            Err(PluginError::internal("unused"))
        }
        async fn invoke_tool(
            &self,
            _: String,
            _: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::internal("unused"))
        }
        async fn ask_user(&self, _: AskUserRequest) -> SdkResult<AskUserResponse> {
            Err(PluginError::internal("unused"))
        }
        async fn check_network_permission(&self, _: String) -> SdkResult<PermissionQuery> {
            Ok(if self.0 {
                PermissionQuery::Allow {}
            } else {
                PermissionQuery::Deny {
                    reason: "fixture network denial".into(),
                }
            })
        }
    }

    #[tokio::test]
    async fn configured_searxng_runs_through_the_plugin_entry_and_reports_partial_results() {
        use agena_plugin_host::sdk::{Plugin, ToolInvokeInput};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/search", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let read = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(request.starts_with("GET /search?"));
                assert!(request.contains("format=json"));
                let body = serde_json::json!({"results":[
                {"title":"Guide","url":"https://docs.example.com/guide", "content":"Preview"},
                {"title":"Blocked","url":"https://ads.example.com"},
                {"title":"Unrelated","url":"https://unrelated.test"}
            ], "unresponsive_engines":[["fixture", "timeout"]]})
                .to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let root = tempfile::tempdir().unwrap();
        let plugin = WebPlugin::new();
        let config = parse_web_config(serde_json::json!({"search":{"provider":"searxng", "endpoint":endpoint, "allow_private_endpoint":true}})).unwrap();
        assert!(
            plugin
                .state
                .set(WebPluginState::new(config, Arc::new(PolicyHost(true))))
                .is_ok()
        );
        for engine in [None, Some("auto")] {
            let mut input = serde_json::json!({"query":"fixture", "allowed_domains":["example.com"], "blocked_domains":["ads.example.com"]});
            if let Some(engine) = engine {
                input["engine"] = serde_json::json!(engine);
            }
            let output = plugin
                .tool_invoke(ToolInvokeInput {
                    tool_name: "search".into(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: root.path().to_string_lossy().into_owned(),
                    input,
                })
                .await
                .unwrap();
            let payload = output.payload.unwrap();
            assert_eq!(payload["engine"], "searxng");
            assert_eq!(payload["attempted_engines"], serde_json::json!(["searxng"]));
            assert_eq!(payload["partial"], true);
            assert_eq!(payload["results"].as_array().unwrap().len(), 1);
            assert_eq!(payload["filtered_result_count"], 2);
            assert!(output.output_text.contains("did not respond"));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn explicit_html_engine_overrides_configured_api_and_keeps_permission_checks() {
        let plugin = WebPlugin::new();
        let config = parse_web_config(serde_json::json!({"search":{"provider":"brave"}})).unwrap();
        assert!(
            plugin
                .state
                .set(WebPluginState::new(config, Arc::new(PolicyHost(false))))
                .is_ok()
        );
        for engine in WebSearchEngine::ALL {
            let name = engine.label();
            let input = serde_json::from_value(serde_json::json!({
                "query":"fixture", "engine":name,
                "max_results_per_engine": 25, "max_pages_per_engine": 3
            }))
            .unwrap();
            let error = plugin.invoke_search(&input).await.unwrap_err();
            assert_eq!(
                error.kind,
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
            );
            assert!(error.diagnostic_message().contains(engine.permission_url()));
        }
    }

    #[tokio::test]
    async fn csv_selection_uses_html_despite_api_config_and_invalid_lists_fail_at_the_tool_boundary()
     {
        use agena_plugin_host::sdk::{Plugin, ToolInvokeInput};
        let config = parse_web_config(serde_json::json!({"search":{"provider":"brave"}})).unwrap();
        let plugin = WebPlugin::new();
        assert!(
            plugin
                .state
                .set(WebPluginState::new(config, Arc::new(PolicyHost(false))))
                .is_ok()
        );
        let root = tempfile::tempdir().unwrap();
        for engine in [
            " baidu,GOOGLE,baidu ",
            "auto,google",
            "baidu,unknown",
            "baidu,",
        ] {
            let error = plugin
                .tool_invoke(ToolInvokeInput {
                    tool_name: "search".into(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: root.path().to_string_lossy().into_owned(),
                    input: serde_json::json!({
                        "query":"fixture", "engine":engine,
                        "max_results_per_engine":25, "max_pages_per_engine":3
                    }),
                })
                .await
                .unwrap_err();
            if engine == " baidu,GOOGLE,baidu " {
                let message = error.diagnostic_message();
                assert!(message.contains("all search engines failed"));
                for engine in WebSearchEngine::ALL {
                    assert_eq!(
                        message.contains(engine.permission_url()),
                        matches!(engine, WebSearchEngine::Baidu | WebSearchEngine::Google)
                    );
                }
            } else {
                assert_eq!(error.kind, PluginError::invalid_params("fixture").kind);
                assert!(
                    !error
                        .diagnostic_message()
                        .contains("all search engines failed")
                );
            }
        }
    }

    #[tokio::test]
    async fn html_auto_reports_network_permission_denials_from_every_selected_engine() {
        let plugin = WebPlugin::new();
        assert!(
            plugin
                .state
                .set(WebPluginState::new(
                    WebConfig::default(),
                    Arc::new(PolicyHost(false))
                ))
                .is_ok()
        );
        let input = serde_json::from_value(serde_json::json!({"query":"fixture"})).unwrap();
        let error = plugin.invoke_search(&input).await.unwrap_err();
        let message = error.diagnostic_message();
        assert!(message.contains("all search engines failed"));
        for engine in WebSearchEngine::ALL {
            assert!(message.contains(engine.permission_url()));
        }
    }

    #[test]
    fn invalid_domain_filters_and_query_limits_fail_before_network_work() {
        for input in [
            serde_json::json!({"query":"q", "allowed_domains":["https://example.com"]}),
            serde_json::json!({"query":"q", "blocked_domains":["*.example.com"]}),
            serde_json::json!({"query":"q", "max_results":0}),
            serde_json::json!({"query":"q", "max_results":51}),
            serde_json::json!({"query":"q", "max_results_per_engine":0}),
            serde_json::json!({"query":"q", "max_results_per_engine":51}),
            serde_json::json!({"query":"q", "max_pages_per_engine":0}),
            serde_json::json!({"query":"q", "max_pages_per_engine":6}),
            serde_json::json!({"query":"汉".repeat(3000)}),
        ] {
            let input: CrawlWebSearchInput = serde_json::from_value(input).unwrap();
            assert!(validate_input(&input).is_err());
        }
    }

    #[tokio::test]
    async fn api_routing_rejects_html_budgets_before_permission_or_credential_work() {
        for provider in ["brave", "tavily", "exa", "searxng"] {
            let mut settings = serde_json::json!({"search":{"provider":provider}});
            if provider == "searxng" {
                settings["search"]["endpoint"] = serde_json::json!("https://example.com/search");
            }
            let plugin = WebPlugin::new();
            assert!(
                plugin
                    .state
                    .set(WebPluginState::new(
                        parse_web_config(settings).unwrap(),
                        Arc::new(PolicyHost(false))
                    ))
                    .is_ok()
            );
            for engine in [None, Some("auto")] {
                for budget in ["max_results_per_engine", "max_pages_per_engine"] {
                    let mut input = serde_json::json!({"query":"fixture", "engine":engine});
                    input[budget] = serde_json::json!(1);
                    let input = serde_json::from_value(input).unwrap();
                    let error = plugin.invoke_search(&input).await.unwrap_err();
                    assert_eq!(error.kind, PluginError::invalid_params("fixture").kind);
                    assert!(error.diagnostic_message().contains("require HTML search"));
                }
            }
        }
    }

    #[test]
    fn html_budget_config_rejects_zero_excessive_or_inverted_limits() {
        // Old settings continue to deserialize with independent HTML defaults.
        let config = parse_web_config(serde_json::json!({"search":{"default_limit":8}})).unwrap();
        assert_eq!(config.search.default_results_per_engine, 10);
        assert_eq!(config.search.default_pages_per_engine, 1);
        for settings in [
            serde_json::json!({"default_results_per_engine":0}),
            serde_json::json!({"max_results_per_engine":0}),
            serde_json::json!({"max_results_per_engine":51}),
            serde_json::json!({"default_results_per_engine":11,"max_results_per_engine":10}),
            serde_json::json!({"default_pages_per_engine":0}),
            serde_json::json!({"max_pages_per_engine":0}),
            serde_json::json!({"max_pages_per_engine":6}),
            serde_json::json!({"default_pages_per_engine":3,"max_pages_per_engine":2}),
        ] {
            assert!(parse_web_config(serde_json::json!({"search":settings})).is_err());
        }
    }

    #[tokio::test]
    async fn denied_search_never_connects_even_when_private_endpoint_is_enabled() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = parse_web_config(serde_json::json!({"search":{
            "provider":"searxng", "endpoint":format!("http://{}/search", listener.local_addr().unwrap()),
            "allow_private_endpoint":true
        }})).unwrap();
        let plugin = WebPlugin::new();
        assert!(
            plugin
                .state
                .set(WebPluginState::new(config, Arc::new(PolicyHost(false))))
                .is_ok()
        );
        let input = serde_json::from_value(serde_json::json!({"query":"fixture"})).unwrap();
        let error = plugin.invoke_search(&input).await.unwrap_err();
        assert_eq!(
            error.kind,
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn backend_config_keeps_defaults_and_rejects_unsafe_credentials_and_endpoints() {
        validate_config(&WebSearchConfig::default()).unwrap();
        for value in [
            serde_json::json!({"provider":"brave", "endpoint":"http://example.com/search"}),
            serde_json::json!({"provider":"tavily", "endpoint":"https://user:secret@example.com/search"}),
            serde_json::json!({"provider":"exa", "endpoint":"https://example.com/search?api_key=secret"}),
            serde_json::json!({"provider":"brave", "allow_private_endpoint":true}),
            serde_json::json!({"provider":"searxng"}),
            serde_json::json!({"provider":"brave", "api_key_env":"not a variable"}),
            serde_json::json!({"provider":"html", "endpoint":"https://example.com"}),
        ] {
            let config: WebSearchConfig = serde_json::from_value(value).unwrap();
            assert!(validate_config(&config).is_err(), "{config:?}");
        }
        let config: WebSearchConfig = serde_json::from_value(serde_json::json!({"provider":"searxng", "endpoint":"http://127.0.0.1:8080/search", "allow_private_endpoint":true})).unwrap();
        validate_config(&config).unwrap();
        assert_eq!(endpoint(&config).unwrap().path(), "/search");
    }

    #[tokio::test]
    async fn private_endpoints_require_explicit_searxng_configuration() {
        let url = url::Url::parse("http://127.0.0.1:8080/search").unwrap();
        let mut config = WebSearchConfig {
            provider: WebSearchBackend::Searxng,
            endpoint: Some(url.to_string()),
            ..Default::default()
        };
        assert!(
            resolve_endpoint(&config, &url, Duration::from_secs(1))
                .await
                .is_err()
        );
        config.allow_private_endpoint = true;
        assert_eq!(
            resolve_endpoint(&config, &url, Duration::from_secs(1))
                .await
                .unwrap(),
            ["127.0.0.1:8080".parse().unwrap()]
        );
    }
}
