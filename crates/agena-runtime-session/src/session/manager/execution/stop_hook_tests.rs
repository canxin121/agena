use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use agena_domain::{ComposerDocument, ComposerNode, Model, ModelId, ModelRef, ProviderId};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionUsage,
};
use agena_runtime_contracts::part_content::{HookContent, decode_hook_notice};
use agena_runtime_provider::{
    ProviderError,
    provider::{CompletionResponse as RuntimeCompletionResponse, ModelRuntime},
};

struct StopHookProbe {
    remaining: Arc<AtomicUsize>,
    active_flags: Arc<std::sync::Mutex<Vec<bool>>>,
    run_errors: Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "test",
    name = "stop-hook-probe",
    version = "test",
    summary = "Records stop-hook inputs and requests repeated plan autoruns."
)]
impl StopHookProbe {
    #[hook(agent.stop)]
    async fn agent_stop(
        &self,
        input: agena_plugin_host::sdk::AgentStopInput,
    ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::sdk::AgentStopPatch>> {
        self.active_flags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(input.stop_hook_active);
        self.run_errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(input.run_error);
        let remaining = self.remaining.load(Ordering::SeqCst);
        if remaining == 0 {
            return Ok(None);
        }
        self.remaining.store(remaining - 1, Ordering::SeqCst);
        Ok(Some(agena_plugin_host::sdk::AgentStopPatch {
            continue_with_message: Some("Continue the active approved plan.".to_owned()),
            reason: Some("workflow plan autorun".to_owned()),
        }))
    }
}

struct PlainTextProvider {
    model: ModelId,
}

#[async_trait::async_trait]
impl ModelRuntime for PlainTextProvider {
    fn id(&self) -> &str {
        "plain-text-test"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn prompt_cache_shape(&self, _: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id(),
            [("destination_revision", "plain-text-test")],
        ))
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<RuntimeCompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: "step complete".to_owned(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }
}

async fn fixture(
    max_turns: usize,
    remaining_autoruns: usize,
) -> (
    SessionManager,
    SessionId,
    Arc<std::sync::Mutex<Vec<bool>>>,
    Arc<std::sync::Mutex<Vec<Option<String>>>>,
) {
    let active_flags = Arc::new(std::sync::Mutex::new(Vec::new()));
    let run_errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let remaining = Arc::new(AtomicUsize::new(remaining_autoruns));
    let mut plugin_config = PluginsConfig::default();
    plugin_config.list.insert(
        "test.stop-hook-probe".to_owned(),
        ConfiguredPlugin::static_default(),
    );
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "test.stop-hook-probe".parse().unwrap(),
            StopHookProbe {
                remaining,
                active_flags: active_flags.clone(),
                run_errors: run_errors.clone(),
            },
        )],
        config: plugin_config,
        workspace_root: "/stop-hook-tests".into(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        "/stop-hook-tests",
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(PlainTextProvider {
        model: ModelId::new("plain-text"),
    });
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let workspace = kernel
        .workspaces
        .ensure_id("/stop-hook-tests")
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Stop hook tests".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let manager = SessionManager::new(
        kernel,
        Arc::new(providers),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        RuntimeSessionManagerConfig {
            max_turns: Some(max_turns),
            ..Default::default()
        },
    );
    (manager, session, active_flags, run_errors)
}

#[tokio::test]
async fn plan_autorun_dispatches_fresh_stops_without_the_generic_continuation_cap() {
    let (manager, session, active_flags, run_errors) = fixture(500, 6).await;
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "run the plan".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let options = SessionRunOptions {
        model: ModelRef::new("plain-text-test", "plain-text"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        model_supports_parallel_tool_calls: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    };
    let run = manager.supervise(
        execution.clone(),
        options,
        false,
        manager.reserve_execution().unwrap(),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), run.wait_settled())
        .await
        .expect("the autorun execution must settle");
    let finished = manager
        .kernel()
        .reader
        .execution(session, execution.id)
        .await
        .unwrap();
    let flags = active_flags
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        flags.len(),
        7,
        "six plan autoruns are followed by the final normal stop"
    );
    assert!(
        flags.iter().all(|active| !active),
        "every stop follows a model round, so stop_hook_active must stay false: {flags:?}"
    );
    assert!(
        run_errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(Option::is_none),
        "successful rounds must not report a run error to the stop hook"
    );
    assert!(finished.phase.is_terminal());
    assert_eq!(
        finished.phase,
        agena_domain::session_kernel::ExecutionPhase::Succeeded
    );

    let history = manager
        .kernel()
        .reader
        .history(session, None, 256)
        .await
        .unwrap();
    let notices = history
        .entries
        .iter()
        .filter_map(|entry| match &entry.payload {
            agena_storage::kernel::EntryPayload::Notice { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        notices.len(),
        6,
        "each plan autorun must leave exactly one structured hook notice"
    );
    for text in notices {
        let hook: HookContent = decode_hook_notice(text)
            .expect("hook continuation must be a structured marker notice")
            .expect("hook marker must decode");
        assert_eq!(hook.hook, "agent.stop");
        assert_eq!(hook.plugin_id.as_deref(), Some("test.stop-hook-probe"));
        assert_eq!(
            hook.message.as_deref(),
            Some("Continue the active approved plan."),
            "the continuation must ride the hook part, not a duplicate notice"
        );
    }
}

#[tokio::test]
async fn model_turn_budget_exhaustion_still_dispatches_the_stop_hook() {
    let (manager, session, active_flags, run_errors) = fixture(2, 3).await;
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "run the plan".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let options = SessionRunOptions {
        model: ModelRef::new("plain-text-test", "plain-text"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        model_supports_parallel_tool_calls: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    };
    let run = manager.supervise(
        execution.clone(),
        options,
        false,
        manager.reserve_execution().unwrap(),
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), run.wait_settled())
        .await
        .expect("the autorun execution must settle");
    let finished = manager
        .kernel()
        .reader
        .execution(session, execution.id)
        .await
        .unwrap();
    let flags = active_flags
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let errors = run_errors
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        flags.len(),
        4,
        "two rounds, the exhausted budget, and the final round each dispatch a stop"
    );
    assert!(flags.iter().all(|active| !active));
    assert!(errors[..2].iter().all(Option::is_none));
    assert!(errors[3].is_none());
    assert!(
        errors[2]
            .as_deref()
            .is_some_and(|error| error.contains("model-turn cap")),
        "the budget-exhaustion dispatch must tell the hook why the run stopped: {errors:?}"
    );
    assert_eq!(
        finished.phase,
        agena_domain::session_kernel::ExecutionPhase::Succeeded,
        "a hook continuation must reset the turn budget and let the run finish"
    );
}
