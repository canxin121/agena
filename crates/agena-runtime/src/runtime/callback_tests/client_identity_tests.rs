use super::*;

fn write_versions(fixture: &Fixture, version: &str) {
    let path = fixture.directory.path().join("config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["runtime"] = json!({"providers": {"client_versions": {
        "codex": version, "claude": version, "gemini": version,
    }}});
    std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
}

async fn discovery_user_agent(runtime: &AgenaRuntime) -> String {
    discovery_user_agent_for(runtime, "openai_responses").await
}

async fn discovery_user_agent_for(runtime: &AgenaRuntime, adapter: &str) -> String {
    let observed = Arc::new(Mutex::new(None));
    let captured = observed.clone();
    let router = axum::Router::new().route(
        "/{vendor}/models",
        axum::routing::get(
            move |axum::extract::Path(vendor): axum::extract::Path<String>,
                  headers: axum::http::HeaderMap| {
                let captured = captured.clone();
                async move {
                    *captured.lock().unwrap() =
                        Some(headers["user-agent"].to_str().unwrap().to_owned());
                    axum::Json(if vendor == "gemini" {
                        json!({"models":[]})
                    } else {
                        json!({"object":"list", "data":[]})
                    })
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let listing = agena_provider::ProviderCatalog::list_draft_adapter_models(
        runtime,
        agena_provider::DraftProviderAdapterModelsRequest::Http(
            agena_provider::DraftHttpProviderAdapterModelsRequest {
                provider_id: Some("identity-fixture".into()),
                base_url,
                protocol_paths: agena_provider::ProviderProtocolPaths {
                    openai: "/openai".into(),
                    anthropic: "/anthropic".into(),
                    gemini: "/gemini".into(),
                },
                api_key: Some(agena_provider::ProviderApiKeySource::Inline(
                    "fixture-key".into(),
                )),
                adapter_ids: vec![adapter.into()],
            },
        ),
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(listing.adapters.len(), 1);
    assert!(listing.adapters[0].failure.is_none(), "{listing:?}");
    observed
        .lock()
        .unwrap()
        .take()
        .expect("actual model discovery request")
}

async fn fixture_with_versions() -> (Fixture, Arc<AgenaRuntime>, String) {
    let mut fixture = Fixture::new().await;
    fixture.write_config("en-US");
    write_versions(&fixture, "0.111.1");
    let runtime = fixture.start().await;
    let original = discovery_user_agent(&runtime).await;
    assert!(original.starts_with("codex_cli_rs/0.111.1 "), "{original}");
    (fixture, runtime, original)
}

#[tokio::test]
async fn unpublished_candidate_keeps_live_discovery_client_identity() {
    let (fixture, runtime, original) = fixture_with_versions().await;
    write_versions(&fixture, "0.222.2");
    let candidate =
        reload_shutdown_tests::build_candidate(&runtime, &runtime.current_snapshot()).await;
    let during = discovery_user_agent(&runtime).await;
    drop(candidate);
    let after = discovery_user_agent(&runtime).await;
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(
        during, original,
        "unpublished candidate leaked into live request headers"
    );
    assert_eq!(
        after, original,
        "discarding a candidate left its identity installed"
    );
}

#[tokio::test]
async fn failed_candidate_keeps_live_discovery_client_identity() {
    let (fixture, runtime, original) = fixture_with_versions().await;
    write_versions(&fixture, "0.222.2");
    fixture
        .observed
        .provider_error
        .store(true, Ordering::SeqCst);
    let error = runtime.reload().await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected provider composition failure")
    );
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(discovery_user_agent(&runtime).await, original);
}

#[tokio::test]
async fn cancelled_candidate_keeps_live_discovery_client_identity() {
    let (fixture, runtime, original) = fixture_with_versions().await;
    write_versions(&fixture, "0.222.2");
    fixture.observed.provider_wait.store(true, Ordering::SeqCst);
    let reload_runtime = runtime.clone();
    let candidate = tokio::spawn(async move { reload_runtime.reload().await });
    tokio::time::timeout(WAIT, fixture.observed.provider_entered.notified())
        .await
        .unwrap();
    let during = discovery_user_agent(&runtime).await;
    candidate.abort();
    assert!(candidate.await.unwrap_err().is_cancelled());
    let after = discovery_user_agent(&runtime).await;
    assert_eq!(runtime.current_snapshot().generation(), 1);
    assert_eq!(during, original);
    assert_eq!(after, original);
}

#[tokio::test]
async fn published_client_identities_are_isolated_between_runtimes_for_all_vendors() {
    let (fixture, runtime, _) = fixture_with_versions().await;
    write_versions(&fixture, "0.222.2");
    runtime.reload().await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"runtime":{"providers":{"client_versions":{
            "codex":"0.333.3", "claude":"0.333.3", "gemini":"0.333.3",
        }}}}))
        .unwrap(),
    )
    .unwrap();
    let mut config = fixture.composition_config();
    config.workspace_root = Some(directory.path().to_owned());
    config.load_request.workspace_root = Some(directory.path().to_owned());
    config.load_request.config_path = Some(path);
    let second = AgenaRuntime::new_with_maintenance(config, false)
        .await
        .unwrap();
    let mut headers = Vec::new();
    for adapter in [
        "openai_responses",
        "openai_chat_completions",
        "openai_realtime",
        "anthropic",
        "gemini",
    ] {
        headers.push((
            adapter,
            discovery_user_agent_for(&runtime, adapter).await,
            discovery_user_agent_for(&second, adapter).await,
        ));
    }
    second.shutdown();
    assert_eq!(runtime.current_snapshot().generation(), 2);
    for (adapter, first, second) in headers {
        assert!(first.contains("/0.222.2"), "{adapter}: {first}");
        assert!(second.contains("/0.333.3"), "{adapter}: {second}");
    }
}

