#![cfg(unix)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use agena_plugin_host::config::{
    ConfiguredPlugin, DurationSpec, PluginPackage, PluginsConfig, RestartMode, RestartPolicy,
};
use agena_plugin_host::host::{PluginHost, PluginHostBuildConfig};
use agena_plugin_host::scoped_registry::PluginScopeKey;
use agena_plugin_host::sdk::host_api::{EventSubscription, HostClient, LogLevel, NoopHostClient};
use agena_plugin_host::sdk::{
    EventEnvelope, EventFilter, PluginManifest, ToolInvokeInput, ToolInvokeOutput,
};
use agena_plugin_host::status::{PluginRunState, PluginStatus};
use serde_json::{Value, json};

const KEY: &str = "test.hosted";

struct MarkerHost {
    marker: &'static str,
    blocked: Option<Arc<BlockedRead>>,
}

#[derive(Default)]
struct BlockedRead {
    entered: tokio::sync::Notify,
    cancelled: tokio_util::sync::CancellationToken,
}

#[async_trait::async_trait]
impl HostClient for MarkerHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, event: EventEnvelope) -> agena_plugin_host::sdk::Result<()> {
        NoopHostClient.publish_event(event).await
    }
    async fn subscribe_events(
        &self,
        filter: EventFilter,
    ) -> agena_plugin_host::sdk::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn read_config(&self, key: Option<String>) -> agena_plugin_host::sdk::Result<Value> {
        if key.as_deref() == Some("test.wait") {
            let blocked = self.blocked.as_ref().unwrap();
            let _drop = blocked.cancelled.clone().drop_guard();
            blocked.entered.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(json!({"host": self.marker}))
    }
    async fn invoke_tool(
        &self,
        tool: String,
        input: Value,
    ) -> agena_plugin_host::sdk::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
}

