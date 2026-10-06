use super::*;

#[tokio::test]
async fn binary_and_legacy_uploads_preserve_content_hash_and_workspace_staging() {
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
        assert!(path.starts_with(".agena/uploads/"));
        assert_eq!(
            std::fs::read(server._workspace.path().join(path)).unwrap(),
            bytes
        );
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

#[cfg(unix)]
#[tokio::test]
async fn binary_upload_preserves_the_symlink_staging_guard() {
    let (provider_url, _, _provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        server._workspace.path().join(".agena/uploads"),
    )
    .unwrap();
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
