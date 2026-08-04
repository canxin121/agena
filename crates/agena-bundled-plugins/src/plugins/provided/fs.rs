//! `agena.fs` plugin: filesystem read/write/search tools.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::message::{ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
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

pub(crate) struct FsPlugin;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("path", "expected_sha256"),
    non_empty("path"),
    non_empty_if_present("expected_sha256")
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
    minimum("expected_occurrences", 1)
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
    display = detailed
)]
impl FsPlugin {
    #[tool(
        tags(query, filesystem),
        summary = "Read workspace files.",
        help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).",
        read_only,

        display = detailed,
        examples(r#"{"path":"Cargo.toml"}"#),
        concurrency_safe
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "read", args)
    }

    #[tool(
        tags(query, filesystem, discovery),
        summary = "Find paths with glob patterns.",
        help = "Use `glob` for focused path discovery before reading or editing files. Results are paginated (default 200, maximum 1000) and dependency/VCS/build directories are skipped unless `include_ignored` is true or the base path explicitly names one.",
        read_only,

        discovery,
        display = detailed,
        examples(r#"{"pattern":"**/*.rs","path":"crates"}"#),
        concurrency_safe
    )]
    async fn invoke_glob(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GlobToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "glob", args)
    }

    #[tool(
        tags(query, filesystem, discovery),
        summary = "Search file contents with regex.",
        help = "Use `grep` for regex text search. `path` may be a directory (searched recursively) or a single file; it defaults to the workspace root.",
        read_only,

        discovery,
        display = detailed,
        examples(r#"{"pattern":"agena_plugin","path":"crates"}"#),
        concurrency_safe
    )]
    async fn invoke_grep(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GrepToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "grep", args)
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Apply a text patch.",
        help = "Use `apply_patch` for explicit text patch operations against workspace files.",
        mutating,

        display = detailed,
        path(requests = permission_paths_internal("apply_patch", input)?),
        concurrency_safe
    )]
    async fn invoke_apply_patch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ApplyPatchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "apply_patch", args)
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Create a UTF-8 text file or replace one at an expected revision.",
        help = "Creating a new file needs no hash. Replacing an existing file requires expected_sha256 from fs.stat, preventing stale or parallel overwrites.",
        mutating,

        display = detailed,
        path(requests = vec![PathRequest::write(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &WriteFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let target = resolve_path(context.workspace_root, input.path.as_str());
        let existed = target.exists();
        if existed {
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
        std::fs::write(&target, input.content.as_bytes()).map_err(fs_error)?;
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
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Replace exact UTF-8 text with occurrence and revision checks.",
        mutating,


        display = detailed,
        path(requests = vec![PathRequest::read(input.path.clone()), PathRequest::write(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_replace(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReplaceFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let target = resolve_path(context.workspace_root, input.path.as_str());
        if !target.is_file() {
            return Err(PluginError::invalid_params(format!(
                "replace target is not a file: {}",
                input.path
            )));
        }
        if let Some(expected) = input.expected_sha256.as_deref() {
            verify_expected_hash(&target, expected)?;
        }
        let original = std::fs::read_to_string(&target).map_err(fs_error)?;
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
        std::fs::write(&target, updated.as_bytes()).map_err(fs_error)?;
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
    }

    #[tool(
        tags(query, filesystem),
        summary = "Read multiple UTF-8 files within one bounded byte budget.",
        read_only,

        display = detailed,
        path(requests = input.paths.iter().cloned().map(PathRequest::read).collect::<Vec<_>>()),
        concurrency_safe
    )]
    async fn invoke_read_many(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReadManyInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let mut remaining = input.max_total_bytes as usize;
        let mut sections = Vec::new();
        let mut entries = Vec::new();
        let mut truncated = false;
        for path in &input.paths {
            let target = resolve_path(context.workspace_root, path);
            if !target.is_file() {
                entries.push(serde_json::json!({ "path": path, "error": "not a file" }));
                continue;
            }
            let bytes = std::fs::read(&target).map_err(fs_error)?;
            let text = String::from_utf8(bytes).map_err(|_| {
                PluginError::invalid_params(format!("read_many target is not UTF-8 text: {path}"))
            })?;
            let take = text.len().min(remaining);
            let boundary = floor_char_boundary(text.as_str(), take);
            let preview = &text[..boundary];
            let file_truncated = boundary < text.len();
            sections.push(format!("===== {path} =====\n{preview}"));
            entries.push(serde_json::json!({
                "path": path,
                "bytes": text.len(),
                "returned_bytes": boundary,
                "truncated": file_truncated,
                "sha256": sha256_bytes(text.as_bytes()),
            }));
            remaining = remaining.saturating_sub(boundary);
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
    }

    #[tool(
        tags(query, filesystem),
        summary = "Inspect file metadata and an optional SHA-256 revision.",
        read_only,

        display = detailed,
        path(requests = vec![PathRequest::read(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_stat(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &StatInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let target = resolve_path(context.workspace_root, input.path.as_str());
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
        let hash = if input.hash && metadata.is_file() {
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
    }

    #[tool(
        tags(query, filesystem),
        summary = "Attach a local image for visual inspection with an explicit detail hint.",
        read_only,

        display = detailed,
        path(requests = vec![PathRequest::read(input.path.clone())]),
        concurrency_safe
    )]
    async fn invoke_view_image(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ViewImageInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let target = resolve_path(context.workspace_root, input.path.as_str());
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
    }
}

fn invoke_internal<T: Serialize>(
    context: &ToolInvokeContext<'_>,
    tool: &str,
    input: T,
) -> SdkResult<ToolInvokeOutput> {
    router::invoke_tool(
        tool,
        json_input(input)?,
        context.session_id,
        context.call_id,
    )
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
        path.to_path_buf()
    } else {
        Path::new(workspace_root).join(path)
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> SdkResult<String> {
    std::fs::read(path)
        .map(|bytes| sha256_bytes(bytes.as_slice()))
        .map_err(fs_error)
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

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
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

    #[test]
    fn byte_budget_never_splits_utf8() {
        assert_eq!(floor_char_boundary("a你b", 2), 1);
        assert_eq!(floor_char_boundary("a你b", 4), 4);
    }
}
