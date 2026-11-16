//! Real offline MXC checks for the bundled business workers.
//! cargo run -p agena-bundled-plugins --example sandbox_local_smoke
use agena_process::sandbox::{SandboxNetwork, SandboxPolicy};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
};
use agena_skills::{
    SkillDetail, SkillImport, SkillImportFile, SkillInstallResult, SkillManager, SkillScope,
    SkillSource,
    source::{SkillSourceOperation, SkillSourceResult},
    worker::{SkillWorkerOperation, SkillWorkerRequest},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{error::Error, io, path::Path, time::Duration};

type CheckResult<T> = Result<T, Box<dyn Error>>;

fn main() -> CheckResult<()> {
    if let Some(code) = agena_process::sandbox::try_run_launcher() {
        std::process::exit(code);
    }
    if let Some(code) = agena_bundled_plugins::worker::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_runtime_tools::sandbox_worker::try_run_worker(
        agena_bundled_plugins::execute_sandbox_tool_with_read_filter,
    ) {
        std::process::exit(code);
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(checks())
}

fn snapshot(root: &Path) -> Value {
    serde_json::to_value(
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .sandbox_permission_snapshot(root),
    )
    .unwrap()
}

async fn raw(argument: &str, request: Value, policy: &SandboxPolicy) -> CheckResult<Value> {
    let (_scratch, command) = agena_process::worker::command(argument, policy)?;
    let input = serde_json::to_vec(&request)?;
    let output =
        agena_process::output_with_input(command, &input, Duration::from_secs(60), 8 * 1024 * 1024)
            .await?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "native worker failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        ))
        .into());
    }
    let response: Value = serde_json::from_slice(&output.stdout)?;
    if response["status"] != "ok" {
        return Err(io::Error::other(format!("worker rejected operation: {response}")).into());
    }
    Ok(response["result"].clone())
}

async fn bundled(
    operation: &str,
    mut input: Value,
    policy: &SandboxPolicy,
    root: &Path,
) -> CheckResult<Value> {
    // The software filter is deliberately permissive. The denied-path checks
    // below must still fail at the real OS boundary.
    input["_agena_permission_snapshot"] = snapshot(root);
    raw(
        "--agena-plugin-worker",
        json!({"operation": operation, "input": input}),
        policy,
    )
    .await
}

async fn skills(
    manager: &SkillManager,
    operation: SkillWorkerOperation,
    policy: &SandboxPolicy,
    root: &Path,
) -> CheckResult<Value> {
    bundled(
        "skills.manager",
        serde_json::to_value(SkillWorkerRequest {
            manager: manager.worker_snapshot(),
            operation,
        })?,
        policy,
        root,
    )
    .await
}

async fn memory(
    directory: &Path,
    operation: Value,
    policy: &SandboxPolicy,
    root: &Path,
) -> CheckResult<Value> {
    bundled(
        "memory.store",
        json!({"directory": directory, "operation": operation}),
        policy,
        root,
    )
    .await
}

fn memory_write(name: &str, content: &str) -> Value {
    json!({
        "operation": "write", "name": name, "content": content, "description": "Native smoke",
        "kind": "fact", "expected": null,
        "options": {"scope": null, "status": null, "importance": null, "source": null, "supersedes": []},
        "session_id": 7, "instance_id": "native-smoke"
    })
}

