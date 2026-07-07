//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolTag,
};
use crate::plugins::provided::router;
use agena_macros::{StaticToolSurface, ToolSuite};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) struct FsPlugin;

pub(crate) fn new_plugin() -> FsPlugin {
    FsPlugin
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "read",
    summary = "Read workspace files.",
    help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`).",
    handler_receiver = FsPlugin,
    handle_with_context = FsPlugin::invoke_read,
    handle_field = args,
    permission_paths_handle = FsPlugin::permission_read,
    handle_by_value = true,
    examples(r#"{"path":"Cargo.toml"}"#),
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct FsReadToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: ReadToolInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "glob",
    summary = "Find paths with glob patterns.",
    help = "Use `glob` for path discovery before reading or editing files.",
    handler_receiver = FsPlugin,
    handle_with_context = FsPlugin::invoke_glob,
    handle_field = args,
    permission_paths_handle = FsPlugin::permission_glob,
    handle_by_value = true,
    examples(r#"{"pattern":"**/*.rs","path":"crates"}"#),
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct FsGlobToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: GlobToolInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "grep",
    summary = "Search file contents with regex.",
    help = "Use `grep` for regex text search across files in the workspace.",
    handler_receiver = FsPlugin,
    handle_with_context = FsPlugin::invoke_grep,
    handle_field = args,
    permission_paths_handle = FsPlugin::permission_grep,
    handle_by_value = true,
    examples(r#"{"pattern":"StaticToolSurface","path":"crates"}"#),
    display = detailed,
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct FsGrepToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: GrepToolInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "apply_patch",
    summary = "Apply a text patch.",
    help = "Use `apply_patch` for explicit text patch operations against workspace files.",
    handler_receiver = FsPlugin,
    handle_with_context = FsPlugin::invoke_apply_patch,
    handle_field = args,
    permission_paths_handle = FsPlugin::permission_apply_patch,
    handle_by_value = true,
    display = detailed,
    tags(ToolTag::Mutating, ToolTag::FilesystemWrite),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct FsApplyPatchToolInput {
    #[serde(flatten)]
    args: ApplyPatchToolInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = FsPlugin)]
enum FsToolSuite {
    Read(FsReadToolInput),
    Glob(FsGlobToolInput),
    Grep(FsGrepToolInput),
    ApplyPatch(FsApplyPatchToolInput),
}

#[crate::plugin::sdk::plugin(
    namespace = "agena",
    name = "fs",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Filesystem command tools for read/search and explicit edits.",
    display = detailed
)]
impl FsPlugin {
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "read", args)
    }

    async fn invoke_glob(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GlobToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "glob", args)
    }

    async fn invoke_grep(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GrepToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "grep", args)
    }

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

    #[tool_suite]
    async fn tool_invoke(
        &self,
        input: FsToolSuite,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        input.dispatch_tool_invoke_with_context(self, context).await
    }

    #[permission(paths, suite)]
    async fn permission_paths(&self, input: FsToolSuite) -> SdkResult<Vec<PathRequest>> {
        input.dispatch_permission_paths(self).await
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
