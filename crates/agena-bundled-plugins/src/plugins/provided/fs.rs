//! `agena.fs` plugin: filesystem read/write/search tools.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fs::File, io::Read};

use crate::part::{ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
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
    trim("path", "expected_sha256"),
    non_empty("path"),
    min_chars("old", 1),
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

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(non_empty("output_id"), minimum("limit", 1), maximum("limit", 16000))]
struct OutputReadInput {
    output_id: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "output_default_limit")]
    limit: usize,
}
const fn output_default_limit() -> usize {
    8000
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(
    non_empty("output_id", "pattern"),
    max_chars("pattern", 4096),
    minimum("limit", 1),
    maximum("limit", 100)
)]
struct OutputSearchInput {
    output_id: String,
    pattern: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "output_default_matches")]
    limit: usize,
}
const fn output_default_matches() -> usize {
    20
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
        name = "output_read",
        tags(query),
        summary = "Read a byte range of captured tool output owned by this session.",
        help = "Use output_id from a tool result. Offsets are UTF-8 byte offsets; next_offset continues the capture. Capture can expire, be evicted, or already be truncated.",
        read_only,
        concurrency_safe
    )]
    async fn output_read(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &OutputReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let result = agena_runtime_tools::output_resources::read(
            Path::new(context.workspace_root),
            context.session_id,
            &input.output_id,
            input.offset,
            input.limit,
        )
        .map_err(PluginError::invalid_params)?;
        let text = format!(
            "{}\n[Next offset: {:?}; captured {}/{} bytes; capture_truncated={}]",
            result.text,
            result.next_offset,
            result.captured_bytes,
            result.original_bytes,
            result.capture_truncated
        );
        Ok(ToolInvokeOutput::from_parts(
            "Captured output",
            format!("{} bytes", result.text.len()),
            text,
            Some(
                serde_json::to_value(result)
                    .map_err(|error| PluginError::internal_error(&error))?,
            ),
            Default::default(),
            Vec::new(),
        ))
    }
    #[tool(
        name = "output_search",
        tags(query),
        summary = "Find literal text within captured tool output owned by this session.",
        help = "Search before reading long logs. Results return byte offsets accepted by output_read. This searches captured bytes only, not content already dropped upstream.",
        read_only,
        concurrency_safe
    )]
    async fn output_search(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &OutputSearchInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let result = agena_runtime_tools::output_resources::search(
            Path::new(context.workspace_root),
            context.session_id,
            &input.output_id,
            &input.pattern,
            input.offset,
            input.limit,
        )
        .map_err(PluginError::invalid_params)?;
        let mut text = result
            .matches
            .iter()
            .map(|hit| format!("byte {}: {}", hit.offset, hit.preview))
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            text = "No matches in captured content.".into();
        }
        text.push_str(&format!(
            "\n[Next search offset: {:?}; capture_truncated={}]",
            result.next_offset, result.capture_truncated
        ));
        Ok(ToolInvokeOutput::from_parts(
            "Search captured output",
            format!("{} matches", result.matches.len()),
            text,
            Some(
                serde_json::to_value(result)
                    .map_err(|error| PluginError::internal_error(&error))?,
            ),
            Default::default(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(query, filesystem),
        summary = "Read workspace files.",
        help = "Use `read` for text previews and directory listings. Binary files return local references, not model-visible bytes. Use a provider cloud_image_understanding/cloud_document_understanding tool or explicitly attach media to the composer to send its contents.",
        read_only,
        examples(r#"{"file_path":"Cargo.toml"}"#),
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
                if input.old.is_empty() {
                    return Err(PluginError::invalid_params("old text must contain at least one byte"));
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
                let before_sha256 = sha256_bytes(original.as_bytes());
                if let Some(expected) = input.expected_sha256.as_deref()
                    && !before_sha256.eq_ignore_ascii_case(expected)
                {
                    return Err(PluginError::invalid_params(format!(
                        "stale file revision for '{}': expected sha256 {}, actual {}",
                        input.path, expected, before_sha256
                    )));
                }
                let occurrences = original.match_indices(input.old.as_str()).count();
                if occurrences != input.expected_occurrences as usize {
                    return Err(PluginError::invalid_params(format!(
                        "expected {} occurrence(s) of old text in '{}', found {occurrences}",
                        input.expected_occurrences, input.path
                    )));
                }
                let replacements = if input.replace_all { occurrences } else { 1 };
                let result_bytes = original.len()
                    .checked_sub(input.old.len().checked_mul(replacements).ok_or_else(|| PluginError::invalid_params("replacement size overflow"))?)
                    .and_then(|size| input.new.len().checked_mul(replacements).and_then(|added| size.checked_add(added)))
                    .filter(|size| *size as u64 <= MAX_MUTATING_TEXT_BYTES)
                    .ok_or_else(|| PluginError::invalid_params("fs.replace result exceeds the 16 MiB limit"))?;
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
                debug_assert_eq!(updated.len(), result_bytes);
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
            let mut succeeded = 0;
            let mut failed = 0;
            for path in &input.paths {
                if remaining == 0 {
                    truncated = true;
                    entries.push(serde_json::json!({"path":path,"status":"not_read_budget","returned_bytes":0}));
                    continue;
                }
                let target = resolve_path(workspace_root.as_str(), path);
                let read = (|| {
                    let metadata = std::fs::metadata(&target).map_err(fs_error)?;
                    if !metadata.is_file() {
                        return Err(PluginError::invalid_params("not a regular file"));
                    }
                    let (preview, returned_bytes, file_truncated) = read_utf8_prefix(&target, remaining, path)?;
                    Ok((metadata.len(), preview, returned_bytes, file_truncated))
                })();
                match read {
                    Ok((bytes, preview, returned_bytes, file_truncated)) => {
                        sections.push(format!("===== {path} =====\n{preview}"));
                        let hash = (!file_truncated).then(|| sha256_bytes(preview.as_bytes()));
                        entries.push(serde_json::json!({"path":path,"status":"read","bytes":bytes,
                            "returned_bytes":returned_bytes,"truncated":file_truncated,"sha256":hash}));
                        remaining = remaining.saturating_sub(returned_bytes);
                        truncated |= file_truncated;
                        succeeded += 1;
                    }
                    Err(error) => {
                        failed += 1;
                        sections.push(format!("===== {path} =====\nRead failed: {error}"));
                        entries.push(serde_json::json!({"path":path,"status":"error","error":error.to_string()}));
                    }
                }
            }
            Ok(ToolInvokeOutput::from_parts(
                format!("read {succeeded} files · {failed} failed"),
                if truncated {
                    format!("{} files · truncated", entries.len())
                } else {
                    format!("{} files", entries.len())
                },
                sections.join("\n\n"),
                Some(serde_json::json!({
                    "files": entries,
                    "success_count": succeeded,
                    "error_count": failed,
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
            let requested = Path::new(&input.path);
            let target = if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                Path::new(&workspace_root).join(requested)
            };
            let mut metadata = std::fs::symlink_metadata(&target).map_err(fs_error)?;
            let mut hash_skipped = false;
            let hash = if input.hash && metadata.is_file() {
                let file = File::open(&target).map_err(fs_error)?;
                metadata = file.metadata().map_err(fs_error)?;
                if !metadata.is_file() {
                    return Err(PluginError::invalid_params(
                        "stat target changed while opening it",
                    ));
                }
                if metadata.len() > MAX_STAT_HASH_BYTES {
                    hash_skipped = true;
                    None
                } else {
                    let mut digest = Sha256::new();
                    let mut reader = (&file).take(MAX_STAT_HASH_BYTES + 1);
                    let mut bytes = 0_u64;
                    let mut buffer = [0_u8; 64 * 1024];
                    loop {
                        let count = reader.read(&mut buffer).map_err(fs_error)?;
                        if count == 0 {
                            break;
                        }
                        bytes += count as u64;
                        if bytes > MAX_STAT_HASH_BYTES {
                            return Err(PluginError::invalid_params(
                                "stat file grew beyond its hash budget",
                            ));
                        }
                        digest.update(&buffer[..count]);
                    }
                    let after = file.metadata().map_err(fs_error)?;
                    if bytes != metadata.len()
                        || after.len() != metadata.len()
                        || after.modified().map_err(fs_error)?
                            != metadata.modified().map_err(fs_error)?
                    {
                        return Err(PluginError::invalid_params(
                            "stat file changed while hashing; retry the read",
                        ));
                    }
                    Some(hex::encode(digest.finalize()))
                }
            } else {
                None
            };
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
                .map_err(fs_error)?
                .duration_since(UNIX_EPOCH)
                .map_err(|error| PluginError::internal_error(&error))?
                .as_millis();
            let modified_at_ms = i64::try_from(modified_at_ms)
                .map_err(|error| PluginError::internal_error(&error))?;
            let symlink_target = if metadata.file_type().is_symlink() {
                Some(
                    std::fs::read_link(&target)
                        .map_err(fs_error)?
                        .display()
                        .to_string(),
                )
            } else {
                None
            };
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
                    .map_err(|error| PluginError::internal_error(&error))?,
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
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
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "acquire a filesystem plugin worker",
                &error,
            ))
        })?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        operation()
    })
    .await
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "filesystem plugin worker failed",
            &error,
        ))
    })?
}

