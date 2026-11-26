//! Isolated workers for native filesystem and syntax tools. Parent-side code
//! owns approval, cross-process mutation locks and exact-path publication.
//! Workers receive only the invocation and its resource snapshot, never the
//! application configuration, model credentials or a live storage handle.
use crate::{
    authorization::{ExecutionPrincipal, SandboxPermissionSnapshot},
    fs_broker::FileCapability,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::{
        ToolError, ToolExecutionView, ToolExecutor, ToolInvocationExecution, ToolRuntimeContext,
    },
};
use agena_domain::{AccessKind, PermissionDecision, ToolInvocation, ToolOutput};
use agena_plugin_host::{PluginHost, registry::RegisteredTool};
use agena_plugin_sdk::ToolInvokeOutput;
use agena_process::sandbox::SandboxPolicy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "sandbox_worker/media.rs"]
mod media;
pub use media::{inspect_media_bytes_with_policy, read_media_with_policy};

const WORKER_ARGUMENT: &str = "--agena-tool-worker";
const MAX_TRANSFER: usize = 96 * 1024 * 1024;
const MAX_FILE: usize = 16 * 1024 * 1024;
const MAX_NOTEBOOK_FILE: usize = 32 * 1024 * 1024;
const MAX_TRANSACTION: usize = 64 * 1024 * 1024;
static WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

pub type BundledWorker = fn(&str, Value, &Path) -> agena_plugin_sdk::Result<ToolInvokeOutput>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    input: Value,
    workspace_root: PathBuf,
    snapshot: SandboxPermissionSnapshot,
    read_paths: Vec<PathBuf>,
    write_paths: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Response {
    Ok { result: ToolInvokeOutput },
    Error { message: String },
}

#[derive(Clone)]
pub(crate) struct Plan {
    operation: String,
    input: Value,
    workspace_root: PathBuf,
    pub(crate) reads: Vec<PathBuf>,
    pub(crate) writes: Vec<PathBuf>,
    labels: Vec<(String, PathBuf)>,
    moves: Vec<(PathBuf, PathBuf)>,
}

fn required_string<'a>(input: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::invalid_input(format!("sandbox tool requires {name}")))
}

