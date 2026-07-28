#![allow(unused_imports)]

//! Runtime primitives, concrete adapters, and application composition for Agena.

extern crate self as agena_runtime;

pub use agena_runtime_contracts::{agent, agents};
mod config;
pub use agena_runtime_contracts::message;
pub use agena_runtime_session::AppError;
pub use agena_runtime_session::event;
mod model_catalog;
pub use agena_runtime_contracts::permission;
pub mod plugins {
    pub use agena_bundled_plugins::plugins::{provided, sources};
    pub use agena_runtime_plugins::plugins::storage;
}
pub use agena_runtime_provider::provider;
mod runtime;
pub use agena_runtime_session as session;
pub mod tool {
    pub use agena_runtime_tools::tool::*;
}

mod application_services;
mod background_task;
mod background_task_completion;
mod background_task_registry;
mod background_task_spec;
mod background_task_state;
mod bootstrap_error;
mod bootstrap_request;
mod bootstrap_result;
mod composition;
mod connect;
mod control_state;
mod invocation_guard;
mod lsp_config;
mod mcp_runtime;
mod model_catalog_cache;
mod model_catalog_composition;
mod model_catalog_curation;
mod model_catalog_http;
mod model_catalog_live;
mod model_catalog_runtime_service;
mod model_catalog_service;
mod model_catalog_source;
mod oauth_callback;
mod optional;
mod output_format;
mod periodic;
mod permission_runtime;
mod permission_store;
mod plugin_composition;
mod policy;
mod process_state;
mod provider_composition;
mod refresh;
mod refresh_policy;
mod registration;
mod reload;
mod reload_gate;
mod reload_watch;
mod runtime_authentication_service;
mod runtime_composition_config;
mod runtime_control_service;
mod runtime_draft_authentication_service;
mod runtime_status_service;
mod runtime_tool_execution_service;
mod scheduler_composition;
mod services;
mod snapshot;
mod snapshot_state;
mod staleness;
mod store;
mod task_state;
mod tracing_config;
mod watch;
mod watch_paths;

pub use permission_runtime::{
    PermissionRuntime, PermissionRuntimeDecision, PermissionRuntimeError,
};
pub use permission_store::{PermissionRuleStore, PermissionStoreError};
pub use runtime::bootstrap_application_services;

