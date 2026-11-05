//! End-to-end macro → manifest → plugin transport → effect-owned pipeline
//! verification for every agent lifecycle event.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct LifecycleProbe {
    fired: Arc<Mutex<Vec<String>>>,
    deny: Arc<AtomicBool>,
    retry: Arc<AtomicBool>,
}

impl LifecycleProbe {
    fn record(&self, event: &str) {
        self.fired.lock().unwrap().push(event.to_owned());
    }
}

#[crate::sdk::agena_plugin(
    namespace = "test",
    name = "lifecycle-probe",
    version = "test",
    summary = "Exercise each typed lifecycle hook through a registered plugin."
)]
impl LifecycleProbe {
    #[hook(
        model.request.before,
        priority = 10,
        summary = "Validate provider requests",
        help = "Checks whether a model request is permitted by the plugin policy.",
        translations(
            locale("zh-CN", summary = "验证模型请求", help = "检查模型请求是否符合插件策略。"),
            locale("fr-FR", summary = "Vérifier les requêtes du modèle", help = "Vérifie que les requêtes du modèle respectent la politique du plugin.")
        )
    )]
    async fn before_model(
        &self,
        input: crate::sdk::ModelRequestBeforeInput,
    ) -> crate::sdk::Result<Option<crate::sdk::ModelRequestBeforePatch>> {
        assert_eq!(input.session_id, 41);
        self.record("model.request.before");
        Ok(Some(crate::sdk::ModelRequestBeforePatch {
            deny_reason: self
                .deny
                .load(Ordering::Relaxed)
                .then(|| "policy denied model".into()),
        }))
    }

    #[hook(
        model.request.before,
        priority = -5,
        summary = "Record request metadata",
        help = "Collects request metadata without changing the provider decision.",
        translations(locale("zh-CN", summary = "记录请求元数据", help = "收集模型请求元数据，而不修改提供方的执行决策。"))
    )]
    async fn observe_model_request(
        &self,
        _input: crate::sdk::ModelRequestBeforeInput,
    ) -> crate::sdk::Result<Option<crate::sdk::ModelRequestBeforePatch>> {
        Ok(None)
    }

    #[hook(model.request.after, summary = "Observe model request outcomes", help = "Records completed and failed model request lifecycle events.")]
    async fn after_model(
        &self,
        input: crate::sdk::ModelRequestAfterInput,
    ) -> crate::sdk::Result<()> {
        assert_eq!(input.session_id, 41);
        self.record("model.request.after");
        Ok(())
    }

    #[hook(model.request.error, summary = "Inspect model retry policy", help = "Tests whether a model failure requests a bounded retry.")]
    async fn model_error(
        &self,
        input: crate::sdk::ModelRequestErrorInput,
    ) -> crate::sdk::Result<Option<crate::sdk::ModelRequestErrorPatch>> {
        assert_eq!(input.session_id, 41);
        self.record("model.request.error");
        Ok(Some(crate::sdk::ModelRequestErrorPatch {
            retry: self.retry.load(Ordering::Relaxed),
        }))
    }

    #[hook(compaction.before, summary = "Check compaction policy", help = "Exercises policy checks before context compaction begins.")]
    async fn before_compaction(
        &self,
        input: crate::sdk::CompactionBeforeInput,
    ) -> crate::sdk::Result<Option<crate::sdk::CompactionBeforePatch>> {
        assert_eq!(input.session_id, 41);
        self.record("compaction.before");
        Ok(Some(crate::sdk::CompactionBeforePatch {
            deny_reason: self
                .deny
                .load(Ordering::Relaxed)
                .then(|| "policy denied compaction".into()),
        }))
    }

    #[hook(compaction.after, summary = "Observe compaction results", help = "Records the terminal status of a context compaction attempt.")]
    async fn after_compaction(
        &self,
        input: crate::sdk::CompactionAfterInput,
    ) -> crate::sdk::Result<()> {
        assert_eq!(input.session_id, 41);
        self.record("compaction.after");
        Ok(())
    }

    #[hook(subagent.start, summary = "Check delegated agent launch", help = "Checks whether a child agent is permitted to start.")]
    async fn start_child(
        &self,
        input: crate::sdk::SubagentStartInput,
    ) -> crate::sdk::Result<Option<crate::sdk::SubagentStartPatch>> {
        assert_eq!(input.session_id, 41);
        self.record("subagent.start");
        Ok(Some(crate::sdk::SubagentStartPatch {
            deny_reason: self
                .deny
                .load(Ordering::Relaxed)
                .then(|| "policy denied child".into()),
        }))
    }

    #[hook(subagent.stop, summary = "Observe delegated agent settlement", help = "Records a child agent completion or failure.")]
    async fn stop_child(&self, input: crate::sdk::SubagentStopInput) -> crate::sdk::Result<()> {
        assert_eq!(input.session_id, 41);
        self.record("subagent.stop");
        Ok(())
    }

    #[hook(permission.request, summary = "Check interactive permissions", help = "Tests policy denial before interactive tool approval.")]
    async fn permission(
        &self,
        input: crate::sdk::PermissionRequestInput,
    ) -> crate::sdk::Result<Option<crate::sdk::PermissionRequestPatch>> {
        assert_eq!(input.session_id, 41);
        self.record("permission.request");
        Ok(Some(crate::sdk::PermissionRequestPatch {
            deny_reason: self
                .deny
                .load(Ordering::Relaxed)
                .then(|| "policy denied permission".into()),
        }))
    }
}

