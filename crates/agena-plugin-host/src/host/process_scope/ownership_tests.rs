use super::*;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
enum Resource {
    Tool,
    Display,
    Theme,
}

impl Resource {
    fn registration(self, id: &str, text: &str) -> (&'static str, Value) {
        match self {
            Self::Tool => (
                "host/tool.registry.register",
                json!({"request": {"tool": {
                    "name": id, "docs": {"summary": text},
                    "contract": {"input_schema": {"type": "object"}}
                }}}),
            ),
            Self::Display => (
                "host/ui.display.contribute",
                json!({"request": {"contribution": {
                    "id": id, "kind": "status_line_text", "priority": 0,
                    "content": {"kind": "text", "text": text}
                }}}),
            ),
            Self::Theme => (
                "host/ui.theme.register",
                json!({"request": {"id": id, "display_name": text, "colors": {}}}),
            ),
        }
    }

    fn removal(self, id: &str) -> (&'static str, Value) {
        match self {
            Self::Tool => (
                "host/tool.registry.remove",
                json!({"request": {"name": id}}),
            ),
            Self::Display => (
                "host/ui.display.remove",
                json!({"request": {"contribution_id": id}}),
            ),
            Self::Theme => ("host/ui.theme.remove", json!({"request": {"id": id}})),
        }
    }

    fn effect_kind(self) -> &'static str {
        match self {
            Self::Tool => "host.tool",
            Self::Display => "host.display",
            Self::Theme => "host.theme",
        }
    }

    fn visible(self, host: &HostHandle) -> BTreeMap<String, String> {
        match self {
            Self::Tool => host
                .registered_tool_list_response()
                .unwrap()
                .tools
                .into_iter()
                .map(|tool| (tool.tool.name, tool.tool.docs.summary.unwrap()))
                .collect(),
            Self::Display => host
                .display_list_response()
                .into_iter()
                .map(|item| {
                    let crate::sdk::PluginDisplayContent::Text { text } = item.contribution.content
                    else {
                        panic!("expected a text contribution")
                    };
                    (item.contribution.id, text)
                })
                .collect(),
            Self::Theme => host
                .theme_list_response()
                .themes
                .into_iter()
                .map(|theme| (theme.id, theme.display_name))
                .collect(),
        }
    }
}

fn host() -> (Arc<HostHandle>, Arc<PluginEffectScope>) {
    let key: PluginKey = "test.exact-owner".parse().unwrap();
    let host = Arc::new(HostHandle::new_with_registry(
        Arc::new(crate::sdk::host_api::NoopHostClient),
        Arc::new(RwLock::new(PluginToolRegistry::new())),
        Arc::new(RwLock::new(HashMap::from([(key.clone(), 0)]))),
    ));
    let logical = host.begin_plugin_instance(key);
    (host, logical)
}

fn child(logical: &Arc<PluginEffectScope>) -> Arc<PluginEffectScope> {
    let process = PluginEffectScope::new(logical.plugin_id().clone());
    logical.own_child(process.clone()).unwrap();
    process
}

async fn register(
    host: &Arc<HostHandle>,
    owner: &Arc<PluginEffectScope>,
    resource: Resource,
    id: &str,
    text: &str,
) {
    let (method, params) = resource.registration(id, text);
    host.run_in_callback_effect_scope(
        owner.clone(),
        host.handle_call_for_plugin(&owner.plugin_id().to_string(), method, params),
    )
    .await
    .unwrap();
}

fn active_labels(owner: &PluginEffectScope, resource: Resource) -> Vec<String> {
    owner
        .inspect()
        .effects
        .into_iter()
        .filter(|effect| {
            effect.kind == resource.effect_kind()
                && effect.state == crate::effect_scope::PluginEffectState::Active
        })
        .map(|effect| effect.label)
        .collect()
}

async fn old_owner_cleanup_preserves_replacement(resource: Resource) {
    // Exercise a process handing a key to the logical owner and to a sibling
    // process. Values are intentionally identical: content equality does not
    // identify the registration that cleanup is allowed to remove.
    for replacement_is_logical in [true, false] {
        let (host, logical) = host();
        let old = child(&logical);
        let replacement = if replacement_is_logical {
            logical.clone()
        } else {
            child(&logical)
        };
        register(&host, &old, resource, "shared", "same content").await;
        register(&host, &replacement, resource, "shared", "same content").await;
        let expected = resource.visible(&host);
        let before_generation = host.tool_registry.read().unwrap().generation();
        let old_report = old.dispose().await;
        let actual = resource.visible(&host);
        let after_generation = host.tool_registry.read().unwrap().generation();
        let new_report = replacement.dispose().await;
        let empty = resource.visible(&host).is_empty();
        logical.dispose().await;
        assert!(old_report.errors.is_empty() && new_report.errors.is_empty());
        assert_eq!(
            actual, expected,
            "{resource:?}: cleanup removed a replacement"
        );
        assert_eq!(
            after_generation, before_generation,
            "stale cleanup changed the catalog epoch"
        );
        assert!(
            empty,
            "the actual owner must still clean up its registration"
        );
    }
}

