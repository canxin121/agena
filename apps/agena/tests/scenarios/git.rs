//! Git workbench coverage through the authenticated real Agena HTTP server.
//! Every repository is created inside this test case; no Git remote is contacted.

use super::fixture;
use agena_testkit::{GitDriver, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

fn git_fixture() -> agena_testkit::TestAgenaBuilder {
    fixture()
        .env("GIT_AUTHOR_NAME", "Agena System Test")
        .env("GIT_AUTHOR_EMAIL", "test-author@invalid.example")
        .env("GIT_COMMITTER_NAME", "Agena System Test")
        .env("GIT_COMMITTER_EMAIL", "test-committer@invalid.example")
        .env("GIT_TERMINAL_PROMPT", "0")
}

async fn initialized_repo(git: &GitDriver) -> TestResult {
    let initialized: Value = git
        .post("init", &json!({"defaultBranch": "main"}))
        .await?
        .require_success()?
        .json()?;
    ensure!(
        initialized["success"] == true,
        "Git init response: {initialized}"
    );
    ensure!(
        git.directory().join(".git").is_dir(),
        "Git init did not create real repository metadata"
    );
    Ok(())
}

async fn committed(git: &GitDriver, message: &str) -> TestResult<String> {
    git.post("stage", &json!({"scope": "all"}))
        .await?
        .require_success()?;
    let result: Value = git
        .post(
            "commit",
            &json!({
                "message": message,
                "noGpgSign": true,
            }),
        )
        .await?
        .require_success()?
        .json()?;
    let id = result["commit"]
        .as_str()
        .context("committed hash missing from real Git response")?;
    ensure!(
        result["success"] == true && id.len() >= 7,
        "commit result: {result}"
    );
    Ok(id.to_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_workbench_initializes_stages_commits_and_switches_real_branches() -> TestResult {
    git_fixture()
        .run(|case| async move {
            let git = case.git();
            let missing = git.get("status").await?;
            ensure!(
                missing.status.as_u16() == 409,
                "non-repo status was not a conflict: {missing:?}"
            );

            initialized_repo(&git).await?;
            let duplicate = git.post("init", &json!({"defaultBranch": "main"})).await?;
            ensure!(
                duplicate.status.as_u16() == 409,
                "duplicate initialization was accepted: {duplicate:?}"
            );

            case.seed_file("workspace/notes.txt", b"first version\n")
                .await?;
            let before: Value = git.get("status").await?.require_success()?.json()?;
            ensure!(
                before["untrackedCount"] == 1 && before["isClean"] == false,
                "{before}"
            );
            let missing_scope = git.post("stage", &json!({"paths": ["notes.txt"]})).await?;
            ensure!(
                missing_scope.status.as_u16() == 400,
                "stage accepted missing scope: {missing_scope:?}"
            );

            let stage: Value = git
                .post("stage", &json!({"scope":"paths", "paths":["notes.txt"]}))
                .await?
                .require_success()?
                .json()?;
            ensure!(stage["success"] == true, "{stage}");
            let staged: Value = git.get("status").await?.require_success()?.json()?;
            ensure!(
                staged["stagedCount"] == 1 && staged["untrackedCount"] == 0,
                "{staged}"
            );
            let initial_commit = committed(&git, "first commit").await?;
            let clean: Value = git.get("status").await?.require_success()?.json()?;
            ensure!(clean["isClean"] == true, "{clean}");

            let branch: Value = git
                .post("branches", &json!({"name":"feature-check"}))
                .await?
                .require_success()?
                .json()?;
            ensure!(branch["success"] == true, "{branch}");
            case.seed_file("workspace/notes.txt", b"feature version\n")
                .await?;
            let feature_commit = committed(&git, "feature commit").await?;
            ensure!(feature_commit != initial_commit);

            let branches: Value = git.get("branches").await?.require_success()?.json()?;
            ensure!(branches["current"] == "feature-check", "{branches}");
            git.post("checkout", &json!({"branch":"main"}))
                .await?
                .require_success()?;
            ensure!(
                tokio::fs::read(git.directory().join("notes.txt")).await? == b"first version\n"
            );
            let main: Value = git.get("status").await?.require_success()?.json()?;
            ensure!(
                main["current"] == "main" && main["isClean"] == true,
                "{main}"
            );

            let tag: Value = git
                .post("tags", &json!({"name":"v-test-1", "ref":initial_commit}))
                .await?
                .require_success()?
                .json()?;
            ensure!(tag["success"] == true, "{tag}");
            let tags: Value = git.get("tags").await?.require_success()?.json()?;
            ensure!(
                tags["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|tag| tag["name"] == "v-test-1")),
                "{tags}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_workbench_rejects_missing_directory_unsafe_paths_and_invalid_requests() -> TestResult {
    git_fixture()
        .run(|case| async move {
            let http = case.http().await;
            let no_dir = http.get("/api/v1/workbench/git/status").await?;
            ensure!(
                no_dir.status.as_u16() == 400,
                "missing directory was accepted: {no_dir:?}"
            );
            ensure!(no_dir.json::<Value>()?["code"] == "missing_directory");

            ensure!(case.git().in_directory("../outside").is_err());
            ensure!(case.git().in_directory("/outside").is_err());
            ensure!(
                case.git()
                    .request(reqwest::Method::GET, "../status")
                    .await
                    .is_err()
            );
            ensure!(
                case.git()
                    .request(reqwest::Method::GET, "branches//delete")
                    .await
                    .is_err()
            );

            let git = case.git();
            let unsafe_init = git
                .post(
                    "init",
                    &json!({"path":"../outside", "defaultBranch":"main"}),
                )
                .await?;
            ensure!(
                unsafe_init.status.as_u16() == 400,
                "unsafe repository path was accepted: {unsafe_init:?}"
            );
            ensure!(unsafe_init.json::<Value>()?["code"] == "invalid_path");
            ensure!(!case.paths().root.join("outside/.git").exists());
            let invalid_branch = git
                .post("init", &json!({"defaultBranch":"bad branch"}))
                .await?;
            ensure!(
                invalid_branch.status.as_u16() == 400,
                "bad initial branch name accepted: {invalid_branch:?}"
            );
            initialized_repo(&git).await?;

            let bad_status = git
                .get_with_query("status", &[("scope", "not-a-scope")])
                .await?;
            ensure!(bad_status.status.as_u16() == 400);
            ensure!(bad_status.json::<Value>()?["code"] == "git_status_scope_invalid");
            let unsafe_stage = git
                .post("stage", &json!({"scope":"paths", "paths":["../outside"]}))
                .await?;
            ensure!(
                unsafe_stage.status.is_client_error(),
                "unsafe stage was accepted: {unsafe_stage:?}"
            );
            let invalid_commit = git.post("commit", &json!({"message":" "})).await?;
            ensure!(invalid_commit.status.as_u16() == 400);
            ensure!(invalid_commit.json::<Value>()?["code"] == "missing_message");

            let unauthorized = case
                .http()
                .await
                .anonymous()
                .request(reqwest::Method::GET, "/api/v1/workbench/git/status")?
                .query(&[("directory", git.directory().to_string_lossy().as_ref())]);
            let unauthorized = case.http().await.anonymous().execute(unauthorized).await?;
            ensure!(
                unauthorized.status.as_u16() == 401,
                "Git route accepted unauthenticated access: {unauthorized:?}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_workbench_status_snapshot_pages_remain_stable_until_explicit_refresh() -> TestResult {
    git_fixture()
        .run(|case| async move {
            let git = case.git().in_directory("workspace/pagination-repo")?;
            case.seed_file("workspace/pagination-repo/seed.txt", b"initial\n")
                .await?;
            initialized_repo(&git).await?;
            committed(&git, "initial repository").await?;
            for n in 0..7 {
                case.seed_file(
                    format!("workspace/pagination-repo/f-{n:02}.txt"),
                    b"untracked\n",
                )
                .await?;
            }

            let first: Value = git
                .get_with_query("status", &[("scope", "untracked"), ("limit", "2")])
                .await?
                .require_success()?
                .json()?;
            ensure!(
                first["totalFiles"] == 7 && first["untrackedCount"] == 7,
                "{first}"
            );
            ensure!(
                first["files"]
                    .as_array()
                    .is_some_and(|files| files.len() == 2),
                "{first}"
            );
            ensure!(first["hasMore"] == true, "{first}");
            let snapshot = first["snapshotId"]
                .as_str()
                .context("Git status did not provide a snapshot ID")?
                .to_owned();
            let second: Value = git
                .get_with_query(
                    "status",
                    &[
                        ("scope", "untracked"),
                        ("limit", "2"),
                        ("offset", "2"),
                        ("snapshotId", &snapshot),
                    ],
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                second["snapshotId"] == snapshot && second["totalFiles"] == 7,
                "{second}"
            );
            let first_paths = first["files"].as_array().context("first page files")?;
            let second_paths = second["files"].as_array().context("second page files")?;
            ensure!(
                first_paths
                    .iter()
                    .all(|a| second_paths.iter().all(|b| a["path"] != b["path"])),
                "pages overlap"
            );

            // A continuation uses the original status snapshot even if a new file
            // appears in the real repository between two HTTP requests.
            case.seed_file("workspace/pagination-repo/f-99.txt", b"later\n")
                .await?;
            let continuation: Value = git
                .get_with_query(
                    "status",
                    &[
                        ("scope", "untracked"),
                        ("limit", "2"),
                        ("offset", "4"),
                        ("snapshotId", &snapshot),
                    ],
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                continuation["totalFiles"] == 7 && continuation["snapshotId"] == snapshot,
                "{continuation}"
            );

            let fresh: Value = git
                .get_with_query("status", &[("scope", "untracked"), ("summary", "true")])
                .await?
                .require_success()?
                .json()?;
            ensure!(
                fresh["totalFiles"] == 8 && fresh["untrackedCount"] == 8,
                "{fresh}"
            );
            let batch: Value = git
                .get_with_query("status", &[("scopes", "untracked,staged"), ("limit", "2")])
                .await?
                .require_success()?
                .json()?;
            ensure!(
                batch["scopePages"]["untracked"]["totalFiles"] == 8,
                "{batch}"
            );
            ensure!(batch["scopePages"]["staged"]["totalFiles"] == 0, "{batch}");
            let invalid_batch = git
                .get_with_query("status", &[("scopes", "untracked,invalid")])
                .await?;
            ensure!(invalid_batch.status.as_u16() == 400);
            ensure!(invalid_batch.json::<Value>()?["code"] == "git_status_scope_invalid");
            Ok(())
        })
        .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_workbench_init_refuses_symlink_escape_before_creating_git_metadata() -> TestResult {
    use std::os::unix::fs::symlink;

    git_fixture()
        .run(|case| async move {
            let outside = case.paths().root.join("other-workspace");
            tokio::fs::create_dir_all(&outside).await?;
            let alias = case.workspace().join("alias");
            symlink(&outside, &alias)?;

            ensure!(
                case.git().in_directory("workspace/alias").is_err(),
                "testkit accepted an existing symlink ancestor as fixture-owned"
            );
            let response = case
                .git()
                .post("init", &json!({"path":"alias", "defaultBranch":"main"}))
                .await?;
            ensure!(
                response.status.as_u16() == 400,
                "Git init followed the symlink: {response:?}"
            );
            ensure!(response.json::<Value>()?["code"] == "invalid_path");
            ensure!(
                !outside.join(".git").exists(),
                "Git metadata escaped into another workspace"
            );
            // Cloning into a nested path has the same path traversal risk as
            // git init. It must reject the existing alias before running git.
            let clone = case
                .git()
                .post(
                    "clone",
                    &json!({
                        "url": case.paths().root.join("unused-local-remote.git"),
                        "path": "alias/cloned-repository",
                    }),
                )
                .await?;
            ensure!(
                clone.status.as_u16() == 400,
                "Git clone traversed a symlink: {clone:?}"
            );
            ensure!(clone.json::<Value>()?["code"] == "invalid_path");
            ensure!(!outside.join("cloned-repository").exists());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_workbench_clones_pushes_fetches_and_pulls_through_a_local_bare_remote() -> TestResult {
    use tokio::process::Command;

    git_fixture()
        .run(|case| async move {
            let source = case.git().in_directory("workspace/source")?;
            case.seed_file("workspace/source/file.txt", b"original from source\n")
                .await?;
            initialized_repo(&source).await?;
            let original_commit = committed(&source, "initial commit").await?;

            // Only fixture preparation uses git directly: the bare remote is a
            // case-owned local directory, not an external Git server. Every tested
            // push, clone, fetch and pull goes through Agena's real REST handlers.
            let remote = case.paths().root.join("offline-bare-origin.git");
            let created = Command::new("git")
                .arg("init")
                .arg("--bare")
                .arg("--initial-branch=main")
                .arg(&remote)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .await?;
            ensure!(
                created.status.success(),
                "unable to prepare local Git fixture: {}",
                String::from_utf8_lossy(&created.stderr)
            );

            let add: Value = source
                .post(
                    "remotes",
                    &json!({
                        "name":"origin", "url":remote,
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(add["success"] == true, "{add}");
            let push: Value = source
                .post(
                    "push",
                    &json!({
                        "remote":"origin", "branch":"main", "setUpstream":true,
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(push["success"] == true, "{push}");

            let cloned_one: Value = case
                .git()
                .post(
                    "clone",
                    &json!({
                        "url":remote, "path":"clone-one",
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            let cloned_two: Value = case
                .git()
                .post(
                    "clone",
                    &json!({
                        "url":remote, "path":"clone-two",
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(cloned_one["success"] == true && cloned_two["success"] == true);
            let first = case.git().in_directory("workspace/clone-one")?;
            let second = case.git().in_directory("workspace/clone-two")?;
            ensure!(
                tokio::fs::read(first.directory().join("file.txt")).await?
                    == b"original from source\n"
            );
            ensure!(
                tokio::fs::read(second.directory().join("file.txt")).await?
                    == b"original from source\n"
            );

            // A request for a force push must be denied by the actual policy;
            // neither the filesystem fixture nor the remote should be changed.
            let denied = first
                .post(
                    "push",
                    &json!({
                        "remote":"origin", "branch":"main", "force":"force",
                    }),
                )
                .await?;
            ensure!(
                denied.status.as_u16() == 403,
                "force push policy was bypassed: {denied:?}"
            );
            ensure!(denied.json::<Value>()?["code"] == "git_force_push_not_allowed");

            case.seed_file(
                "workspace/clone-one/file.txt",
                b"new version from clone one\n",
            )
            .await?;
            let changed_commit = committed(&first, "publish new version").await?;
            ensure!(changed_commit != original_commit);
            let repushed: Value = first
                .post(
                    "push",
                    &json!({
                        "remote":"origin", "branch":"main",
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(repushed["success"] == true, "{repushed}");

            let fetch: Value = second
                .post(
                    "fetch",
                    &json!({
                        "remote":"origin", "branch":"main",
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(fetch["success"] == true, "{fetch}");
            let pulled: Value = second
                .post(
                    "pull",
                    &json!({
                        "remote":"origin", "branch":"main",
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(pulled["success"] == true, "{pulled}");
            ensure!(
                tokio::fs::read(second.directory().join("file.txt")).await?
                    == b"new version from clone one\n",
                "local clone never received published bytes"
            );
            let clean: Value = second.get("status").await?.require_success()?.json()?;
            ensure!(
                clean["isClean"] == true && clean["current"] == "main",
                "{clean}"
            );

            let invalid = second
                .post("fetch", &json!({"all":true, "remote":"origin"}))
                .await?;
            ensure!(invalid.status.as_u16() == 400);
            ensure!(invalid.json::<Value>()?["code"] == "invalid_fetch_args");
            Ok(())
        })
        .await
}
