use super::*;

#[tokio::test]
async fn runtime_task_activity_reads_are_conditional_and_terminal_changes_invalidate_them() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let application = application_for_test(&server.runtime);
    let control = application.runtime_control();
    let task = control
        .start_background_task(
            agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginInstall,
            agena_runtime::RuntimeBackgroundTaskOrigin::User,
            "conditional task fixture".into(),
            None,
            true,
            Box::new(|_| Box::pin(std::future::pending())),
        )
        .unwrap()
        .task;
    let http = reqwest::Client::new();
    let endpoint = format!("{}/api/v1/activities/{}", server.url, task.id);
    let first = http
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let etag = first.headers()["etag"].to_str().unwrap().to_owned();
    let body: serde_json::Value = first.json().await.unwrap();
    assert_eq!(body["id"], task.id);
    assert_eq!(body["status"], "running");
    let idle = http
        .get(&endpoint)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(idle.status(), reqwest::StatusCode::NOT_MODIFIED);
    assert!(idle.bytes().await.unwrap().is_empty());

    control.cancel_background_task(&task.id).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let current = http
                .get(&endpoint)
                .header("if-none-match", &etag)
                .send()
                .await
                .unwrap();
            if current.status() == reqwest::StatusCode::OK {
                let body: serde_json::Value = current.json().await.unwrap();
                if body["status"] == "cancelled" {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("terminal task activity advances its resource revision");
}

async fn tokens(base: &str, keys: &[String]) -> BTreeMap<String, String> {
    reqwest::Client::new()
        .get(format!("{base}/api/v1/changes/revisions"))
        .query(&[("resources", serde_json::to_string(keys).unwrap())])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn resource_versions_are_scoped_and_conditional_reads_survive_mutation_and_deletion() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let http = reqwest::Client::new();
    let second = tempfile::tempdir().unwrap();
    let workspace: serde_json::Value = http
        .post(format!("{}/api/v1/workspaces", server.url))
        .json(&serde_json::json!({"path": second.path().to_string_lossy()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_id = workspace["id"].as_i64().unwrap();
    let session = client
        .create_session(server.workspace_id, "before", None)
        .await
        .unwrap();
    let keys = vec![
        "workspaces".into(),
        "sessions".into(),
        format!("workspace:{}:sessions", server.workspace_id),
        format!("workspace:{second_id}:sessions"),
        format!("session:{}:state", session.id),
        format!("session:{}:transcript", session.id),
    ];
    let initial = tokens(&server.url, &keys).await;
    assert_eq!(
        initial,
        tokens(&server.url, &keys).await,
        "an idle version check must not mutate tokens"
    );
    let endpoint = format!("{}/api/v1/sessions/{}/state", server.url, session.id);
    let response = http
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let etag = response.headers()["etag"].to_str().unwrap().to_owned();
    let cached = client.get_session_state(session.id).await.unwrap();
    assert_eq!(cached.session.title, "before");
    assert_eq!(response.headers()["cache-control"], "private, no-cache");
    let unchanged = http
        .get(&endpoint)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(unchanged.status(), reqwest::StatusCode::NOT_MODIFIED);
    assert!(unchanged.bytes().await.unwrap().is_empty());
    let transcript_endpoint = format!("{}/api/v1/sessions/{}/transcript", server.url, session.id);
    let transcript = http
        .get(&transcript_endpoint)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let transcript_etag = transcript.headers()["etag"].to_str().unwrap().to_owned();
    let unchanged_transcript = http
        .get(&transcript_endpoint)
        .header("if-none-match", &transcript_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        unchanged_transcript.status(),
        reqwest::StatusCode::NOT_MODIFIED
    );
    assert!(unchanged_transcript.bytes().await.unwrap().is_empty());
    http.put(format!("{}/api/v1/sessions/{}", server.url, session.id))
        .json(&serde_json::json!({"title": "after"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let changed = tokens(&server.url, &keys).await;
    assert_eq!(
        client
            .get_session_state(session.id)
            .await
            .unwrap()
            .session
            .title,
        "after",
        "explicit reads without a live subscription must validate cached state"
    );
    assert_ne!(initial["sessions"], changed["sessions"]);
    assert_ne!(initial[&keys[2]], changed[&keys[2]]);
    assert_ne!(
        initial[&keys[5]], changed[&keys[5]],
        "transcript envelope version follows the changed session"
    );
    assert_eq!(
        initial[&keys[3]], changed[&keys[3]],
        "another workspace must retain its list revision"
    );
    let fresh = http
        .get(&endpoint)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(fresh.status(), reqwest::StatusCode::OK);
    let current_etag = fresh.headers()["etag"].to_str().unwrap().to_owned();
    let value: serde_json::Value = fresh.json().await.unwrap();
    assert_eq!(value["session"]["title"], "after");
    http.delete(format!("{}/api/v1/sessions/{}", server.url, session.id))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let removed = http
        .get(&endpoint)
        .header("if-none-match", current_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        removed.status(),
        reqwest::StatusCode::NOT_FOUND,
        "cached existence must be invalidated on deletion"
    );
}

#[tokio::test]
async fn revision_endpoint_rejects_unbounded_and_invalid_keys() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let http = reqwest::Client::new();
    for resources in [
        serde_json::json!(vec!["sessions"; 129]),
        serde_json::json!(["session:-1:state"]),
        serde_json::json!(["invalid"]),
    ] {
        let response = http
            .get(format!("{}/api/v1/changes/revisions", server.url))
            .query(&[("resources", resources.to_string())])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn directory_catalog_stats_and_global_buckets_have_independent_revisions() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let http = reqwest::Client::new();
    let second = tempfile::tempdir().unwrap();
    let workspace: serde_json::Value = http
        .post(format!("{}/api/v1/workspaces", server.url))
        .json(&serde_json::json!({"path": second.path().to_string_lossy()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_id = workspace["id"].as_i64().unwrap();
    let session = client
        .create_session(server.workspace_id, "scoped title", None)
        .await
        .unwrap();
    // Seed exact derived state from the same list read the sidebar uses.
    http.get(format!(
        "{}/api/v1/sessions?workspace_id={}&roots=true",
        server.url, server.workspace_id
    ))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let keys = vec![
        "workspaces:catalog".into(),
        format!("workspace:{}:stats", server.workspace_id),
        format!("workspace:{second_id}:stats"),
        "sessions:bucket:pinned".into(),
        "sessions:bucket:favorite".into(),
        "sessions:bucket:recent".into(),
        "sessions:bucket:running".into(),
    ];
    let initial = tokens(&server.url, &keys).await;
    let catalog = http
        .get(format!(
            "{}/api/v1/workspaces?include_session_count=false",
            server.url
        ))
        .send()
        .await
        .unwrap();
    let catalog_etag = catalog.headers()["etag"].to_str().unwrap().to_owned();
    let bucket = http
        .get(format!("{}/api/v1/sessions?bucket=favorite", server.url))
        .send()
        .await
        .unwrap();
    let bucket_etag = bucket.headers()["etag"].to_str().unwrap().to_owned();
    http.put(format!("{}/api/v1/sessions/{}", server.url, session.id))
        .json(&serde_json::json!({"title": "renamed in A"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let renamed = tokens(&server.url, &keys).await;
    for key in &keys[..5] {
        assert_eq!(initial[key], renamed[key], "title change must retain {key}");
    }
    assert_ne!(initial[&keys[5]], renamed[&keys[5]]);
    assert_eq!(
        initial[&keys[6]], renamed[&keys[6]],
        "a ready session's title does not invalidate the running bucket"
    );
    assert_eq!(
        http.get(format!(
            "{}/api/v1/workspaces?include_session_count=false",
            server.url
        ))
        .header("if-none-match", &catalog_etag)
        .send()
        .await
        .unwrap()
        .status(),
        reqwest::StatusCode::NOT_MODIFIED
    );
    assert_eq!(
        http.get(format!("{}/api/v1/sessions?bucket=favorite", server.url))
            .header("if-none-match", &bucket_etag)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_MODIFIED
    );
    http.put(format!("{}/api/v1/sessions/{}", server.url, session.id))
        .json(&serde_json::json!({"pinned": true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let pinned = tokens(&server.url, &keys).await;
    assert_eq!(renamed[&keys[0]], pinned[&keys[0]]);
    assert_ne!(renamed[&keys[1]], pinned[&keys[1]]);
    assert_eq!(
        renamed[&keys[2]], pinned[&keys[2]],
        "B stats do not change when A is pinned"
    );
    assert_ne!(renamed[&keys[3]], pinned[&keys[3]]);
    assert_eq!(renamed[&keys[4]], pinned[&keys[4]]);
    let stats: serde_json::Value = http
        .get(format!(
            "{}/api/v1/workspaces/session-stats?ids={}",
            server.url, server.workspace_id
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["items"].as_array().unwrap().len(), 1);
    assert_eq!(stats["items"][0]["workspace_id"], server.workspace_id);
    assert_eq!(stats["items"][0]["stats"]["pinned"], 1);
    assert_eq!(stats["items"][0]["revision"], pinned[&keys[1]]);
    http.delete(format!("{}/api/v1/sessions/{}", server.url, session.id))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let deleted = tokens(&server.url, &keys).await;
    assert_eq!(
        pinned[&keys[0]], deleted[&keys[0]],
        "deleting a session does not change the directory catalog"
    );
    assert_eq!(
        pinned[&keys[2]], deleted[&keys[2]],
        "deleting in A retains B's stats"
    );
    assert_eq!(
        pinned[&keys[4]], deleted[&keys[4]],
        "deleting a non-favorite retains the favorite bucket"
    );
    for invalid in ["-1".to_owned(), "bad".to_owned(), vec!["1"; 129].join(",")] {
        assert_eq!(
            http.get(format!("{}/api/v1/workspaces/session-stats", server.url))
                .query(&[("ids", invalid)])
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn a_new_server_identity_invalidates_the_same_durable_resource() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let first = AppState::from_application(application_for_test(&server.runtime));
    let restarted = AppState::from_application(application_for_test(&server.runtime));
    assert_ne!(
        first.revisions().unwrap().token("sessions"),
        restarted.revisions().unwrap().token("sessions")
    );
}

#[tokio::test]
async fn tool_section_versions_isolate_output_and_invalidate_deleted_memberships() {
    use agena_storage::store::{NewPart, PartDelta, PartRole, PartState};
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let http = reqwest::Client::new();
    let session = client
        .create_session(server.workspace_id, "tool versions", None)
        .await
        .unwrap();
    let store = application_for_test(&server.runtime)
        .session_store_facade()
        .unwrap();
    let mut tool = agena_runtime_contracts::part_content::ToolCallContent {
        name: "shell.exec".into(),
        input: serde_json::json!({"command":"echo hello"}),
        state: agena_domain::ToolResultState::Running,
        ..Default::default()
    };
    tool.set_live_output("hello");
    let content = tool.as_value();
    let run = store
        .submit_user_run(
            session.id,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending("tool_call", PartRole::Assistant, content.clone())
            }],
            None,
        )
        .await
        .unwrap();
    let part = run
        .parts
        .iter()
        .find(|part| part.kind == "tool_call")
        .unwrap();
    let endpoint = |section: &str| {
        format!(
            "{}/api/v1/sessions/{}/parts/{}/tool-sections/{section}",
            server.url, session.id, part.part_id
        )
    };
    let input = http
        .get(endpoint("input"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let input_etag = input.headers()["etag"].to_str().unwrap().to_owned();
    let output = http
        .get(endpoint("output"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let output_etag = output.headers()["etag"].to_str().unwrap().to_owned();
    let mut next = content;
    next["metadata"]["live_output"] = serde_json::json!("hello again");
    store
        .update_part(
            session.id,
            part.part_id,
            PartDelta {
                content: Some(next),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let unchanged = http
        .get(endpoint("input"))
        .header("if-none-match", &input_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        unchanged.status(),
        reqwest::StatusCode::NOT_MODIFIED,
        "new output must not reload unchanged input"
    );
    let changed = http
        .get(endpoint("output"))
        .header("if-none-match", &output_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), reqwest::StatusCode::OK);
    assert_ne!(changed.headers()["etag"].to_str().unwrap(), output_etag);
    store.delete(session.id).await.unwrap();
    let deleted = http
        .get(endpoint("input"))
        .header("if-none-match", input_etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        deleted.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a deleted membership cannot return a cached section"
    );
}

#[tokio::test]
async fn directory_tree_pages_and_closed_bucket_counts_have_independent_revisions() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let http = reqwest::Client::new();
    let a = client
        .create_session(server.workspace_id, "root A", None)
        .await
        .unwrap();
    let b = client
        .create_session(server.workspace_id, "root B", None)
        .await
        .unwrap();
    let child = client
        .create_session(server.workspace_id, "child A", Some(a.id))
        .await
        .unwrap();
    let base = format!("workspace:{}:sessions", server.workspace_id);
    let keys = vec![
        format!("{base}:roots"),
        format!("{base}:parent:{}", a.id),
        format!("{base}:parent:{}", b.id),
        format!("{base}:bucket:pinned"),
        "sessions:bucket:recent".into(),
        "sessions:bucket:recent:count".into(),
    ];
    let queries = [
        "roots=true".to_owned(),
        format!("parent_id={}", a.id),
        format!("parent_id={}", b.id),
        "bucket=pinned".to_owned(),
    ];
    let mut etags = Vec::new();
    for query in &queries {
        let response = http
            .get(format!(
                "{}/api/v1/sessions?workspace_id={}&{query}",
                server.url, server.workspace_id
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        etags.push(response.headers()["etag"].to_str().unwrap().to_owned());
    }
    let count_url = format!(
        "{}/api/v1/sessions?bucket=recent&exclude_subagents=true&count_only=true",
        server.url
    );
    let response = http
        .get(&count_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let count_etag = response.headers()["etag"].to_str().unwrap().to_owned();
    let count: serde_json::Value = response.json().await.unwrap();
    assert!(count["items"].as_array().unwrap().is_empty());
    assert_eq!(count["total"], 3);
    let before = tokens(&server.url, &keys).await;
    http.put(format!("{}/api/v1/sessions/{}", server.url, child.id))
        .json(&serde_json::json!({"title":"new child title"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let after = tokens(&server.url, &keys).await;
    assert_ne!(before[&keys[1]], after[&keys[1]]);
    assert_ne!(
        before[&keys[4]], after[&keys[4]],
        "the recent rows contain the renamed child"
    );
    for index in [0, 2, 3, 5] {
        assert_eq!(
            before[&keys[index]], after[&keys[index]],
            "child title must retain {}",
            keys[index]
        );
    }
    for index in [0, 2, 3] {
        assert_eq!(
            http.get(format!(
                "{}/api/v1/sessions?workspace_id={}&{}",
                server.url, server.workspace_id, queries[index]
            ))
            .header("if-none-match", &etags[index])
            .send()
            .await
            .unwrap()
            .status(),
            reqwest::StatusCode::NOT_MODIFIED
        );
    }
    assert_eq!(
        http.get(&count_url)
            .header("if-none-match", &count_etag)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_MODIFIED
    );
    http.delete(format!("{}/api/v1/sessions/{}", server.url, child.id))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let removed = tokens(&server.url, &keys).await;
    assert_ne!(
        after[&keys[0]], removed[&keys[0]],
        "parent's child count changes in the roots page"
    );
    assert_ne!(after[&keys[1]], removed[&keys[1]]);
    assert_eq!(after[&keys[2]], removed[&keys[2]]);
    assert_eq!(after[&keys[3]], removed[&keys[3]]);
    assert_ne!(after[&keys[5]], removed[&keys[5]]);
    let count: serde_json::Value = http
        .get(&count_url)
        .header("if-none-match", &count_etag)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(count["total"], 2);
    assert!(count["items"].as_array().unwrap().is_empty());
}
