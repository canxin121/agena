//! `agena.fs` plugin: filesystem read/write/search tools.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fs::File, io::Read};

use crate::part::{ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::attachment::{AttachmentItem, AttachmentKind, AttachmentSource};
use agena_plugin_host::sdk::{
    PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";
const MAX_MUTATING_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STAT_HASH_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct FsPlugin;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("path", "expected_sha256"),
    non_empty("path"),
    non_empty_if_present("expected_sha256"),
    max_chars("content", 16777216)
)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    path: String,
    content: String,
    #[serde(default)]
    create_parents: bool,
    /// Required when replacing an existing file. Use the hash returned by
    /// `fs.stat` or a prior mutating result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("path", "old", "expected_sha256"),
    non_empty("path", "old"),
    non_empty_if_present("expected_sha256"),
    minimum("expected_occurrences", 1),
    max_chars("old", 16777216),
    max_chars("new", 16777216)
)]
#[serde(deny_unknown_fields)]
struct ReplaceFileInput {
    path: String,
    old: String,
    new: String,
    #[serde(default = "default_expected_occurrences")]
    expected_occurrences: u32,
    #[serde(default)]
    replace_all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_sha256: Option<String>,
}

const fn default_expected_occurrences() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("paths[]"),
    non_empty("paths[]"),
    min_items("paths", 1),
    max_items("paths", 64),
    minimum("max_total_bytes", 1),
    maximum("max_total_bytes", 1048576)
)]
#[serde(deny_unknown_fields)]
struct ReadManyInput {
    paths: Vec<String>,
    #[serde(default = "default_read_many_budget")]
    max_total_bytes: u32,
}