pub(crate) fn plan(
    executor: &ToolExecutor,
    resolution: &RegisteredTool,
    invocation: &ToolInvocation,
) -> Result<Option<Plan>, ToolError> {
    if resolution.namespace() != "agena" {
        return Ok(None);
    }
    let operation = match (resolution.plugin_name(), resolution.tool_name()) {
        ("fs", "read") => "read",
        ("fs", "glob") => "glob",
        ("fs", "grep") => "grep",
        ("fs", "apply_patch") => "apply_patch",
        ("fs", "write") => "fs.write",
        ("fs", "replace") => "fs.replace",
        ("fs", "read_many") => "fs.read_many",
        ("fs", "stat") => "fs.stat",
        ("notebook", "edit_cell") => "notebook.edit_cell",
        _ => return Ok(None),
    };
    let mut input = Value::from(invocation.input.clone());
    let folder = input
        .get("folder")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let workspace_root = executor.resolve_folder_base(folder.as_deref());
    let resolve = |path: &str| -> Result<PathBuf, ToolError> {
        if executor.is_global()
            && !Path::new(path).is_absolute()
            && folder
                .as_deref()
                .is_none_or(|base| !Path::new(base).is_absolute())
        {
            return Err(ToolError::invalid_input(
                "global filesystem tools require an absolute path or folder",
            ));
        }
        let path = executor.resolve_target_path_in_folder(path, folder.as_deref());
        if !path.is_absolute() {
            return Err(ToolError::invalid_input(
                "sandbox tool path must be absolute",
            ));
        }
        Ok(path)
    };
    let mut labels = Vec::new();
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut moves = Vec::new();
    match operation {
        "apply_patch" => {
            for (source, target) in
                crate::tool::apply_patch::planned_moves(required_string(&input, "patch")?)?
            {
                moves.push((resolve(&source)?, resolve(&target)?));
            }
            for label in crate::tool::apply_patch::planned_paths(required_string(&input, "patch")?)?
            {
                let path = resolve(&label)?;
                reads.push(path.clone());
                writes.push(path.clone());
                labels.push((label, path));
            }
        }
        "fs.read_many" => {
            let paths = input
                .get("paths")
                .and_then(Value::as_array)
                .ok_or_else(|| ToolError::invalid_input("fs.read_many requires paths"))?;
            let mut absolute = Vec::new();
            for path in paths {
                let label = path
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_input("paths must contain strings"))?;
                let path = resolve(label)?;
                reads.push(path.clone());
                labels.push((label.to_owned(), path.clone()));
                absolute.push(Value::String(path.to_string_lossy().into_owned()));
            }
            input["paths"] = Value::Array(absolute);
        }
        _ => {
            let field = if operation == "read" {
                "file_path"
            } else {
                "path"
            };
            let label = match input.get(field).and_then(Value::as_str) {
                Some(value) if !value.trim().is_empty() => value.to_owned(),
                _ if matches!(operation, "glob" | "grep") => {
                    if workspace_root.as_os_str().is_empty() {
                        return Err(ToolError::invalid_input(
                            "global search requires a path or folder",
                        ));
                    }
                    workspace_root.to_string_lossy().into_owned()
                }
                _ => {
                    return Err(ToolError::invalid_input(format!(
                        "{operation} requires {field}"
                    )));
                }
            };
            let path = resolve(&label)?;
            input[field] = Value::String(path.to_string_lossy().into_owned());
            reads.push(path.clone());
            if matches!(operation, "fs.write" | "fs.replace" | "notebook.edit_cell") {
                writes.push(path.clone());
            }
            labels.push((label, path));
        }
    }
    reads.sort();
    reads.dedup();
    writes.sort();
    writes.dedup();
    Ok(Some(Plan {
        operation: operation.into(),
        input,
        workspace_root,
        reads,
        writes,
        labels,
        moves,
    }))
}

pub(crate) async fn execute(
    executor: &ToolExecutor,
    plan: Plan,
) -> Result<ToolInvocationExecution, ToolError> {
    executor.ensure_not_cancelled()?;
    let executor = executor.clone();
    let runtime = tokio::runtime::Handle::current();
    WORKERS
        .run(move || {
            let targets = plan.writes.clone();
            if targets.is_empty() {
                execute_locked(&executor, plan, &runtime)
            } else {
                crate::with_file_mutation_locks(&targets, || {
                    execute_locked(&executor, plan, &runtime)
                })?
            }
        })
        .await
        .map_err(|error| ToolError::plugin(format!("sandbox tool worker failed: {error}")))?
}