async fn fixture() -> (Arc<PluginHost>, LifecycleProbe) {
    let probe = LifecycleProbe {
        fired: Arc::new(Mutex::new(Vec::new())),
        deny: Arc::new(AtomicBool::new(false)),
        retry: Arc::new(AtomicBool::new(false)),
    };
    let mut config = crate::config::PluginsConfig::default();
    config.list.insert(
        "test.lifecycle-probe".into(),
        crate::config::ConfiguredPlugin::static_default(),
    );
    let host = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![crate::StaticPluginRegistration::new(
            "test.lifecycle-probe".parse().unwrap(),
            probe.clone(),
        )],
        config,
        workspace_root: "/lifecycle-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    (host, probe)
}

fn before() -> crate::sdk::ModelRequestBeforeInput {
    crate::sdk::ModelRequestBeforeInput {
        session_id: 41,
        provider: "test".into(),
        model: "model".into(),
        request_id: "request-1".into(),
    }
}

#[tokio::test]
async fn lifecycle_macro_manifest_and_pipelines_fire_all_eight_hooks() {
    let (host, probe) = fixture().await;
    let event_names = [
        "model.request.before",
        "model.request.after",
        "model.request.error",
        "compaction.before",
        "compaction.after",
        "subagent.start",
        "subagent.stop",
        "permission.request",
    ];

    // A macro-generated manifest is the single source for subscription and
    // the architecture catalog exposes each owned pipeline.
    let manifest = &host.plugins()[0].manifest;
    for name in event_names {
        assert!(
            manifest.hooks.names().contains(&name),
            "missing subscription: {name}"
        );
        assert!(
            host.architecture_catalog()
                .pipelines
                .iter()
                .any(|p| { p.definition.id == name && !p.handlers.is_empty() }),
            "missing registered pipeline: {name}"
        );
    }

    host.dispatch_model_request_before(before()).await.unwrap();
    host.broadcast_model_request_after(crate::sdk::ModelRequestAfterInput {
        session_id: 41,
        provider: "test".into(),
        model: "model".into(),
        request_id: "request-1".into(),
        status: "completed".into(),
        duration_ms: 7,
        usage: None,
    })
    .await;
    assert!(
        !host
            .dispatch_model_request_error(crate::sdk::ModelRequestErrorInput {
                session_id: 41,
                provider: "test".into(),
                model: "model".into(),
                request_id: "request-1".into(),
                message: "provider error".into(),
                attempt: 1,
            })
            .await
    );
    host.dispatch_compaction_before(crate::sdk::CompactionBeforeInput {
        session_id: 41,
        model: "model".into(),
        automatic: false,
        estimated_tokens: None,
    })
    .await
    .unwrap();
    host.broadcast_compaction_after(crate::sdk::CompactionAfterInput {
        session_id: 41,
        model: "model".into(),
        automatic: false,
        status: "committed".into(),
    })
    .await;
    host.dispatch_subagent_start(crate::sdk::SubagentStartInput {
        session_id: 41,
        child_session_id: 42,
        name: "child".into(),
    })
    .await
    .unwrap();
    host.broadcast_subagent_stop(crate::sdk::SubagentStopInput {
        session_id: 41,
        child_session_id: 42,
        name: "child".into(),
        status: "completed".into(),
    })
    .await;
    host.dispatch_permission_request(crate::sdk::PermissionRequestInput {
        session_id: 41,
        tool_name: "shell".into(),
        action: "shell/execute".into(),
        description: "execute".into(),
    })
    .await
    .unwrap();

    let fired = probe.fired.lock().unwrap().clone();
    for event in event_names {
        assert_eq!(
            fired.iter().filter(|name| name.as_str() == event).count(),
            1,
            "event never reached its plugin handler: {event}; actual: {fired:?}"
        );
    }
}