struct Fixture {
    root: tempfile::TempDir,
    config: PluginsConfig,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("plugin.py");
        std::fs::write(&script, include_str!("fixtures/stdio_host.py")).unwrap();
        let mut config = PluginsConfig::default();
        config.list.insert(
            KEY.into(),
            ConfiguredPlugin {
                package: PluginPackage::Stdio {
                    command: "/usr/bin/python3".into(),
                    args: vec![script.to_str().unwrap().into()],
                    env: BTreeMap::from([
                        (
                            "AGENA_TEST_CONTROL".into(),
                            root.path().join("control.json").to_str().unwrap().into(),
                        ),
                        (
                            "AGENA_TEST_EVENTS".into(),
                            root.path().join("events.jsonl").to_str().unwrap().into(),
                        ),
                        (
                            "AGENA_TEST_STARTS".into(),
                            root.path().join("starts").to_str().unwrap().into(),
                        ),
                        (
                            "AGENA_TEST_MANIFEST".into(),
                            serde_json::to_string(&PluginManifest::new("test", "hosted", "1.0.0"))
                                .unwrap(),
                        ),
                    ]),
                    cwd: Some(root.path().to_path_buf()),
                    sha256: None,
                    restart: RestartPolicy {
                        policy: RestartMode::OnFailure,
                        max_retries: 2,
                        min_backoff: DurationSpec(Duration::from_millis(5)),
                        max_backoff: DurationSpec(Duration::from_millis(5)),
                    },
                },
                ..ConfiguredPlugin::default()
            },
        );
        Self { root, config }
    }

    fn control(&self, control: Value) {
        std::fs::write(
            self.root.path().join("control.json"),
            serde_json::to_vec(&control).unwrap(),
        )
        .unwrap();
    }

    fn with_manifest_tool(&mut self, name: &str) {
        let PluginPackage::Stdio { env, .. } = &mut self.config.list.get_mut(KEY).unwrap().package
        else {
            unreachable!()
        };
        let mut manifest: Value = serde_json::from_str(&env["AGENA_TEST_MANIFEST"]).unwrap();
        manifest["tools"] = json!([{"name": name, "docs": {"summary": "manifest"}, "contract": {"input_schema": {"type": "object"}}}]);
        env.insert("AGENA_TEST_MANIFEST".into(), manifest.to_string());
    }

    fn configured_as(&self, name: &str) -> ConfiguredPlugin {
        let mut configured = self.config.list[KEY].clone();
        let PluginPackage::Stdio { env, .. } = &mut configured.package else {
            unreachable!()
        };
        let mut manifest: Value = serde_json::from_str(&env["AGENA_TEST_MANIFEST"]).unwrap();
        manifest["name"] = json!(name);
        env.insert("AGENA_TEST_MANIFEST".into(), manifest.to_string());
        configured
    }

    async fn wait_for_initializations(&self, count: usize) -> Vec<Value> {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let events = std::fs::read_to_string(self.root.path().join("events.jsonl"))
                    .unwrap_or_default();
                let events = events
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect::<Vec<Value>>();
                if events.len() >= count {
                    return events;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn build(
        &self,
        marker: &'static str,
        previous: Option<Arc<PluginHost>>,
    ) -> Arc<PluginHost> {
        PluginHost::new(self.build_config(marker, previous))
            .await
            .unwrap()
    }

    fn build_config(
        &self,
        marker: &'static str,
        previous: Option<Arc<PluginHost>>,
    ) -> PluginHostBuildConfig {
        PluginHostBuildConfig {
            static_plugins: vec![],
            config: self.config.clone(),
            workspace_root: self.root.path().to_path_buf(),
            agena_version: "test-version".into(),
            callback_base_url: None,
            host_client: Some(Arc::new(MarkerHost {
                marker,
                blocked: None,
            })),
            previous_plugins: if previous.is_some() {
                PluginHostBuildConfig::previous_plugins(&self.config)
            } else {
                Default::default()
            },
            previous,
        }
    }
}

#[tokio::test]
async fn session_tools_survive_reuse_and_are_released_on_process_restart() {
    let mut fixture = Fixture::new();
    fixture.with_manifest_tool("shared");
    let first = fixture.build("first", None).await;
    let tool = first.registered_tools()[0].clone();
    for session in [11, 22] {
        first
            .invoke_tool(
                &tool,
                ToolInvokeInput {
                    tool_name: "shared".into(),
                    session_id: session,
                    call_id: session,
                    workspace_root: fixture.root.path().to_str().unwrap().into(),
                    input: json!({
                        "method": "host/tool.registry.register",
                        "params": {"request": {"tool": {
                            "name": "shared",
                            "docs": {"summary": format!("session-{session}")},
                            "contract": {"input_schema": {"type": "object"}}
                        }}}
                    }),
                },
                None,
            )
            .await
            .unwrap();
    }
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    let summaries = [11, 22, 33].map(|session| {
        second.registered_tools_for_scope(Some(&PluginScopeKey::session(session)))[0]
            .definition
            .docs
            .summary
            .clone()
    });
    assert!(call(&second, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&second).await;
    let after = second.registered_tools_for_scope(Some(&PluginScopeKey::session(11)));
    second.shutdown().await;
    assert_eq!(
        summaries,
        [
            Some("session-11".into()),
            Some("session-22".into()),
            Some("manifest".into())
        ]
    );
    restarted.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].definition.docs.summary.as_deref(),
        Some("manifest")
    );
}

#[tokio::test]
async fn display_and_themes_survive_reuse_and_are_released_on_process_restart() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    for callback in [
        json!({"method": "host/ui.display.contribute", "params": {"request": {"contribution": {
            "id": "live-status", "kind": "status_line_text", "priority": 3,
            "content": {"kind": "text", "text": "live"}
        }}}}),
        json!({"method": "host/ui.theme.register", "params": {"request": {
            "id": "live-theme", "display_name": "Live", "colors": {}
        }}}),
    ] {
        call(&first, "test/callback", callback).await.unwrap();
    }
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    let display = second.display_contributions();
    let themes = second.theme_palettes();
    assert!(call(&second, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&second).await;
    let remaining_display = second.display_contributions();
    let remaining_themes = second.theme_palettes();
    second.shutdown().await;
    assert_eq!(display.len(), 1);
    assert_eq!(display[0].contribution.id, "live-status");
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].id, "live-theme");
    restarted.unwrap();
    assert!(remaining_display.is_empty());
    assert!(remaining_themes.is_empty());
}

