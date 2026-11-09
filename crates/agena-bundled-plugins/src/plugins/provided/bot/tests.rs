use super::*;
use agena_plugin_host::sdk::Plugin;

#[test]
fn bot_manifest_exposes_only_native_session_and_integration_tools() {
    let manifest = BotPlugin::new().manifest();
    assert_eq!(manifest.namespace, "agena");
    assert_eq!(manifest.name, "bot");
    let tools = manifest
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(tools, ["connections", "sessions"].into_iter().collect());
    assert!(!tools.contains("manage"));
    let prompt = include_str!("../../../../assets/prompts/operator-bot.md");
    assert!(prompt.contains("bot.sessions"));
    for removed in [
        "work.manage",
        "work.sessions",
        "conversation_work",
        "Task Worker",
    ] {
        assert!(
            !prompt.contains(removed),
            "retired instruction remains: {removed}"
        );
    }
}

#[test]
fn management_contract_has_native_model_shape_and_all_documented_actions() {
    use agena_plugin_host::sdk::host_api::BotRequest;
    let schema = serde_json::to_value(schemars::schema_for!(BotRequest)).unwrap();
    let encoded = schema.to_string();
    for action in [
        "project",
        "read_text",
        "organize",
        "fork",
        "delete",
        "models",
        "configure",
        "queue",
        "queue_pause",
        "queue_edit",
        "queue_remove",
        "export",
        "send",
        "stop",
    ] {
        assert!(
            encoded.contains(&format!("\"{action}\"")),
            "missing action {action}"
        );
    }
    for field in [
        "provider_id",
        "adapter_id",
        "model_id",
        "expected_control_version",
        "expected_entry_count",
        "at_message_id",
        "reset_model",
        "paused",
    ] {
        assert!(encoded.contains(field), "missing field {field}");
    }
    let request:BotRequest=serde_json::from_value(serde_json::json!({"action":"configure","session_id":1,"expected_version":1,"model":{"provider_id":"provider","model_id":"model"}})).unwrap();
    assert_eq!(request.model.unwrap().provider_id.as_ref(), "provider");
    let prompt = include_str!("../../../../assets/prompts/operator-bot.md");
    for guidance in [
        "never ask humans to look up IDs",
        "existing conversation",
        "expected_control_version",
        "human must then confirm",
        "Do not silently overwrite",
        "polling forever",
    ] {
        assert!(
            prompt.contains(guidance),
            "missing workflow guidance {guidance}"
        );
    }
}

#[test]
fn model_selection_rejects_ignored_nested_policy_fields() {
    let parsed =
        serde_json::from_value::<agena_plugin_host::sdk::host_api::BotRequest>(serde_json::json!({
            "action":"configure","session_id":1,"expected_version":1,
            "model":{"provider_id":"provider","model_id":"model","permission":"allow_all"}
        }));
    assert!(parsed.is_err());
}
