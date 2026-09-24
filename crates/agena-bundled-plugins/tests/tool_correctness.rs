//! Audit regressions through the real bundled-plugin/executor JSON boundary.
//! Each fixture owns an isolated workspace; no personal config or service calls.
use agena_domain::{StructuredObject, ToolInvocation};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::{ToolError, ToolExecutor},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

struct Fixture {
    root: tempfile::TempDir,
    executor: ToolExecutor,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let mut config = PluginsConfig::default();
        for name in ["agena.fs", "agena.notebook", "agena.memory", "agena.report"] {
            config
                .list
                .insert(name.into(), ConfiguredPlugin::static_default());
        }
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    "agena.fs".parse().unwrap(),
                    agena_bundled_plugins::tool::new_fs_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.notebook".parse().unwrap(),
                    agena_bundled_plugins::tool::new_notebook_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.memory".parse().unwrap(),
                    agena_bundled_plugins::tool::new_memory_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.report".parse().unwrap(),
                    agena_bundled_plugins::tool::new_report_plugin(),
                ),
            ],
            config,
            workspace_root: workspace.clone(),
            agena_version: "audit-test".into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .unwrap();
        Self {
            root,
            executor: ToolExecutor::new(
                workspace,
                ExecutionPrincipal::new(
                    PermissionPolicy::allow_all(),
                    ToolPermissionPolicy::allow_all(),
                ),
                plugins,
                None,
                None,
                None,
            ),
        }
    }
    fn write(&self, path: &str, value: &str) {
        std::fs::write(self.root.path().join(path), value).unwrap();
    }
    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.path().join(path)).unwrap()
    }
    async fn call(&self, name: &str, value: Value) -> Result<Value, ToolError> {
        let call = ToolInvocation::new(name, StructuredObject::try_from(value).unwrap());
        let prepared = self.executor.prepare_invocation(&call, 41, 1).await?;
        let result = self
            .executor
            .execute_invocation_detailed(&prepared.invocation, 41, 1)
            .await?;
        let payload = Value::from(result.output.payload);
        if matches!(name, "memory.get" | "memory.write") {
            assert!(
                result
                    .view
                    .output_text
                    .contains(payload["sha256"].as_str().expect("revision")),
                "memory revision must reach the text tool channel"
            );
        }
        Ok(payload)
    }
    async fn patch(&self, body: &str) -> Result<Value, ToolError> {
        self.call(
            "fs.apply_patch",
            json!({"patch":format!("*** Begin Patch\n{body}\n*** End Patch")}),
        )
        .await
    }
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[tokio::test]
async fn audit_replace_preserves_exact_whitespace_unicode_and_crlf() {
    let f = Fixture::new().await;
    for (original, old, new, expected) in [
        (
            "def f():\n    return 1\n",
            "    return 1\n",
            "    return 2\n",
            "def f():\n    return 2\n",
        ),
        (
            "前缀\r\n  你好\r\n",
            "  你好\r\n",
            "\t世界\r\n",
            "前缀\r\n\t世界\r\n",
        ),
        ("a  b\n", "  ", "\t", "a\tb\n"),
        ("one\n\ntwo", "\n\n", "\n", "one\ntwo"),
    ] {
        f.write("file.txt", original);
        f.call(
            "fs.replace",
            json!({"path":"file.txt", "old":old, "new":new,"expected_sha256":hash(original)}),
        )
        .await
        .unwrap();
        assert_eq!(f.read("file.txt"), expected, "old={old:?}");
    }
}

#[tokio::test]
async fn audit_replace_empty_search_and_stale_revision_do_not_write() {
    let f = Fixture::new().await;
    f.write("file.txt", "abc\n");
    for input in [
        json!({"path":"file.txt","old":"","new":"x"}),
        json!({"path":"file.txt","old":"abc","new":"x","expected_sha256":hash("stale")}),
    ] {
        assert!(f.call("fs.replace", input).await.is_err());
        assert_eq!(f.read("file.txt"), "abc\n");
    }
}

#[tokio::test]
async fn audit_patch_obeys_anchor_and_never_guesses_an_ambiguous_target() {
    let f = Fixture::new().await;
    let original = "def first():\n    return 1\n\ndef second():\n    return 1\n";
    f.write("file.py", original);
    f.patch("*** Update File: file.py\n@@ def second():\n-    return 1\n+    return 2")
        .await
        .unwrap();
    assert_eq!(
        f.read("file.py"),
        "def first():\n    return 1\n\ndef second():\n    return 2\n"
    );
    f.write("file.py", original);
    assert!(
        f.patch("*** Update File: file.py\n@@\n-    return 1\n+    return 9")
            .await
            .is_err()
    );
    assert_eq!(f.read("file.py"), original);
    assert!(
        f.patch("*** Update File: file.py\n@@ missing_anchor\n-    return 1\n+    return 9")
            .await
            .is_err()
    );
    assert_eq!(f.read("file.py"), original);
}

