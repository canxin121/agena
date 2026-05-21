//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
    ViewFileToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::ToolTag;
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        "agena-fs",
        "Filesystem command tools for read/search and explicit edits.",
        vec![FsToolInput::tool_decl()],
        FsToolInput::resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "fs",
    description = "Filesystem command tool. Set action to read, glob, grep, apply_patch, or notebook_edit.",
    summary = "Read, search, or edit workspace files.",
    help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`). Use `glob` for path discovery, `grep` for regex text search, `apply_patch` for text patch operations, and `notebook_edit` for notebook cell edits. Legacy `view_file` and `command/args` inputs are still accepted for compatibility.",
    tags(
        ToolTag::ReadOnly,
        ToolTag::Mutating,
        ToolTag::FilesystemRead,
        ToolTag::FilesystemWrite
    ),
    legacy_entries("fs_edit"),
    concurrency_safe = true,
    load = "always",
    fallback = parse_legacy_fs_input
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum FsToolInput {
    #[tool(exec = "read")]
    Read {
        #[serde(flatten)]
        args: ReadToolInput,
    },
    #[tool(exec = "glob")]
    Glob {
        #[serde(flatten)]
        args: GlobToolInput,
    },
    #[tool(exec = "grep")]
    Grep {
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
        #[serde(flatten)]
        args: NotebookEditToolInput,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum LegacyFsToolInput {
    Read(ReadToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    ApplyPatch(ApplyPatchToolInput),
    NotebookEdit(NotebookEditToolInput),
    ViewFile(ViewFileToolInput),
}

fn parse_legacy_fs_input(
    input: JsonValue,
    primary: PluginError,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    if input
        .get("action")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|action| action == "view_file")
    {
        let JsonValue::Object(mut object) = input.clone() else {
            return Err(primary);
        };
        object.remove("action");
        let args = serde_json::from_value::<ViewFileToolInput>(JsonValue::Object(object))
            .map_err(|_| primary.clone())?;
        return tool_args("view_file", args);
    }

    match serde_json::from_value::<LegacyFsToolInput>(input) {
        Ok(LegacyFsToolInput::Read(args)) => tool_args("read", args),
        Ok(LegacyFsToolInput::Glob(args)) => tool_args("glob", args),
        Ok(LegacyFsToolInput::Grep(args)) => tool_args("grep", args),
        Ok(LegacyFsToolInput::ApplyPatch(args)) => tool_args("apply_patch", args),
        Ok(LegacyFsToolInput::NotebookEdit(args)) => tool_args("notebook_edit", args),
        Ok(LegacyFsToolInput::ViewFile(args)) => tool_args("view_file", args),
        Err(_) => Err(primary),
    }
}

fn tool_args<T: serde::Serialize>(
    tool: &str,
    args: T,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    Ok((
        tool.to_string(),
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    ))
}
