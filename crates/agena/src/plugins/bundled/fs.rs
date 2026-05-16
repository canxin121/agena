//! First-party `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
    ViewFileToolInput,
};
use crate::plugin::sdk::manifest::{InputPathSpec, PathKind};
use crate::plugin::sdk::{PluginToolDecl, ToolTag};
use crate::plugins::bundled::router::BundledRouterPlugin;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) fn new_plugin() -> BundledRouterPlugin {
    BundledRouterPlugin::new(
        "agena-fs",
        "Filesystem tools (read, view_file, glob, grep, apply_patch, notebook_edit).",
        entries(),
    )
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "read",
            crate::entry::definition::json_schema_for::<ReadToolInput>(),
        )
        .description("Read a UTF-8 text file or list a directory with optional pagination.")
        .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead])
        .input_path(required_path("$.file_path", PathKind::Read))
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "view_file",
            crate::entry::definition::json_schema_for::<ViewFileToolInput>(),
        )
        .description(
            "Load a local file and attach it back to the conversation as inline multimodal input.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead])
        .input_path(required_path("$.path", PathKind::Read))
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "glob",
            crate::entry::definition::json_schema_for::<GlobToolInput>(),
        )
        .description("Search files by glob pattern from the workspace or a subdirectory.")
        .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead])
        .input_path(optional_path("$.path", PathKind::Read))
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "grep",
            crate::entry::definition::json_schema_for::<GrepToolInput>(),
        )
        .description("Search file contents by regex pattern with optional include glob.")
        .tags([ToolTag::ReadOnly, ToolTag::FilesystemRead])
        .input_path(optional_path("$.path", PathKind::Read))
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "apply_patch",
            crate::entry::definition::json_schema_for::<ApplyPatchToolInput>(),
        )
        .description("Apply a structured patch that can add, update, move, or delete files.")
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite])
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "notebook_edit",
            crate::entry::definition::json_schema_for::<NotebookEditToolInput>(),
        )
        .description("Edit a Jupyter .ipynb cell by replacing, inserting, or deleting a cell.")
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite])
        .input_path(required_path("$.notebook_path", PathKind::Write))
        .concurrency_safe(false)
        .deferred_load(),
    ]
}

fn required_path(jsonpath: &str, kind: PathKind) -> InputPathSpec {
    InputPathSpec {
        jsonpath: jsonpath.to_string(),
        kind,
        optional: false,
    }
}

fn optional_path(jsonpath: &str, kind: PathKind) -> InputPathSpec {
    InputPathSpec {
        jsonpath: jsonpath.to_string(),
        kind,
        optional: true,
    }
}
