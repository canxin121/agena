use super::*;
use crate::services::{PluginServiceBinding, PluginServiceBindingKey};
use serde_json::{Value, json};

const KEY: &str = "test.host-lifecycle";

fn key() -> PluginKey {
    KEY.parse().unwrap()
}

fn host() -> Arc<HostHandle> {
    Arc::new(HostHandle::new_with_registry(
        Arc::new(crate::sdk::host_api::NoopHostClient),
        Arc::new(RwLock::new(PluginToolRegistry::new())),
        Arc::new(RwLock::new(HashMap::from([(key(), 0)]))),
    ))
}

fn hook(source: &str) -> HostHookRegistration {
    serde_json::from_value(json!({
        "plugin_id": KEY, "trust_level": "static", "trust_status": "trusted",
        "source": source, "hooks": []
    }))
    .unwrap()
}

struct RecordingTransport {
    label: &'static str,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait::async_trait]
impl PluginTransport for RecordingTransport {
    async fn dispatch(
        &self,
        _method: &str,
        _params: Value,
    ) -> Result<Value, crate::TransportError> {
        self.calls.lock().unwrap().push(self.label);
        Ok(json!({"transport": self.label}))
    }

    async fn ingest_stream_event(
        &self,
        _method: &str,
        _params: Value,
    ) -> Result<bool, crate::TransportError> {
        self.calls.lock().unwrap().push(self.label);
        Ok(true)
    }
}

fn transport(
    label: &'static str,
    calls: &Arc<Mutex<Vec<&'static str>>>,
) -> Arc<dyn PluginTransport> {
    Arc::new(RecordingTransport {
        label,
        calls: calls.clone(),
    })
}

async fn has_transport(host: &HostHandle) -> bool {
    host.ingest_stream_event_for_plugin(KEY, "test/event", json!({}))
        .await
        .unwrap()
}

#[tokio::test]
async fn old_hook_scope_cleanup_preserves_successor_catalog() {
    let host = host();
    let previous = host.begin_plugin_instance(key());
    host.set_plugin_hook_catalog(hook("previous")).unwrap();
    let successor = host.begin_plugin_instance(key());
    host.set_plugin_hook_catalog(hook("successor")).unwrap();
    previous.dispose().await;
    let after_old_cleanup = host.hook_list_response().await.hooks;
    successor.dispose().await;
    assert_eq!(
        after_old_cleanup.len(),
        1,
        "old scope removed the successor's hooks"
    );
    assert_eq!(after_old_cleanup[0].source, "successor");
    assert!(host.hook_list_response().await.hooks.is_empty());
}

#[tokio::test]
async fn old_transport_scope_cleanup_preserves_successor_routing() {
    let host = host();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let previous = host.begin_plugin_instance(key());
    host.register_plugin_transport(key(), transport("previous", &calls))
        .await
        .unwrap();
    let successor = host.begin_plugin_instance(key());
    host.register_plugin_transport(key(), transport("successor", &calls))
        .await
        .unwrap();
    previous.dispose().await;
    let routed = has_transport(&host).await;
    let observed = calls.lock().unwrap().clone();
    successor.dispose().await;
    assert!(routed, "old scope removed the successor's transport");
    assert_eq!(observed, ["successor"]);
    assert!(!has_transport(&host).await);
}

#[tokio::test]
async fn rejected_hook_registration_preserves_catalog_and_original_cleanup() {
    let host = host();
    let owner = host.begin_plugin_instance(key());
    host.set_plugin_hook_catalog(hook("original")).unwrap();
    owner.quiesce().await;
    let result = host.set_plugin_hook_catalog(hook("rejected"));
    let after = host.hook_list_response().await.hooks;
    owner.dispose().await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::HostUnavailable);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].source, "original");
    assert!(host.hook_list_response().await.hooks.is_empty());
}

#[tokio::test]
async fn rejected_transport_registration_preserves_routing_and_original_cleanup() {
    let host = host();
    let owner = host.begin_plugin_instance(key());
    let calls = Arc::new(Mutex::new(Vec::new()));
    host.register_plugin_transport(key(), transport("original", &calls))
        .await
        .unwrap();
    owner.quiesce().await;
    let result = host
        .register_plugin_transport(key(), transport("rejected", &calls))
        .await;
    let routed = has_transport(&host).await;
    let observed = calls.lock().unwrap().clone();
    owner.dispose().await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::HostUnavailable);
    assert!(routed);
    assert_eq!(observed, ["original"]);
    assert!(!has_transport(&host).await);
}