#[tokio::test]
async fn restart_after_reuse_uses_the_successor_initialization_context() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let mut config = fixture.build_config("second", Some(first.clone()));
    config.agena_version = "successor-version".into();
    config.workspace_root = fixture.root.path().join("successor");
    let second = PluginHost::new(config).await.unwrap();
    first.shutdown().await;
    assert!(call(&second, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&second).await;
    let state = call(&second, "test/state", json!({})).await;
    second.shutdown().await;
    restarted.unwrap();
    let state = state.unwrap();
    assert_eq!(state["host"], json!({"host": "second"}));
    assert_eq!(state["context"]["agena_version"], "successor-version");
    assert_eq!(
        state["context"]["workspace_root"],
        fixture.root.path().join("successor").to_str().unwrap()
    );
}

async fn call(
    host: &PluginHost,
    method: &str,
    params: Value,
) -> Result<Value, agena_plugin_host::TransportError> {
    tokio::time::timeout(
        Duration::from_secs(5),
        host.plugins()[0].transport.dispatch(method, params),
    )
    .await
    .expect("RPC deadline")
}

async fn wait_for_restart(host: &PluginHost) -> Result<PluginStatus, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let status = host.plugin_status(KEY).unwrap();
            if status.restart_count == 1 && status.state == PluginRunState::Running {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
}

#[tokio::test]
async fn activation_keeps_the_actual_process_status() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    let state = call(&host, "test/state", json!({})).await.unwrap();
    let status = host.plugin_status(KEY).unwrap();
    host.shutdown().await;
    assert_eq!(status.pid.map(u64::from), state["pid"].as_u64());
    assert_eq!(state["initialized"], true);
}

#[tokio::test]
async fn automatic_restart_reinitializes_before_reporting_running() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    assert!(call(&host, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&host).await;
    let state = call(&host, "test/state", json!({})).await;
    host.shutdown().await;
    restarted.unwrap();
    let state = state.unwrap();
    assert_eq!(state["initialized"], true);
    assert_eq!(state["context"]["agena_version"], "test-version");
    assert_eq!(state["host"], json!({"host": "first"}));
}

#[tokio::test]
async fn reused_process_routes_callbacks_to_the_successor_host() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let second = fixture.build("second", Some(first.clone())).await;
    let same_transport = Arc::ptr_eq(
        &first.plugins()[0].transport,
        &second.plugins()[0].transport,
    );
    first.shutdown().await;
    let callback = call(&second, "test/host", json!({})).await;
    second.shutdown().await;
    assert!(
        same_transport,
        "unchanged configurations keep the running process"
    );
    assert_eq!(callback.unwrap(), json!({"host": "second"}));
}

#[tokio::test]
async fn reused_process_keeps_publishing_status_to_the_successor_host() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    assert!(call(&second, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&second).await;
    second.shutdown().await;
    restarted.expect("successor must observe lifecycle changes after reusing a process");
}

#[tokio::test]
async fn restart_disposes_old_dynamic_registrations() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    call(&host, "test/register", json!({"name": "old-process"}))
        .await
        .unwrap();
    assert_eq!(host.registered_tools().len(), 1);
    assert!(call(&host, "test/exit", json!({})).await.is_err());
    let restarted = wait_for_restart(&host).await;
    let remaining = host.registered_tools();
    host.shutdown().await;
    restarted.unwrap();
    assert!(
        remaining.is_empty(),
        "a dead process cannot retain its dynamic tools"
    );
}

