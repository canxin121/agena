use std::path::Path;

use agena_git_http::{
    DirectoryQuery, GitBranchesQuery, GitReposQuery, GitWorktreeAddBody, git_branches, git_repos,
    git_worktree_add, git_worktrees,
};
use axum::{Json, extract::Query, response::Response};
use serde_json::{Value, json};

fn repository(path: &Path) -> git2::Repository {
    let repo = git2::Repository::init(path).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Navigation test", "test@example.invalid").unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "Initial commit",
        &tree,
        &[],
    )
    .unwrap();
    drop(tree);
    repo
}

async fn body(response: Response) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{status}: {value}");
    value
}

#[tokio::test]
async fn current_repository_lookup_does_not_wait_for_nested_discovery_and_resolves_subfolders() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    let nested = temp.path().join("vendor/deep/nested");
    repository(&nested);
    let child = temp.path().join("src/components");
    std::fs::create_dir_all(&child).unwrap();
    for directory in [temp.path(), child.as_path()] {
        let query: GitReposQuery =
            serde_json::from_value(json!({"directory": directory, "scope": "current"})).unwrap();
        let result = body(git_repos(Query(query)).await).await;
        assert_eq!(result["repos"].as_array().unwrap().len(), 1);
        assert_eq!(result["repos"][0]["relative"], ".");
        assert_eq!(
            Path::new(result["repos"][0]["root"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }
    let query: GitReposQuery =
        serde_json::from_value(json!({"directory": temp.path(), "scope": "all"})).unwrap();
    let result = body(git_repos(Query(query)).await).await;
    assert!(
        result["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|repo| repo["relative"] == "vendor/deep/nested")
    );
}

#[tokio::test]
async fn branch_listing_is_local_and_includes_arbitrary_remotes_and_checked_out_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main");
    let repo = repository(&root);
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feature", &head, false).unwrap();
    repo.branch("origin/local-name", &head, false).unwrap();
    repo.reference("refs/remotes/company/review", head.id(), true, "test")
        .unwrap();
    repo.reference_symbolic(
        "refs/remotes/company/HEAD",
        "refs/remotes/company/review",
        true,
        "test",
    )
    .unwrap();
    repo.remote("origin", "ssh://git@example.invalid/repo")
        .unwrap();
    // Any unexpected ls-remote/fetch attempts fail immediately and leave proof.
    let marker = temp.path().join("network-was-used");
    let ssh = format!("sh -c 'touch {}; exit 1' --", marker.display());
    repo.config()
        .unwrap()
        .set_str("core.sshCommand", &ssh)
        .unwrap();
    let linked = temp.path().join("linked worktree");
    let created = body(
        git_worktree_add(
            Query(DirectoryQuery {
                directory: Some(root.to_string_lossy().into_owned()),
            }),
            Json(GitWorktreeAddBody {
                path: Some(linked.to_string_lossy().into_owned()),
                branch: Some("feature".into()),
                start_point: None,
                create_branch: false,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(created["success"], true);
    let query: GitBranchesQuery = serde_json::from_value(json!({"directory": root})).unwrap();
    let result = body(git_branches(Query(query)).await).await;
    assert!(
        !marker.exists(),
        "listing branches must never contact origin"
    );
    assert!(result["branches"].get("remotes/company/review").is_some());
    assert!(result["branches"].get("origin/local-name").is_some());
    assert!(result["branches"].get("remotes/company/HEAD").is_none());
    assert_eq!(
        Path::new(
            result["branches"]["feature"]["worktreePath"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap(),
        linked.canonicalize().unwrap()
    );
    assert!(
        result["all"]
            .as_array()
            .unwrap()
            .iter()
            .all(|name| !name.as_str().unwrap().starts_with('+'))
    );
    let query: GitBranchesQuery = serde_json::from_value(
        json!({"directory": root, "page": 1, "pageSize": 1, "search": "company"}),
    )
    .unwrap();
    let page = body(git_branches(Query(query)).await).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["all"][0], "remotes/company/review");
    let query: GitBranchesQuery =
        serde_json::from_value(json!({"directory": root, "localOnly": true})).unwrap();
    let local = body(git_branches(Query(query)).await).await;
    assert!(local["branches"].get("remotes/company/review").is_none());
    assert_eq!(
        local["all"].as_array().unwrap().len(),
        local["branches"].as_object().unwrap().len()
    );
}

#[tokio::test]
async fn sibling_worktrees_can_be_created_listed_and_resolved_independently() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main");
    repository(&root);
    let linked = temp.path().join("feature 中文 space");
    body(
        git_worktree_add(
            Query(DirectoryQuery {
                directory: Some(root.to_string_lossy().into_owned()),
            }),
            Json(GitWorktreeAddBody {
                path: Some(linked.to_string_lossy().into_owned()),
                branch: Some("feature/new".into()),
                start_point: Some("HEAD".into()),
                create_branch: true,
            }),
        )
        .await,
    )
    .await;
    let result = body(
        git_worktrees(Query(DirectoryQuery {
            directory: Some(linked.to_string_lossy().into_owned()),
        }))
        .await,
    )
    .await;
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert!(result.as_array().unwrap().iter().any(|wt| {
        wt["branch"] == "feature/new"
            && Path::new(wt["worktree"].as_str().unwrap())
                .canonicalize()
                .unwrap()
                == linked.canonicalize().unwrap()
    }));
    let query: GitReposQuery =
        serde_json::from_value(json!({"directory": linked, "scope": "current"})).unwrap();
    let found = body(git_repos(Query(query)).await).await;
    assert_eq!(found["repos"][0]["kind"], "worktree");

    let relative = body(
        git_worktree_add(
            Query(DirectoryQuery {
                directory: Some(root.to_string_lossy().into_owned()),
            }),
            Json(GitWorktreeAddBody {
                path: Some("../relative worktree".into()),
                branch: Some("relative-feature".into()),
                start_point: Some("HEAD".into()),
                create_branch: true,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(
        Path::new(relative["path"].as_str().unwrap()),
        temp.path()
            .join("relative worktree")
            .canonicalize()
            .unwrap()
    );
}
