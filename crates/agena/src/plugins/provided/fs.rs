//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{
    PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolTag,
};
use crate::plugins::provided::router;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) struct FsPlugin;

pub(crate) fn new_plugin() -> FsPlugin {
    FsPlugin
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "fs",
    description = "Filesystem command tool. Set action to read, glob, grep, apply_patch, or notebook_edit.",
    summary = "Read, search, or edit workspace files.",
    help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`). Use `glob` for path discovery, `grep` for regex text search, `apply_patch` for text patch operations, and `notebook_edit` for notebook cell edits.",
    examples(
        r#"{"action":"read","path":"Cargo.toml"}"#,
        r#"{"action":"grep","pattern":"StaticToolSurface","path":"crates"}"#
    ),
    display = detailed,
    tags(
        ToolTag::ReadOnly,
        ToolTag::Mutating,
        ToolTag::FilesystemRead,
        ToolTag::FilesystemWrite
    ),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum FsToolInput {
    #[tool(exec = "read")]
    Read {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: ReadToolInput,
    },
    #[tool(exec = "glob")]
    Glob {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: GlobToolInput,
    },
    #[tool(exec = "grep")]
    Grep {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: GrepToolInput,
    },
    #[tool(exec = "apply_patch")]
    ApplyPatch {
        #[serde(flatten)]
        args: ApplyPatchToolInput,
    },
    #[tool(exec = "notebook_edit")]
    NotebookEdit {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        args: NotebookEditToolInput,
    },
}

#[crate::plugin::sdk::plugin(
    id = FS_PLUGIN_ID,
    version = env!("CARGO_PKG_VERSION"),
    description = "Filesystem command tools for read/search and explicit edits.",
    display = detailed
)]
impl FsPlugin {
    #[tool]
    async fn tool_invoke(
        &self,
        input: FsToolInput,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        match input {
            FsToolInput::Read { args } => invoke_internal(context, "read", args),
            FsToolInput::Glob { args } => invoke_internal(context, "glob", args),
            FsToolInput::Grep { args } => invoke_internal(context, "grep", args),
            FsToolInput::ApplyPatch { args } => invoke_internal(context, "apply_patch", args),
            FsToolInput::NotebookEdit { args } => invoke_internal(context, "notebook_edit", args),
        }
    }

    #[permission(paths)]
    async fn permission_paths(&self, input: FsToolInput) -> SdkResult<Vec<PathRequest>> {
        match input {
            FsToolInput::Read { args } => permission_paths_internal("read", args),
            FsToolInput::Glob { args } => permission_paths_internal("glob", args),
            FsToolInput::Grep { args } => permission_paths_internal("grep", args),
            FsToolInput::ApplyPatch { args } => permission_paths_internal("apply_patch", args),
            FsToolInput::NotebookEdit { args } => permission_paths_internal("notebook_edit", args),
        }
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

#[cfg(test)]
mod tests {
    use super::FsToolInput;
    use crate::tool::definition::schema_usage_text;

    #[test]
    fn fs_tool_schema_includes_nested_input_docs() {
        let usage = schema_usage_text(&FsToolInput::tool_decl().input_schema)
            .expect("fs usage text should render");
        assert!(usage.contains("File or directory path to read."));
        assert!(usage.contains("Glob pattern to match."));
        assert!(usage.contains("Regex pattern to search for."));
    }
}