#[tokio::test]
async fn reuse_preserves_dynamic_registrations_and_rebinds_their_owner() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    call(&first, "test/register", json!({"name": "live-process"}))
        .await
        .unwrap();
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    let tools = second.registered_tools();
    let loaded_scope = second.plugins()[0].effect_scope.clone();
    let current_scope = second
        .host_handle()
        .effect_scope(&KEY.parse().unwrap())
        .unwrap();
    let old_scope_state = first.plugins()[0].effect_scope.state();
    second.shutdown().await;
    assert_eq!(
        tools.len(),
        1,
        "live process contributions must survive host reuse"
    );
    assert!(Arc::ptr_eq(&loaded_scope, &current_scope));
    assert_eq!(
        old_scope_state,
        agena_plugin_host::effect_scope::PluginEffectScopeState::Disposed
    );
}

#[tokio::test]
async fn handoff_cancels_old_host_callbacks_without_registering_late_resources() {
    let fixture = Fixture::new();
    let blocked = Arc::new(BlockedRead::default());
    let mut config = fixture.build_config("first", None);
    config.host_client = Some(Arc::new(MarkerHost {
        marker: "first",
        blocked: Some(blocked.clone()),
    }));
    let first = PluginHost::new(config).await.unwrap();
    let callback = tokio::spawn({
        let first = first.clone();
        async move { call(&first, "test/wait_then_register", json!({})).await }
    });
    tokio::time::timeout(Duration::from_secs(1), blocked.entered.notified())
        .await
        .unwrap();
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    let callback_result = tokio::time::timeout(Duration::from_secs(1), callback).await;
    let state = call(&second, "test/state", json!({})).await;
    let tools = second.registered_tools();
    second.shutdown().await;
    assert!(callback_result.unwrap().unwrap().is_err());
    assert!(blocked.cancelled.is_cancelled());
    assert!(state.unwrap()["initialized"].as_bool().unwrap());
    assert!(tools.is_empty());
}

#[tokio::test]
async fn dropping_a_stdio_host_releases_its_router_and_transport() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    let state = call(&host, "test/state", json!({})).await.unwrap();
    let pid = state["pid"].as_u64().unwrap();
    let weak_handle = Arc::downgrade(&host.host_handle());
    let weak_transport = Arc::downgrade(&host.plugins()[0].transport);
    let weak_scope = Arc::downgrade(&host.plugins()[0].effect_scope);
    drop(host);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let present = tokio::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .output()
                .await
                .unwrap()
                .status
                .success();
            if !present {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("dropping the last owner must reap the child");
    assert!(weak_handle.upgrade().is_none());
    assert!(weak_transport.upgrade().is_none());
    assert!(weak_scope.upgrade().is_none());
}

#[tokio::test]
async fn cancelled_successor_build_closes_transports_already_handed_over() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let slow = Fixture::new();
    slow.control(json!({"init_delay_ms": 30_000}));
    let mut configured = slow.config.list[KEY].clone();
    let PluginPackage::Stdio { env, .. } = &mut configured.package else {
        unreachable!()
    };
    env.insert(
        "AGENA_TEST_MANIFEST".into(),
        serde_json::to_string(&PluginManifest::new("test", "slow", "1.0.0")).unwrap(),
    );
    let mut config = fixture.build_config("second", Some(first.clone()));
    config.config.list.insert("test.slow".into(), configured);
    let build = tokio::spawn(PluginHost::new(config));
    slow.wait_for_initializations(1).await;
    // The earlier plugin has already been reused, but there is no completed
    // successor host that can own it after this build future is cancelled.
    build.abort();
    assert!(matches!(build.await, Err(error) if error.is_cancelled()));
    let stopped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if first.plugin_status(KEY).unwrap().state != PluginRunState::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let work = call(&first, "test/state", json!({})).await;
    // Explicit cleanup also makes a failing regression leave no orphan.
    first.plugins()[0].transport.close().await.unwrap();
    first.shutdown().await;
    stopped.expect("an abandoned successor cannot leave a transferred plugin running");
    assert!(work.is_err());
}