const fn default_read_many_budget() -> u32 {
    131_072
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct StatInput {
    path: String,
    #[serde(default = "default_true")]
    hash: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum ImageDetail {
    Low,
    #[default]
    High,
    Original,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct ViewImageInput {
    path: String,
    #[serde(default)]
    detail: ImageDetail,
}

const fn default_true() -> bool {
    true
}

pub(crate) fn new_plugin() -> FsPlugin {
    FsPlugin
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "fs",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Filesystem command tools for read/search and explicit edits.",
)]
impl FsPlugin {
    #[tool(
        tags(query, filesystem),
        summary = "Read workspace files.",
        help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).",
        read_only,
        examples(r#"{"path":"Cargo.toml"}"#),
        concurrency_safe
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "read", args).await
    }

    #[tool(
        tags(query, filesystem, discovery),
        summary = "Find paths with glob patterns.",
        help = "Use `glob` for focused path discovery before reading or editing files. Results are paginated (default 200, maximum 1000) and ripgrep-compatible hidden/ignore rules are applied unless `include_ignored` is true or the base path explicitly names an ignored directory.",
        read_only,
        discovery,
        examples(r#"{"pattern":"**/*.rs","path":"crates"}"#),
        concurrency_safe
    )]
    async fn invoke_glob(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GlobToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "glob", args).await
    }

    #[tool(
        tags(query, filesystem, discovery),
        summary = "Search file contents with regex.",
        help = "Use `grep` for ripgrep-compatible, streaming regex text search. `path` may be a directory or a single file and defaults to the workspace root. Hidden/ignored files, binary files, oversized files, and runaway scans are bounded by default; narrow `path` or `include` when a search is truncated.",
        read_only,
        discovery,
        examples(r#"{"pattern":"agena_plugin","path":"crates"}"#),
        concurrency_safe
    )]
    async fn invoke_grep(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GrepToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "grep", args).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Apply a text patch to workspace files.",
        help = "Use `apply_patch` for explicit text patch operations against workspace files. The `patch` argument is a plain-text patch that MUST start with the exact marker line `*** Begin Patch` and end with the exact marker line `*** End Patch`. Inside, use only these directives: `*** Update File: <path>` followed by `@@`-separated hunks (context lines start with a space, removed lines with `-`, added lines with `+`), `*** Add File: <path>` with every content line prefixed by `+`, or `*** Delete File: <path>`. A patch that does not start with `*** Begin Patch` is rejected. Use paths relative to the workspace root.",
        mutating,

        examples(r#"{"patch":"*** Begin Patch\n*** Update File: README.md\n@@\n-old line\n+new line\n*** End Patch"}"#),
        path(requests = permission_paths_internal("apply_patch", input)?)
    )]
    async fn invoke_apply_patch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ApplyPatchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "apply_patch", args).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Create a UTF-8 text file or replace one at an expected revision.",
        help = "Creating a new file needs no hash. Replacing an existing file requires expected_sha256 from fs.stat, preventing stale or parallel overwrites.",
        mutating,

        path(requests = vec![PathRequest::write(input.path.clone())])
    )]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &WriteFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&target), || {
                if input.content.len() as u64 > MAX_MUTATING_TEXT_BYTES {
                    return Err(PluginError::invalid_params(format!(
                        "fs.write supports content up to {} MiB",
                        MAX_MUTATING_TEXT_BYTES / 1024 / 1024
                    )));
                }
                let existed = target.exists();
                if existed {
                    if !target.is_file() {
                        return Err(PluginError::invalid_params(format!(
                            "write target is not a file: {}",
                            input.path
                        )));
                    }
                    let expected = input.expected_sha256.as_deref().ok_or_else(|| {
                        PluginError::invalid_params(
                            "expected_sha256 is required when fs.write replaces an existing file",
                        )
                    })?;
                    verify_expected_hash(&target, expected)?;
                } else if input.expected_sha256.is_some() {
                    return Err(PluginError::invalid_params(
                        "expected_sha256 was supplied but the target does not exist",
                    ));
                }
                if let Some(parent) = target.parent()
                    && !parent.exists()
                {
                    if input.create_parents {
                        std::fs::create_dir_all(parent).map_err(fs_error)?;
                    } else {
                        return Err(PluginError::invalid_params(format!(
                            "parent directory does not exist: {}",
                            parent.display()
                        )));
                    }
                }
                if existed {
                    agena_runtime_tools::atomic_replace_file(&target, input.content.as_bytes())
                        .map_err(fs_error)?;
                } else {
                    agena_runtime_tools::atomic_create_file(
                        &target,
                        input.content.as_bytes(),
                        None,
                    )
                    .map_err(fs_error)?;
                }
                let hash = sha256_bytes(input.content.as_bytes());
                Ok(ToolInvokeOutput::from_parts(
                    format!(
                        "{} {}",
                        if existed { "updated" } else { "created" },
                        input.path
                    ),
                    format!(
                        "{} · {} bytes",
                        if existed { "Updated" } else { "Created" },
                        input.content.len()
                    ),
                    format!(
                        "{} '{}' ({} bytes, sha256={hash}).",
                        if existed { "Updated" } else { "Created" },
                        input.path,
                        input.content.len()
                    ),
                    Some(serde_json::json!({
                        "path": input.path,
                        "kind": if existed { "updated" } else { "created" },
                        "bytes": input.content.len(),
                        "sha256": hash,
                    })),
                    std::collections::BTreeMap::from([
                        ("agena.effect".to_string(), "file_changes".to_string()),
                        ("path".to_string(), input.path.clone()),
                        ("sha256".to_string(), hash),
                    ]),
                    Vec::new(),
                ))
            })
            .map_err(fs_error)?
        })
        .await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Replace exact UTF-8 text with occurrence and revision checks.",
        mutating,


        path(requests = vec![PathRequest::read(input.path.clone()), PathRequest::write(input.path.clone())])
    )]
    async fn invoke_replace(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReplaceFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&target), || {
                if !target.is_file() {
                    return Err(PluginError::invalid_params(format!(
                        "replace target is not a file: {}",
                        input.path
                    )));
                }
                if let Some(expected) = input.expected_sha256.as_deref() {
                    verify_expected_hash(&target, expected)?;
                }
                let original = String::from_utf8(read_file_bounded(
                    &target,
                    MAX_MUTATING_TEXT_BYTES,
                    "fs.replace",
                )?)
                .map_err(|_| {
                    PluginError::invalid_params(format!(
                        "replace target is not UTF-8 text: {}",
                        input.path
                    ))
                })?;
                let occurrences = original.match_indices(input.old.as_str()).count();
                if occurrences != input.expected_occurrences as usize {
                    return Err(PluginError::invalid_params(format!(
                        "expected {} occurrence(s) of old text in '{}', found {occurrences}",
                        input.expected_occurrences, input.path
                    )));
                }
                let updated = if input.replace_all {
                    original.replace(input.old.as_str(), input.new.as_str())
                } else {
                    original.replacen(input.old.as_str(), input.new.as_str(), 1)
                };
                if updated.len() as u64 > MAX_MUTATING_TEXT_BYTES {
                    return Err(PluginError::invalid_params(format!(
                        "fs.replace result exceeds the {} MiB limit",
                        MAX_MUTATING_TEXT_BYTES / 1024 / 1024
                    )));
                }
                agena_runtime_tools::atomic_replace_file(&target, updated.as_bytes())
                    .map_err(fs_error)?;
                let before_sha256 = sha256_bytes(original.as_bytes());
                let after_sha256 = sha256_bytes(updated.as_bytes());
                Ok(ToolInvokeOutput::from_parts(
                    format!("replaced text in {}", input.path),
                    format!(
                        "{} replacements",
                        if input.replace_all { occurrences } else { 1 }
                    ),
                    format!(
                        "Replaced {} occurrence(s) in '{}' (sha256 {before_sha256} -> {after_sha256}).",
                        if input.replace_all { occurrences } else { 1 },
                        input.path
                    ),
                    Some(serde_json::json!({
                        "path": input.path,
                        "replacements": if input.replace_all { occurrences } else { 1 },
                        "before_sha256": before_sha256,
                        "after_sha256": after_sha256,
                    })),
                    std::collections::BTreeMap::from([
                        ("agena.effect".to_string(), "file_changes".to_string()),
                        ("path".to_string(), input.path.clone()),
                        ("before_sha256".to_string(), before_sha256),
                        ("after_sha256".to_string(), after_sha256),
                    ]),
                    Vec::new(),
                ))
            })
            .map_err(fs_error)?
        })
        .await
    }

    #[tool(
        tags(query, filesystem),
        summary = "Read multiple UTF-8 files within one bounded byte budget.",
        read_only,

        path(requests = input.paths.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()),
        concurrency_safe
    )]
    async fn invoke_read_many(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReadManyInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let mut remaining = input.max_total_bytes as usize;
            let mut sections = Vec::new();
            let mut entries = Vec::new();
            let mut truncated = false;
            for path in &input.paths {
                if remaining == 0 {
                    truncated = true;
                    break;
                }
                let target = resolve_path(workspace_root.as_str(), path);
                if !target.is_file() {
                    entries.push(serde_json::json!({ "path": path, "error": "not a file" }));
                    continue;
                }
                let metadata = std::fs::metadata(&target).map_err(fs_error)?;
                let (preview, returned_bytes, file_truncated) =
                    read_utf8_prefix(&target, remaining, path)?;
                sections.push(format!("===== {path} =====\n{preview}"));
                let hash = (!file_truncated).then(|| sha256_bytes(preview.as_bytes()));
                entries.push(serde_json::json!({
                    "path": path,
                    "bytes": metadata.len(),
                    "returned_bytes": returned_bytes,
                    "truncated": file_truncated,
                    "sha256": hash,
                }));
                remaining = remaining.saturating_sub(returned_bytes);
                truncated |= file_truncated;
                if remaining == 0 {
                    truncated |= entries.len() < input.paths.len();
                    break;
                }
            }
            Ok(ToolInvokeOutput::from_parts(
                format!("read {} files", entries.len()),
                if truncated {
                    format!("{} files · truncated", entries.len())
                } else {
                    format!("{} files", entries.len())
                },
                sections.join("\n\n"),
                Some(serde_json::json!({
                    "files": entries,
                    "max_total_bytes": input.max_total_bytes,
                    "remaining_bytes": remaining,
                    "truncated": truncated,
                })),
                std::collections::BTreeMap::from([
                    ("file_count".to_string(), entries.len().to_string()),
                    ("truncated".to_string(), truncated.to_string()),
                ]),
                Vec::new(),
            ))
        })
        .await
    }

    #[tool(
        tags(query, filesystem),
        summary = "Inspect file metadata and an optional SHA-256 revision.",
        read_only,

        path(requests = vec![PathRequest::read(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_stat(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &StatInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            let metadata = std::fs::symlink_metadata(&target).map_err(fs_error)?;
            let file_type = if metadata.file_type().is_symlink() {
                "symlink"
            } else if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let modified_at_ms = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .and_then(|value| i64::try_from(value.as_millis()).ok());
            let hash_skipped =
                input.hash && metadata.is_file() && metadata.len() > MAX_STAT_HASH_BYTES;
            let hash = if input.hash && metadata.is_file() && !hash_skipped {
                Some(sha256_file(&target)?)
            } else {
                None
            };
            let symlink_target = metadata
                .file_type()
                .is_symlink()
                .then(|| std::fs::read_link(&target).ok())
                .flatten()
                .map(|path| path.display().to_string());
            let payload = serde_json::json!({
                "path": input.path,
                "kind": file_type,
                "size": metadata.len(),
                "modified_at_ms": modified_at_ms,
                "readonly": metadata.permissions().readonly(),
                "sha256": hash,
                "hash_skipped": hash_skipped,
                "symlink_target": symlink_target,
            });
            Ok(ToolInvokeOutput::from_parts(
                format!("stat {}", input.path),
                format!("{file_type} · {} bytes", metadata.len()),
                serde_json::to_string_pretty(&payload)
                    .map_err(|error| PluginError::internal(error.to_string()))?,
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ))
        })
        .await
    }

    #[tool(
        tags(query, filesystem),
        summary = "Attach a local image for visual inspection with an explicit detail hint.",
        read_only,

        path(requests = vec![PathRequest::read(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_view_image(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ViewImageInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            let metadata = std::fs::metadata(&target).map_err(fs_error)?;
            if !metadata.is_file() {
                return Err(PluginError::invalid_params(format!(
                    "image target is not a file: {}",
                    input.path
                )));
            }
            const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
            if metadata.len() > MAX_IMAGE_BYTES {
                return Err(PluginError::invalid_params(format!(
                    "image exceeds the {} MiB safety limit",
                    MAX_IMAGE_BYTES / 1024 / 1024
                )));
            }
            let mime = image_mime(&target).ok_or_else(|| {
                PluginError::invalid_params(
                    "unsupported image extension; expected png, jpg/jpeg, gif, webp, bmp, or svg",
                )
            })?;
            let hash = sha256_file(&target)?;
            let detail = match input.detail {
                ImageDetail::Low => "low",
                ImageDetail::High => "high",
                ImageDetail::Original => "original",
            };
            let attachment = AttachmentItem {
                kind: AttachmentKind::Image,
                mime: mime.to_string(),
                source: AttachmentSource::LocalPath {
                    path: target.to_string_lossy().to_string(),
                },
                filename: target
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(ToOwned::to_owned),
                title: Some(format!("{} ({detail} detail)", input.path)),
                size_bytes: Some(metadata.len()),
                sha256: Some(hash.clone()),
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            };
            Ok(ToolInvokeOutput::from_parts(
                format!("view image {}", input.path),
                format!("{mime} · {} bytes · {detail}", metadata.len()),
                format!(
                    "Attached '{}' for visual inspection (detail={detail}, {} bytes).",
                    input.path,
                    metadata.len()
                ),
                Some(serde_json::json!({
                    "path": input.path,
                    "detail": detail,
                    "mime": mime,
                    "size_bytes": metadata.len(),
                    "sha256": hash,
                })),
                std::collections::BTreeMap::from([
                    ("detail".to_string(), detail.to_string()),
                    ("sha256".to_string(), hash),
                ]),
                vec![attachment],
            ))
        })
        .await
    }
}

async fn invoke_internal<T: Serialize + Send + 'static>(
    context: &ToolInvokeContext<'_>,
    tool: &'static str,
    input: T,
) -> SdkResult<ToolInvokeOutput> {
    let input = json_input(input)?;
    let session_id = context.session_id;
    let call_id = context.call_id;
    run_fs_blocking(move || router::invoke_tool(tool, input, session_id, call_id)).await
}

