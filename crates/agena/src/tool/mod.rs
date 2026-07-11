pub(crate) mod apply_patch;
pub(crate) mod ask_user;
pub(crate) mod bash;
pub(crate) mod catalog;
pub(crate) mod cron;
pub(crate) mod definition;
mod executor;
pub(crate) mod file_attachment;
pub(crate) mod glob;
pub(crate) mod grep;
pub(crate) mod lsp;
pub(crate) mod monitor;
pub(crate) mod orchestrator;
mod output_helpers;
pub(crate) mod payload;
pub(crate) mod powershell;
pub(crate) mod process_tool;
pub(crate) mod read;
pub(crate) mod result;
pub(crate) mod shell;
pub(crate) mod shell_tools;
pub(crate) mod snapshot;
pub(crate) mod task;
mod tool_registry;
pub(crate) mod tool_search;
pub(crate) mod truncation;

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    AskUserToolInput, FilesystemEffect, Message, NetworkEffect, PluginInvocation, StructuredObject,
    ToolInvocation, ToolOutput,
};
use crate::permission::{AccessKind, NetworkTarget, PermissionAction, PermissionDecision};
use crate::plugin::{
    PluginHost, PluginHostBuildConfig, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolDefinitionInput as PluginToolDefinitionInput,
    ToolFailureInput as PluginToolFailureInput, ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionNetworksInput as PluginToolPermissionNetworksInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    registry::RegisteredTool,
    sdk::{
        InputNetworkSpec as SdkInputNetworkSpec, InputPathSpec as SdkInputPathSpec,
        NetworkAccessSpec as SdkNetworkAccessSpec, PathAccessSpec as SdkPathAccessSpec,
        PathKind as SdkPathKind, ShellEnvInput as PluginShellEnvInput,
        ToolResultPolicy as SdkToolResultPolicy, ToolStreamingMode as SdkToolStreamingMode,
    },
};
use crate::plugins::provided::{
    catalog as provided_catalog, code as provided_code, cron as provided_cron, fs as provided_fs,
    lsp as provided_lsp, mcp, planning as provided_planning, process as provided_process,
    repo as provided_repo, router as in_process_router, runtime as provided_runtime,
    schema_lab as provided_schema_lab, settings as provided_settings, skills,
    tasks as provided_tasks,
};

// Model-facing tool results must be small enough that a sequence of noisy
// commands cannot consume the whole context window.  The complete result is
// durably stored under `.agena/tool-results`; this preview deliberately keeps
// both the beginning and end, as those normally contain the command setup and
// final diagnostics respectively.
const TOOL_MODEL_OUTPUT_MAX_LINES: usize = 400;
const TOOL_MODEL_OUTPUT_MAX_BYTES: usize = 16 * 1024;
const TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES: usize = 12 * 1024;
const TOOL_MODEL_STRUCTURED_MAX_DEPTH: usize = 6;
const TOOL_MODEL_STRUCTURED_MAX_FIELDS: usize = 32;
const TOOL_MODEL_STRUCTURED_MAX_ITEMS: usize = 32;
const TOOL_MODEL_STRUCTURED_STRING_MAX_BYTES: usize = 768;
const MODEL_TOOLS_LIST: &str = "tools_list";
const MODEL_TOOLS_SEARCH: &str = "tools_search";
const MODEL_TOOLS_HELP: &str = "tools_help";
const MODEL_TOOLS_TAGS: &str = "tools_tags";
const MODEL_TOOLS_CALL: &str = "tools_call";