#[tokio::test]
async fn audit_patch_eof_preserves_missing_newline_and_crlf() {
    let f = Fixture::new().await;
    for (original, expected) in [
        ("first\nlast", "first\nchanged"),
        ("first\r\nlast\r\n", "first\r\nchanged\r\n"),
    ] {
        f.write("file.txt", original);
        f.patch("*** Update File: file.txt\n@@\n-last\n+changed\n*** End of File")
            .await
            .unwrap();
        assert_eq!(f.read("file.txt"), expected);
    }
}

#[tokio::test]
async fn audit_patch_matches_full_lines_and_preflights_every_file() {
    let f = Fixture::new().await;
    f.write("a.txt", "safe\n");
    f.write("b.txt", "foobar\n");
    assert!(
        f.patch(
            "*** Update File: a.txt\n@@\n-safe\n+changed\n*** Update File: b.txt\n@@\n-bar\n+bad"
        )
        .await
        .is_err()
    );
    assert_eq!(f.read("a.txt"), "safe\n");
    assert_eq!(f.read("b.txt"), "foobar\n");
}

#[cfg(unix)]
#[tokio::test]
async fn audit_stat_reports_link_and_dangling_link_without_hiding_identity() {
    let f = Fixture::new().await;
    f.write("file.txt", "data\n");
    std::os::unix::fs::symlink("file.txt", f.root.path().join("link.txt")).unwrap();
    std::os::unix::fs::symlink("missing", f.root.path().join("dangling.txt")).unwrap();
    for (path, target) in [("link.txt", "file.txt"), ("dangling.txt", "missing")] {
        let out = f.call("fs.stat", json!({"path":path})).await.unwrap();
        assert_eq!(out["kind"], "symlink");
        assert_eq!(out["symlink_target"], target);
        assert!(out["sha256"].is_null());
    }
    let file = f.call("fs.stat", json!({"path":"file.txt"})).await.unwrap();
    assert_eq!(file["sha256"], hash("data\n"));
}

fn notebook(cell_type: &str) -> Value {
    let mut cell = json!({"id":"existing-cell","cell_type":cell_type,"metadata":{"user":"preserved"},"source":["old\n"]});
    if cell_type == "code" {
        cell["outputs"] = json!([{"output_type":"stream","name":"stdout","text":["old\n"]}]);
        cell["execution_count"] = json!(1);
    }
    if cell_type == "markdown" {
        cell["attachments"] = json!({"image.png":{"image/png":"aGVsbG8="}});
    }
    json!({"cells":[cell],"metadata":{"kernelspec":{"name":"python3","display_name":"Python"}},"nbformat":4,"nbformat_minor":5})
}

#[tokio::test]
async fn audit_notebook_all_type_conversions_preserve_metadata_and_invalidate_execution() {
    let f = Fixture::new().await;
    for before in ["code", "markdown", "raw"] {
        for after in ["code", "markdown", "raw"] {
            let old = notebook(before).to_string();
            f.write("test.ipynb", &old);
            f.call("notebook.edit_cell",json!({"path":"test.ipynb","action":"replace","cell_index":0,"cell_type":after,"source":"new\n","preserve_outputs":false,"expected_sha256":hash(&old)})).await.unwrap();
            let out: Value = serde_json::from_str(&f.read("test.ipynb")).unwrap();
            let cell = &out["cells"][0];
            assert_eq!(cell["id"], "existing-cell");
            assert_eq!(cell["metadata"]["user"], "preserved");
            assert_eq!(cell["cell_type"], after);
            if after == "code" {
                assert_eq!(cell["outputs"], json!([]));
                assert!(cell["execution_count"].is_null());
            } else {
                assert!(cell.get("outputs").is_none(), "{before}->{after}: {cell}");
                assert!(cell.get("execution_count").is_none());
            }
            if after != "markdown" {
                assert!(cell.get("attachments").is_none());
            }
        }
    }
}

