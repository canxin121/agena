//! First-party `agena.fs` plugin: filesystem read/write/search tools.

use crate::message::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, NotebookEditToolInput, ReadToolInput,
    ViewFileToolInput,
};
use crate::plugin::sdk::manifest::{InputPathSpec, PathKind};
use crate::plugin::sdk::{EntryBehavior as SdkEntryBehavior, PluginEntryDecl};
use crate::plugins::bundled::router::FirstPartyRouterPlugin;

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";

pub(crate) fn new_plugin() -> FirstPartyRouterPlugin {
    FirstPartyRouterPlugin::new(
        "agena-fs",
        "Filesystem tools (read, view_file, glob, grep, apply_patch, notebook_edit).",
        entries(),
    )
}

fn entries() -> Vec<PluginEntryDecl> {
    vec![
        PluginEntryDecl::new(
            "read",
            crate::entry::definition::json_schema_for::<ReadToolInput>(),
        )
        .description("Read a UTF-8 text file or list a directory with optional pagination.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .input_path(required_path("$.file_path", PathKind::Read))
        .search_terms(["open file", "view file", "cat", "inspect"])
        .always_load(),
        PluginEntryDecl::new(
            "view_file",
            crate::entry::definition::json_schema_for::<ViewFileToolInput>(),
        )
        .description(
            "Load a local file and attach it back to the conversation as inline multimodal input.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .input_path(required_path("$.path", PathKind::Read))
        .search_terms(["file", "image", "pdf", "audio", "document"])
        .always_load(),
        PluginEntryDecl::new(
            "glob",
            crate::entry::definition::json_schema_for::<GlobToolInput>(),
        )
        .description("Search files by glob pattern from the workspace or a subdirectory.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .input_path(optional_path("$.path", PathKind::Read))
        .search_terms(["find files", "list files", "pattern search"])
        .always_load(),
        PluginEntryDecl::new(
            "grep",
            crate::entry::definition::json_schema_for::<GrepToolInput>(),
        )
        .description("Search file contents by regex pattern with optional include glob.")
        .behavior(SdkEntryBehavior::ReadOnly)
        .input_path(optional_path("$.path", PathKind::Read))
        .search_terms(["search text", "regex search", "ripgrep"])
        .always_load(),
        PluginEntryDecl::new(
            "apply_patch",
            crate::entry::definition::json_schema_for::<ApplyPatchToolInput>(),
        )
        .description("Apply a structured patch that can add, update, move, or delete files.")
        .behavior(SdkEntryBehavior::Mutating)
        .search_terms(["patch", "diff", "multi-file edit"])
        .deferred_load(),
        PluginEntryDecl::new(
            "notebook_edit",
            crate::entry::definition::json_schema_for::<NotebookEditToolInput>(),
        )
        .description("Edit a Jupyter .ipynb cell by replacing, inserting, or deleting a cell.")
        .behavior(SdkEntryBehavior::Mutating)
        .input_path(required_path("$.notebook_path", PathKind::Write))
        .search_terms(["notebook", "jupyter", "ipynb", "cell edit"])
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