#[tokio::test]
async fn old_tool_owner_cleanup_preserves_replacement() {
    old_owner_cleanup_preserves_replacement(Resource::Tool).await;
}

#[tokio::test]
async fn old_display_owner_cleanup_preserves_replacement() {
    old_owner_cleanup_preserves_replacement(Resource::Display).await;
}

#[tokio::test]
async fn old_theme_owner_cleanup_preserves_replacement() {
    old_owner_cleanup_preserves_replacement(Resource::Theme).await;
}

async fn process_handoff_copies_only_current_owned_entries(resource: Resource) {
    let (source, logical) = host();
    let process = child(&logical);
    let sibling = child(&logical);
    register(&source, &logical, resource, "logical", "root").await;
    register(&source, &process, resource, "process", "live").await;
    register(&source, &sibling, resource, "sibling", "other process").await;
    register(&source, &process, resource, "shared", "obsolete").await;
    register(&source, &logical, resource, "shared", "root replacement").await;
    process.quiesce().await;
    let snapshot = source.process_contributions(&process);
    let (successor, successor_logical) = host();
    let successor_process = child(&successor_logical);
    successor
        .import_process_contributions(successor_process.clone(), snapshot)
        .await
        .unwrap();
    let imported = resource.visible(&successor);
    let report = process.dispose().await;
    let source_after = resource.visible(&source);
    successor_process.dispose().await;
    let successor_empty = resource.visible(&successor).is_empty();
    logical.dispose().await;
    successor_logical.dispose().await;
    assert!(report.errors.is_empty());
    assert_eq!(
        imported,
        BTreeMap::from([("process".into(), "live".into())])
    );
    assert_eq!(
        source_after,
        BTreeMap::from([
            ("logical".into(), "root".into()),
            ("shared".into(), "root replacement".into()),
            ("sibling".into(), "other process".into()),
        ])
    );
    assert!(successor_empty);
}

#[tokio::test]
async fn process_tool_handoff_copies_only_current_owned_entries() {
    process_handoff_copies_only_current_owned_entries(Resource::Tool).await;
}

#[tokio::test]
async fn process_display_handoff_copies_only_current_owned_entries() {
    process_handoff_copies_only_current_owned_entries(Resource::Display).await;
}

#[tokio::test]
async fn process_theme_handoff_copies_only_current_owned_entries() {
    process_handoff_copies_only_current_owned_entries(Resource::Theme).await;
}

async fn explicit_remove_releases_actual_owner(resource: Resource) {
    let (host, logical) = host();
    let process = child(&logical);
    register(&host, &process, resource, "shared", "live").await;
    let (method, params) = resource.removal("shared");
    host.handle_call_for_plugin(&logical.plugin_id().to_string(), method, params)
        .await
        .unwrap();
    let empty = resource.visible(&host).is_empty();
    let active = active_labels(&process, resource);
    register(&host, &logical, resource, "shared", "replacement").await;
    process.dispose().await;
    let after = resource.visible(&host);
    logical.dispose().await;
    assert!(empty);
    assert!(
        active.is_empty(),
        "removed resource retains a live disposer: {active:?}"
    );
    assert_eq!(
        after,
        BTreeMap::from([("shared".into(), "replacement".into())])
    );
}

#[tokio::test]
async fn explicit_tool_remove_releases_actual_owner() {
    explicit_remove_releases_actual_owner(Resource::Tool).await;
}

#[tokio::test]
async fn explicit_display_remove_releases_actual_owner() {
    explicit_remove_releases_actual_owner(Resource::Display).await;
}

#[tokio::test]
async fn explicit_theme_remove_releases_actual_owner() {
    explicit_remove_releases_actual_owner(Resource::Theme).await;
}

#[tokio::test]
async fn obsolete_manifest_cleanup_preserves_successor_tools() {
    let (host, previous) = host();
    let mut manifest = PluginManifest::new("test", "exact-owner", "0.1.0");
    manifest
        .tools
        .push(serde_json::from_value(json!({"name": "shared"})).unwrap());
    host.tool_registry
        .write()
        .unwrap()
        .extend_from_plugin(previous.plugin_id(), &manifest.tools)
        .unwrap();
    host.own_manifest_resources(previous.plugin_id(), &manifest)
        .unwrap();
    let successor = host.begin_plugin_instance(previous.plugin_id().clone());
    register(&host, &successor, Resource::Tool, "shared", "successor").await;
    host.dispose_plugin_resources_for_scope(previous.plugin_id(), &previous)
        .await;
    let actual = Resource::Tool.visible(&host);
    successor.dispose().await;
    assert_eq!(
        actual,
        BTreeMap::from([("shared".into(), "successor".into())])
    );
}

