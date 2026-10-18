use super::*;

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

async fn read(
    executor: &ToolExecutor,
    session_id: i64,
    input: serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    let invocation =
        ToolInvocation::new("content.read", StructuredObject::try_from(input).unwrap());
    let prepared = executor
        .prepare_invocation(&invocation, session_id, 100)
        .await?;
    let result = executor
        .execute_invocation_detailed(&prepared.invocation, session_id, 100)
        .await?;
    Ok(result.output.to_json_payload().unwrap())
}

#[tokio::test]
async fn content_read_executes_through_the_registry_and_enforces_membership_and_epochs() {
    let fixture = crate::kernel_fixture::fixture().await;
    let root = fixture._root.path().to_path_buf();
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
    .with_kernel_reader(fixture.runtime.ports.reader.clone());
    let owner = fixture.session;
    let unrelated = fixture
        .runtime
        .ports
        .sessions
        .create(
            agena_storage::kernel::RequestId::new(),
            1,
            "unrelated".into(),
        )
        .await
        .unwrap()
        .value
        .id;
    let writer = fixture
        .tool
        .begin_text(agena_storage::kernel::RequestId::new())
        .await
        .unwrap()
        .value;
    let body = format!("  {}\n\n", "中文🙂 ".repeat(1000));
    writer.append(&body).await.unwrap();
    crate::kernel_fixture::complete(&fixture.tool).await;
    let id = writer.id();
    let mut input = serde_json::json!({"resource_id": id, "max_bytes": 257});
    let mut restored = String::new();
    loop {
        let page = read(&executor, owner.get(), input).await.unwrap();
        restored.push_str(page["text"].as_str().unwrap());
        if page["has_more"] == false {
            break;
        }
        input = serde_json::json!({"resource_id":id,"max_bytes":257,
            "epoch":page["next_position"]["epoch"],"after":page["next_position"]["after"],"offset":page["next_position"]["offset"]});
    }
    assert_eq!(restored, body);
    assert!(
        read(
            &executor,
            unrelated.get(),
            serde_json::json!({"resource_id":id})
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &executor,
            owner.get(),
            serde_json::json!({"resource_id":id,"offset":1})
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &executor,
            owner.get(),
            serde_json::json!({"resource_id":id,"max_bytes":4097})
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &executor,
            owner.get(),
            serde_json::json!({"resource_id":id,"epoch":agena_domain::ContentEpoch::new()})
        )
        .await
        .is_err()
    );
    let entry = fixture
        .runtime
        .ports
        .reader
        .tool(owner, fixture.tool.id())
        .await
        .unwrap()
        .outputs[0]
        .entry;
    let branch = fixture
        .runtime
        .ports
        .sessions
        .fork(agena_storage::kernel::RequestId::new(), owner, entry)
        .await
        .unwrap()
        .value
        .id;
    assert!(
        read(
            &executor,
            branch.get(),
            serde_json::json!({"resource_id":id})
        )
        .await
        .is_ok()
    );
    fixture
        .runtime
        .ports
        .sessions
        .delete(agena_storage::kernel::RequestId::new(), branch)
        .await
        .unwrap();
    assert!(
        read(
            &executor,
            branch.get(),
            serde_json::json!({"resource_id":id})
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &executor,
            owner.get(),
            serde_json::json!({"resource_id":id})
        )
        .await
        .is_ok()
    );
    fixture.runtime.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn content_read_resumes_large_events_without_losing_channel_or_utf8() {
    let fixture = crate::kernel_fixture::fixture().await;
    let writer = fixture
        .tool
        .begin_log(agena_storage::kernel::RequestId::new())
        .await
        .unwrap()
        .value;
    let body = "中文🙂 ".repeat(1000);
    writer
        .append(agena_domain::CommandOutputStream::Stderr, &body)
        .await
        .unwrap();
    crate::kernel_fixture::complete(&fixture.tool).await;
    let root = fixture._root.path().to_path_buf();
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
    .with_kernel_reader(fixture.runtime.ports.reader.clone());
    let id = writer.id();
    let mut input = serde_json::json!({"resource_id":id,"max_bytes":257});
    let mut restored = String::new();
    loop {
        let page = read(&executor, fixture.session.get(), input).await.unwrap();
        for fragment in page["fragments"].as_array().unwrap() {
            assert_eq!(fragment["channel"], "stderr");
            let text = fragment["text"].as_str().unwrap();
            assert!(text.len() <= 257);
            restored.push_str(text);
        }
        if page["has_more"] == false {
            break;
        }
        input = serde_json::json!({"resource_id":id,"max_bytes":257,
            "epoch":page["next_position"]["epoch"],"after":page["next_position"]["after"],"offset":page["next_position"]["offset"]});
    }
    assert_eq!(restored, body);
    fixture.runtime.ports.shutdown().await.unwrap();
}
