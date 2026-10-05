use super::*;

#[tokio::test]
async fn workspace_git_reads_are_compatible_with_the_rust_client_in_plain_and_git_directories() {
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, super::super::workspace_git_reads_router::<()>())
            .await
            .unwrap();
    });
    let client = agena_client::AgenaClient::new(format!("http://{address}")).unwrap();
    let root = directory.path().to_str().unwrap();
    for result in [
        client.workspace_git_status(root, 0, true).await,
        client
            .workspace_git_diff(root, "file.txt", false, 1024, None)
            .await,
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.problem().unwrap().code.as_str(), "not_git_repo");
        assert!(!error.to_string().contains("shared API envelope"));
    }
    assert!(
        tokio::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(directory.path())
            .status()
            .await
            .unwrap()
            .success()
    );
    assert_eq!(
        client.workspace_git_status(root, 0, true).await.unwrap()["totalFiles"],
        0
    );
    std::fs::write(
        directory.path().join("file.txt"),
        "readable workspace change\n",
    )
    .unwrap();
    let status = client.workspace_git_status(root, 0, false).await.unwrap();
    assert_eq!(status["totalFiles"], 1);
    assert_eq!(status["files"][0]["path"], "file.txt");
    let diff = client
        .workspace_git_diff(root, "file.txt", false, 1024, None)
        .await
        .unwrap();
    assert!(
        diff["diff"]
            .as_str()
            .unwrap()
            .contains("+readable workspace change")
    );
    server.abort();
}

#[tokio::test]
async fn workspace_git_query_rejections_use_the_shared_envelope() {
    for path in [
        "/api/v1/workbench/git/status?directory=/tmp&limit=invalid",
        "/api/v1/workbench/git/status",
        "/api/v1/workbench/git/diff?directory=/tmp&path=file&maxBytes=invalid",
        "/api/v1/workbench/git/diff?directory=/tmp&path=../unsafe",
    ] {
        let response = super::super::workspace_git_reads_router::<()>()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: agena_api::error::ApiError = serde_json::from_slice(&bytes).unwrap();
        assert!(!error.to_string().is_empty());
    }
}