#[tokio::test]
async fn audit_notebook_insert_generates_unique_cell_ids_and_default_edit_clears_old_output() {
    let f = Fixture::new().await;
    let old = notebook("code").to_string();
    f.write("test.ipynb", &old);
    f.call("notebook.edit_cell",json!({"path":"test.ipynb","action":"replace","cell_index":0,"source":"changed","expected_sha256":hash(&old)})).await.unwrap();
    let updated = f.read("test.ipynb");
    let out: Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(out["cells"][0]["outputs"], json!([]));
    f.call("notebook.edit_cell",json!({"path":"test.ipynb","action":"insert_after","cell_index":0,"source":"new","expected_sha256":hash(&updated)})).await.unwrap();
    let out: Value = serde_json::from_str(&f.read("test.ipynb")).unwrap();
    let id = out["cells"][1]["id"].as_str().unwrap();
    assert!(!id.is_empty());
    assert_ne!(id, "existing-cell");
}

#[tokio::test]
async fn audit_read_many_keeps_successes_and_reports_every_requested_path() {
    let f = Fixture::new().await;
    f.write("a.txt", "hello");
    f.write("b.txt", "world");
    std::fs::write(f.root.path().join("bad.bin"), [0xff]).unwrap();
    let result = f
        .call(
            "fs.read_many",
            json!({"paths":["a.txt","bad.bin","b.txt","missing.txt"],"max_total_bytes":100}),
        )
        .await
        .unwrap();
    let entries = result["files"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries[0]["error"].is_null());
    assert!(entries[1]["error"].is_string());
    assert!(entries[2]["error"].is_null());
    assert!(entries[3]["error"].is_string());
    let limited = f
        .call(
            "fs.read_many",
            json!({"paths":["a.txt","b.txt"],"max_total_bytes":5}),
        )
        .await
        .unwrap();
    assert_eq!(limited["files"].as_array().unwrap().len(), 2);
    assert_eq!(limited["files"][1]["status"], "not_read_budget");
}