use self::output_helpers::*;
pub use self::tool_registry::*;

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use payload::{CronJobSummary, ToolPayloadInput, ToolPayloadOutput, WebSearchHit};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use shell::{ShellError, ShellOutput, ShellRequest};
pub use snapshot::{
    ActiveSnapshot, ManagedSnapshot, SnapshotBackend, SnapshotBackendCapabilities,
    SnapshotBackendSupport, SnapshotRegistry,
    backend_capabilities as snapshot_backend_capabilities, list_active as snapshot_list_active,
    list_managed as snapshot_list_managed, prune_stale as snapshot_prune_stale,
    registry_for_executor as snapshot_registry_for_executor,
};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::{
        StructuredObject, TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES, ToolError, ToolExecutor,
        ToolInvocation, ToolOutput, bounded_model_output_preview, canonicalize_path_for_execution,
        compact_tool_output_payload_for_model, line_count,
    };
    use crate::{
        agent::Agent,
        agents::SubagentRegistry,
        permission::{PermissionMode, PermissionPolicy, ToolPermissionPolicy},
        plugin::{
            ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig,
            StaticPluginRegistration, ToolPresentationConfig,
        },
    };

    #[derive(Default)]
    struct ChokePointPlugin;

    #[crate::plugin::sdk::agena_plugin(
        namespace = "test",
        name = "choke",
        version = "0.1.0",
        summary = "Permission choke-point regression fixture."
    )]
    impl ChokePointPlugin {
        #[tool(name = "run", summary = "Run the regression fixture.")]
        async fn run(&self) -> String {
            "should not execute when denied".to_string()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detailed_execution_enforces_permissions_without_caller_preflight() {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config
            .list
            .insert("test.choke".to_string(), ConfiguredPlugin::static_default());
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "test.choke".parse().expect("valid test plugin key"),
                ChokePointPlugin,
            )],
            config: plugins_config,
            workspace_root: workspace_root.clone(),
            agena_version: "test".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .expect("build test plugin host");
        let executor = ToolExecutor::new(
            workspace_root,
            Agent::new(
                "test",
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::new(PermissionMode::Ask),
            ),
            SubagentRegistry::default(),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let tool_name = plugins
            .registered_tools()
            .into_iter()
            .next()
            .expect("test plugin registers one tool")
            .model_name()
            .to_string();
        let invocation = ToolInvocation::new(tool_name, StructuredObject::default());

        let error = executor
            .execute_invocation_detailed(&invocation, 1, 1)
            .expect_err("the final execution boundary must reject unapproved tools");

        assert!(
            matches!(error, ToolError::PermissionAsk(_)),
            "expected final-boundary permission prompt, got: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_path_resolution_follows_existing_symlink_parents() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("agena-path-test-{}", uuid::Uuid::new_v4()));
        let external = root.join("external");
        std::fs::create_dir_all(&external).expect("create test directories");
        symlink(&external, root.join("workspace-link")).expect("create test symlink");

        let resolved = canonicalize_path_for_execution(&root.join("workspace-link/new.txt"));
        assert_eq!(resolved, external.join("new.txt"));

        std::fs::remove_dir_all(root).expect("remove test directories");
    }

    #[test]
    fn bounded_model_output_preview_keeps_the_head_and_tail() {
        let output = (0..20)
            .map(|index| format!("line-{index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let preview = bounded_model_output_preview(&output, "[full output: saved.txt]", 9, 120);

        assert!(preview.contains("line-00"));
        assert!(preview.contains("line-19"));
        assert!(preview.contains("[full output: saved.txt]"));
        assert!(line_count(&preview) <= 9);
        assert!(preview.len() <= 120);
    }

    #[test]
    fn compact_tool_payload_preserves_patch_changes_without_the_full_diff() {
        let payload = serde_json::json!({
            "changes": [{ "path": "src/lib.rs", "kind": "updated" }],
            "diff": "x".repeat(TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES * 2),
        });
        let mut output = ToolOutput::from_json_payload(Some(&payload)).expect("tool output");

        compact_tool_output_payload_for_model(&mut output, ".agena/tool-results/full.txt", 50_000)
            .expect("compact payload");

        let compacted = output.to_json_payload().expect("compacted payload");
        let serialized = serde_json::to_string(&compacted).expect("serialize compact payload");
        assert!(serialized.len() <= TOOL_MODEL_STRUCTURED_OUTPUT_MAX_BYTES);
        assert_eq!(
            compacted
                .pointer("/changes/0/path")
                .and_then(serde_json::Value::as_str),
            Some("src/lib.rs")
        );
        assert!(
            compacted
                .pointer("/diff")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|diff| diff.contains("full output persisted"))
        );
    }
}