pub(crate) async fn read_media(
    executor: &ToolExecutor,
    target: &Path,
    expected_sha256: Option<&str>,
) -> Result<crate::media_input::PreparedMedia, ToolError> {
    executor.ensure_not_cancelled()?;
    let executor = executor.clone();
    let target = target.to_path_buf();
    let expected_sha256 = expected_sha256.map(str::to_owned);
    let runtime = tokio::runtime::Handle::current();
    WORKERS
        .run(move || {
            executor.authorize_sandbox_paths(std::slice::from_ref(&target), &[])?;
            if !executor.sandbox_enabled() {
                return crate::media_input::read_local(
                    Path::new(""),
                    &target.to_string_lossy(),
                    expected_sha256.as_deref(),
                )
                .map_err(ToolError::invalid_input);
            }
            let mut policy = crate::shell_sandbox::policy_for_paths(
                &executor,
                std::slice::from_ref(&target),
                &[],
                &[],
            )?;
            let scratch = tempfile::Builder::new()
                .prefix("agena-media-worker-")
                .tempdir()?;
            let scratch_path = fs::canonicalize(scratch.path())?;
            policy.readwrite_paths.push(scratch_path.clone());
            policy.private_paths.push(scratch_path.clone());
            let request = Request {
                operation: "media_read".into(),
                input: serde_json::json!({"path": target, "expected_sha256": expected_sha256}),
                workspace_root: executor.workspace_root().to_path_buf(),
                snapshot: executor.sandbox_permission_snapshot(),
                read_paths: policy.readonly_paths.clone(),
                write_paths: policy.readwrite_paths.clone(),
            };
            let result =
                runtime.block_on(run_worker(&executor, &request, &scratch_path, &policy))?;
            executor.ensure_not_cancelled()?;
            let transfer: crate::media_input::PreparedMediaTransfer = serde_json::from_value(
                result
                    .payload
                    .ok_or_else(|| ToolError::plugin("sandbox media response has no payload"))?,
            )
            .map_err(|error| ToolError::invalid_input_error(&error))?;
            let prepared = transfer.into_prepared().map_err(ToolError::invalid_input)?;
            if let Some(expected) = expected_sha256.as_ref()
                && !prepared.sha256.eq_ignore_ascii_case(expected)
            {
                return Err(ToolError::invalid_input(
                    "media revision changed before sandbox result delivery",
                ));
            }
            Ok(prepared)
        })
        .await
        .map_err(|error| ToolError::plugin(format!("sandbox media worker failed: {error}")))?
}

struct StagedFile {
    label: String,
    actual: PathBuf,
    staged: PathBuf,
    original: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
    capability: FileCapability,
    output_capability: FileCapability,
}

