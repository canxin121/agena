//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use crate::plugins::provided::router;
use serde::Serialize;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) struct FsPlugin;

pub(crate) fn new_plugin() -> FsPlugin {
    FsPlugin
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "fs",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Filesystem command tools for read/search and explicit edits.",
    display = detailed
)]
impl FsPlugin {
    #[tool(
        summary = "Read workspace files.",
        help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).",
        read_only,
        filesystem_read,
        display = detailed,
        examples(r#"{"path":"Cargo.toml"}"#),
        permission(paths = permission_read),
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
        summary = "Find paths with glob patterns.",
        help = "Use `glob` for path discovery before reading or editing files.",
        read_only,
        filesystem_read,
        discovery,
        display = detailed,
        examples(r#"{"pattern":"**/*.rs","path":"crates"}"#),
        permission(paths = permission_glob),
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
        summary = "Search file contents with regex.",
        help = "Use `grep` for regex text search across files in the workspace.",
        read_only,
        filesystem_read,
        discovery,
        display = detailed,
        examples(r#"{"pattern":"agena_plugin","path":"crates"}"#),
        permission(paths = permission_grep),
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
        summary = "Apply a text patch.",
        help = "Use `apply_patch` for explicit text patch operations against workspace files.",
        mutating,
        filesystem_write,
        display = detailed,
        permission(paths = permission_apply_patch),
        concurrency_safe
    )]
    async fn invoke_apply_patch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ApplyPatchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "apply_patch", args)
    }

    async fn permission_read(&self, args: ReadToolInput) -> SdkResult<Vec<PathRequest>> {
        permission_paths_internal("read", args)
    }

    async fn permission_glob(&self, args: GlobToolInput) -> SdkResult<Vec<PathRequest>> {
        permission_paths_internal("glob", args)
    }

    async fn permission_grep(&self, args: GrepToolInput) -> SdkResult<Vec<PathRequest>> {
        permission_paths_internal("grep", args)
    }

    async fn permission_apply_patch(
        &self,
        args: ApplyPatchToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        permission_paths_internal("apply_patch", args)
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

fn permission_paths_internal<T: Serialize>(tool: &str, input: T) -> SdkResult<Vec<PathRequest>> {
    let input = json_input(input)?;
    router::permission_paths_for(tool, &input)
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params(err.to_string()))
}
