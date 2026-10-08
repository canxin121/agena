use super::*;
use agena_domain::{ContentKind, ContentTextPage, SessionRelationKind};
use agena_storage::store::{
    InMemoryEngine, NewPart, NewSession, PartDelta, PartRole, PartVisibility, SessionFacade,
    SessionStore,
};

struct ContentFixture;

#[agena_plugin_host::sdk::async_trait]
impl agena_plugin_host::sdk::Plugin for ContentFixture {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest = agena_plugin_host::sdk::PluginManifest::new("agena", "content", "0.1.0");
        let mut tool = scoped_dynamic_tool_definition("read");
        tool.contract.input_schema = serde_json::json!({"type": "object"});
        manifest.tools = vec![tool];
        manifest
    }
}

async fn session(store: &dyn SessionStore, title: &str) -> i64 {
    store
        .create_session(NewSession {
            workspace_id: 1,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: title.into(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .unwrap()
        .id
}

async fn read(
    executor: &ToolExecutor,
    session_id: i64,
    input: serde_json::Value,
) -> Result<ContentTextPage, ToolError> {
    let invocation =
        ToolInvocation::new("content.read", StructuredObject::try_from(input).unwrap());
    let prepared = executor
        .prepare_invocation(&invocation, session_id, 100)
        .await?;
    let result = executor
        .execute_invocation_detailed(&prepared.invocation, session_id, 100)
        .await?;
    Ok(serde_json::from_value(result.output.to_json_payload().unwrap()).unwrap())
}

#[tokio::test]
async fn content_read_executes_through_the_registry_and_enforces_membership_and_typed_references() {
    let root = std::env::current_dir().unwrap();
    let mut config = PluginsConfig::default();
    config
        .list
        .insert("agena.content".into(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.content".parse().unwrap(),
            ContentFixture,
        )],
        config,
        workspace_root: root.clone(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let store = Arc::new(SessionFacade::new(InMemoryEngine::default(), 16));
    let executor = ToolExecutor::new(
        root,
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        plugins,
        None,
        None,
    )
    .with_content_store(store.clone());
    let owner = session(store.as_ref(), "source").await;
    let unrelated = session(store.as_ref(), "unrelated").await;
    let parts = store
        .submit_user_run(
            owner,
            vec![
                NewPart::pending("text", PartRole::Assistant, serde_json::json!({"text":""})),
                NewPart {
                    visibility: PartVisibility::User,
                    ..NewPart::pending("text", PartRole::Assistant, serde_json::json!({"text":""}))
                },
                NewPart::pending("text", PartRole::Assistant, serde_json::json!({"text":""})),
            ],
            None,
        )
        .await
        .unwrap()
        .parts;
    let body = format!("  {}\n\n", "中文🙂 ".repeat(1000));
    let mut resources = vec![];
    for (index, part) in parts.iter().skip(1).enumerate() {
        let writer = store
            .contents()
            .open(owner, part.part_id, ContentKind::Text)
            .await
            .unwrap();
        writer.append_text(&body).await.unwrap();
        let resource = writer.finish().await.unwrap();
        let mut reference = resource.reference();
        if index == 2 {
            reference.kind = ContentKind::Log;
        }
        store
            .update_part(
                owner,
                part.part_id,
                PartDelta {
                    content: Some(serde_json::json!({"text":"", "resources":[reference]})),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        resources.push(resource);
    }
    let mut input = serde_json::json!({"resource_id": resources[0].resource_id, "max_bytes": 257});
    let mut restored = String::new();
    loop {
        let page = read(&executor, owner, input).await.unwrap();
        assert!(!page.gap);
        assert!(page.slices.iter().all(|slice| slice.captured_at_ms > 0));
        for slice in page.slices {
            restored.push_str(&slice.text);
        }
        if !page.has_more {
            break;
        }
        input = serde_json::json!({"resource_id": resources[0].resource_id, "max_bytes":257,
            "epoch":page.next_position.after.epoch, "after":page.next_position.after.sequence, "offset":page.next_position.offset});
    }
    assert_eq!(restored, body);
    assert!(
        read(
            &executor,
            unrelated,
            serde_json::json!({"resource_id":resources[0].resource_id})
        )
        .await
        .is_err()
    );
    for resource in &resources[1..] {
        assert!(
            read(
                &executor,
                owner,
                serde_json::json!({"resource_id":resource.resource_id})
            )
            .await
            .is_err()
        );
    }
    assert!(
        read(
            &executor,
            owner,
            serde_json::json!({"resource_id":resources[0].resource_id,"offset":1})
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &executor,
            owner,
            serde_json::json!({"resource_id":resources[0].resource_id,"max_bytes":4097})
        )
        .await
        .is_err()
    );
    let branch = store
        .fork(owner, parts.last().unwrap().part_id, "fork".into())
        .await
        .unwrap();
    assert!(
        read(
            &executor,
            branch,
            serde_json::json!({"resource_id":resources[0].resource_id})
        )
        .await
        .is_ok()
    );
    store.delete(owner).await.unwrap();
    // Public session deletion removes descendants, including the fork. A
    // formerly valid resource must not remain readable through that session.
    assert!(
        read(
            &executor,
            branch,
            serde_json::json!({"resource_id":resources[0].resource_id})
        )
        .await
        .is_err()
    );
    assert!(
        store
            .load_part_ids(branch, &[parts[1].part_id])
            .await
            .is_err()
    );
    assert!(
        read(
            &executor,
            branch,
            serde_json::json!({"resource_id":resources[0].resource_id})
        )
        .await
        .is_err()
    );
}