pub(crate) use agena_bundled_plugins::tool::{memory_plugin_id, new_memory_plugin};
pub use agena_runtime_config::default_config_path;
pub use agena_runtime_config::runtime_config_settings_service::list_json_path;
pub use agena_runtime_config::{
    AgentConfig, AmazonBedrockProviderOptions, AnthropicProviderOptions, AppliedLayer,
    BrowserHarnessConfig, ConfigResolution, ConfigResolutionMeta, ConfigSource,
    EditorHarnessConfig, GeminiProviderOptions, GitlabProviderOptions, HarnessViewportConfig,
    HarnessesConfig, HttpProviderAdapterConfig, OllamaProviderOptions,
    OpenAiChatCompletionsProviderOptions, OpenAiRealtimeProviderOptions,
    OpenAiResponsesProviderOptions, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderClientVersionSettings, ProviderDefaultsConfig,
    ProviderGitlabAuthConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, RuntimeConfig, RuntimeProvidersConfig, SessionCompactionConfig,
    SessionConfig, ShellHarnessConfig, SimpleHttpProviderOptions, TuiColorSchemeConfig,
    TuiGraphicsModeConfig, TuiUiConfig, UiConfig, config_resolution_json_value,
    resolved_config_json_value,
};
pub use agena_runtime_config::{ConfigEnvironment, ProcessEnvironment};
pub(crate) use agena_runtime_config::{
    ConfigError, apply_config_env_number, config_error_to_settings_error, merge_optional_config,
    normalize_config_optional, parse_config_bool, parse_config_json, read_config_json,
    reject_unsupported_mode_environment, settings_error_to_config_error,
};
pub(crate) use agena_runtime_config::{
    ConfigOverride, LoadConfigRequest, RuntimeConfigOverrideError,
    parse_config_override_expressions,
};
pub use agena_runtime_config::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsGetInput, ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsListItem,
    ConfigSettingsListResponse, ConfigSettingsPatchInput, ConfigSettingsPathInput,
    ConfigSettingsReadResponse, ConfigSettingsReloadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateInput, ConfigSettingsValidateResponse,
    RuntimeConfigSettingsError, RuntimeConfigSettingsErrorKind, RuntimeConfigSettingsService,
    RuntimeSettingsDocumentValidator, config_settings_layer_path, delete_runtime_file_setting,
    list_runtime_file_settings, patch_runtime_file_settings, read_runtime_file_setting,
    set_runtime_file_setting, validate_runtime_file_settings,
};
pub(crate) use agena_runtime_config::{LSP_PLUGIN_ID, LspConfig};
pub use agena_runtime_config::{
    RuntimeConfigurationError, RuntimeConfigurationService, RuntimeConfigurationSnapshot,
    RuntimeTuiColorScheme, RuntimeTuiGraphicsMode, RuntimeUiConfiguration,
};
pub(crate) use agena_runtime_config::{default_workspace_root, project_config_path};
pub(crate) use agena_runtime_plugins::CallbackOnDrop;
pub(crate) use agena_runtime_plugins::plugin_config::{
    dispatch_config_if_nonempty, merge_bundled_plugin_config,
};
pub(crate) use agena_runtime_plugins::plugin_runtime_service::dispatch_plugin_rpc;
pub use agena_runtime_plugins::plugin_runtime_service::{
    PluginRuntimeRpcError, PluginRuntimeService, PluginToolDescriptor, RuntimePluginToolCatalogItem,
};
pub(crate) use agena_runtime_plugins::plugin_shutdown::plugin_shutdown_guard;
pub(crate) use agena_runtime_plugins::plugin_slot::{current_plugin_host, install_plugin_host};
pub(crate) use agena_runtime_provider::fetch_latest_provider_client_versions;
pub(crate) use agena_runtime_provider::provider_model_catalog_priorities;
pub(crate) use agena_runtime_provider::{
    JsonEventPayload, ProviderJsonStreamError, json_events, json_events_with_done, json_lines,
};
pub(crate) use agena_runtime_provider::{
    RUNTIME_CODEX_MCP_CLIENT_NAME, claude_code_api_user_agent, claude_user_web_fetch_user_agent,
    codex_package_version, codex_user_agent, gemini_cli_user_agent, set_provider_client_versions,
};
pub(crate) use agena_runtime_provider::{RUNTIME_CODEX_ORIGINATOR, runtime_codex_user_agent};
pub(crate) use agena_runtime_provider::{configured_enabled_adapter_ids, configured_local_models};
pub(crate) use agena_runtime_session::ContextGovernor;
pub use agena_runtime_session::RuntimeMetricsSnapshot;
pub(crate) use agena_runtime_session::RuntimeSessionManagerConfig;
pub(crate) use agena_runtime_session::SessionCachePolicy;
pub(crate) use agena_runtime_session::merge_system_prompts;
pub(crate) use agena_runtime_session::resolve_installation_id;
pub(crate) use agena_runtime_session::run_session_maintenance;
pub(crate) use agena_runtime_session::runtime_metrics_snapshot;
pub(crate) use agena_runtime_session::{
    APPROX_CHARS_PER_TOKEN, MIN_PROMPT_BUDGET_TOKENS, estimate_prompt_tokens_from_chars,
    prompt_token_budget,
};
pub(crate) use agena_runtime_session::{AbortOnDrop, spawn_abortable, spawn_detached};
pub(crate) use agena_runtime_session::{CompletionRequestInputs, build_completion_request};
pub(crate) use agena_runtime_session::{
    DEFAULT_COMPACTION_OUTPUT_TOKENS, MAX_COMPACTION_FAILURES, MAX_COMPACTOR_MESSAGE_CHARS,
    MAX_RECENT_CONTEXT_CHARS, MAX_RECENT_USER_TURNS,
};
pub(crate) use agena_runtime_session::{
    ExecutionControl, ExecutionControlError, ExecutionRegistry,
};
pub use agena_runtime_session::{
    RuntimeActiveSnapshot, RuntimeManagedSnapshot, RuntimeSnapshotStatus, SessionExecutionControl,
    SessionExecutionControlError,
};
pub use agena_runtime_session::{
    RuntimeEvent, RuntimeEventQueryError, RuntimeEventQueryService, RuntimeEventRange,
    RuntimeEventStreamService, RuntimeLiveEventSubscription, RuntimeLiveEventSubscriptionItem,
    RuntimeReverseEventRange, RuntimeTimelineDetailLine, RuntimeTimelineEvent,
};
pub use agena_runtime_session::{
    RuntimeEventPublishError, RuntimeEventPublishRequest, RuntimeEventPublishService,
};
pub(crate) use agena_runtime_session::{
    RuntimeEventSubscription, RuntimeEventSubscriptionItem, spawn_event_forwarder,
};
pub use agena_runtime_session::{
    RuntimeLivePresentationSubscription, RuntimeLivePresentationSubscriptionItem,
    RuntimeMessageMetadata, RuntimeMessagePartCheckpoint, RuntimePresentationEvent,
    RuntimePresentationEventKind,
};
pub use agena_runtime_session::{
    SessionAgentRestoreOutcome, SessionAgentSwitchOutcome, SessionCreateRequest,
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest,
    SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions, SessionUserMessagePart,
    SessionUserMessageRequest,
};
pub use agena_runtime_session::{
    SessionExecutionCommandError, SessionExecutionCommandOutcome, SessionExecutionCommandService,
};
pub use agena_runtime_session::{
    SessionExecutionContext, SessionPresentation, SessionProjectedActivityError,
    SessionProjectedActivityKind, SessionProjectedActivityPart, SessionProjectedMessage,
    SessionProjectedMessageHeader, SessionProjectedMessagePart, SessionProjectedModelVisibleOutput,
    SessionProjectedOperationBlock, SessionProjectedOperationPart, SessionProjectedPartDetail,
    SessionProjectedToolResult, SessionQueryError, SessionQueryService,
};
pub use agena_runtime_session::{
    SessionPluginCommandError, SessionPluginCommandRequest, SessionPluginCommandService,
};
pub use agena_runtime_session::{SessionToolExecutionError, SessionToolExecutionService};
pub(crate) use agena_runtime_session::{
    estimate_auto_compaction_limit_tokens, estimate_auto_compaction_reserve_tokens,
    estimate_prompt_budget_threshold_tokens, estimate_session_context_usable_tokens,
};
pub(crate) use agena_runtime_session::{
    record_provider_call, record_provider_stream, record_tool_execution, session_finished,
    session_started,
};
pub(crate) use agena_runtime_tools::list_managed_snapshots;
pub(crate) use agena_runtime_tools::prune_stale_managed_snapshots;
pub(crate) use agena_runtime_tools::snapshot_backend_capabilities;
pub(crate) use agena_runtime_tools::snapshot_rift_binary;
pub(crate) use agena_runtime_tools::truncate_tool_output_text;
pub(crate) use agena_runtime_tools::{
    MonitorError, MonitorRead, MonitorReadParams, MonitorService, MonitorStart, MonitorStartParams,
    MonitorStopOutcome, default_monitor_registry,
};
pub(crate) use agena_runtime_tools::{
    SnapshotCreation, attach_existing_snapshot, create_managed_snapshot, remove_managed_snapshot,
    snapshot_has_local_changes,
};
pub(crate) use agena_runtime_tools::{
    SnapshotRegistry, SnapshotSession, list_active_snapshots, snapshot_registry,
};
pub(crate) use agena_runtime_tools::{
    agena_home_dir, generated_image_artifact_path, project_state_dir, snapshot_managed_dir,
    snapshot_rift_database_path,
};
pub use application_services::{RuntimeApplicationRepositories, RuntimeApplicationServices};
pub(crate) use application_services::{
    RuntimeApplicationServiceCompositionInputs, compose_runtime_application_services,
};
pub use background_task::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskStart,
    RuntimeBackgroundTaskStatus,
};
pub(crate) use background_task_completion::RuntimeBackgroundTaskCompletion;
pub(crate) use background_task_registry::RuntimeBackgroundTaskRegistry;
pub(crate) use background_task_spec::RuntimeBackgroundTaskSpec;
pub(crate) use background_task_state::RuntimeBackgroundTaskState;
pub use bootstrap_error::{RuntimeBootstrapError, RuntimeBootstrapErrorKind};
pub use bootstrap_request::{
    RuntimeBootstrapPreflight, RuntimeBootstrapRequest, resolve_runtime_bootstrap_preflight,
};
pub use bootstrap_result::RuntimeBootstrapResult;
pub(crate) use bootstrap_result::{
    RuntimeBootstrapComposition, RuntimeBootstrapLifecycle, compose_runtime_bootstrap,
};
pub(crate) use composition::{
    DatabaseCompositionInputs, ModelCatalogCompositionInputs, ModelCatalogRuntimeConfig,
    PluginCompositionInputs, RuntimeSessionBuildConfig, RuntimeSnapshotCompositionInputs,
    SessionCompositionInputs, ToolCompositionInputs, compose_runtime_snapshot_state,
    session_build_config_from_resolved, spawn_runtime_maintenance_loops,
};
pub(crate) use connect::connect_or_initialize;
pub(crate) use control_state::RuntimeControlState;
pub(crate) use invocation_guard::try_enter_invocation;
pub(crate) use lsp_config::compose_lsp_services;
pub(crate) use mcp_runtime::{
    MCP_PLUGIN_ID, McpConfig, McpRuntimeConfig, build_configured_mcp_manager,
    mcp_config_from_plugins,
};
pub(crate) use model_catalog_cache::{
    ModelCatalogCacheCodecError, model_catalog_cache_record_from_document,
    model_catalog_snapshot_from_cache_record,
};
pub(crate) use model_catalog_composition::{
    ModelCatalogCompositionError, ModelCatalogPublicSourceResult, compose_model_catalog_document,
};
pub(crate) use model_catalog_curation::{
    ModelCatalogCurationError, curate_catalog_document, curate_live_catalog_document,
};
pub(crate) use model_catalog_http::build_default_public_model_catalog_source;
pub(crate) use model_catalog_live::{
    LiveProviderCatalogBuildError, build_live_provider_catalog_document,
};
pub use model_catalog_runtime_service::{ModelCatalogRefreshError, ModelCatalogRuntimeService};
pub(crate) use model_catalog_service::{ModelCatalogPublicSource, ModelCatalogService};
pub(crate) use model_catalog_source::{
    ModelCatalogConfiguredPublicSource, ModelCatalogRemoteDocumentFetcher,
    ModelCatalogRemoteSource, ModelCatalogRemoteSourceKind, default_public_model_catalog_sources,
    public_model_catalog_sources_enabled,
};
pub use oauth_callback::{
    RuntimeOAuthCallbackError, parse_oauth_callback_url, wait_for_oauth_callback,
};
pub use output_format::{OutputFormat, OutputFormatParseError};
pub(crate) use periodic::{run_periodic, wait_for_tick_or_shutdown};
pub(crate) use plugin_composition::{compose_and_install_plugin_host, install_plugin_host_client};
pub(crate) use policy::RuntimeSchedulingPolicy;
pub(crate) use process_state::RuntimeProcessState;
pub(crate) use provider_composition::{
    ProviderListPatchTarget, apply_provider_list_patch, dispatch_provider_list_patch,
    provider_descriptors_from_ids,
};
pub(crate) use refresh::run_cancellable_refresh;
pub(crate) use refresh_policy::should_refresh;
pub(crate) use registration::{configured_agent_registrations, spawn_registration_batch};
pub use reload::{RuntimeReloadCause, RuntimeReloadReport};
pub(crate) use reload_gate::ReloadGate;
pub(crate) use reload_watch::run_reload_watch_loop;
pub use runtime_authentication_service::{
    RuntimeAuthBrowserStart, RuntimeAuthCredentialIssuer, RuntimeAuthCredentialType,
    RuntimeAuthDeviceStart, RuntimeAuthLoginKind, RuntimeAuthProvider, RuntimeAuthenticationError,
    RuntimeAuthenticationErrorKind, RuntimeAuthenticationService,
};
pub(crate) use runtime_composition_config::RuntimeCompositionConfig;
pub use runtime_control_service::{
    RuntimeBackgroundTaskWork, RuntimeControlService, RuntimeControlServiceError,
};
pub use runtime_draft_authentication_service::{
    RuntimeDraftAuthBrowserStart, RuntimeDraftAuthDeviceStart, RuntimeDraftAuthKind,
    RuntimeDraftAuthToken, RuntimeDraftAuthenticationService, finish_gitlab_draft_auth_browser,
    finish_openai_draft_auth_browser, poll_copilot_draft_auth_device,
    poll_openai_draft_auth_device, start_copilot_draft_auth_device,
    start_gitlab_draft_auth_browser, start_openai_draft_auth_browser,
    start_openai_draft_auth_device,
};
pub use runtime_status_service::{
    RuntimeAgentProfile, RuntimeAgentSelectionStatus, RuntimeAgentStatus, RuntimeAgentsStatus,
    RuntimeLspServerStatus, RuntimeLspStatus, RuntimeMcpCredentialMigration, RuntimeMcpOAuthHealth,
    RuntimeMcpServerStatus, RuntimeMcpStatus, RuntimeSkillStatus, RuntimeSkillsStatus,
    RuntimeStatusService, RuntimeStatusSnapshot,
};
pub use runtime_tool_execution_service::{
    RuntimeToolDescriptor, RuntimeToolExecutionError, RuntimeToolExecutionService,
};
pub(crate) use scheduler_composition::compose_scheduler;
pub(crate) use services::RuntimeServiceBundle;
pub(crate) use snapshot::SnapshotMetadata;
pub(crate) use snapshot_state::RuntimeSnapshotState;
pub(crate) use staleness::is_stale;
pub(crate) use store::{SnapshotStore, TaskControl};
pub(crate) use task_state::RuntimeTaskState;