#[tokio::test]
async fn manifest_defaults_do_not_overwrite_dynamic_tools_during_activation_or_reuse() {
    let mut fixture = Fixture::new();
    fixture.with_manifest_tool("shared");
    fixture.control(json!({"init_tool_name": "shared"}));
    let first = fixture.build("first", None).await;
    let initial = first.registered_tools()[0].definition.docs.summary.clone();
    let second = fixture.build("second", Some(first.clone())).await;
    first.shutdown().await;
    let reused = second.registered_tools()[0].definition.docs.summary.clone();
    second.shutdown().await;
    assert_eq!(initial.as_deref(), Some("dynamic"));
    assert_eq!(reused.as_deref(), Some("dynamic"));
}

#[tokio::test]
async fn initialization_uses_fresh_authority_and_rejects_work_until_ready() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    fixture.control(json!({"init_delay_ms": 300}));
    assert!(call(&host, "test/exit", json!({})).await.is_err());
    let events = fixture.wait_for_initializations(2).await;
    let status_during_init = host.plugin_status(KEY).unwrap();
    let work = call(&host, "test/state", json!({})).await;
    let notification = host.plugins()[0]
        .transport
        .notify("test/notify", json!({}))
        .await;
    let restarted = wait_for_restart(&host).await;
    host.shutdown().await;
    assert_eq!(status_during_init.state, PluginRunState::Restarting);
    assert!(work.is_err());
    assert!(notification.is_err());
    restarted.unwrap();
    assert_ne!(
        events[0]["context"]["authority_token"],
        events[1]["context"]["authority_token"]
    );
}

#[tokio::test]
async fn shutdown_cancels_restart_initialization_promptly() {
    let fixture = Fixture::new();
    let host = fixture.build("first", None).await;
    fixture.control(json!({"init_delay_ms": 30_000}));
    assert!(call(&host, "test/exit", json!({})).await.is_err());
    fixture.wait_for_initializations(2).await;
    tokio::time::timeout(Duration::from_secs(2), host.shutdown())
        .await
        .unwrap();
    assert_eq!(
        host.plugin_status(KEY).unwrap().state,
        PluginRunState::Stopped
    );
}

#[tokio::test]
async fn restarted_plugin_must_revalidate_manifest_protocol_and_init_result() {
    for control in [
        json!({"manifest_version": "2.0.0"}),
        json!({"protocol_version": 999_999}),
        json!({"init_failure": true}),
    ] {
        let fixture = Fixture::new();
        let host = fixture.build("first", None).await;
        fixture.control(control);
        assert!(call(&host, "test/exit", json!({})).await.is_err());
        let exhausted = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if host
                    .plugin_logs(KEY, None, 0)
                    .iter()
                    .any(|record| record.message == "restart budget exhausted")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        let status = host.plugin_status(KEY).unwrap();
        let work = call(&host, "test/state", json!({})).await;
        host.shutdown().await;
        exhausted.unwrap();
        assert_eq!(status.state, PluginRunState::Failed);
        assert_eq!(status.restart_count, 0);
        assert!(work.is_err());
    }
}