#[tokio::test]
async fn candidate_and_retained_provider_registries_use_their_own_identity() {
    let mut fixture = Fixture::new().await;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let router = axum::Router::new().route(
        "/{vendor}/models",
        axum::routing::get(
            move |axum::extract::Path(vendor): axum::extract::Path<String>,
                  headers: axum::http::HeaderMap| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push((
                        vendor.clone(),
                        headers["user-agent"].to_str().unwrap().to_owned(),
                    ));
                    axum::Json(if vendor == "gemini" {
                        json!({"models":[]})
                    } else {
                        json!({"object":"list", "data":[]})
                    })
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    std::fs::write(fixture.directory.path().join("config.json"), serde_json::to_vec(&json!({
        "plugins":fixture.plugins,
        "providers":{"identity-fixture":{
            "auth":{"mode":"api", "subtype":"custom", "base_url":base_url,
                "protocol_paths":{"openai":"/openai", "anthropic":"/anthropic", "gemini":"/gemini"},
                "api_key":{"kind":"inline", "value":"fixture-key"}},
            "adapters":{
                "openai_responses":{"enabled":true, "models":{"gpt-test":{}}},
                "anthropic":{"enabled":true, "models":{"claude-test":{}}},
                "gemini":{"enabled":true, "models":{"gemini-test":{}}},
            },
        }},
    })).unwrap()).unwrap();
    write_versions(&fixture, "0.111.1");
    let runtime = fixture.start().await;
    let gate = runtime.inner.control_state.reload_gate().acquire().await;
    let original = runtime.current_snapshot();
    write_versions(&fixture, "0.222.2");
    let candidate = Arc::new(reload_shutdown_tests::build_candidate(&runtime, &original).await);
    let old_registry = original.catalog_source_provider_registry();
    let candidate_registry = candidate.catalog_source_provider_registry();
    let mut results = Vec::new();
    for (registry, version) in [(&old_registry, "0.111.1"), (&candidate_registry, "0.222.2")] {
        registry.list_models("identity-fixture").await.unwrap();
        results.push((version, std::mem::take(&mut *observed.lock().unwrap())));
    }
    let report = runtime
        .publish_reload_candidate(original, candidate, crate::RuntimeReloadCause::Manual)
        .unwrap();
    drop(gate);
    assert_eq!(report.generation, 2);
    old_registry.list_models("identity-fixture").await.unwrap();
    results.push(("0.111.1", std::mem::take(&mut *observed.lock().unwrap())));
    runtime
        .current_snapshot()
        .catalog_source_provider_registry()
        .list_models("identity-fixture")
        .await
        .unwrap();
    results.push(("0.222.2", std::mem::take(&mut *observed.lock().unwrap())));
    server.abort();
    for (version, requests) in results {
        assert_eq!(
            requests.len(),
            3,
            "all configured vendors must make a real model request"
        );
        for (vendor, user_agent) in requests {
            assert!(
                user_agent.contains(&format!("/{version}")),
                "{vendor}: {user_agent}"
            );
        }
    }
}

#[tokio::test]
async fn plugin_host_version_is_independent_of_provider_client_versions() {
    let (mut fixture, runtime, _) = fixture_with_versions().await;
    let initial = fixture.observed.contexts.lock().unwrap()[0]
        .agena_version
        .clone();
    fixture.plugins.list.get_mut(KEY).unwrap().settings = json!({"revision": 2});
    fixture.write_config("en-US");
    write_versions(&fixture, "0.222.2");
    runtime.reload().await.unwrap();
    let after = fixture
        .observed
        .contexts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .agena_version
        .clone();
    assert_eq!(initial, env!("CARGO_PKG_VERSION"));
    assert_eq!(after, env!("CARGO_PKG_VERSION"));
}
