use super::*;
use crate::sdk::{Plugin, ToolContract, ToolDefinition};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ContractPlugin {
    calls: Arc<AtomicUsize>,
    definition: ToolDefinition,
    payload: Option<serde_json::Value>,
}

#[async_trait::async_trait]
impl Plugin for ContractPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("test", "contracts", "1.0.0");
        manifest.tools = vec![self.definition.clone()];
        manifest
    }

    async fn tool_invoke(&self, _: ToolInvokeInput) -> crate::sdk::Result<ToolInvokeOutput> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ToolInvokeOutput::from_parts(
            "Run",
            "Completed",
            "",
            self.payload.clone(),
            BTreeMap::new(),
            vec![],
        ))
    }
}

fn fixture(
    payload: Option<serde_json::Value>,
) -> (Arc<PluginHost>, RegisteredTool, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let plugin = ContractPlugin {
        calls: calls.clone(),
        definition: ToolDefinition {
            name: "run".into(),
            contract: ToolContract {
                input_schema: json!({"type":"object","properties":{"count":{"type":"integer","minimum":1}},"required":["count"]}),
                output_schema: json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}),
            },
            docs: Default::default(),
            runtime: Default::default(),
            tags: vec![],
        },
        payload,
    };
    let manifest = plugin.manifest();
    let key: PluginKey = "test.contracts".parse().unwrap();
    let tool = RegisteredTool::new(key.clone(), plugin.definition.clone()).unwrap();
    let mut host = PluginHost::new_empty();
    let mutable = Arc::get_mut(&mut host).unwrap();
    let owner = mutable._host_handle.begin_plugin_instance(key.clone());
    let loaded = Arc::new(LoadedPlugin::new_with_scope(
        "inproc",
        Default::default(),
        Arc::new(InProcessTransport::new(plugin)),
        owner,
        manifest,
        "test".into(),
        vec![],
    ));
    mutable.plugins.push(loaded.clone());
    mutable.plugins_by_id.insert(key, loaded);
    (host, tool, calls)
}

fn input(value: serde_json::Value) -> ToolInvokeInput {
    ToolInvokeInput {
        tool_name: "run".into(),
        session_id: 1,
        call_id: 1,
        workspace_root: ".".into(),
        input: value,
    }
}

fn assert_output_violation(error: PluginError) {
    assert_eq!(
        error.failure.code.as_str(),
        "tool.output_contract_violation"
    );
    assert_eq!(error.failure.retry, agena_failure::RetryDirective::Never);
    assert_eq!(
        error.failure.impact,
        agena_failure::FailureImpact::PartialSuccess
    );
    assert_eq!(
        error.diagnostic.data.unwrap()["agena_public_problem"],
        crate::schema::OUTPUT_CONTRACT_VIOLATION_MARKER,
    );
}

#[tokio::test]
async fn invalid_input_is_rejected_before_either_plugin_entry_point() {
    let (host, tool, calls) = fixture(Some(json!({"ok":true})));
    let error = host
        .invoke_tool(&tool, input(json!({"count":0})), None)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::InvalidParams);
    let error = host
        .invoke_tool_stream(&tool, input(json!({"count":"invalid"})))
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, PluginErrorKind::InvalidParams);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn ordinary_and_streaming_results_validate_business_payloads() {
    let (host, tool, calls) = fixture(Some(json!({"ok":"invalid"})));
    let error = host
        .invoke_tool(&tool, input(json!({"count":1})), None)
        .await
        .unwrap_err();
    assert_output_violation(error);
    let stream = host
        .invoke_tool_stream(&tool, input(json!({"count":1})))
        .await
        .unwrap();
    let error = stream.end.await.unwrap().unwrap_err();
    assert_output_violation(error);
    assert_eq!(calls.load(Ordering::Relaxed), 2);

    let (host, tool, _) = fixture(Some(json!({"ok":true})));
    assert!(
        host.invoke_tool(&tool, input(json!({"count":1})), None)
            .await
            .is_ok()
    );
    let stream = host
        .invoke_tool_stream(&tool, input(json!({"count":1})))
        .await
        .unwrap();
    assert_eq!(
        stream.end.await.unwrap().unwrap().payload,
        Some(json!({"ok":true}))
    );
}
