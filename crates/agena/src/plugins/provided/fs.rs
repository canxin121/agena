//! `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
};
use crate::plugin::sdk::ToolTag;
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::Deserialize;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        FS_PLUGIN_ID,
        "Filesystem command tools for read/search and explicit edits.",
        vec![FsToolInput::tool_decl()],
        FsToolInput::resolve_tool,
    )
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "fs",
    description = "Filesystem command tool. Set action to read, glob, grep, apply_patch, or notebook_edit.",
    summary = "Read, search, or edit workspace files.",
    help = "Use `read` for text previews, directory listings, or file attachments via `mode = text|attachment|auto` (default `auto`). Use `glob` for path discovery, `grep` for regex text search, `apply_patch` for text patch operations, and `notebook_edit` for notebook cell edits.",
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