async fn checks() -> CheckResult<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().canonicalize()?;
    let allowed = root.join("allowed");
    let denied = root.join("denied");
    std::fs::create_dir_all(&allowed)?;
    std::fs::create_dir_all(&denied)?;
    let policy = SandboxPolicy {
        readwrite_paths: vec![allowed.clone()],
        denied_paths: vec![denied.clone()],
        network: SandboxNetwork::Disabled,
        configured_filesystem: true,
        ..Default::default()
    };

    let document =
        b"---\nname: native-skill\ndescription: Native worker fixture\n---\nOriginal skill\n";
    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(document.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, "native-skill/SKILL.md", &document[..])?;
    let imported: SkillSourceResult = serde_json::from_value(
        bundled(
            "skills.source",
            serde_json::to_value(SkillSourceOperation::ImportArchive {
                filename: "skills.tar".into(),
                bytes: archive.into_inner()?,
                subpath: None,
            })?,
            &policy,
            &allowed,
        )
        .await?,
    )?;
    let SkillSourceResult::Package(package) = imported else {
        return Err(io::Error::other("archive worker did not return a package").into());
    };
    let expected = SkillImport::from_files(vec![SkillImportFile {
        path: "SKILL.md".into(),
        content: document.to_vec(),
        executable: false,
    }])?;
    assert_eq!(package.revision(), expected.revision());
    println!("PASS: TAR interpretation and bounded base64 transfer in MXC");

    let manager = SkillManager::new(allowed.clone(), denied.join("global-skills"));
    let installed: SkillInstallResult = serde_json::from_value(
        skills(
            &manager,
            SkillWorkerOperation::Install {
                package,
                scope: SkillScope::Workspace,
                selected: Vec::new(),
                replace: false,
                source: SkillSource {
                    kind: "upload".into(),
                    location: "native fixture".into(),
                    reference: None,
                    subpath: None,
                    revision: None,
                },
            },
            &policy,
            &allowed,
        )
        .await?,
    )?;
    let skill = &installed.installed[0];
    let detail: SkillDetail = serde_json::from_value(
        skills(
            &manager,
            SkillWorkerOperation::Get {
                id: skill.id.clone(),
            },
            &policy,
            &allowed,
        )
        .await?,
    )?;
    assert!(detail.document.contains("Original skill"));
    let disabled: SkillDetail = serde_json::from_value(
        skills(
            &manager,
            SkillWorkerOperation::SetEnabled {
                id: skill.id.clone(),
                enabled: false,
                expected_revision: detail.summary.revision.clone(),
            },
            &policy,
            &allowed,
        )
        .await?,
    )?;
    assert!(!disabled.summary.enabled);
    assert!(
        skills(
            &manager,
            SkillWorkerOperation::Update {
                id: skill.id.clone(),
                document: String::from_utf8(document.to_vec())?,
                expected_revision: detail.summary.revision,
            },
            &policy,
            &allowed,
        )
        .await
        .is_err()
    );
    println!("PASS: skill install/get/state and stale revision protection in MXC");

    let source_secret = denied.join("secret");
    std::fs::write(&source_secret, b"never disclosed")?;
    assert!(
        bundled(
            "skills.source",
            serde_json::to_value(SkillSourceOperation::ReadFile {
                path: source_secret.clone(),
                limit: 1024,
            })?,
            &policy,
            &allowed
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&source_secret)?, b"never disclosed");
    println!("PASS: unapproved skills source blocked by the OS");

    let memory_dir = allowed.join("memory");
    let written = memory(
        &memory_dir,
        memory_write("decision", "native_memory_marker"),
        &policy,
        &allowed,
    )
    .await?;
    let revision = written["payload"]["sha256"]
        .as_str()
        .ok_or_else(|| io::Error::other("memory write omitted revision"))?
        .to_owned();
    let readonly = SandboxPolicy {
        readonly_paths: vec![allowed.clone()],
        readwrite_paths: Vec::new(),
        ..policy.clone()
    };
    let read = memory(
        &memory_dir,
        json!({
            "operation": "get", "name": "decision", "session_id": 7,
            "instance_id": "native-smoke", "offset": 0, "limit": 8000
        }),
        &readonly,
        &allowed,
    )
    .await?;
    assert_eq!(read["payload"]["sha256"], revision);
    let found = memory(
        &memory_dir,
        json!({
            "operation": "search", "query": "native_memory_marker", "limit": 20,
            "interactive": true, "include_history": false, "session_id": 7,
            "context": {"global": false, "instance_id": "native-smoke", "checkpoint_entry_id": null}
        }),
        &readonly,
        &allowed,
    )
    .await?;
    assert_eq!(found["indexed_records"], 1);
    assert_eq!(found["items"].as_array().map(Vec::len), Some(1));
    let listed = memory(&memory_dir, json!({
        "operation": "list", "limit": 200, "offset": 0, "include_history": false,
        "session_id": 7, "context": {"global": false, "instance_id": "native-smoke", "checkpoint_entry_id": null}
    }), &readonly, &allowed).await?;
    assert_eq!(
        listed["payload"]["memories"].as_array().map(Vec::len),
        Some(1)
    );
    assert!(
        memory(
            &memory_dir,
            memory_write("readonly-blocked", "must not persist"),
            &readonly,
            &allowed
        )
        .await
        .is_err()
    );
    assert!(!memory_dir.join("readonly-blocked.md").exists());
    let absent = allowed.join("uncreated-memory");
    let empty = memory(&absent, json!({
        "operation": "list", "limit": 200, "offset": 0, "include_history": false,
        "session_id": 7, "context": {"global": false, "instance_id": "native-smoke", "checkpoint_entry_id": null}
    }), &readonly, &allowed).await?;
    assert_eq!(
        empty["payload"]["memories"].as_array().map(Vec::len),
        Some(0)
    );
    let empty = memory(
        &absent,
        json!({
            "operation": "search", "query": "absent", "limit": 20,
            "interactive": true, "include_history": false, "session_id": 7,
            "context": {"global": false, "instance_id": "native-smoke", "checkpoint_entry_id": null}
        }),
        &readonly,
        &allowed,
    )
    .await?;
    assert_eq!(empty["indexed_records"], 0);
    assert!(!absent.exists());
    println!(
        "PASS: memory readonly reads/search and absent-directory reads do not create host files"
    );
    memory(
        &memory_dir,
        json!({
            "operation": "delete", "name": "decision", "expected": revision,
            "session_id": 7, "instance_id": "native-smoke"
        }),
        &policy,
        &allowed,
    )
    .await?;
    assert!(!memory_dir.join("decision.md").exists());
    assert!(
        memory(
            &denied.join("memory"),
            memory_write("blocked", "must not persist"),
            &policy,
            &allowed,
        )
        .await
        .is_err()
    );
    assert!(!denied.join("memory").exists());
    println!("PASS: memory write/get/search/delete and denied directory in MXC");

    let notebook = allowed.join("notebook.ipynb");
    let original = serde_json::to_vec(&json!({
        "nbformat": 4, "nbformat_minor": 5, "metadata": {},
        "cells": [{"id": "original", "cell_type": "code", "metadata": {},
            "source": ["print(1)"], "execution_count": null, "outputs": []}]
    }))?;
    std::fs::write(&notebook, &original)?;
    let notebook_request = |path: &Path| {
        json!({
            "operation": "notebook.edit_cell",
            "input": {"path": path, "expected_sha256": hex::encode(Sha256::digest(&original)),
                "cell_index": 0, "action": "replace", "source": "print(2)"},
            "workspace_root": allowed, "snapshot": snapshot(&allowed),
            "read_paths": [path], "write_paths": [path]
        })
    };
    raw("--agena-tool-worker", notebook_request(&notebook), &policy).await?;
    let updated: Value = serde_json::from_slice(&std::fs::read(&notebook)?)?;
    assert_eq!(updated["cells"][0]["source"], json!(["print(2)"]));
    assert!(
        raw("--agena-tool-worker", notebook_request(&notebook), &policy)
            .await
            .is_err()
    );
    let blocked_notebook = denied.join("notebook.ipynb");
    std::fs::write(&blocked_notebook, &original)?;
    assert!(
        raw(
            "--agena-tool-worker",
            notebook_request(&blocked_notebook),
            &policy
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&blocked_notebook)?, original);
    println!("PASS: notebook edit, stale revision and denied file in MXC");
    Ok(())
}