fn binding() -> (PluginServiceBindingKey, PluginServiceBinding) {
    (
        PluginServiceBindingKey {
            consumer: KEY.into(),
            service: "test.service".into(),
            api_version: 1,
        },
        PluginServiceBinding {
            consumer: KEY.into(),
            provider: "test.provider".into(),
            service: "test.service".into(),
            api_version: 1,
            optional: false,
            methods: BTreeMap::new(),
        },
    )
}

async fn publish_successor_resources(host: &HostHandle) {
    host.plugin_indices.write().unwrap().insert(key(), 7);
    host.plugin_names
        .write()
        .unwrap()
        .insert(key(), "successor".into());
    host.set_plugin_hook_catalog(hook("successor")).unwrap();
    host.register_plugin_transport(
        key(),
        transport("successor", &Arc::new(Mutex::new(Vec::new()))),
    )
    .await
    .unwrap();
    host.tool_upsert_for_plugin(
        KEY,
        serde_json::from_value(json!({"name": "kept"})).unwrap(),
    )
    .unwrap();
    host.display_contribute(
        KEY,
        serde_json::from_value(json!({"contribution": {
            "id": "kept", "kind": "status_line_text", "priority": 0,
            "content": {"kind": "text", "text": "successor"}
        }}))
        .unwrap(),
    )
    .unwrap();
    host.theme_register(
        KEY,
        serde_json::from_value(json!({"id": "kept", "display_name": "successor", "colors": {}}))
            .unwrap(),
    )
    .unwrap();
}

async fn snapshot(host: &HostHandle) -> Value {
    json!({
        "token": host.tokens.lock().await.get(&key()).cloned(),
        "bindings": host.service_bindings().await.into_values().collect::<Vec<_>>(),
        "index": host.plugin_indices.read().unwrap().get(&key()).copied(),
        "name": host.plugin_names.read().unwrap().get(&key()).cloned(),
        "hooks": host.hook_list_response().await.hooks,
        "transport": has_transport(host).await,
        "tools": host.registered_tool_list_response().unwrap().tools,
        "display": host.display_list_response(),
        "themes": host.theme_list_response().themes,
    })
}