#[tokio::test]
async fn audit_report_rejects_reversed_line_ranges() {
    let f = Fixture::new().await;
    f.call("report.findings",json!({"findings":[{"severity":"high","file":"src/lib.rs","line":10,"end_line":20,"title":"fixture","body":"fixture"}]})).await.unwrap();
    let finding = json!({"severity":"high","file":"src/lib.rs","line":20,"end_line":10,"title":"fixture","body":"fixture"});
    assert!(
        f.call("report.findings", json!({"findings":[finding]}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audit_memory_requires_revision_and_index_failure_preserves_original() {
    if isolated_memory_test("audit_memory_requires_revision_and_index_failure_preserves_original") {
        return;
    }
    let f = Fixture::new().await;
    let first = f
        .call(
            "memory.write",
            json!({"name":"decision","description":"true","content":"original"}),
        )
        .await
        .unwrap();
    assert!(first["sha256"].is_string());
    assert!(
        f.call(
            "memory.write",
            json!({"name":"decision","content":"unreviewed"})
        )
        .await
        .is_err()
    );
    let updated = f
        .call(
            "memory.write",
            json!({"name":"decision","content":"updated","expected_sha256":first["sha256"]}),
        )
        .await
        .unwrap();
    assert!(
        f.call(
            "memory.write",
            json!({"name":"decision","content":"stale","expected_sha256":first["sha256"]})
        )
        .await
        .is_err()
    );
    let store = agena_storage::MemoryStore::for_workspace(f.executor.workspace_root());
    let index = store.dir().join("MEMORY.md");
    std::fs::remove_file(&index).unwrap();
    std::fs::create_dir(&index).unwrap();
    assert!(f.call("memory.write",json!({"name":"decision","content":"must not replace","expected_sha256":updated["sha256"]})).await.is_err());
    let retained = f
        .call("memory.get", json!({"name":"decision"}))
        .await
        .unwrap();
    assert_eq!(retained["body"].as_str().unwrap().trim(), "updated");
    assert_eq!(retained["sha256"], updated["sha256"]);
}

#[tokio::test]
async fn audit_memory_index_uses_exact_names_and_quotes_yaml_scalars() {
    if isolated_memory_test("audit_memory_index_uses_exact_names_and_quotes_yaml_scalars") {
        return;
    }
    let f = Fixture::new().await;
    for name in ["a", "ba", "true"] {
        let out = f
            .call(
                "memory.write",
                json!({"name":name,"description":"true","content":"正文"}),
            )
            .await
            .unwrap();
        assert_eq!(out["name"], name);
        assert_eq!(out["description"], "true");
    }
    f.call("memory.delete", json!({"name":"a"})).await.unwrap();
    let store = agena_storage::MemoryStore::for_workspace(f.executor.workspace_root());
    let index = store.index_lines().unwrap().join("\n");
    assert!(!index.contains("(a.md)"));
    assert!(index.contains("(ba.md)"));
}

/// Memory's production location is HOME/agena/projects/<workspace>. Give only
/// the child test process an isolated HOME; never mutate the test process env.
fn isolated_memory_test(name: &str) -> bool {
    if std::env::var("AGENA_AUDIT_MEMORY_CHILD").as_deref() == Ok(name) {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("AGENA_AUDIT_MEMORY_CHILD", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated memory regression failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[tokio::test]
async fn audit_patch_move_and_inverse_roundtrip_preserve_exact_file_contents() {
    let f = Fixture::new().await;
    for original in ["a\nb\n", "a\nb", "a\r\nb\r\n", "\n"] {
        f.write("original.txt", original);
        let output = f
            .patch("*** Update File: original.txt\n*** Move to: renamed.txt\n@@\n+added")
            .await
            .unwrap();
        assert!(!f.root.path().join("original.txt").exists());
        f.call("fs.apply_patch", json!({"patch":output["inverse_patch"]}))
            .await
            .unwrap();
        assert_eq!(f.read("original.txt"), original);
        assert!(!f.root.path().join("renamed.txt").exists());
    }
}

#[tokio::test]
async fn audit_patch_multi_hunk_does_not_match_newly_inserted_content() {
    let f = Fixture::new().await;
    f.write("test.txt", "one\ntwo\nthree\n");
    f.patch("*** Update File: test.txt\n@@\n-one\n+two\n@@\n-two\n+four")
        .await
        .unwrap();
    assert_eq!(f.read("test.txt"), "two\nfour\nthree\n");
}

#[tokio::test]
async fn audit_patch_blank_addition_and_unknown_marker_are_exact() {
    let f = Fixture::new().await;
    f.patch("*** Add File: blank.txt\n+").await.unwrap();
    assert_eq!(f.read("blank.txt"), "\n");
    assert!(
        f.patch("*** Update File: blank.txt\n@@bad anchor\n-\n+x")
            .await
            .is_err()
    );
    assert_eq!(f.read("blank.txt"), "\n");
}

#[tokio::test]
async fn audit_notebook_duplicate_ids_are_rejected_without_any_commit() {
    let f = Fixture::new().await;
    let mut book = notebook("code");
    let duplicate = book["cells"][0].clone();
    book["cells"].as_array_mut().unwrap().push(duplicate);
    let original = book.to_string();
    f.write("test.ipynb", &original);
    assert!(f.call("notebook.edit_cell",json!({"path":"test.ipynb","action":"replace","cell_index":0,"source":"new","expected_sha256":hash(&original)})).await.is_err());
    assert_eq!(f.read("test.ipynb"), original);
}

#[tokio::test]
async fn audit_report_empty_findings_still_has_a_valid_outcome_summary() {
    let f = Fixture::new().await;
    let output = f.call("report.findings", json!({})).await.unwrap();
    assert_eq!(output["findings"], json!([]));
}

#[tokio::test]
async fn audit_patch_deleted_files_restore_without_added_blank_lines_or_eol_conversion() {
    let f = Fixture::new().await;
    for original in ["a\r\nb\n", "last", "\n", "", "last\r"] {
        f.write("deleted.txt", original);
        let output = f.patch("*** Delete File: deleted.txt").await.unwrap();
        f.call("fs.apply_patch", json!({"patch":output["inverse_patch"]}))
            .await
            .unwrap();
        assert_eq!(f.read("deleted.txt"), original);
    }
}

#[tokio::test]
async fn audit_large_file_small_page_is_streamed() {
    let f = Fixture::new().await;
    let path = f.root.path().join("large.txt");
    std::fs::write(&path, "first\nsecond\n").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    let output = f
        .call(
            "fs.read",
            json!({"file_path":"large.txt","mode":"text","offset":1,"limit":1}),
        )
        .await
        .unwrap();
    assert!(output["preview"].as_str().unwrap().starts_with("1: first"));
    assert_eq!(output["read_info"]["next_offset"], 2);
    assert!(output["read_info"]["scanned_bytes"].as_u64().unwrap() < 100);
}
#[tokio::test]
async fn audit_long_tool_result_is_recoverable_only_by_its_owner() {
    let f = Fixture::new().await;
    let text = (0..1000)
        .map(|i| {
            if i == 501 {
                "UNIQUE_MIDDLE_FAILURE".to_owned()
            } else {
                format!("line {i}: fixture output")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    f.write("log.txt", &text);
    let call = ToolInvocation::new(
        "fs.read",
        StructuredObject::try_from(json!({"file_path":"log.txt","mode":"text"})).unwrap(),
    );
    let prepared = f.executor.prepare_invocation(&call, 41, 81).await.unwrap();
    let result = f
        .executor
        .execute_invocation_detailed(&prepared.invocation, 41, 81)
        .await
        .unwrap();
    let id = result.view.metadata.get("output_resource_id").unwrap();
    assert!(result.view.output_text.contains(id));
    let hits = f
        .call(
            "fs.output_search",
            json!({"output_id":id,"pattern":"UNIQUE_MIDDLE_FAILURE"}),
        )
        .await
        .unwrap();
    let offset = hits["matches"][0]["offset"].as_u64().unwrap();
    let page = f
        .call(
            "fs.output_read",
            json!({"output_id":id,"offset":offset,"limit":100}),
        )
        .await
        .unwrap();
    assert!(
        page["text"]
            .as_str()
            .unwrap()
            .starts_with("UNIQUE_MIDDLE_FAILURE")
    );
    let foreign = ToolInvocation::new(
        "fs.output_read",
        StructuredObject::try_from(json!({"output_id":id})).unwrap(),
    );
    let foreign = f
        .executor
        .prepare_invocation(&foreign, 42, 82)
        .await
        .unwrap();
    assert!(
        f.executor
            .execute_invocation_detailed(&foreign.invocation, 42, 82)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audit_nested_project_guidance_reaches_the_file_read_result() {
    let f = Fixture::new().await;
    std::fs::create_dir(f.root.path().join("src")).unwrap();
    f.write("AGENTS.md", "ROOT_PROJECT_RULE");
    f.write("src/AGENTS.override.md", "NESTED_PROJECT_RULE");
    f.write("src/lib.rs", "fn main() {}\n");
    let input = ToolInvocation::new(
        "fs.read",
        StructuredObject::try_from(json!({"file_path":"src/lib.rs"})).unwrap(),
    );
    let prepared = f.executor.prepare_invocation(&input, 41, 90).await.unwrap();
    let result = f
        .executor
        .execute_invocation_detailed(&prepared.invocation, 41, 90)
        .await
        .unwrap();
    assert!(result.view.output_text.contains("ROOT_PROJECT_RULE"));
    assert!(result.view.output_text.contains("NESTED_PROJECT_RULE"));
    assert!(result.view.output_text.contains("AGENTS.override.md"));
}

#[tokio::test]
async fn audit_edit_without_lsp_does_not_claim_validation_success() {
    let f = Fixture::new().await;
    let invocation = ToolInvocation::new(
        "fs.write",
        StructuredObject::try_from(
            json!({"path":"unchecked.rs","content":"syntactically broken fixture"}),
        )
        .unwrap(),
    );
    let prepared = f
        .executor
        .prepare_invocation(&invocation, 41, 99)
        .await
        .unwrap();
    let result = f
        .executor
        .execute_invocation_detailed(&prepared.invocation, 41, 99)
        .await
        .unwrap();
    assert_eq!(f.read("unchecked.rs"), "syntactically broken fixture");
    assert!(result.view.output_text.contains("not_configured"));
    assert!(
        result
            .view
            .output_text
            .contains("not evidence of clean code")
    );
}

#[tokio::test]
async fn audit_binary_fs_read_is_a_local_reference_not_an_implicit_model_upload() {
    let f = Fixture::new().await;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
    use base64::Engine as _;
    std::fs::write(
        f.root.path().join("picture.png"),
        base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap(),
    )
    .unwrap();
    let invocation = ToolInvocation::new(
        "fs.read",
        StructuredObject::try_from(json!({"file_path":"picture.png","mode":"attachment"})).unwrap(),
    );
    let prepared = f
        .executor
        .prepare_invocation(&invocation, 41, 201)
        .await
        .unwrap();
    let result = f
        .executor
        .execute_invocation_detailed(&prepared.invocation, 41, 201)
        .await
        .unwrap();
    assert!(result.view.output_text.contains("Local reference only"));
    assert!(!serde_json::to_string(&result.output).unwrap().contains(png));
    assert!(result.view.attachments.iter().all(|attachment| matches!(
        attachment.source,
        agena_domain::AttachmentSource::LocalPath { .. }
    )));
}