#[tokio::test]
async fn guards_can_only_deny_and_model_error_recovery_is_explicit() {
    let (host, probe) = fixture().await;
    probe.deny.store(true, Ordering::Relaxed);
    let error = host
        .dispatch_model_request_before(before())
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::sdk::PluginErrorKind::PolicyDenied);
    assert!(
        host.dispatch_compaction_before(crate::sdk::CompactionBeforeInput {
            session_id: 41,
            model: "model".into(),
            automatic: true,
            estimated_tokens: Some(100),
        })
        .await
        .is_err()
    );
    assert!(
        host.dispatch_subagent_start(crate::sdk::SubagentStartInput {
            session_id: 41,
            child_session_id: 42,
            name: "child".into(),
        })
        .await
        .is_err()
    );
    assert!(
        host.dispatch_permission_request(crate::sdk::PermissionRequestInput {
            session_id: 41,
            tool_name: "tool".into(),
            action: "tool/run".into(),
            description: "approval".into(),
        })
        .await
        .is_err()
    );

    probe.retry.store(true, Ordering::Relaxed);
    assert!(
        host.dispatch_model_request_error(crate::sdk::ModelRequestErrorInput {
            session_id: 41,
            provider: "test".into(),
            model: "model".into(),
            request_id: "attempt-1".into(),
            message: "timeout".into(),
            attempt: 1,
        })
        .await
    );
}

#[tokio::test]
async fn plugin_authored_handler_descriptions_keep_distinct_ids_and_translations() {
    let (host, _) = fixture().await;
    let manifest = &host.plugins()[0].manifest;
    let model_handlers = manifest
        .hook_handlers
        .iter()
        .filter(|handler| handler.event == "model.request.before")
        .collect::<Vec<_>>();
    assert_eq!(
        model_handlers.len(),
        2,
        "two methods share one subscription"
    );
    let guard = model_handlers
        .iter()
        .find(|handler| handler.id == "before_model")
        .unwrap();
    assert_eq!(guard.summary, "Validate provider requests");
    assert_eq!(guard.priority, 10);
    assert!(guard.is_async);
    assert_eq!(guard.summary_for_locale("zh-CN"), "验证模型请求");
    assert_eq!(
        guard.help_for_locale("zh-CN"),
        "检查模型请求是否符合插件策略。"
    );
    assert_eq!(
        guard.summary_for_locale("fr-FR"),
        "Vérifier les requêtes du modèle"
    );
    assert_eq!(
        guard.help_for_locale("fr-FR"),
        "Vérifie que les requêtes du modèle respectent la politique du plugin.",
    );
    assert_eq!(
        guard.summary_for_locale("es-ES"),
        "Validate provider requests"
    );
    let documented = manifest
        .hook_handlers
        .iter()
        .find(|handler| handler.id == "after_model")
        .unwrap();
    assert_eq!(
        documented.summary_for_locale("zh-CN"),
        "Observe model request outcomes"
    );
    let descriptions = super::hook_registration_for_plugin(host.plugins()[0].as_ref()).unwrap();
    let listed = descriptions
        .hooks
        .iter()
        .filter(|hook| hook.name == "model.request.before")
        .collect::<Vec<_>>();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|hook| hook.handler_id == "before_model"));
    assert!(
        listed
            .iter()
            .any(|hook| hook.handler_id == "observe_model_request")
    );
    assert!(
        listed
            .iter()
            .any(|hook| hook.summary == "Record request metadata")
    );
}

#[test]
fn runtime_registration_rejects_invalid_manual_hook_descriptions() {
    use crate::sdk::{
        HookHandlerDefinition, HookHandlerFilters, HookHandlerTranslation, HookSubscription,
    };
    let mut manifest = crate::sdk::PluginManifest::new("test", "manual", "0.1.0");
    manifest.hooks = HookSubscription::TOOL_BEFORE;
    let mut documented = HookHandlerDefinition {
        id: "check_shell".into(),
        event: "tool.before".into(),
        summary: "Check shell commands".into(),
        help: "Checks a shell command before execution.".into(),
        translations: Default::default(),
        priority: 0,
        filters: HookHandlerFilters::default(),
        is_async: true,
    };
    manifest.hook_handlers.push(documented.clone());
    assert!(super::validate_hook_handler_catalog(&manifest).is_ok());

    documented.help = " ".into();
    manifest.hook_handlers[0] = documented.clone();
    assert!(super::validate_hook_handler_catalog(&manifest).is_err());

    documented.help = "Checks a shell command before execution.".into();
    manifest.hook_handlers[0] = documented.clone();
    manifest.hook_handlers.push(documented.clone());
    assert!(super::validate_hook_handler_catalog(&manifest).is_err());

    manifest.hook_handlers.pop();
    documented.translations.insert(
        "zh-CN".into(),
        HookHandlerTranslation {
            summary: "检查 Shell 命令".into(),
            help: "".into(),
        },
    );
    manifest.hook_handlers[0] = documented;
    assert!(super::validate_hook_handler_catalog(&manifest).is_err());
}
