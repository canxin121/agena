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
        help = "Use `glob` for focused path discovery before reading or editing files. Results are paginated (default 200, maximum 1000) and dependency/VCS/build directories are skipped unless `include_ignored` is true or the base path explicitly names one.",
        read_only,
        filesystem_read,
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
        summary = "Search file contents with regex.",
        help = "Use `grep` for regex text search across files in the workspace.",
        read_only,
        filesystem_read,
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
        summary = "Apply a text patch.",
        help = "Use `apply_patch` for explicit text patch operations against workspace files.",
        mutating,
        filesystem_write,
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