fn execute_locked(
    executor: &ToolExecutor,
    mut plan: Plan,
    runtime: &tokio::runtime::Handle,
) -> Result<ToolInvocationExecution, ToolError> {
    executor.ensure_not_cancelled()?;
    executor.authorize_sandbox_paths(&plan.reads, &plan.writes)?;
    let max_file = if plan.operation == "notebook.edit_cell" {
        MAX_NOTEBOOK_FILE
    } else {
        MAX_FILE
    };
    let mut policy =
        crate::shell_sandbox::policy_for_paths(executor, &plan.reads, &plan.writes, &[])?;
    // Kernel mount policies resolve symlinks. Preserve lstat's lexical-link
    // contract in the trusted metadata broker; no target bytes are read here.
    if plan.operation == "fs.stat" {
        let label = &plan.labels[0].0;
        let raw = if Path::new(label).is_absolute() {
            PathBuf::from(label)
        } else {
            plan.workspace_root.join(label)
        };
        if let Ok(metadata) = fs::symlink_metadata(&raw)
            && metadata.file_type().is_symlink()
        {
            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| ToolError::invalid_input_error(&error))?
                .as_millis();
            let payload = serde_json::json!({
                "path": label, "kind": "symlink", "size": metadata.len(),
                "modified_at_ms": i64::try_from(modified).map_err(|error| ToolError::invalid_input_error(&error))?,
                "readonly": metadata.permissions().readonly(), "sha256": Value::Null,
                "hash_skipped": false, "symlink_target": fs::read_link(&raw)?.to_string_lossy(),
            });
            executor.ensure_not_cancelled()?;
            return from_response(ToolInvokeOutput::from_parts(
                format!("stat {label}"),
                format!("symlink · {} bytes", metadata.len()),
                serde_json::to_string_pretty(&payload)
                    .map_err(|error| ToolError::invalid_input_error(&error))?,
                Some(payload),
                BTreeMap::from([("sandbox".into(), "authorized_metadata_broker".into())]),
                Vec::new(),
            ));
        }
    }
    let scratch = tempfile::Builder::new()
        .prefix("agena-tool-worker-")
        .tempdir()?;
    let scratch_path = fs::canonicalize(scratch.path())?;
    policy.private_paths.push(scratch_path.clone());
    let mut staged = Vec::new();
    if !plan.writes.is_empty() {
        let mut total = 0_usize;
        let mut unique = BTreeMap::new();
        for (label, path) in &plan.labels {
            unique.entry(path.clone()).or_insert(label.clone());
        }
        let create_parents = plan.operation == "apply_patch"
            || (plan.operation == "fs.write"
                && plan
                    .input
                    .get("create_parents")
                    .and_then(Value::as_bool)
                    .unwrap_or(false));
        for (index, (path, label)) in unique.into_iter().enumerate() {
            // Keep the same directory object from the initial read through
            // publication and rollback, even if its ambient path is replaced.
            let capability = FileCapability::open(&path, create_parents)?;
            let (original, permissions) = match capability.read_existing(max_file)? {
                Some((bytes, permissions)) => (Some(bytes), Some(permissions)),
                None => (None, None),
            };
            total = total
                .checked_add(original.as_ref().map_or(0, Vec::len))
                .ok_or_else(|| ToolError::invalid_input("sandbox staging size overflow"))?;
            if total > MAX_TRANSACTION {
                return Err(ToolError::invalid_input(
                    "sandbox transaction exceeds 64 MiB",
                ));
            }
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file");
            let stage = scratch_path.join(format!("{index:04}-{filename}"));
            if let Some(bytes) = &original {
                fs::write(&stage, bytes)?;
            }
            let output_capability = FileCapability::open(&stage, false)?;
            staged.push(StagedFile {
                label,
                actual: path,
                staged: stage,
                original,
                permissions,
                capability,
                output_capability,
            });
        }
        if plan.operation == "apply_patch" {
            let text = required_string(&plan.input, "patch")?;
            let rewritten = text
                .split_inclusive('\n')
                .map(|line| {
                    for prefix in [
                        "*** Add File: ",
                        "*** Update File: ",
                        "*** Delete File: ",
                        "*** Move to: ",
                    ] {
                        if let Some(raw) = line.strip_prefix(prefix) {
                            let raw_path = raw.trim_end_matches(['\r', '\n']);
                            if let Some((_, actual)) =
                                plan.labels.iter().find(|(label, _)| label == raw_path)
                                && let Some(file) =
                                    staged.iter().find(|file| &file.actual == actual)
                            {
                                let ending = if line.ends_with("\r\n") {
                                    "\r\n"
                                } else if line.ends_with('\n') {
                                    "\n"
                                } else {
                                    ""
                                };
                                return format!("{prefix}{}{ending}", file.staged.display());
                            }
                        }
                    }
                    line.to_owned()
                })
                .collect::<String>();
            plan.input["patch"] = Value::String(rewritten);
        } else {
            plan.input["path"] = Value::String(staged[0].staged.to_string_lossy().into_owned());
        }
        plan.input["folder"] = Value::Null;
        plan.workspace_root = scratch_path.clone();
        // A mutation worker sees copied inputs and private outputs. No host
        // writable directory is mounted; exact publication happens below.
        policy.readonly_paths.clear();
        policy.readwrite_paths = vec![scratch_path.clone()];
    } else {
        policy.readwrite_paths.push(scratch_path.clone());
    }
    // Private copies have already passed the host's original-path authority.
    // The worker filter must use their new locations; applying host path rules
    // to random staging names would reject valid workspace-only policies.
    let snapshot = if staged.is_empty() {
        executor.sandbox_permission_snapshot()
    } else {
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .sandbox_permission_snapshot(&scratch_path)
    };
    let request = Request {
        operation: plan.operation.clone(),
        input: plan.input,
        workspace_root: plan.workspace_root,
        snapshot,
        read_paths: policy
            .readonly_paths
            .iter()
            .chain(&policy.readwrite_paths)
            .cloned()
            .collect(),
        write_paths: policy.readwrite_paths.clone(),
    };
    let mut response = runtime.block_on(run_worker(executor, &request, &scratch_path, &policy))?;
    executor.ensure_not_cancelled()?;
    if !staged.is_empty() {
        let mut changes = Vec::new();
        let mut total = 0;
        for file in &staged {
            let updated = file
                .output_capability
                .read_existing(max_file)?
                .map(|(bytes, _)| bytes);
            if plan.operation != "apply_patch" && updated.is_none() {
                return Err(ToolError::invalid_input(
                    "sandbox mutation removed its output file",
                ));
            }
            total += updated.as_ref().map_or(0, Vec::len);
            if total > MAX_TRANSACTION {
                return Err(ToolError::invalid_input("sandbox output exceeds 64 MiB"));
            }
            changes.push(crate::tool::apply_patch::SandboxFileChange {
                path: file.actual.clone(),
                original: file.original.clone(),
                updated,
                permissions: file.permissions.clone(),
            });
        }
        // Permission is rechecked against the captured authority before any
        // host publication. A cancelled or failed worker never reaches here.
        executor
            .sandbox_permission_snapshot()
            .filesystem_restrictions(&plan.reads, &plan.writes)
            .map_err(|error| {
                ToolError::invalid_input(format!("sandbox publication denied: {error}"))
            })?;
        executor.ensure_not_cancelled()?;
        if plan.operation == "apply_patch" {
            // A move keeps the trusted source mode, including executable bits.
            for (source_path, target_path) in &plan.moves {
                if let Some(source) = staged.iter().find(|file| &file.actual == source_path)
                    && let Some(change) = changes
                        .iter_mut()
                        .find(|change| &change.path == target_path)
                {
                    change.permissions = source.permissions.clone();
                }
            }
        }
        let changes = staged
            .iter()
            .map(|file| file.capability.clone())
            .zip(changes)
            .collect();
        if plan.operation == "notebook.edit_cell" {
            // Carry the same trusted notebook limit through the final revision
            // check and atomic publication, including the original snapshot.
            crate::fs_broker::commit_with_file_limit(changes, max_file)?;
        } else {
            crate::fs_broker::commit(changes)?;
        }
        let mut value = serde_json::to_value(response)
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        for file in &staged {
            replace_strings(&mut value, &file.staged.to_string_lossy(), &file.label);
        }
        response = serde_json::from_value(value)
            .map_err(|error| ToolError::invalid_input_error(&error))?;
    }
    response.metadata.insert("sandbox".into(), "mxc".into());
    if !plan.writes.is_empty() {
        response
            .metadata
            .insert("sandbox_publication".into(), "approved_paths_only".into());
    }
    from_response(response)
}

