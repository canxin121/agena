//! Permissions exercised through the real user → model → plugin → host path.
//! Each refusal is checked in the model-facing failure and in independent effects.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use agena_failure::{Failure, ModelFeedbackKind};
use agena_storage_layout::StorageLayout;
use agena_testkit::{SessionOptions, TestCase, TestResult, ToolResult, UserSession};
use anyhow::{Result, ensure};
use axum::{Router, routing::get};
use serde_json::{Value, json};

use super::{call, fixture, string, user};

async fn user_in(case: &TestCase, workspace: &Path) -> Result<UserSession> {
    tokio::fs::create_dir_all(workspace).await?;
    let user = case
        .user()
        .new_session_in_with(
            workspace,
            SessionOptions {
                title: "Invoking workspace permission regression".into(),
                ..Default::default()
            },
        )
        .await?;
    super::allow_permissions(&user).await?;
    Ok(user)
}

pub(super) fn failure(result: &ToolResult, kind: ModelFeedbackKind) -> Result<Failure> {
    result.assert_failure_kind(kind)?;
    result.failure()
}

async fn path_rules(user: &UserSession, rules: &[(PathBuf, &str, &str)]) -> Result<()> {
    // Preserve the explicitly allowed tool names; only the concrete effect is denied.
    let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
    let mut paths = serde_json::Map::new();
    for (path, read, write) in rules {
        paths.insert(
            path.to_string_lossy().into_owned(),
            json!({"read":read,"write":write}),
        );
    }
    policy["path"]["rules"] = Value::Object(paths);
    user.client()
        .set_session_permission(user.id(), serde_json::from_value(policy)?)
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn filesystem_and_notebook_denials_precede_io_and_leave_no_side_effects() -> TestResult {
    fixture()
        .expect_plugin("agena.fs")
        .expect_plugin("agena.notebook")
        .run(|case| async move {
            let user = user(&case).await?;
            path_rules(
                &user,
                &[
                    (case.workspace().join("blocked/child.txt"), "allow", "deny"),
                    (case.workspace().join("missing.txt"), "deny", "deny"),
                    (case.workspace().join("other.txt"), "deny", "deny"),
                    (case.workspace().join("missing.pdf"), "deny", "deny"),
                    (case.workspace().join("missing.ipynb"), "allow", "deny"),
                ],
            )
            .await?;

            let write = user.call_tool(
                "fs.write",
                json!({"path":"blocked/child.txt","content":"forbidden-write","create_parents":true}),
            ).await?;
            failure(&write, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!case.workspace().join("blocked").exists());

            // These files do not exist. A metadata/conversion/notebook error would
            // prove that the product accessed them before checking permission.
            for (tool, input) in [
                ("fs.stat", json!({"path":"missing.txt"})),
                ("fs.read_many", json!({"paths":["missing.txt","other.txt"]})),
                ("fs.document", json!({"path":"missing.pdf"})),
                ("notebook.edit_cell", json!({
                    "path":"missing.ipynb","action":"delete","cell_index":0,"expected_sha256":"abc"
                })),
            ] {
                failure(&user.call_tool(tool, input).await?, ModelFeedbackKind::PermissionDenied)?;
            }
            for path in ["missing.txt", "other.txt", "missing.pdf", "missing.ipynb"] {
                ensure!(!case.workspace().join(path).exists());
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structural_search_checks_children_and_preview_respects_write_denial() -> TestResult {
    const SOURCE: &str = "fn main() { let x = 1; }\n";
    fixture()
        .expect_plugin("agena.code")
        .seed_file(
            "workspace/search/secret.rs",
            b"fn private_child_marker() {}\n".to_vec(),
        )
        .seed_file("workspace/source.rs", SOURCE.as_bytes().to_vec())
        .run(|case| async move {
            let user = user(&case).await?;
            path_rules(
                &user,
                &[
                    (case.workspace().join("search/secret.rs"), "deny", "deny"),
                    (case.workspace().join("source.rs"), "allow", "deny"),
                ],
            )
            .await?;
            let search = user
                .call_tool(
                    "code.search_ast",
                    json!({"path":"search","language":"rust","pattern":"fn $NAME() { $$$ }"}),
                )
                .await?;
            failure(&search, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!search.text().contains("private_child_marker"));

            let mut request = json!({
                "path":"source.rs","pattern":"let $A = $B;","replacement":"let $A = 2;"
            });
            let preview = call(&user, "code.rewrite_ast", request.clone()).await?;
            let revision = string(preview.data()?, "/before_sha256")?;
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("source.rs")).await?,
                SOURCE
            );
            request["apply"] = json!(true);
            request["expected_sha256"] = json!(revision);
            let denied = user.call_tool("code.rewrite_ast", request).await?;
            failure(&denied, ModelFeedbackKind::PermissionDenied)?;
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("source.rs")).await?,
                SOURCE
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_record_and_index_denials_use_the_invoking_workspace() -> TestResult {
    fixture()
        .expect_plugin("agena.memory")
        .configure_global(|config| {
            config["plugins"]["list"]["agena.memory"] = json!({
                "package":{"kind":"static"},
                "settings":{"retrieval":{"enabled":false}}
            });
        })
        .run(|case| async move {
            let workspace = case.workspace().join("invoking-workspace");
            let user = user_in(&case, &workspace).await?;
            let layout = StorageLayout::new(case.paths().data_dir.clone());
            let memory = layout.memory_dir(&workspace);
            let initial_memory = layout.memory_dir(case.workspace());
            assert_ne!(memory, initial_memory);
            call(
                &user, "memory.write",
                json!({"name":"permission-seed","content":"existing-memory-marker","memory_type":"project"}),
            ).await?;
            // The product was started in case.workspace(), but this session
            // belongs to a distinct workspace resolved through the public API.
            ensure!(memory.join("permission-seed.md").is_file());
            ensure!(!initial_memory.join("permission-seed.md").exists());
            let index = memory.join("MEMORY.md");
            let before = tokio::fs::read(&index).await?;
            let seed = tokio::fs::read(memory.join("permission-seed.md")).await?;

            path_rules(&user, &[(memory.join("denied-record.md"), "allow", "deny")]).await?;
            let record = user.call_tool(
                "memory.write",
                json!({"name":"denied-record","content":"forbidden-record-marker"}),
            ).await?;
            failure(&record, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!memory.join("denied-record.md").exists());
            assert_eq!(tokio::fs::read(&index).await?, before);

            // The record path is writable again. Refusing just its shared index
            // must still happen before any new record is created.
            path_rules(&user, &[(index.clone(), "allow", "deny")]).await?;
            let index_denied = user.call_tool(
                "memory.write",
                json!({"name":"denied-index","content":"forbidden-index-marker"}),
            ).await?;
            failure(&index_denied, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!memory.join("denied-index.md").exists());
            assert_eq!(tokio::fs::read(&index).await?, before);
            assert_eq!(tokio::fs::read(memory.join("permission-seed.md")).await?, seed);
            // A mistakenly initialized/global store must not bypass the exact
            // workspace rules above and leave a second successful record.
            assert_no_named_file(&case.paths().data_dir, "denied-record.md").await?;
            assert_no_named_file(&case.paths().data_dir, "denied-index.md").await?;
            Ok(())
        })
        .await
}

async fn assert_no_named_file(root: &Path, name: &str) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            ensure!(
                entry.file_name() != name,
                "denied file appeared at {}",
                entry.path().display()
            );
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_source_denials_precede_access_and_create_denial_uses_workspace() -> TestResult {
    const DOCUMENT: &str = "---\nname: denied-create\ndescription: Permission fixture.\n---\nNever create this package.\n";
    fixture()
        .expect_plugin("agena.skills")
        .run(|case| async move {
            let workspace = case.workspace().join("invoking-workspace");
            let user = user_in(&case, &workspace).await?;
            let library = workspace.join(".agena/skills");
            path_rules(
                &user,
                &[
                    (library.clone(), "allow", "deny"),
                    (workspace.join("missing-package"), "deny", "deny"),
                    (workspace.join("missing.zip"), "deny", "deny"),
                ],
            )
            .await?;
            let created = user
                .call_tool(
                    "skills.create",
                    json!({"scope":"workspace","document":DOCUMENT}),
                )
                .await?;
            failure(&created, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!library.exists());
            ensure!(!case.workspace().join(".agena/skills").exists());
            ensure!(!case.paths().data_dir.join(".agena/skills").exists());

            for tool in ["skills.preview_install", "skills.install"] {
                for source in [
                    json!({"kind":"folder","path":"missing-package"}),
                    json!({"kind":"archive","path":"missing.zip"}),
                ] {
                    let result = user
                        .call_tool(tool, json!({"scope":"workspace","source":source}))
                        .await?;
                    failure(&result, ModelFeedbackKind::PermissionDenied)?;
                    ensure!(!library.exists());
                }
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_install_preview_is_read_only_and_denied_install_preserves_source() -> TestResult {
    const DOCUMENT: &str = "---\nname: install-check\ndescription: Keep the complete package together.\n---\nInstall source marker.\n";
    fixture()
        .expect_plugin("agena.skills")
        .seed_file(
            "workspace/source-package/SKILL.md",
            DOCUMENT.as_bytes().to_vec(),
        )
        .seed_file(
            "workspace/source-package/reference.txt",
            b"original-resource-marker".to_vec(),
        )
        .run(|case| async move {
            let user = user(&case).await?;
            let library = case.workspace().join(".agena/skills");
            path_rules(&user, &[(library.clone(), "allow", "deny")]).await?;
            let mut request = json!({
                "scope":"workspace","source":{"kind":"folder","path":"source-package"}
            });
            let preview = call(&user, "skills.preview_install", request.clone()).await?;
            assert_eq!(preview.data()?["candidates"][0]["name"], "install-check");
            assert_eq!(preview.data()?["candidates"][0]["valid"], true);
            ensure!(
                !library.exists(),
                "read-only preview created its destination"
            );
            request["expected_source_revision"] = preview.data()?["revision"].clone();
            let install = user.call_tool("skills.install", request).await?;
            failure(&install, ModelFeedbackKind::PermissionDenied)?;
            ensure!(!library.exists());
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("source-package/SKILL.md")).await?,
                DOCUMENT,
            );
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("source-package/reference.txt"))
                    .await?,
                "original-resource-marker",
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_downloads_wait_for_permission_and_preserve_user_declines() -> TestResult {
    fixture()
        .expect_plugin("agena.skills")
        .run(|case| async move {
            let requests = Arc::new(AtomicUsize::new(0));
            let seen = requests.clone();
            let service = case.serve_http(Router::new().route(
                "/skills.zip",
                get(move || {
                    seen.fetch_add(1, Ordering::SeqCst);
                    async { "permission must be resolved before this handler" }
                }),
            )).await?;
            let archive_url = service.url("/skills.zip");
            let user = user(&case).await?;
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["network"] = json!({"loopback":"ask","internet":"ask","private":"deny"});
            user.client()
                .set_session_permission(user.id(), serde_json::from_value(policy)?)
                .await?;

            for tool in ["skills.preview_install", "skills.install"] {
                for (source, target) in [
                    (json!({"kind":"archive","url":archive_url}), archive_url.clone()),
                    (
                        json!({"kind":"github","url":"https://github.com/agena-fixtures/skills","reference":"main"}),
                        "https://codeload.github.com/agena-fixtures/skills/tar.gz/main".to_owned(),
                    ),
                ] {
                    let run = user.start_tool(tool, json!({"scope":"workspace","source":source})).await?;
                    let permission = run.wait_permission().await?;
                    let request = serde_json::to_value(&permission.request.request)?;
                    assert_eq!(request.pointer("/action/target"), Some(&json!(target)));
                    assert_eq!(requests.load(Ordering::SeqCst), 0);
                    ensure!(!case.workspace().join(".agena/skills").exists());
                    permission.mark_presented().await?;
                    permission.deny_once().await?;
                    failure(&run.wait_settled().await?, ModelFeedbackKind::UserDeclined)?;
                    assert_eq!(requests.load(Ordering::SeqCst), 0);
                    ensure!(!case.workspace().join(".agena/skills").exists());
                }
            }
            Ok(())
        })
        .await
}