#[tokio::test]
async fn repeated_registration_and_ownership_return_retire_only_superseded_effects() {
    for resource in [Resource::Tool, Resource::Display, Resource::Theme] {
        let (host, logical) = host();
        let first = child(&logical);
        let second = child(&logical);
        for owner in [&first, &first, &second, &first] {
            register(&host, owner, resource, "shared", "identical").await;
        }
        let first_labels = active_labels(&first, resource);
        let second_labels = active_labels(&second, resource);
        second.dispose().await;
        let actual = resource.visible(&host);
        first.dispose().await;
        let empty = resource.visible(&host).is_empty();
        logical.dispose().await;
        assert_eq!(first_labels, ["shared"]);
        assert!(second_labels.is_empty());
        assert_eq!(
            actual,
            BTreeMap::from([("shared".into(), "identical".into())])
        );
        assert!(empty);
    }
}

#[tokio::test]
async fn manifest_ownership_keeps_init_overrides_and_restored_defaults_separate() {
    let (host, logical) = host();
    let process = child(&logical);
    let mut manifest = PluginManifest::new("test", "exact-owner", "0.1.0");
    for name in ["default", "shared"] {
        manifest.tools.push(
            serde_json::from_value(json!({
                "name": name, "docs": {"summary": "manifest"}
            }))
            .unwrap(),
        );
    }
    host.tool_registry
        .write()
        .unwrap()
        .extend_from_plugin(logical.plugin_id(), &manifest.tools)
        .unwrap();
    register(&host, &process, Resource::Tool, "shared", "init override").await;
    host.own_manifest_resources(logical.plugin_id(), &manifest)
        .unwrap();
    let snapshot = host.process_contributions(&process);
    process.dispose().await;
    let after_process = Resource::Tool.visible(&host);
    host.restore_manifest_tools(logical.plugin_id(), &manifest)
        .unwrap();
    let restored = Resource::Tool.visible(&host);
    let restored_owner = host.process_contributions(&logical).tools;
    logical.dispose().await;
    let empty = Resource::Tool.visible(&host).is_empty();
    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(
        snapshot.tools[0].docs.summary.as_deref(),
        Some("init override")
    );
    assert_eq!(
        after_process,
        BTreeMap::from([("default".into(), "manifest".into())])
    );
    assert_eq!(
        restored,
        BTreeMap::from([
            ("default".into(), "manifest".into()),
            ("shared".into(), "manifest".into())
        ])
    );
    assert_eq!(restored_owner.len(), 2);
    assert!(empty, "restored defaults must belong to the logical scope");
}

#[tokio::test]
async fn cloned_tool_registry_mutation_does_not_release_live_ownership() {
    let (host, logical) = host();
    register(&host, &logical, Resource::Tool, "shared", "live").await;
    let mut snapshot = host.tool_registry.read().unwrap().clone();
    snapshot.remove_plugin(logical.plugin_id());
    let live_effects = active_labels(&logical, Resource::Tool);
    let live_value = Resource::Tool.visible(&host);
    logical.dispose().await;
    assert_eq!(snapshot.count(), 0);
    assert_eq!(live_effects, ["shared"]);
    assert_eq!(
        live_value,
        BTreeMap::from([("shared".into(), "live".into())])
    );
    assert!(Resource::Tool.visible(&host).is_empty());
}

