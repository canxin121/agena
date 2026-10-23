use super::*;

#[tokio::test]
async fn binary_and_json_uploads_preserve_content_hash_and_managed_staging() {
    let (provider_url, _, _provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let http = reqwest::Client::new();
    let endpoint = format!(
        "{}/api/v1/workspaces/{}/files",
        server.url, server.workspace_id
    );
    let bytes = "binary\0中文😀\n".as_bytes().to_vec();
    let binary = http
        .post(&endpoint)
        .query(&[("filename", "../safe.txt"), ("mime", "text/plain")])
        .header("Content-Type", "application/octet-stream; charset=utf-8")
        .body(bytes.clone())
        .send()
        .await
        .unwrap();
    assert!(
        binary.status().is_success(),
        "{}",
        binary.text().await.unwrap()
    );
    let binary: serde_json::Value = binary.json().await.unwrap();
    let legacy = http
        .post(&endpoint)
        .json(&serde_json::json!({
            "filename": "legacy.txt", "mime": "text/plain",
            "data_base64": "YmluYXJ5AOS4reaWh/CfmIAK",
        }))
        .send()
        .await
        .unwrap();
    assert!(
        legacy.status().is_success(),
        "{}",
        legacy.text().await.unwrap()
    );
    let legacy: serde_json::Value = legacy.json().await.unwrap();
    assert_eq!(binary["sha256"], legacy["sha256"]);
    assert_eq!(binary["size_bytes"], bytes.len());
    assert_eq!(binary["name"], "safe.txt");
    assert_eq!(binary["mime"], "text/plain");
    for value in [&binary, &legacy] {
        let path = value["path"].as_str().unwrap();
        let path = std::path::Path::new(path);
        let upload_root =
            agena_runtime_tools::project_state_dir(server._workspace.path()).join("uploads");
        assert!(path.is_absolute());
        assert!(path.starts_with(upload_root));
        assert!(!path.starts_with(server._workspace.path()));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        let download = http
            .get(endpoint.replace("/files", "/download"))
            .query(&[("path", path.to_str().unwrap())])
            .send()
            .await
            .unwrap();
        assert!(download.status().is_success());
        assert_eq!(download.bytes().await.unwrap(), bytes);
    }
    let empty = http
        .post(&endpoint)
        .query(&[("filename", "empty.txt")])
        .header("Content-Type", "application/octet-stream")
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), reqwest::StatusCode::BAD_REQUEST);
    let missing = http
        .post(&endpoint)
        .header("Content-Type", "application/octet-stream")
        .body("body")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn managed_image_uploads_freeze_in_session_storage_and_reject_foreign_workspaces() {
    use agena_domain::{
        ActivityId, ActivityPayload, ComposerActivity, ComposerDocument, ComposerNode,
        ResourceActivity, ResourceDelivery, ResourceKind, ResourceReference,
    };
    use agena_storage::kernel::EntryPayload;

    const IMAGE_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
    let (provider_url, mut requests, provider) = spawn_fake_responses_provider(vec![
        FakeProviderPlan {
            events: terminal_response_events("uploaded image received", "resp_uploaded_image"),
            release: None,
        },
        FakeProviderPlan {
            events: terminal_response_events("frozen image retained", "resp_frozen_image"),
            release: None,
        },
    ])
    .await;
    let server = start_test_server(&provider_url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let foreign_root = tempfile::tempdir().unwrap();
    let foreign = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: foreign_root.path().to_string_lossy().into_owned(),
            create_if_missing: true,
        }))
        .await
        .unwrap();
    let CommandResult::Workspace(foreign) = foreign else {
        panic!("expected the foreign workspace");
    };
    let http = reqwest::Client::new();
    let mut uploads = Vec::new();
    for workspace_id in [server.workspace_id, foreign.id] {
        let response = http
            .post(format!(
                "{}/api/v1/workspaces/{workspace_id}/files",
                server.url
            ))
            .json(&serde_json::json!({
                "filename": "selected.png",
                "mime": "image/png",
                "data_base64": IMAGE_BASE64,
            }))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
        uploads.push(response.json::<serde_json::Value>().await.unwrap());
    }
    let selected_path = uploads[0]["path"].as_str().unwrap();
    let foreign_path = uploads[1]["path"].as_str().unwrap();
    assert!(std::path::Path::new(selected_path).is_absolute());
    assert_ne!(selected_path, foreign_path);
    let image_document = |path: &str| {
        ComposerDocument(vec![
            ComposerNode::Text {
                text: "inspect the uploaded image".into(),
            },
            ComposerNode::Activity {
                activity: Box::new(ComposerActivity {
                    id: ActivityId::new(),
                    provenance: Default::default(),
                    payload: ActivityPayload::Resource(ResourceActivity {
                        delivery: ResourceDelivery::ModelInput,
                        kind: ResourceKind::Image,
                        reference: ResourceReference::WorkspacePath { path: path.into() },
                        name: "selected.png".into(),
                        media_type: Some("image/png".into()),
                        size_bytes: None,
                        width: None,
                        height: None,
                        duration_ms: None,
                        page_count: None,
                    }),
                }),
            },
        ])
    };
    let options = RunOptions {
        model: Some(agena_api::resource::ModelRef::new_with_adapter(
            "fake",
            "openai_responses",
            "fake-model",
        )),
        ..Default::default()
    };
    let session = client
        .create_session(server.workspace_id, "managed image input", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);

    let rejected = client
        .submit_message(SubmitRunParams {
            session_id: session.id,
            options: options.clone(),
            document: image_document(foreign_path),
        })
        .await
        .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("escapes the active workspace"),
        "{rejected:?}"
    );
    assert!(
        kernel_fixture::entries(&application, session.id)
            .await
            .is_empty()
    );
    assert!(matches!(
        requests.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    client
        .submit_message(SubmitRunParams {
            session_id: session.id,
            options: options.clone(),
            document: image_document(selected_path),
        })
        .await
        .unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    let image_url = format!("data:image/png;base64,{IMAGE_BASE64}");
    let assert_original_image = |request: &serde_json::Value| {
        let images = request["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["content"].as_array())
            .flatten()
            .filter(|content| content["type"] == "input_image")
            .map(|content| content["image_url"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(images, vec![image_url.as_str()]);
    };
    assert_original_image(&first);
    wait_for_execution(&client, session.id, |execution| {
        execution.session.state.as_str() == "ready"
            && execution_text(execution).contains("uploaded image received")
    })
    .await;

    let entries = kernel_fixture::entries(&application, session.id).await;
    let input = entries
        .iter()
        .find(|entry| matches!(&entry.payload, EntryPayload::User { .. }))
        .unwrap();
    let EntryPayload::User { document } = &input.payload else {
        unreachable!()
    };
    let frozen = document
        .0
        .iter()
        .find_map(|node| match node {
            ComposerNode::Activity { activity } => match &activity.payload {
                ActivityPayload::Resource(resource) => Some(resource),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let ResourceReference::Artifact { uri, sha256 } = &frozen.reference else {
        panic!("model input must be stored as an immutable attachment");
    };
    assert!(uri.starts_with("agena:media:v1:"));
    assert_eq!(sha256, uploads[0]["sha256"].as_str().unwrap());
    let blobs = application
        .session_kernel()
        .unwrap()
        .reader
        .entry_blobs(session.id.try_into().unwrap(), input.id)
        .await
        .unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(
        blobs[0].byte_len,
        uploads[0]["size_bytes"].as_u64().unwrap()
    );

    // A later turn must hydrate the committed bytes after upload staging is gone.
    std::fs::remove_file(selected_path).unwrap();
    client
        .submit_message(SubmitRunParams {
            session_id: session.id,
            options,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "describe the same image again".into(),
            }]),
        })
        .await
        .unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert_original_image(&second);
    wait_for_execution(&client, session.id, |execution| {
        execution.session.state.as_str() == "ready"
            && execution_text(execution).contains("frozen image retained")
    })
    .await;
    provider.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn binary_upload_preserves_the_symlink_staging_guard() {
    let (provider_url, _, _provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let outside = tempfile::tempdir().unwrap();
    let managed_workspace = agena_runtime_tools::project_state_dir(server._workspace.path());
    std::fs::create_dir_all(&managed_workspace).unwrap();
    std::os::unix::fs::symlink(outside.path(), managed_workspace.join("uploads")).unwrap();
    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/workspaces/{}/files?filename=escape.txt",
            server.url, server.workspace_id
        ))
        .header("Content-Type", "application/octet-stream")
        .body("body")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}
