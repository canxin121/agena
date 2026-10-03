use super::*;

fn watch_request(root: &Path, paths: &[String]) -> Request<Body> {
    Request::builder()
        .uri(format!(
            "/api/v1/workbench/fs/stream?directory={}&paths={}",
            urlencoding::encode(root.to_str().unwrap()),
            urlencoding::encode(&serde_json::to_string(paths).unwrap())
        ))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn fs_watch_stream_reports_external_changes_and_rejects_unbounded_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let response = crate::server::app::fs_router::<()>()
        .oneshot(watch_request(&root, &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let ready = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&ready).contains("fs.ready"));
    let file = root.join("external.txt");
    for action in ["create", "write", "remove"] {
        if action == "remove" {
            std::fs::remove_file(&file).unwrap();
        } else {
            std::fs::write(&file, action).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let chunk = stream.next().await.unwrap().unwrap();
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("agena:fs-changed") && text.contains("external.txt") {
                    break;
                }
            }
        })
        .await
        .expect("OS changes arrive through the HTTP stream");
    }
    drop(stream);
    for paths in [
        vec!["/outside".into()],
        vec![format!("{}/../escape", root.display())],
        vec![root.to_str().unwrap().into(); 129],
    ] {
        let response = crate::server::app::fs_router::<()>()
            .oneshot(watch_request(&root, &paths))
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
}

#[tokio::test]
async fn replacing_a_watched_directory_closes_the_stream_so_clients_can_reestablish_watches() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let expanded = root.join("expanded");
    std::fs::create_dir(&expanded).unwrap();
    let response = crate::server::app::fs_router::<()>()
        .oneshot(watch_request(&root, &[expanded.to_str().unwrap().into()]))
        .await
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    stream.next().await.unwrap().unwrap();
    std::fs::remove_dir(&expanded).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut saw_removal = false;
        while let Some(chunk) = stream.next().await {
            saw_removal |= String::from_utf8_lossy(&chunk.unwrap()).contains("expanded");
        }
        assert!(saw_removal);
    })
    .await
    .expect("replacement must end the old watch stream");
    std::fs::create_dir(&expanded).unwrap();
    let response = crate::server::app::fs_router::<()>()
        .oneshot(watch_request(&root, &[expanded.to_str().unwrap().into()]))
        .await
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    stream.next().await.unwrap().unwrap();
    std::fs::write(expanded.join("fresh.txt"), "new inode").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let chunk = stream.next().await.unwrap().unwrap();
            if String::from_utf8_lossy(&chunk).contains("fresh.txt") {
                break;
            }
        }
    })
    .await
    .expect("the reconnected stream watches the replacement directory");
}

#[cfg(unix)]
#[tokio::test]
async fn fs_watch_preserves_the_path_alias_used_by_the_file_pane() {
    let temp = tempfile::tempdir().unwrap();
    let physical = temp.path().join("physical");
    let alias = temp.path().join("alias");
    std::fs::create_dir(&physical).unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let response = crate::server::app::fs_router::<()>()
        .oneshot(watch_request(&alias, &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    stream.next().await.unwrap().unwrap();
    std::fs::write(physical.join("changed.txt"), "external write").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let chunk = stream.next().await.unwrap().unwrap();
            let text = String::from_utf8_lossy(&chunk);
            if text.contains("changed.txt") {
                assert!(
                    text.contains(&format!("{}/changed.txt", alias.display())),
                    "{text}"
                );
                break;
            }
        }
    })
    .await
    .expect("aliased workspace event");
}