#[tokio::test]
async fn identical_resource_names_in_other_plugins_keep_their_owners() {
    for resource in [Resource::Tool, Resource::Display, Resource::Theme] {
        let (host, logical) = host();
        let other_key: PluginKey = "test.other-owner".parse().unwrap();
        host.plugin_indices
            .write()
            .unwrap()
            .insert(other_key.clone(), 1);
        let other = host.begin_plugin_instance(other_key);
        register(&host, &logical, resource, "shared", "original plugin").await;
        register(&host, &other, resource, "shared", "other plugin").await;
        host.dispose_plugin_resources_for_scope(logical.plugin_id(), &logical)
            .await;
        let remaining = resource.visible(&host);
        let other_effects = active_labels(&other, resource);
        other.dispose().await;
        assert_eq!(
            remaining,
            BTreeMap::from([("shared".into(), "other plugin".into())])
        );
        assert_eq!(other_effects, ["shared"]);
        assert!(resource.visible(&host).is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_callbacks_racing_old_cleanup_keep_new_registrations() {
    for resource in [Resource::Tool, Resource::Display, Resource::Theme] {
        for _ in 0..20 {
            let (host, logical) = host();
            let old = child(&logical);
            let current = child(&logical);
            let old_handler = host.host_handler_for_process(logical.clone(), old.clone());
            let current_handler = host.host_handler_for_process(logical.clone(), current.clone());
            let (method, params) = resource.registration("shared", "old");
            old_handler(method.into(), params).await.unwrap();
            let start = Arc::new(tokio::sync::Barrier::new(2));
            let cleanup = tokio::spawn({
                let start = start.clone();
                let old = old.clone();
                async move {
                    start.wait().await;
                    old.dispose().await
                }
            });
            start.wait().await;
            let (method, params) = resource.registration("shared", "current");
            current_handler(method.into(), params).await.unwrap();
            let report = tokio::time::timeout(std::time::Duration::from_secs(5), cleanup)
                .await
                .unwrap()
                .unwrap();
            let actual = resource.visible(&host);
            let (method, params) = resource.registration("shared", "stale callback");
            let stale = old_handler(method.into(), params).await;
            let after_stale = resource.visible(&host);
            logical.dispose().await;
            assert!(report.errors.is_empty());
            assert_eq!(
                actual,
                BTreeMap::from([("shared".into(), "current".into())])
            );
            assert!(stale.is_err());
            assert_eq!(after_stale, actual);
            assert!(resource.visible(&host).is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_flight_tool_disposer_checks_identity_after_replacement() {
    let (host, logical) = host();
    let old = child(&logical);
    let current = child(&logical);
    let key: ToolKey = "test.exact-owner.shared".parse().unwrap();
    let (entered, running) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let registry = Arc::downgrade(&host.tool_registry);
    let cleanup_key = key.clone();
    // Pause the real registry disposer after the effect has left the scope's
    // live stack. Releasing the old handle can no longer stop this callback;
    // only checking its identity under the registry lock protects the value.
    let ownership = crate::registration_owner::RegistrationOwner::new(
        &old,
        "host.tool",
        "shared".into(),
        move |id| {
            entered
                .send(())
                .map_err(|_| "test receiver closed".to_string())?;
            paused
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            if let Some(registry) = registry.upgrade() {
                registry.write().unwrap().remove_exact(&cleanup_key, &id);
            }
            Ok(())
        },
    )
    .unwrap();
    host.tool_registry.write().unwrap().upsert_owned(
        RegisteredTool::new(
            logical.plugin_id().clone(),
            serde_json::from_value(json!({
                "name": "shared", "docs": {"summary": "same"}
            }))
            .unwrap(),
        )
        .unwrap(),
        ownership,
    );
    let cleanup = tokio::spawn(async move { old.dispose().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap();
    register(&host, &current, Resource::Tool, "shared", "same").await;
    let before_generation = host.tool_registry.read().unwrap().generation();
    resume.send(()).unwrap();
    let report = tokio::time::timeout(std::time::Duration::from_secs(5), cleanup)
        .await
        .unwrap()
        .unwrap();
    let actual = Resource::Tool.visible(&host);
    let after_generation = host.tool_registry.read().unwrap().generation();
    logical.dispose().await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(actual, BTreeMap::from([("shared".into(), "same".into())]));
    assert_eq!(after_generation, before_generation);
    assert!(Resource::Tool.visible(&host).is_empty());
}

#[tokio::test]
async fn tool_removal_listener_can_read_the_committed_registry() {
    let (host, logical) = host();
    register(&host, &logical, Resource::Tool, "shared", "live").await;
    let registry = host.tool_registry.clone();
    let observed = Arc::new(std::sync::Mutex::new(None));
    let callback_observed = observed.clone();
    host.set_tool_registry_event_listener(Some(Arc::new(move |event| {
        // A normal blocking read would deadlock if notification still held
        // the write lock. Record try_read's result so this regression fails
        // promptly and can always clean up its owner.
        let readable = registry.try_read().is_ok_and(|registry| {
            event.kind == ToolRegistryChangeKind::Removed
                && registry.lookup_tool_by_key(&event.tool_key).is_none()
                && registry.generation() == event.generation
        });
        *callback_observed.lock().unwrap() = Some(readable);
    })));
    let (method, params) = Resource::Tool.removal("shared");
    let response = host
        .handle_call_for_plugin(&logical.plugin_id().to_string(), method, params)
        .await
        .unwrap();
    let actual = *observed.lock().unwrap();
    logical.dispose().await;
    assert!(response["tool"].is_object());
    assert_eq!(
        actual,
        Some(true),
        "listener cannot read the committed removal"
    );
}