async fn assert_terminal_reload_recovers(terminal: &str) {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    match terminal {
        "closed" => first.plugins()[0].transport.close().await.unwrap(),
        "stopped" => {
            assert!(call(&first, "test/exit", json!({"code": 0})).await.is_err());
            tokio::time::timeout(Duration::from_secs(3), async {
                while first.plugin_status(KEY).unwrap().state != PluginRunState::Stopped {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
        "failed" => {
            fixture.control(json!({"init_failure": true}));
            assert!(call(&first, "test/exit", json!({})).await.is_err());
            tokio::time::timeout(Duration::from_secs(3), async {
                while !first
                    .plugin_logs(KEY, None, 0)
                    .iter()
                    .any(|record| record.message == "restart budget exhausted")
                {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
        _ => unreachable!(),
    }
    // An explicit reload may validate a newly installed manifest, even when
    // the configured executable path and settings have not changed.
    fixture.control(json!({"manifest_version": "2.0.0"}));
    let successor = PluginHost::new(fixture.build_config("second", Some(first.clone()))).await;
    first.shutdown().await;
    let second = successor.expect("reload must recover a terminal transport");
    let state = call(&second, "test/state", json!({})).await;
    let same_transport = Arc::ptr_eq(
        &first.plugins()[0].transport,
        &second.plugins()[0].transport,
    );
    let manifest = second.plugins()[0].manifest.clone();
    let decision = second
        .architecture_catalog()
        .reload
        .decisions
        .into_iter()
        .find(|decision| decision.plugin_id.to_string() == KEY)
        .unwrap();
    second.shutdown().await;
    assert!(!same_transport, "terminal transports cannot be reused");
    let state = state.unwrap();
    assert_eq!(state["initialized"], true);
    assert_eq!(state["host"], json!({"host": "second"}));
    assert_eq!(manifest.version, "2.0.0");
    assert_eq!(
        decision.action,
        agena_plugin_host::activation::PluginReloadAction::Restart
    );
    assert!(
        decision
            .reasons
            .contains(&agena_plugin_host::activation::PluginReloadReason::RuntimeUnavailable)
    );
}

#[tokio::test]
async fn reload_recovers_a_cleanly_stopped_process() {
    assert_terminal_reload_recovers("stopped").await;
}

#[tokio::test]
async fn reload_recovers_an_exhausted_restart_budget() {
    assert_terminal_reload_recovers("failed").await;
}

#[tokio::test]
async fn reload_recovers_a_closed_transport() {
    assert_terminal_reload_recovers("closed").await;
}

#[tokio::test]
async fn stale_predecessor_cannot_steal_a_live_successor_process() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let second = fixture.build("second", Some(first.clone())).await;
    let third = fixture.build("third", Some(first.clone())).await;
    let second_callback = call(&second, "test/host", json!({})).await;
    let third_callback = call(&third, "test/host", json!({})).await;
    let same = Arc::ptr_eq(
        &second.plugins()[0].transport,
        &third.plugins()[0].transport,
    );
    first.shutdown().await;
    second.shutdown().await;
    third.shutdown().await;
    assert!(!same);
    assert_eq!(second_callback.unwrap(), json!({"host": "second"}));
    assert_eq!(third_callback.unwrap(), json!({"host": "third"}));
}

#[tokio::test]
async fn replacing_a_failed_provider_restarts_transitive_consumers_only() {
    let fixture = Fixture::new();
    let consumer = Fixture::new();
    let leaf = Fixture::new();
    let unrelated = Fixture::new();
    let mut config = fixture.config.clone();
    let mut configured = consumer.configured_as("consumer");
    configured.activation.requires = vec![KEY.parse().unwrap()];
    config.list.insert("test.consumer".into(), configured);
    let mut configured = leaf.configured_as("leaf");
    configured.activation.requires = vec!["test.consumer".parse().unwrap()];
    config.list.insert("test.leaf".into(), configured);
    config.list.insert(
        "test.unrelated".into(),
        unrelated.configured_as("unrelated"),
    );
    let mut build = fixture.build_config("first", None);
    build.config = config.clone();
    let first = PluginHost::new(build).await.unwrap();
    let find = |host: &PluginHost, key: &str| {
        host.plugins()
            .iter()
            .find(|plugin| plugin.key().to_string() == key)
            .unwrap()
            .transport
            .clone()
    };
    find(&first, KEY).close().await.unwrap();
    let mut build = fixture.build_config("second", Some(first.clone()));
    build.previous_plugins = PluginHostBuildConfig::previous_plugins(&config);
    build.config = config;
    let second = PluginHost::new(build).await.unwrap();
    let reused = [KEY, "test.consumer", "test.leaf", "test.unrelated"]
        .map(|key| Arc::ptr_eq(&find(&first, key), &find(&second, key)));
    let decisions = second.architecture_catalog().reload.decisions;
    first.shutdown().await;
    let callback = find(&second, "test.unrelated")
        .dispatch("test/host", json!({}))
        .await;
    second.shutdown().await;
    assert_eq!(reused, [false, false, false, true]);
    assert_eq!(callback.unwrap(), json!({"host": "second"}));
    for (key, dependency) in [("test.consumer", KEY), ("test.leaf", "test.consumer")] {
        let decision = decisions
            .iter()
            .find(|decision| decision.plugin_id.to_string() == key)
            .unwrap();
        assert_eq!(
            decision.action,
            agena_plugin_host::activation::PluginReloadAction::Restart
        );
        assert!(
            decision
                .triggered_by
                .iter()
                .any(|key| key.to_string() == dependency)
        );
    }
}

#[tokio::test]
async fn reload_rechecks_a_reuse_candidate_after_other_plugins_initialize() {
    let fixture = Fixture::new();
    let first = fixture.build("first", None).await;
    let gate = Fixture::new();
    let release = gate.root.path().join("release");
    gate.control(json!({"init_gate_file": release}));
    let mut config = fixture.build_config("second", Some(first.clone()));
    config
        .config
        .list
        .insert("test.aaa".into(), gate.configured_as("aaa"));
    let build = tokio::spawn(PluginHost::new(config));
    gate.wait_for_initializations(1).await;
    // Manifest planning selected the still-running process. It exits while
    // an earlier activation is waiting, before its own handoff can begin.
    first.plugins()[0].transport.close().await.unwrap();
    std::fs::write(release, b"ready").unwrap();
    let second = build.await.unwrap().unwrap();
    let plugin = second
        .plugins()
        .iter()
        .find(|plugin| plugin.key().to_string() == KEY)
        .unwrap();
    let same = Arc::ptr_eq(&first.plugins()[0].transport, &plugin.transport);
    let state = plugin.transport.dispatch("test/state", json!({})).await;
    first.shutdown().await;
    second.shutdown().await;
    assert!(!same);
    assert_eq!(state.unwrap()["initialized"], true);
}

#[tokio::test]
async fn manifest_change_after_reuse_planning_blocks_activation_until_retry() {
    let mut fixture = Fixture::new();
    fixture.with_manifest_tool("shared");
    let first = fixture.build("first", None).await;
    let gate = Fixture::new();
    let release = gate.root.path().join("release");
    gate.control(json!({"init_gate_file": release}));
    let mut config = fixture.build_config("second", Some(first.clone()));
    config
        .config
        .list
        .insert("test.aaa".into(), gate.configured_as("aaa"));
    let reload_config = config.config.clone();
    let build = tokio::spawn(PluginHost::new(config));
    gate.wait_for_initializations(1).await;
    fixture.control(json!({"manifest_version": "2.0.0"}));
    first.plugins()[0].transport.close().await.unwrap();
    std::fs::write(release, b"ready").unwrap();
    let second = build.await.unwrap().unwrap();
    first.shutdown().await;
    assert_eq!(
        second.plugin_status(KEY).unwrap().state,
        PluginRunState::Failed
    );
    assert!(
        second
            .plugins()
            .iter()
            .all(|plugin| plugin.key().to_string() != KEY)
    );
    assert!(second.registered_tools().is_empty());
    assert_eq!(
        second
            .host_handle()
            .effect_scope(&KEY.parse().unwrap())
            .unwrap()
            .state(),
        agena_plugin_host::effect_scope::PluginEffectScopeState::Disposed
    );

    let mut config = fixture.build_config("third", Some(second.clone()));
    config.previous_plugins = PluginHostBuildConfig::previous_plugins(&reload_config);
    config.config = reload_config;
    let third = PluginHost::new(config).await.unwrap();
    second.shutdown().await;
    let plugin = third
        .plugins()
        .iter()
        .find(|plugin| plugin.key().to_string() == KEY)
        .unwrap();
    let manifest = plugin.manifest.clone();
    let callback = plugin.transport.dispatch("test/host", json!({})).await;
    third.shutdown().await;
    assert_eq!(manifest.version, "2.0.0");
    assert_eq!(callback.unwrap(), json!({"host": "third"}));
}
