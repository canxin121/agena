use super::*;

fn spec(name: &str, extensions: &[&str]) -> LspServerSpec {
    LspServerSpec {
        name: name.into(),
        command: "python3".into(),
        args: vec![],
        env: Default::default(),
        file_extensions: extensions.iter().map(|s| (*s).into()).collect(),
        root_markers: vec!["project.marker".into()],
        initialization_options: None,
    }
}

#[tokio::test]
async fn routing_prefers_extensions_then_lexical_server_name() {
    let registry = LspRegistry::new(PathBuf::from("/workspace"), "test", "1");
    for server in [
        spec("z-rust", &["RS"]),
        spec("a-any", &[]),
        spec("b-rust", &[".rs"]),
    ] {
        registry.register(server).await;
    }
    assert_eq!(registry.server_names().await, ["a-any", "b-rust", "z-rust"]);
    assert_eq!(
        registry
            .server_for_path(Path::new("file.rs"))
            .await
            .unwrap()
            .name,
        "b-rust"
    );
    assert_eq!(
        registry
            .server_for_path(Path::new("file.PY"))
            .await
            .unwrap()
            .name,
        "a-any"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn availability_uses_configured_path_without_running_the_command() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("custom-lsp");
    std::fs::write(&executable, "#!/bin/sh\nexit 98\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o644)).unwrap();
    let registry = LspRegistry::new(root.path().to_owned(), "test", "1");
    let mut server = spec("custom", &["rs"]);
    server.command = "custom-lsp".into();
    server
        .env
        .insert("PATH".into(), root.path().display().to_string());
    registry.register(server).await;
    assert_eq!(
        registry.server_statuses().await.unwrap()[0].command_available,
        Some(false)
    );
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let status = registry.server_statuses().await.unwrap().remove(0);
    assert_eq!(status.command_available, Some(true));
    assert_eq!(status.executable, Some(executable));
    assert!(status.running_roots.is_empty());
    assert!(registry.spawned.read().await.is_empty());
}

// These integration checks use a tiny Python stdio peer, just like the
// repository's existing Python verification gates. No Agena service is started.
#[cfg(unix)]
#[tokio::test]
async fn clients_are_root_scoped_single_flight_and_replaced_with_configuration() {
    let workspace = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    for project in ["one", "two"] {
        std::fs::create_dir(root.join(project)).unwrap();
        std::fs::write(root.join(project).join("project.marker"), "").unwrap();
    }
    let log = root.join("starts.jsonl");
    let fixture = root.join("lsp_peer.py");
    std::fs::write(&fixture, include_str!("fixture.py")).unwrap();
    let mut server = spec("fixture", &["rs"]);
    server.args = vec![fixture.display().to_string()];
    server
        .env
        .insert("AGENA_LSP_FIXTURE_LOG".into(), log.display().to_string());
    let registry = Arc::new(LspRegistry::new(root.clone(), "test", "1"));
    registry.register(server.clone()).await;

    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let registry = registry.clone();
        let file = root.join("one/file.rs");
        calls.spawn(async move { registry.client_for_path(&file).await.unwrap() });
    }
    let first = calls.join_next().await.unwrap().unwrap();
    while let Some(client) = calls.join_next().await {
        assert!(Arc::ptr_eq(&first, &client.unwrap()));
    }
    let second = registry
        .client_for_path(&root.join("two/file.rs"))
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    // Canonical root aliases must share the same process.
    std::os::unix::fs::symlink(root.join("one"), root.join("alias")).unwrap();
    let alias = registry
        .client_for_path(&root.join("alias/file.rs"))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first, &alias));
    let starts = || {
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>()
    };
    let initial = starts();
    assert_eq!(initial.len(), 2);
    for (row, project) in initial.iter().zip(["one", "two"]) {
        assert_eq!(row["cwd"], root.join(project).display().to_string());
        assert_eq!(
            row["root"],
            url::Url::from_directory_path(root.join(project))
                .unwrap()
                .as_str()
        );
    }
    registry.register(server.clone()).await;
    assert!(Arc::ptr_eq(
        &first,
        &registry
            .client_for_path(&root.join("one/file.rs"))
            .await
            .unwrap()
    ));
    server.initialization_options = Some(serde_json::json!({"newConfiguration":true}));
    registry.register(server).await;
    assert!(registry.spawned.read().await.is_empty());
    let replaced = registry
        .client_for_path(&root.join("one/file.rs"))
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &replaced));
    assert_eq!(starts().len(), 3);
    registry.shutdown_all().await;
    assert!(registry.spawned.read().await.is_empty());
    // Kept Arc handles must not keep the replaced/shut-down processes alive.
    for row in starts() {
        let status = std::process::Command::new("/bin/kill")
            .args(["-0", &row["pid"].to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "fixture child survived shutdown");
    }
}
