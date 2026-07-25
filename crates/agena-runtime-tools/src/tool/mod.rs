pub(crate) mod apply_patch;
pub mod ask_user;
pub(crate) mod bash;
pub(crate) mod builtin_tools;
pub(crate) mod cron;
pub mod definition;
mod executor;
pub(crate) mod file_attachment;
pub(crate) mod glob;
pub(crate) mod grep;
pub(crate) mod lsp;
pub(crate) mod mcp;
pub(crate) mod orchestrator;
mod output_helpers;
pub mod payload;
pub(crate) mod powershell;
pub(crate) mod process_tool;
pub(crate) mod read;
pub mod result;
pub mod router;
pub(crate) mod shell;
pub(crate) mod shell_tools;
pub(crate) mod snapshot;
pub(crate) mod task;
pub mod tool_registry;
pub(crate) mod tool_search;
pub(crate) mod truncation;

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::agent::Agent;
use crate::message::AskUserToolInput;
use agena_domain::AccessKind;
use agena_domain::NetworkTarget;
use agena_domain::PermissionDecision;
use agena_domain::StructuredObject;
use agena_domain::ToolInvocation;
use agena_domain::ToolOutput;
use agena_plugin_host::{
    PluginHost, ToolAfterInput as PluginToolAfterInput, ToolBeforeInput as PluginToolBeforeInput,
    ToolDefinitionInput as PluginToolDefinitionInput, ToolFailureInput as PluginToolFailureInput,
    ToolInvokeInput as PluginToolInvokeInput,
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
use agena_tool::{
    PreparedShellCommand, PreparedToolInvocation, ShellError, ShellOutput, ShellRequest,
    ToolPermissionCheck,
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
use self::output_helpers::*;
pub(crate) use self::router as in_process_router;
pub use self::tool_registry::*;

pub use crate::{MonitorError, MonitorRead, MonitorReadParams, MonitorService, MonitorStartParams};
pub use builtin_tools::BuiltinToolSet;
pub use payload::{ToolPayloadInput, ToolPayloadOutput};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use snapshot::registry_for_executor as snapshot_registry_for_executor;
pub use tool_registry::{
    ExecutionPermissionInspector, ExecutionTool, ToolApiBinding, ToolError, ToolExecutor,
    ToolExposure, tool_exposure,
};
pub(crate) use truncation::ToolOutputTruncator;

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
        permission::{PermissionPolicy, ToolPermissionPolicy},
    };
    use agena_domain::PermissionMode;
    use agena_plugin_host::{
        ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig,
        StaticPluginRegistration, ToolPresentationConfig,
    };

    #[derive(Default)]
    struct ChokePointPlugin;

    #[derive(Default)]
    struct ToolApiFixture;

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "agena",
        name = "tools",
        version = "test",
        summary = "Tool API registry fixture."
    )]
    impl ToolApiFixture {
        #[tool(name = "list", summary = "List tools.")]
        async fn list(&self, _input: &crate::message::AskUserToolInput) -> String {
            String::new()
        }

        #[tool(name = "search", summary = "Search tools.")]
        async fn search(&self, _input: &crate::message::AskUserToolInput) -> String {
            String::new()
        }

        #[tool(name = "help", summary = "Describe a tool.")]
        async fn help(&self, _input: &crate::message::AskUserToolInput) -> String {
            String::new()
        }

        #[tool(name = "tags", summary = "List tool tags.")]
        async fn tags(&self, _input: &crate::message::AskUserToolInput) -> String {
            String::new()
        }

        #[tool(name = "call", summary = "Call a tool.")]
        async fn call(&self, _input: &crate::message::AskUserToolInput) -> String {
            String::new()
        }
    }

    #[agena_plugin_host::sdk::agena_plugin(
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
            .canonical_name()
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_tool_api_is_permission_transparent_but_execution_tools_are_not() {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "agena.tools".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config
            .list
            .insert("test.choke".to_string(), ConfiguredPlugin::static_default());
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    "agena.tools".parse().expect("valid Tool API plugin key"),
                    ToolApiFixture,
                ),
                StaticPluginRegistration::new(
                    "test.choke".parse().expect("valid test plugin key"),
                    ChokePointPlugin,
                ),
            ],
            config: plugins_config,
            workspace_root: workspace_root.clone(),
            agena_version: "test".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .expect("build Tool API test plugin host");
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

        let bindings = executor.available_tool_api_bindings();
        assert_eq!(
            bindings.len(),
            5,
            "all provider Tool API functions remain visible"
        );
        for binding in bindings {
            let mut invocation =
                ToolInvocation::new(binding.function_name(), StructuredObject::default());
            invocation.tool_api_function = Some(binding.function());
            assert!(
                executor
                    .collect_permission_checks_for_invocation(&invocation)
                    .expect("collect Tool API permission checks")
                    .is_empty(),
                "provider function {} must not enter the permission system",
                binding.function_name()
            );
        }

        let execution_tool = plugins
            .lookup_tool("test.choke.run")
            .expect("execution tool is registered");
        let checks = executor
            .collect_permission_checks_for_invocation(&ToolInvocation::new(
                execution_tool.canonical_name(),
                StructuredObject::default(),
            ))
            .expect("collect execution-tool permission checks");
        assert_eq!(checks.len(), 1);
        assert!(matches!(
            checks[0].decision,
            agena_domain::PermissionDecision::Ask { .. }
        ));

        let denied_execution_tools = ToolExecutor::new(
            std::env::current_dir().expect("resolve test workspace"),
            Agent::new(
                "deny-execution",
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::new(PermissionMode::Deny),
            ),
            SubagentRegistry::default(),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        assert_eq!(
            denied_execution_tools.available_tool_api_bindings().len(),
            5,
            "execution-tool deny rules must not hide provider protocol functions"
        );

        let no_tools_agent = Agent::new(
            "no-tools",
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        )
        .restricted_to_allowed_tools(["__agena_no_tools__"]);
        let no_tools_executor = ToolExecutor::new(
            std::env::current_dir().expect("resolve test workspace"),
            no_tools_agent,
            SubagentRegistry::default(),
            plugins,
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        assert!(
            no_tools_executor.available_tool_api_bindings().is_empty(),
            "an explicit no-tools capability must still remove the Tool API"
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
        let canonical_external = external
            .canonicalize()
            .expect("canonicalize expected symlink target");
        assert_eq!(resolved, canonical_external.join("new.txt"));

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