fn permission_paths_internal<T: Serialize + ?Sized>(
    tool: &str,
    input: &T,
) -> SdkResult<Vec<PathRequest>> {
    let input = json_input(input)?;
    router::permission_paths_for(tool, &input)
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
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
    let capacity = match file.metadata() {
        Ok(metadata) => usize::try_from(metadata.len().min(max_bytes)).unwrap_or_default(),
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "read metadata used to preallocate a bounded file read",
                    &error,
                ),
                "bounded file read is continuing without a preallocated buffer"
            );
            0
        }
    };
    let mut bytes = Vec::with_capacity(capacity);
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
        Err(error) => {
            return Err(PluginError::invalid_params_with_public_detail(
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("read_many target is not UTF-8 text: {display_path}"),
                    &error,
                ),
                format!("The requested file is not UTF-8 text: {display_path}"),
            ));
        }
    };
    let text = std::str::from_utf8(valid_bytes)
        .expect("valid UTF-8 prefix was checked above")
        .to_string();
    Ok((text, valid_bytes.len(), truncated))
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
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        "filesystem operation failed",
        &error,
    ))
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
                "output_read",
                "output_search",
                "read",
                "glob",
                "grep",
                "apply_patch",
                "write",
                "replace",
                "read_many",
                "stat",
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
