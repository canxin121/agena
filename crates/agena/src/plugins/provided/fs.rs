//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
    ViewFileToolInput,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::{PluginToolDecl, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        "agena-fs",
        "Filesystem command tools for read/search and explicit edits.",
        entries(),
        resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum FsToolInput {
    Read(ReadToolInput),
    ViewFile(ViewFileToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum FsEditToolInput {
    ApplyPatch(ApplyPatchToolInput),
    NotebookEdit(NotebookEditToolInput),
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "fs",
            crate::entry::definition::json_schema_for::<FsToolInput>(),
        )
        .description(
            "Filesystem read/search command. Set command to read, view_file, glob, or grep; pass that command's payload in args.",
        )
        .summary("Read, view, glob, or grep workspace files.")
        .help("Use `read` for raw file content, `view_file` for line-oriented viewing, `glob` for path discovery, and `grep` for regex text search. Pass the selected command payload under `args`.")
        .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead])
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "fs_edit",
            crate::entry::definition::json_schema_for::<FsEditToolInput>(),
        )
        .description(
            "Filesystem edit command. Set command to apply_patch or notebook_edit; pass that command's payload in args.",
        )
        .summary("Apply file patches or edit notebooks.")
        .help("Use `apply_patch` for text patch operations and `notebook_edit` for notebook cell edits. Pass the selected command payload under `args`; this tool mutates files and remains deferred until loaded.")
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite])
        .concurrency_safe(false)
        .deferred_load(),
    ]
}

fn resolve_entry(entry: &str, input: JsonValue) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    match entry {
        "fs" => match serde_json::from_value::<FsToolInput>(input)? {
            FsToolInput::Read(args) => tool_args("read", args),
            FsToolInput::ViewFile(args) => tool_args("view_file", args),
            FsToolInput::Glob(args) => tool_args("glob", args),
            FsToolInput::Grep(args) => tool_args("grep", args),
        },
        "fs_edit" => match serde_json::from_value::<FsEditToolInput>(input)? {
            FsEditToolInput::ApplyPatch(args) => tool_args("apply_patch", args),
            FsEditToolInput::NotebookEdit(args) => tool_args("notebook_edit", args),
        },
        other => Err(PluginError::invalid_params(format!(
            "unknown filesystem entry '{other}'"
        ))),
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