impl agena_runtime_session::PeriodicControl for store::TaskControl {
    fn is_shutdown(&self) -> bool {
        self.is_shutdown()
    }

    fn notify(&self) -> &tokio::sync::Notify {
        self.notify()
    }
}
pub(crate) use agena_bundled_plugins::tool::{new_web_plugin, web_plugin_id};
pub(crate) use agena_runtime_session::{UsageStatRecord, summarize_usage_records};
pub(crate) use tracing_config::{
    RuntimeDatabaseCompositionError, apply_runtime_tracing_filter, connect_runtime_database,
};
pub use tracing_config::{RuntimeTracingConfiguration, runtime_env_filter};
pub(crate) use watch::{capture_watch_path_stamps, diff_watch_path_stamps};
pub(crate) use watch_paths::{WatchPathSet, runtime_watch_paths};

/// Handle used by runtime composition to reload the active tracing filter.
pub type TracingFilterReloadHandle =
    tracing_subscriber::reload::Handle<tracing_subscriber::EnvFilter, tracing_subscriber::Registry>;

/// Production worker-thread stack size used by deep tool and session flows.
pub const APP_RUNTIME_THREAD_STACK_SIZE: usize = 16 * 1024 * 1024;

/// Build the multi-thread Tokio runtime used by Agena binaries.
pub fn build_app_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(APP_RUNTIME_THREAD_STACK_SIZE)
        .build()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        APP_RUNTIME_THREAD_STACK_SIZE, ReloadGate, RuntimeBackgroundTaskCompletion,
        RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskOutcome,
        RuntimeBackgroundTaskRegistry, RuntimeBackgroundTaskSpec, RuntimeBackgroundTaskState,
        RuntimeBackgroundTaskStatus, RuntimeControlState, RuntimeSchedulingPolicy,
        RuntimeSnapshotState, RuntimeTaskState, SnapshotMetadata, SnapshotStore, TaskControl,
        WatchPathSet, build_app_runtime, capture_watch_path_stamps, connect_or_initialize,
        diff_watch_path_stamps, is_stale, run_cancellable_refresh, run_periodic, should_refresh,
        wait_for_tick_or_shutdown,
    };
    use crate::optional::build_optional;
    use crate::watch::WatchPathStamp;

    #[test]
    fn builds_multi_thread_runtime() {
        const {
            assert!(APP_RUNTIME_THREAD_STACK_SIZE >= 8 * 1024 * 1024);
        }
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async { tokio::task::yield_now().await });
    }

    #[test]
    fn snapshot_store_swaps_and_task_control_notifies() {
        let store = SnapshotStore::new(Arc::new(1_u32));
        assert_eq!(*store.current(), 1);
        assert_eq!(*store.swap(Arc::new(2)), 1);
        assert_eq!(*store.current(), 2);

        let control = TaskControl::default();
        assert!(!control.is_shutdown());
        control.shutdown();
        assert!(control.is_shutdown());
    }

    #[test]
    fn control_state_owns_snapshot_swap_and_shutdown() {
        let state = RuntimeControlState::<u32, String>::new(Arc::new(1), None);
        assert_eq!(*state.current_snapshot(), 1);
        assert_eq!(*state.swap_snapshot(Arc::new(2)), 1);
        assert_eq!(*state.current_snapshot(), 2);
        assert!(!state.task_control().is_shutdown());
        assert!(state.background_tasks().list().is_empty());
        state.task_control().shutdown();
        assert!(state.task_control().is_shutdown());
    }

    #[tokio::test]
    async fn connect_or_initialize_reuses_and_initializes_connection() {
        let initialized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let initialized_for_init = Arc::clone(&initialized);
        let connection = connect_or_initialize(
            Some(Arc::new(7_u32)),
            true,
            || async { Ok::<_, ()>(99_u32) },
            move |value| {
                initialized_for_init
                    .fetch_add(*value as usize, std::sync::atomic::Ordering::SeqCst);
                async { Ok::<_, ()>(()) }
            },
        )
        .await
        .expect("connection should initialize");
        assert_eq!(*connection.expect("connection"), 7);
        assert_eq!(initialized.load(std::sync::atomic::Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn build_optional_only_invokes_enabled_builder() {
        assert_eq!(build_optional(false, || async { 7_u32 }).await, None);
        assert_eq!(build_optional(true, || async { 7_u32 }).await, Some(7));
    }

    #[tokio::test]
    async fn cancellable_refresh_reloads_only_after_successful_refresh() {
        let reloaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reloaded_for_reload = Arc::clone(&reloaded);
        let result = run_cancellable_refresh(
            tokio_util::sync::CancellationToken::new(),
            || false,
            || async { Ok::<_, ()>(11_u32) },
            move || async move {
                reloaded_for_reload.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, ()>(())
            },
        )
        .await
        .expect("refresh should succeed");
        assert_eq!(result, Some(11));
        assert!(reloaded.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellable_refresh_skips_work_when_cancelled_before_start() {
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let result = run_cancellable_refresh(
            cancel,
            || false,
            || async { Ok::<_, ()>(11_u32) },
            || async { Ok::<_, ()>(()) },
        )
        .await
        .expect("cancelled refresh should be handled");
        assert_eq!(result, None);
    }

    #[test]
    fn staleness_treats_missing_and_old_timestamps_as_stale() {
        let now = chrono::Utc::now();
        assert!(is_stale(None, chrono::Duration::seconds(60)));
        assert!(is_stale(
            Some(now - chrono::Duration::seconds(61)),
            chrono::Duration::seconds(60)
        ));
        assert!(!is_stale(
            Some(now + chrono::Duration::seconds(60)),
            chrono::Duration::seconds(60)
        ));
    }

    #[test]
    fn refresh_policy_requires_entries_and_fresh_timestamp() {
        let now = chrono::Utc::now();
        assert!(should_refresh(
            false,
            Some(now),
            chrono::Duration::seconds(60)
        ));
        assert!(should_refresh(
            true,
            Some(now - chrono::Duration::seconds(61)),
            chrono::Duration::seconds(60)
        ));
        assert!(!should_refresh(
            true,
            Some(now),
            chrono::Duration::seconds(60)
        ));
    }

    #[test]
    fn snapshot_metadata_preserves_generation_and_load_time() {
        let metadata = SnapshotMetadata::new(7);
        assert_eq!(metadata.generation(), 7);
        assert!(metadata.loaded_at() <= chrono::Utc::now());
    }

    #[test]
    fn snapshot_state_retains_generic_resolution_and_tasks() {
        let state = RuntimeSnapshotState::new(
            11,
            "resolved-config",
            42_u32,
            RuntimeTaskState::new(WatchPathSet::new()),
        );
        assert_eq!(state.metadata().generation(), 11);
        assert_eq!(state.resolution(), &"resolved-config");
        assert_eq!(state.services(), &42_u32);
        assert!(state.tasks().watch_paths().is_empty());
    }

    #[test]
    fn scheduling_policy_has_stable_runtime_defaults() {
        let policy = RuntimeSchedulingPolicy::default();
        assert!(policy.reload_enabled);
        assert_eq!(
            policy.reload_poll_interval,
            std::time::Duration::from_secs(2)
        );
        assert!(policy.session_gc_enabled);
        assert_eq!(
            policy.session_gc_interval,
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn watch_path_set_is_sorted_and_deduplicated() {
        let paths =
            WatchPathSet::from_paths(vec!["b.toml".into(), "a.toml".into(), "b.toml".into()]);
        assert_eq!(
            paths.as_slice(),
            &[
                std::path::PathBuf::from("a.toml"),
                std::path::PathBuf::from("b.toml")
            ]
        );
        assert!(!paths.is_empty());
    }

    #[test]
    fn watch_path_set_insert_and_extend_preserve_set_invariant() {
        let mut paths = WatchPathSet::new();
        paths.insert("b.toml".into());
        paths.extend(["a.toml".into(), "b.toml".into()]);
        assert_eq!(
            paths.as_slice(),
            &[
                std::path::PathBuf::from("a.toml"),
                std::path::PathBuf::from("b.toml")
            ]
        );
    }

    #[test]
    fn runtime_watch_paths_include_config_and_agent_directories() {
        let paths = crate::runtime_watch_paths(
            std::path::Path::new("/tmp/agena/config.json"),
            std::path::Path::new("/workspace/.agena.json"),
            &agena_plugin_host::PluginsConfig::default(),
        );
        assert_eq!(
            paths.as_slice(),
            &[
                std::path::PathBuf::from("/tmp/agena/agents"),
                std::path::PathBuf::from("/tmp/agena/config.json"),
                std::path::PathBuf::from("/workspace/.agena.json"),
                std::path::PathBuf::from("/workspace/agents"),
            ]
        );
    }

    #[test]
    fn reload_gate_can_be_acquired() {
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async {
            let gate = ReloadGate::default();
            let _guard = gate.acquire().await;
        });
    }

    #[test]
    fn background_task_spec_exposes_registration_inputs() {
        let spec = RuntimeBackgroundTaskSpec::new(
            RuntimeBackgroundTaskKind::RuntimeReload,
            RuntimeBackgroundTaskOrigin::System,
            "Reload runtime",
            Some("reload".to_owned()),
            false,
        );
        assert_eq!(spec.kind(), RuntimeBackgroundTaskKind::RuntimeReload);
        assert_eq!(spec.origin(), RuntimeBackgroundTaskOrigin::System);
        assert_eq!(spec.title(), "Reload runtime");
        assert_eq!(spec.dedupe_key(), Some("reload"));
        assert!(!spec.cancellable());
    }

    #[test]
    fn background_task_completion_carries_terminal_state() {
        let completion = RuntimeBackgroundTaskCompletion::Failed {
            error_message: "failed".to_owned(),
        };
        assert!(matches!(
            completion,
            RuntimeBackgroundTaskCompletion::Failed { .. }
        ));
    }

    #[test]
    fn background_task_state_starts_empty() {
        let state = RuntimeBackgroundTaskState::default();
        assert!(state.order.is_empty());
        assert!(state.tasks.is_empty());
        assert!(state.controls.is_empty());
    }

    #[test]
    fn registry_runs_and_records_a_task() {
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async {
            let registry = RuntimeBackgroundTaskRegistry::<String>::default();
            let spec = RuntimeBackgroundTaskSpec::new(
                RuntimeBackgroundTaskKind::RuntimeReload,
                RuntimeBackgroundTaskOrigin::System,
                "Reload runtime",
                None,
                false,
            );
            let start = registry.spawn(spec, |_cancel| async {
                Ok::<_, String>(RuntimeBackgroundTaskOutcome::succeeded("done"))
            });
            assert!(start.started);
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let task = registry
                .list()
                .into_iter()
                .find(|task| task.id == start.task.id)
                .expect("task should remain in history");
            assert_eq!(task.status, RuntimeBackgroundTaskStatus::Succeeded);
        });
    }

    #[test]
    fn periodic_wait_returns_when_shutdown_is_signaled() {
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async {
            let control = TaskControl::default();
            control.shutdown();
            assert!(wait_for_tick_or_shutdown(&control, std::time::Duration::from_secs(60)).await);
        });
    }

    #[test]
    fn watch_stamp_diff_reports_added_removed_and_modified_paths() {
        let first = std::collections::HashMap::from([(
            std::path::PathBuf::from("a"),
            WatchPathStamp {
                exists: true,
                modified: None,
            },
        )]);
        let second = std::collections::HashMap::from([(
            std::path::PathBuf::from("b"),
            WatchPathStamp {
                exists: true,
                modified: None,
            },
        )]);
        assert_eq!(
            diff_watch_path_stamps(&first, &second),
            vec![std::path::PathBuf::from("a"), std::path::PathBuf::from("b")]
        );
        let _ = capture_watch_path_stamps(&["definitely-missing".into()]);
    }

    #[test]
    fn periodic_runner_stops_before_first_tick_when_shutdown_is_set() {
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async {
            let control = std::sync::Arc::new(TaskControl::default());
            control.shutdown();
            let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = std::sync::Arc::clone(&ticks);
            run_periodic(
                control,
                || std::time::Duration::from_secs(60),
                move || {
                    let seen = std::sync::Arc::clone(&seen);
                    async move {
                        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                },
            )
            .await;
            assert_eq!(ticks.load(std::sync::atomic::Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn abort_guard_cancels_owned_task_on_drop() {
        let runtime = build_app_runtime().expect("runtime should build");
        runtime.block_on(async {
            let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let completed_in_task = Arc::clone(&completed);
            let control = TaskControl::default();
            control.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                completed_in_task.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            control.shutdown();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            assert!(!completed.load(std::sync::atomic::Ordering::SeqCst));
        });
    }
}