fn replace_strings(value: &mut Value, source: &str, target: &str) {
    match value {
        Value::String(value) => {
            *value = value.replace(source, target);
        }
        Value::Array(values) => {
            for value in values {
                replace_strings(value, source, target);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_strings(value, source, target);
            }
        }
        _ => {}
    }
}

async fn run_worker(
    executor: &ToolExecutor,
    request: &Request,
    cwd: &Path,
    policy: &SandboxPolicy,
) -> Result<ToolInvokeOutput, ToolError> {
    run_worker_request(request, cwd, policy, executor.cancellation_token()).await
}

async fn run_worker_request(
    request: &Request,
    cwd: &Path,
    policy: &SandboxPolicy,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<ToolInvokeOutput, ToolError> {
    if !policy.disabled && !agena_process::sandbox::enabled() {
        return Err(ToolError::invalid_input(
            "MXC is required by this worker but the native sandbox launcher is disabled",
        ));
    }
    let bytes =
        serde_json::to_vec(request).map_err(|error| ToolError::invalid_input_error(&error))?;
    if bytes.len() > MAX_TRANSFER {
        return Err(ToolError::invalid_input(
            "sandbox worker request exceeds 96 MiB",
        ));
    }
    let executable = agena_process::sandbox::launcher_executable()?;
    let command = agena_process::sandbox::command(
        executable
            .to_str()
            .ok_or_else(|| ToolError::invalid_input("sandbox worker executable is not UTF-8"))?,
        &[WORKER_ARGUMENT.into()],
        cwd,
        &std::env::vars().collect(),
        policy,
    )?;
    let result =
        agena_process::output_with_input(command, &bytes, Duration::from_secs(120), MAX_TRANSFER);
    let output = match cancel {
        Some(cancel) => {
            tokio::select! { biased; _ = cancel.cancelled() => return Err(ToolError::Cancelled), result = result => result? }
        }
        None => result.await?,
    };
    if !output.status.success() {
        return Err(ToolError::invalid_input(format!(
            "MXC tool worker failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    match serde_json::from_slice::<Response>(&output.stdout)
        .map_err(|error| ToolError::plugin(format!("invalid sandbox worker response: {error}")))?
    {
        Response::Ok { result } => Ok(result),
        Response::Error { message } => Err(ToolError::invalid_input(message)),
    }
}

fn from_response(response: ToolInvokeOutput) -> Result<ToolInvocationExecution, ToolError> {
    let output = ToolOutput::from_json_payload(response.payload.as_ref()).map_err(|error| {
        ToolError::from_plugin_error(agena_plugin_host::schema::output_contract_error(error))
    })?;
    let apply_patch = agena_tool::ApplyPatchExecution::from_tool_payload(
        response.payload.as_ref().unwrap_or(&Value::Null),
    );
    Ok(ToolInvocationExecution {
        output,
        apply_patch,
        view: ToolExecutionView {
            title: response.title,
            summary: response.summary,
            output_text: response.output_text,
            metadata: response.metadata,
            attachments: response.attachments,
        },
    })
}

/// Called by the executable before normal CLI parsing or runtime initialization.
/// Return None for ordinary invocations. The bundled handler contains only pure
/// native file operations, with no model or plugin bootstrap.
pub fn try_run_worker(bundled: BundledWorker) -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER_ARGUMENT)) {
        return None;
    }
    let result = (|| -> Result<ToolInvokeOutput, ToolError> {
        if std::env::args_os().count() != 2 {
            return Err(ToolError::invalid_input("invalid sandbox worker arguments"));
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_TRANSFER as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_TRANSFER {
            return Err(ToolError::invalid_input("sandbox worker request too large"));
        }
        let request: Request = serde_json::from_slice(&bytes)
            .map_err(|error| ToolError::invalid_input_error(&error))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let _guard = runtime.enter();
        let snapshot = request.snapshot.clone();
        let reads = request.read_paths.clone();
        let writes = request.write_paths.clone();
        let filter = agena_tool::path_filter::PathFilter::<ToolError>::new(move |path| {
            match snapshot.check_granted_path(AccessKind::Read, path, &reads, &writes) {
                PermissionDecision::Allow => Ok(()),
                other => Err(ToolError::invalid_input(format!(
                    "sandbox read denied for {}: {other:?}",
                    path.display()
                ))),
            }
        });
        let predicate = filter.predicate();
        let executor = ToolExecutor::new(
            &request.workspace_root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            PluginHost::new_empty(),
            None,
            None,
        )
        .with_read_path_authorizer(Arc::new({
            let predicate = predicate.clone();
            move |path| {
                if predicate.allows(path) {
                    Ok(())
                } else {
                    Err(ToolError::invalid_input(format!(
                        "sandbox read is outside approved paths: {}",
                        path.display()
                    )))
                }
            }
        }));
        let result = if request.operation == "media_read" {
            let path = required_string(&request.input, "path")?;
            if !predicate.allows(Path::new(path)) {
                return Err(ToolError::invalid_input(
                    "sandbox media path is outside approved reads",
                ));
            }
            let prepared = crate::media_input::read_local(
                Path::new(""),
                path,
                request.input.get("expected_sha256").and_then(Value::as_str),
            )
            .map_err(ToolError::invalid_input)?;
            let transfer = crate::media_input::PreparedMediaTransfer::from(prepared);
            let mut result = ToolInvokeOutput::text("Media prepared in MXC sandbox");
            result.payload = Some(
                serde_json::to_value(transfer)
                    .map_err(|error| ToolError::invalid_input_error(&error))?,
            );
            Ok(result)
        } else if request.operation == "media_inspect" {
            media::inspect_in_worker(request.input)
        } else if matches!(
            request.operation.as_str(),
            "read" | "glob" | "grep" | "apply_patch"
        ) {
            crate::tool::orchestrator::execute_tool(
                &executor,
                &request.operation,
                request.input,
                ToolRuntimeContext::default(),
            )
            .map(crate::tool::router::tool_execution_to_invoke_output)
        } else {
            bundled(&request.operation, request.input, &request.workspace_root)
                .map_err(ToolError::from_plugin_error)
        };
        filter.finish(result)
    })();
    let response = match result {
        Ok(result) => Response::Ok { result },
        Err(error) => Response::Error {
            message: error.to_string(),
        },
    };
    let result = serde_json::to_writer(std::io::stdout().lock(), &response);
    match result {
        Ok(()) => {
            let _ = std::io::stdout().flush();
            Some(0)
        }
        Err(error) => {
            eprintln!("sandbox response serialization failed: {error}");
            Some(125)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    #[ignore = "requires AGENA_TEST_SANDBOX_LAUNCHER and native MXC prerequisites"]
    fn native_filesystem_worker_writes_only_the_writable_child_of_a_readonly_parent() {
        use sha2::Digest;

        let launcher = std::env::var_os("AGENA_TEST_SANDBOX_LAUNCHER")
            .expect("set AGENA_TEST_SANDBOX_LAUNCHER to an existing Agena executable");
        agena_process::sandbox::configure_launcher(PathBuf::from(launcher)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        fs::create_dir(root.join("output")).unwrap();
        let path = root.join("output/result.txt");
        let executor = ToolExecutor::new(
            &root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            PluginHost::new_empty(),
            None,
            None,
        )
        .with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace":"readonly", "external":"deny", "rules": {
                    "<workspace>/output":"readwrite"
                }},
                "network": {"default":"deny"}
            }))
            .unwrap(),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for content in ["created through MXC", "replaced through MXC"] {
            let mut input = serde_json::json!({"path":path,"content":content});
            if path.exists() {
                input["expected_sha256"] =
                    Value::String(hex::encode(sha2::Sha256::digest(fs::read(&path).unwrap())));
            }
            let plan = Plan {
                operation: "fs.write".into(),
                input,
                workspace_root: root.clone(),
                reads: vec![path.clone()],
                writes: vec![path.clone()],
                labels: vec![("output/result.txt".into(), path.clone())],
                moves: vec![],
            };
            let result = runtime.block_on(execute(&executor, plan)).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
            assert_eq!(
                result.view.metadata.get("sandbox").map(String::as_str),
                Some("mxc")
            );
        }
        let readonly = root.join("readonly.txt");
        fs::write(&readonly, "unchanged").unwrap();
        let denied = Plan {
            operation: "fs.write".into(),
            input: serde_json::json!({
                "path":readonly,
                "content":"forbidden",
                "expected_sha256":hex::encode(sha2::Sha256::digest(fs::read(&readonly).unwrap()))
            }),
            workspace_root: root,
            reads: vec![readonly.clone()],
            writes: vec![readonly.clone()],
            labels: vec![("readonly.txt".into(), readonly.clone())],
            moves: vec![],
        };
        assert!(runtime.block_on(execute(&executor, denied)).is_err());
        assert_eq!(fs::read_to_string(readonly).unwrap(), "unchanged");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "requires AGENA_TEST_SANDBOX_LAUNCHER and native MXC prerequisites"]
    fn native_notebook_worker_preserves_large_notebooks_and_revision_checks() {
        use sha2::Digest;

        let launcher = std::env::var_os("AGENA_TEST_SANDBOX_LAUNCHER")
            .expect("set AGENA_TEST_SANDBOX_LAUNCHER to an existing Agena executable");
        agena_process::sandbox::configure_launcher(PathBuf::from(launcher)).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("notebook.ipynb");
        let original = serde_json::to_vec(&serde_json::json!({
            "nbformat": 4, "nbformat_minor": 5,
            "metadata": {"large": "x".repeat(17 * 1024 * 1024)},
            "cells": [{"id": "cell-one", "cell_type": "code", "metadata": {},
                "source": ["print(1)"], "execution_count": 1,
                "outputs": [{"output_type": "stream", "name": "stdout", "text": ["1"]}]}]
        }))
        .unwrap();
        assert!(original.len() > MAX_FILE && original.len() < MAX_NOTEBOOK_FILE);
        fs::write(&path, &original).unwrap();
        let executor = ToolExecutor::new(
            &root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            PluginHost::new_empty(),
            None,
            None,
        )
        .with_permission_config(
            &serde_json::from_value(serde_json::json!({
                "path": {"workspace":"readonly", "external":"deny",
                    "rules":{"<workspace>/notebook.ipynb":"readwrite"}},
                "network":{"default":"deny"}
            }))
            .unwrap(),
        );
        let plan = Plan {
            operation: "notebook.edit_cell".into(),
            input: serde_json::json!({
                "path":path, "expected_sha256":hex::encode(sha2::Sha256::digest(&original)),
                "action":"replace", "cell_index":0, "source":"print(2)"
            }),
            workspace_root: root,
            reads: vec![path.clone()],
            writes: vec![path.clone()],
            labels: vec![("notebook.ipynb".into(), path.clone())],
            moves: vec![],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(execute(&executor, plan.clone())).unwrap();
        let updated = fs::read(&path).unwrap();
        let document: Value = serde_json::from_slice(&updated).unwrap();
        assert_eq!(
            document["metadata"]["large"].as_str().unwrap().len(),
            17 * 1024 * 1024
        );
        assert!(
            document["cells"][0]["outputs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(document["cells"][0]["execution_count"].is_null());
        assert!(runtime.block_on(execute(&executor, plan)).is_err());
        assert_eq!(fs::read(&path).unwrap(), updated);
    }

    #[test]
    fn sandbox_mutation_rechecks_write_before_reading_or_starting_worker() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("input.txt");
        fs::write(&path, b"original").unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = seen.clone();
        let executor = ToolExecutor::new(
            &root,
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            PluginHost::new_empty(),
            None,
            None,
        )
        .with_path_effect_authorizer(Arc::new(move |access, path| {
            observed.lock().unwrap().push((access, path.to_owned()));
            if access == AccessKind::Write {
                Err(ToolError::invalid_input(
                    "write target needs its current invocation approval",
                ))
            } else {
                Ok(())
            }
        }));
        let plan = Plan {
            operation: "fs.write".into(),
            input: serde_json::json!({"path":path,"content":"changed"}),
            workspace_root: root,
            reads: vec![path.clone()],
            writes: vec![path.clone()],
            labels: vec![("input.txt".into(), path.clone())],
            moves: vec![],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = execute_locked(&executor, plan, runtime.handle()).unwrap_err();
        assert!(error.to_string().contains("current invocation approval"));
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(AccessKind::Read, path.clone()), (AccessKind::Write, path)]
        );
    }

    #[test]
    fn output_path_rewrite_preserves_structured_types() {
        let mut value = serde_json::json!({"path":"/private/stage/a","diff":"--- /private/stage/a\n+x","count":2,"ok":true});
        replace_strings(&mut value, "/private/stage/a", "src/a.rs");
        assert_eq!(value["path"], "src/a.rs");
        assert_eq!(value["diff"], "--- src/a.rs\n+x");
        assert_eq!(value["count"], 2);
        assert_eq!(value["ok"], true);
    }
}