async fn stale_fallback_preserves_successor(wait_on_tokens: bool) {
    let host = host();
    let previous = host.begin_plugin_instance(key());
    host.tokens
        .lock()
        .await
        .insert(key(), "fixture-token".into());
    // Finish effect disposal first, so the next poll reaches a known fallback
    // registry lock and cannot stop at the scope's own disposal notification.
    previous.dispose().await;
    let plugin = key();
    let mut cleanup = Box::pin(host.dispose_plugin_resources_for_scope(&plugin, &previous));
    let successor;
    if wait_on_tokens {
        let mut tokens = host.tokens.lock().await;
        assert!(futures_util::poll!(cleanup.as_mut()).is_pending());
        successor = host.begin_plugin_instance(key());
        tokens.insert(key(), "successor-token".into());
        publish_successor_resources(&host).await;
        drop(tokens);
        host.install_service_bindings(BTreeMap::from([binding()]))
            .await;
    } else {
        let mut bindings = host.service_bindings.write().await;
        assert!(futures_util::poll!(cleanup.as_mut()).is_pending());
        successor = host.begin_plugin_instance(key());
        bindings.extend([binding()]);
        publish_successor_resources(&host).await;
        drop(bindings);
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), cleanup)
        .await
        .unwrap();
    let actual = snapshot(&host).await;
    host.dispose_plugin_resources_for_scope(&plugin, &successor)
        .await;
    assert_eq!(
        actual["index"], 7,
        "fallback from a stale scope deleted successor metadata"
    );
    assert_eq!(actual["name"], "successor");
    assert_eq!(
        actual["token"],
        if wait_on_tokens {
            "successor-token"
        } else {
            "fixture-token"
        }
    );
    assert_eq!(actual["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(actual["hooks"][0]["source"], "successor");
    assert_eq!(actual["transport"], true);
    assert_eq!(actual["tools"].as_array().unwrap().len(), 1);
    assert_eq!(actual["display"].as_array().unwrap().len(), 1);
    assert_eq!(actual["themes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn fallback_waiting_on_tokens_preserves_successor_resources() {
    stale_fallback_preserves_successor(true).await;
}

#[tokio::test]
async fn fallback_waiting_on_service_bindings_preserves_successor_resources() {
    stale_fallback_preserves_successor(false).await;
}

#[test]
fn concurrent_first_use_creates_one_logical_scope() {
    for round in 0..32 {
        let host = host();
        let start = Arc::new(std::sync::Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let (host, start) = (host.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    host.ensure_effect_scope(&key())
                })
            })
            .collect();
        let scopes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        let current = host.effect_scope(&key()).unwrap();
        assert!(
            scopes.iter().all(|scope| Arc::ptr_eq(scope, &current)),
            "first-use race created multiple owners in round {round}: {:?}",
            scopes
                .iter()
                .map(|scope| scope.generation())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn cancelled_fallback_preserves_metadata_until_a_retry_can_finish() {
    let host = host();
    let owner = host.begin_plugin_instance(key());
    host.tokens
        .lock()
        .await
        .insert(key(), "fixture-token".into());
    host.plugin_names
        .write()
        .unwrap()
        .insert(key(), "original".into());
    owner.dispose().await;
    let bindings = host.service_bindings.write().await;
    let plugin = key();
    let mut cleanup = Box::pin(host.dispose_plugin_resources_for_scope(&plugin, &owner));
    assert!(futures_util::poll!(cleanup.as_mut()).is_pending());
    drop(cleanup);
    let token = host.tokens.lock().await.get(&plugin).cloned();
    let name = host.plugin_names.read().unwrap().get(&plugin).cloned();
    drop(bindings);
    host.dispose_plugin_resources_for_scope(&plugin, &owner)
        .await;
    let after_retry = snapshot(&host).await;
    assert_eq!(
        token.as_deref(),
        Some("fixture-token"),
        "cancelled fallback partially deleted metadata"
    );
    assert_eq!(name.as_deref(), Some("original"));
    assert_eq!(after_retry["token"], Value::Null);
    assert_eq!(after_retry["name"], Value::Null);
    assert_eq!(after_retry["index"], Value::Null);
}

#[tokio::test]
async fn service_calls_use_the_successor_transport_after_old_scope_cleanup() {
    let host = host();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let previous = host.begin_plugin_instance(key());
    host.register_plugin_transport(key(), transport("previous", &calls))
        .await
        .unwrap();
    let successor = host.begin_plugin_instance(key());
    host.register_plugin_transport(key(), transport("successor", &calls))
        .await
        .unwrap();
    let (mut binding_key, mut service) = binding();
    binding_key.consumer = "test.consumer".into();
    service.consumer = binding_key.consumer.clone();
    service.provider = KEY.into();
    service.methods.insert(
        "query".into(),
        crate::sdk::PluginServiceMethod::new(
            "query",
            crate::sdk::SettingsContract::empty_object("Input", ""),
            crate::sdk::SettingsContract::bounded_json("Output", "", 1024, 8),
        ),
    );
    host.install_service_bindings(BTreeMap::from([(binding_key, service)]))
        .await;
    previous.dispose().await;
    let result = host
        .invoke_service_for_plugin(
            "test.consumer",
            serde_json::from_value(json!({
                "service": "test.service", "api_version": 1, "method": "query", "input": {}
            }))
            .unwrap(),
            None,
        )
        .await;
    let observed = calls.lock().unwrap().clone();
    successor.dispose().await;
    let result = result.unwrap();
    assert_eq!(result.provider, KEY);
    assert_eq!(result.output, json!({"transport": "successor"}));
    assert_eq!(observed, ["successor"]);
}

#[tokio::test]
async fn identical_transport_objects_keep_separate_registration_ownership() {
    let host = host();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let shared = transport("same instance", &calls);
    let previous = host.begin_plugin_instance(key());
    host.register_plugin_transport(key(), shared.clone())
        .await
        .unwrap();
    host.set_plugin_hook_catalog(hook("identical")).unwrap();
    let successor = host.begin_plugin_instance(key());
    for _ in 0..4 {
        host.register_plugin_transport(key(), shared.clone())
            .await
            .unwrap();
        host.set_plugin_hook_catalog(hook("identical")).unwrap();
    }
    let old_active = previous
        .inspect()
        .effects
        .into_iter()
        .filter(|effect| effect.state == crate::effect_scope::PluginEffectState::Active)
        .count();
    let mut active: Vec<_> = successor
        .inspect()
        .effects
        .into_iter()
        .filter(|effect| effect.state == crate::effect_scope::PluginEffectState::Active)
        .map(|effect| effect.kind)
        .collect();
    active.sort();
    previous.dispose().await;
    let hooks = host.hook_list_response().await.hooks;
    let routed = has_transport(&host).await;
    successor.dispose().await;
    assert_eq!(old_active, 0);
    assert_eq!(active, ["host.hooks", "host.transport"]);
    assert_eq!(hooks.len(), 1);
    assert!(routed);
    assert_eq!(*calls.lock().unwrap(), ["same instance"]);
    assert!(host.hook_list_response().await.hooks.is_empty());
    assert!(!has_transport(&host).await);
}

#[tokio::test]
async fn current_fallback_is_repeatable_and_keeps_other_plugins() {
    let host = host();
    let current = host.begin_plugin_instance(key());
    publish_successor_resources(&host).await;
    host.tokens
        .lock()
        .await
        .insert(key(), "fixture-token".into());
    let other_key: PluginKey = "test.unrelated".parse().unwrap();
    let other = host.begin_plugin_instance(other_key.clone());
    let mut other_hook = hook("unrelated");
    other_hook.plugin_id = other_key.clone();
    host.set_plugin_hook_catalog(other_hook).unwrap();
    host.register_plugin_transport(
        other_key.clone(),
        transport("unrelated", &Arc::new(Mutex::new(Vec::new()))),
    )
    .await
    .unwrap();
    host.plugin_indices
        .write()
        .unwrap()
        .insert(other_key.clone(), 42);
    host.tokens
        .lock()
        .await
        .insert(other_key.clone(), "unrelated-token".into());
    let (mut other_binding_key, mut other_binding) = binding();
    other_binding_key.consumer = other_key.to_string();
    other_binding.consumer = other_key.to_string();
    let expected_binding = serde_json::to_value(&other_binding).unwrap();
    host.install_service_bindings(BTreeMap::from([
        binding(),
        (other_binding_key, other_binding),
    ]))
    .await;
    host.dispose_plugin_resources_for_scope(&key(), &current)
        .await;
    let after = snapshot(&host).await;
    let other_index = host.plugin_indices.read().unwrap().get(&other_key).copied();
    let other_token = host.tokens.lock().await.get(&other_key).cloned();
    let other_routed = host
        .ingest_stream_event_for_plugin(&other_key.to_string(), "test/event", json!({}))
        .await
        .unwrap();
    host.dispose_plugin_resources_for_scope(&key(), &current)
        .await;
    let again = snapshot(&host).await;
    host.dispose_plugin_resources_for_scope(&other_key, &other)
        .await;
    assert_eq!(after, again);
    assert_eq!(after["token"], Value::Null);
    assert_eq!(after["name"], Value::Null);
    assert_eq!(after["index"], Value::Null);
    assert_eq!(after["transport"], false);
    for field in ["tools", "display", "themes"] {
        assert_eq!(after[field], json!([]));
    }
    assert_eq!(after["bindings"], json!([expected_binding]));
    assert_eq!(after["hooks"].as_array().unwrap().len(), 1);
    assert_eq!(after["hooks"][0]["plugin_id"], other_key.to_string());
    assert_eq!(other_index, Some(42));
    assert_eq!(other_token.as_deref(), Some("unrelated-token"));
    assert!(other_routed);
    assert!(host.hook_list_response().await.hooks.is_empty());
    assert!(host.tokens.lock().await.is_empty());
    assert!(host.service_bindings().await.is_empty());
}
