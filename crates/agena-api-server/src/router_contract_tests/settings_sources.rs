use super::*;

#[tokio::test]
async fn settings_batches_match_individual_sources_including_quoted_array_and_missing_paths() {
    let (provider_url, _, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let http = reqwest::Client::new();
    // Only the isolated test workspace is changed, with no runtime reload.
    let global = server._workspace.path().join("isolated-global-agena.json");
    std::fs::write(
        &global,
        serde_json::to_vec(&serde_json::json!({
            "ui": {"fixture.key": {"items": ["first", "second"]}, "locale": "en-US"},
        }))
        .unwrap(),
    )
    .unwrap();
    let paths = [
        "ui.locale",
        r#"ui."fixture.key".items.1"#,
        "ui.absent",
        "providers.fake.auth.mode",
        "providers.fake.adapters.openai_responses.models.fake-model.agena_tools.mode",
    ];
    let response = http
        .get(format!("{}/api/v1/settings/sources", server.url))
        .query(&[("paths", serde_json::to_string(&paths).unwrap())])
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    let batch: serde_json::Value = response.json().await.unwrap();
    for path in paths {
        for (source, endpoint, query_source) in [
            ("effective", "/api/v1/settings", "effective"),
            ("file", "/api/v1/settings", "file"),
            ("global", "/api/v1/settings/layers/global", "file"),
            ("workspace", "/api/v1/settings/layers/workspace", "file"),
        ] {
            let individual = http
                .get(format!("{}{endpoint}", server.url))
                .query(&[("source", query_source), ("path", path)])
                .send()
                .await
                .unwrap();
            assert!(individual.status().is_success());
            let individual: serde_json::Value = individual.json().await.unwrap();
            assert_eq!(batch[path][source], individual, "{path} / {source}");
        }
    }
    assert_eq!(
        batch[r#"ui."fixture.key".items.1"#]["global"]["value"],
        "second"
    );
    assert!(batch["ui.absent"]["workspace"]["value"].is_null());
    assert!(batch["ui.locale"]["global"]["revision"].is_string());
    provider.abort();
}

#[tokio::test]
async fn settings_batches_bound_input_and_validate_every_path_before_reading() {
    let (provider_url, _, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let http = reqwest::Client::new();
    let endpoint = format!("{}/api/v1/settings/sources", server.url);
    for input in [
        "not json".to_owned(),
        "{}".to_owned(),
        "[]".to_owned(),
        "[\"\"]".to_owned(),
        "[\"ui.locale\",\"invalid..path\"]".to_owned(),
        serde_json::to_string(&vec!["ui.locale"; 65]).unwrap(),
        serde_json::to_string(&vec!["x".repeat(513)]).unwrap(),
        " ".repeat(16_385),
    ] {
        let response = http
            .get(&endpoint)
            .query(&[("paths", input)])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    let paths: Vec<_> = (0..64).map(|index| format!("ui.missing_{index}")).collect();
    let response = http
        .get(&endpoint)
        .query(&[("paths", serde_json::to_string(&paths).unwrap())])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let result: serde_json::Value = response.json().await.unwrap();
    assert_eq!(result.as_object().unwrap().len(), 64);
    let duplicate = http
        .get(&endpoint)
        .query(&[("paths", "[\"ui.locale\",\"ui.locale\"]")])
        .send()
        .await
        .unwrap();
    assert!(duplicate.status().is_success());
    assert_eq!(
        duplicate
            .json::<serde_json::Value>()
            .await
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        1
    );
    provider.abort();
}

#[tokio::test]
async fn concurrent_settings_edits_preserve_the_file_lock_and_each_distinct_value() {
    let (provider_url, _, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let http = reqwest::Client::new();
    let endpoint = format!("{}/api/v1/settings/layers/global", server.url);
    let requests = (0..16).map(|index| {
        let http = http.clone();
        let endpoint = endpoint.clone();
        async move {
            let response = http
                .put(&endpoint)
                .json(&serde_json::json!({
                    "path": format!("ui.concurrent_fixture_{index}"), "value": index,
                    "reload": false, "validate": false,
                }))
                .send()
                .await
                .unwrap();
            assert!(
                response.status().is_success(),
                "{}",
                response.text().await.unwrap()
            );
        }
    });
    futures_util::future::join_all(requests).await;
    let result: serde_json::Value = http
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for index in 0..16 {
        assert_eq!(
            result["value"]["ui"][format!("concurrent_fixture_{index}")],
            index
        );
    }
    provider.abort();
}

/// Layer edits validate the composed configuration rather than the edited
/// document alone, so a workspace override may reference a provider that only
/// the global layer declares (the normal setup for `providers.default_selection`).
#[tokio::test]
async fn workspace_layer_edits_validate_against_the_global_layer() {
    let (provider_url, _, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let http = reqwest::Client::new();
    // Only the isolated test workspace and its global fixture file change.
    let global = server._workspace.path().join("isolated-global-agena.json");
    std::fs::write(
        &global,
        serde_json::to_vec_pretty(&serde_json::json!({
            "providers": {
                "globalfake": {
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": provider_url,
                        "api_key": {"kind": "inline", "value": "fake-test-key"}
                    },
                    "adapters": {
                        "openai_responses": {
                            "enabled": true,
                            "models": {"fake-model": {"agena_tools": {"mode": "provider_protocol"}}}
                        }
                    }
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let response = http
        .put(format!("{}/api/v1/settings/layers/workspace", server.url))
        .json(&serde_json::json!({
            "path": "providers.default_selection",
            "value": {
                "provider": "globalfake",
                "adapter": "openai_responses",
                "model": "fake-model"
            },
            "dry_run": false,
            "validate": true,
            "reload": false
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "a workspace default model may reference a globally declared provider: {}",
        response.text().await.unwrap()
    );
    let workspace_config =
        std::fs::read_to_string(server._workspace.path().join(".agena/agena.json")).unwrap();
    assert!(
        workspace_config.contains("globalfake"),
        "workspace override must be persisted: {workspace_config}"
    );

    // The composed configuration still has to resolve, so an edit naming a
    // provider no layer declares stays rejected.
    let response = http
        .put(format!("{}/api/v1/settings/layers/workspace", server.url))
        .json(&serde_json::json!({
            "path": "providers.default_selection",
            "value": {
                "provider": "ghost",
                "adapter": "openai_responses",
                "model": "fake-model"
            },
            "dry_run": false,
            "validate": true,
            "reload": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    provider.abort();
}
