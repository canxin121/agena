use super::*;
use agena_domain::{StructuredObject, ToolApiCall, ToolApiFunction};
use serde_json::json;

fn call(function: ToolApiFunction, arguments: serde_json::Value) -> ToolApiCall {
    ToolApiCall {
        function,
        arguments: StructuredObject::try_from(arguments).unwrap(),
    }
}

fn catalog() -> ToolApiCatalog {
    ToolApiCatalog {
        tools: vec![
            CatalogTool {
                name: "fs.read".into(),
                canonical_name: "agena.fs.read".into(),
                plugin_id: "agena.fs".into(),
                summary: "Read a file".into(),
                help: "Read UTF-8 files; line numbers start at 1.".into(),
                tags: vec!["read_only".into()],
                input_schema: json!({"type":"object","required":["file_path"],"properties":{"file_path":{"type":"string"}}}),
                input_usage: Some("file_path: required string".into()),
                input_examples: vec![r#"{"file_path":"a.txt"}"#.into()],
                output_schema: json!({"type":"object"}),
            },
            CatalogTool {
                name: "shell.exec".into(),
                canonical_name: "agena.shell.exec".into(),
                plugin_id: "agena.shell".into(),
                summary: "Run a command".into(),
                help: "Wait for completion.".into(),
                tags: vec!["shell".into()],
                input_schema: json!({"type":"object"}),
                input_usage: None,
                input_examples: vec![],
                output_schema: json!({}),
            },
        ],
        plugins: vec![CatalogPlugin {
            id: "agena.fs".into(),
            version: "1".into(),
            summary: "Filesystem".into(),
            tags: vec![],
            tool_count: 1,
        }],
    }
}

#[test]
fn protocol_registry_exists_without_any_plugin_host_or_execution_registry() {
    let definitions = ToolApiRegistry.definitions();
    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        ["tools_list", "tools_help", "tools_call", "plugins_list"]
    );
    for definition in definitions {
        assert_eq!(definition.input_schema["type"], "object");
        let wire = serde_json::to_value(definition).unwrap();
        assert!(wire.get("handler_key").is_none());
        assert!(wire.get("plugin_name").is_none());
    }
    let result = ToolApiCatalog::default()
        .dispatch(&call(ToolApiFunction::List, json!({})))
        .unwrap();
    let ToolApiAction::Respond(response) = result else {
        panic!("discovery must not execute");
    };
    assert!(response.output_text.contains("returned 0 of 0"));
}

#[test]
fn only_tools_call_produces_an_execution_request_and_preserves_inner_input() {
    let input = json!({"file_path":"a.txt","offset":7});
    let action = ToolApiCatalog::default()
        .dispatch(&call(
            ToolApiFunction::Call,
            json!({
                "tool":"fs.read","input":input,
            }),
        ))
        .unwrap();
    let ToolApiAction::Execute(request) = action else {
        panic!("expected execution boundary");
    };
    assert_eq!(request.tool, "fs.read");
    assert_eq!(serde_json::Value::from(request.input), input);
}

#[test]
fn malformed_call_envelopes_and_nested_protocol_calls_never_reach_execution() {
    for arguments in [
        json!({}),
        json!({"tool":"fs.read"}),
        json!({"tool":"fs.read","input":"{}"}),
        json!({"tool":"fs.read","input":[],"extra":true}),
        json!({"tool":"fs.read","input":{},"extra":true}),
        json!({"tool":" ","input":{}}),
        json!({"tool":"tools_call","input":{}}),
        json!({"tool":"tools_help","input":{}}),
        json!({"tool":"tools_list","input":{}}),
        json!({"tool":"plugins_list","input":{}}),
        json!({"tool":"approve_action","input":{}}),
        json!({"tool":"block_action","input":{}}),
        json!({"tool":"agena.tools.call","input":{}}),
    ] {
        assert!(
            catalog()
                .dispatch(&call(ToolApiFunction::Call, arguments.clone()))
                .is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn tool_listing_filters_plugin_batches_and_retains_pagination() {
    let action = catalog()
        .dispatch(&call(
            ToolApiFunction::List,
            json!({
                "plugin":["fs","agena.shell"],"limit":1,
            }),
        ))
        .unwrap();
    let ToolApiAction::Respond(response) = action else {
        panic!();
    };
    assert!(response.output_text.contains("returned 1 of 2"));
    assert!(response.output_text.contains("fs.read"));
    assert!(!response.output_text.contains("shell.exec"));
    assert!(response.output_text.contains("offset: 1"));
    let ToolApiAction::Respond(response) = catalog()
        .dispatch(&call(ToolApiFunction::List, json!({"offset":1})))
        .unwrap()
    else {
        panic!();
    };
    assert!(response.output_text.contains("shell.exec"));
    assert!(response.output_text.contains("More available: no"));
}

#[test]
fn help_returns_live_contracts_and_batch_failures_without_executing() {
    let action = catalog()
        .dispatch(&call(
            ToolApiFunction::Help,
            json!({"tool":["agena.fs.read","missing.tool"]}),
        ))
        .unwrap();
    let ToolApiAction::Respond(response) = action else {
        panic!();
    };
    assert!(response.output_text.contains("file_path"));
    assert!(response.output_text.contains("line numbers start at 1"));
    assert!(response.output_text.contains("file_path: required string"));
    assert!(response.output_text.contains(r#"{"file_path":"a.txt"}"#));
    assert!(response.output_text.contains("missing.tool"));
    assert!(response.summary.contains("1 unavailable"));
    assert!(
        catalog()
            .dispatch(&call(ToolApiFunction::Help, json!({"tool":"tools_list"})))
            .is_err()
    );
}

#[test]
fn empty_batches_unknown_fields_and_invalid_pagination_are_rejected() {
    for arguments in [
        json!({"plugin":[]}),
        json!({"plugin":[""]}),
        json!({"offset":-1}),
        json!({"unknown":true}),
    ] {
        assert!(
            catalog()
                .dispatch(&call(ToolApiFunction::List, arguments))
                .is_err()
        );
    }
    for arguments in [
        json!({"tool":[]}),
        json!({"tool":[""]}),
        json!({"tool":"fs.read","extra":true}),
    ] {
        assert!(
            catalog()
                .dispatch(&call(ToolApiFunction::Help, arguments))
                .is_err()
        );
    }
}

#[test]
fn plugin_listing_is_a_catalog_query_with_no_protocol_plugin() {
    let action = catalog()
        .dispatch(&call(ToolApiFunction::PluginsList, json!({})))
        .unwrap();
    let ToolApiAction::Respond(response) = action else {
        panic!();
    };
    assert!(response.output_text.contains("agena.fs"));
    assert!(response.output_text.contains("1 tool(s)"));
    assert!(!response.output_text.contains("agena.tools"));
}

#[test]
fn shell_help_retains_the_visible_shell_family_without_plugin_handles() {
    let mut catalog = catalog();
    let mut spawn = catalog.tools[1].clone();
    spawn.name = "shell.spawn".into();
    spawn.canonical_name = "agena.shell.spawn".into();
    catalog.tools.push(spawn);
    let ToolApiAction::Respond(response) = catalog
        .dispatch(&call(ToolApiFunction::Help, json!({"tool":"shell.exec"})))
        .unwrap()
    else {
        panic!();
    };
    assert!(response.output_text.contains("Available Shell tools:"));
    assert!(response.output_text.contains("`shell.exec`"));
    assert!(response.output_text.contains("`shell.spawn`"));
    assert!(!response.output_text.contains("`shell.stop`"));
}