async fn run_fs_blocking<T, F>(operation: F) -> SdkResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> SdkResult<T> + Send + 'static,
{
    let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|_| PluginError::internal("filesystem worker pool is unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        operation()
    })
    .await
    .map_err(|error| PluginError::internal(format!("filesystem worker failed: {error}")))?
}

fn permission_paths_internal<T: Serialize + ?Sized>(
    tool: &str,
    input: &T,
) -> SdkResult<Vec<PathRequest>> {
    let input = json_input(input)?;
    router::permission_paths_for(tool, &input)
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}

fn resolve_path(workspace_root: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        agena_runtime_tools::canonicalize_mutation_path(path)
    } else {
        agena_runtime_tools::canonicalize_mutation_path(&Path::new(workspace_root).join(path))
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> SdkResult<String> {
    let mut file = File::open(path).map_err(fs_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(fs_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn read_file_bounded(path: &Path, max_bytes: u64, operation: &str) -> SdkResult<Vec<u8>> {
    let file = File::open(path).map_err(fs_error)?;
    let mut bytes = Vec::with_capacity(
        file.metadata()
            .ok()
            .and_then(|metadata| usize::try_from(metadata.len().min(max_bytes)).ok())
            .unwrap_or_default(),
    );
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(fs_error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(PluginError::invalid_params(format!(
            "{operation} supports files up to {} MiB: {}",
            max_bytes / 1024 / 1024,
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_utf8_prefix(
    path: &Path,
    max_bytes: usize,
    display_path: &str,
) -> SdkResult<(String, usize, bool)> {
    let mut file = File::open(path).map_err(fs_error)?;
    let metadata = file.metadata().map_err(fs_error)?;
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.by_ref()
        .take(max_bytes as u64)
        .read_to_end(&mut bytes)
        .map_err(fs_error)?;
    let truncated = metadata.len() > bytes.len() as u64;
    let valid_bytes = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.as_slice(),
        Err(error) if truncated && error.error_len().is_none() => &bytes[..error.valid_up_to()],
        Err(_) => {
            return Err(PluginError::invalid_params(format!(
                "read_many target is not UTF-8 text: {display_path}"
            )));
        }
    };
    let text = std::str::from_utf8(valid_bytes)
        .expect("valid UTF-8 prefix was checked above")
        .to_string();
    Ok((text, valid_bytes.len(), truncated))
}

fn image_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn verify_expected_hash(path: &Path, expected: &str) -> SdkResult<()> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(PluginError::invalid_params(format!(
            "stale file revision for '{}': expected sha256 {}, actual {}",
            path.display(),
            expected.trim(),
            actual
        )))
    }
}

fn fs_error(error: std::io::Error) -> PluginError {
    PluginError::internal(format!("filesystem operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{Plugin, ToolInvokeContext};

    use super::*;

    #[test]
    fn manifest_exposes_safe_high_frequency_file_tools() {
        let manifest = FsPlugin.manifest();
        assert_eq!(
            manifest
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            [
                "read",
                "glob",
                "grep",
                "apply_patch",
                "write",
                "replace",
                "read_many",
                "stat",
                "view_image",
            ]
        );
    }

    #[tokio::test]
    async fn write_requires_revision_before_overwriting_and_replace_checks_count() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().display().to_string();
        let context = ToolInvokeContext {
            tool_name: "write",
            session_id: 1,
            call_id: 1,
            workspace_root: root.as_str(),
        };
        let plugin = FsPlugin;
        plugin
            .invoke_write(
                &context,
                &WriteFileInput {
                    path: "demo.txt".to_string(),
                    content: "one one".to_string(),
                    create_parents: false,
                    expected_sha256: None,
                },
            )
            .await
            .expect("create file");
        assert!(
            plugin
                .invoke_write(
                    &context,
                    &WriteFileInput {
                        path: "demo.txt".to_string(),
                        content: "stale".to_string(),
                        create_parents: false,
                        expected_sha256: None,
                    },
                )
                .await
                .is_err()
        );
        let hash = sha256_file(&dir.path().join("demo.txt")).expect("hash");
        plugin
            .invoke_replace(
                &context,
                &ReplaceFileInput {
                    path: "demo.txt".to_string(),
                    old: "one".to_string(),
                    new: "two".to_string(),
                    expected_occurrences: 2,
                    replace_all: true,
                    expected_sha256: Some(hash),
                },
            )
            .await
            .expect("replace file");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("demo.txt")).expect("read result"),
            "two two"
        );
    }

    #[tokio::test]
    async fn parallel_writes_with_one_revision_cannot_both_commit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().display().to_string();
        let context = ToolInvokeContext {
            tool_name: "write",
            session_id: 1,
            call_id: 1,
            workspace_root: root.as_str(),
        };
        let path = dir.path().join("race.txt");
        std::fs::write(&path, "original").expect("race fixture");
        let expected = sha256_file(&path).expect("fixture revision");
        let plugin = FsPlugin;
        let first = WriteFileInput {
            path: "race.txt".to_string(),
            content: "first".to_string(),
            create_parents: false,
            expected_sha256: Some(expected.clone()),
        };
        let second = WriteFileInput {
            path: "race.txt".to_string(),
            content: "second".to_string(),
            create_parents: false,
            expected_sha256: Some(expected),
        };

        let (first_result, second_result) = tokio::join!(
            plugin.invoke_write(&context, &first),
            plugin.invoke_write(&context, &second)
        );

        assert_ne!(first_result.is_ok(), second_result.is_ok());
        let final_text = std::fs::read_to_string(path).expect("final race content");
        assert!(matches!(final_text.as_str(), "first" | "second"));
    }

    #[test]
    fn byte_budget_never_splits_utf8() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("utf8.txt");
        std::fs::write(&path, "a你b").expect("write UTF-8 fixture");

        let (prefix, returned, truncated) =
            read_utf8_prefix(&path, 2, "utf8.txt").expect("bounded UTF-8 prefix");

        assert_eq!(prefix, "a");
        assert_eq!(returned, 1);
        assert!(truncated);
    }
}
