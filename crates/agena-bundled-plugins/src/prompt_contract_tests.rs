//! Validate prompt examples against production contracts without launching a
//! plugin host, model, tool invocation, or external service.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::ToolApiFunction;
use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::PluginKey;
use agena_runtime_tools::tool::tool_registry::{
    ExecutionTool, ToolApiBinding, compact_tool_call_name,
};
use serde_json::{Value, json};

fn registered_tools() -> Vec<RegisteredTool> {
    crate::capability_manifest::bundled_plugin_manifests()
        .into_iter()
        .flat_map(|(manifest, _)| {
            let key = PluginKey::new(manifest.namespace, manifest.name).unwrap();
            manifest
                .tools
                .into_iter()
                .map(move |definition| RegisteredTool::new(key.clone(), definition).unwrap())
        })
        .collect()
}

fn validate(schema: &Value, input: &Value) {
    let validator = jsonschema::validator_for(schema).expect("valid production schema");
    let errors = validator
        .iter_errors(input)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "invalid prompt input {input}: {errors:?}"
    );
}

#[test]
fn prompt_call_examples_match_live_gateway_and_execution_schemas() {
    let tools = registered_tools();
    let gateways = tools
        .iter()
        .cloned()
        .filter_map(ToolApiBinding::from_registered_tool)
        .chain(std::iter::once(ToolApiBinding::call_gateway()))
        .map(|binding| (binding.function_name().to_owned(), binding))
        .collect::<BTreeMap<_, _>>();
    let prompt = agena_runtime_contracts::identity::system_prompt();
    let mut example_functions = BTreeSet::new();
    for code in prompt.split('`').skip(1).step_by(2) {
        let Some((function, arguments)) = code.split_once('(') else {
            continue;
        };
        let binding = gateways
            .get(function)
            .unwrap_or_else(|| panic!("prompt calls an undeclared function: {code}"));
        let arguments: Value =
            serde_json::from_str(arguments.strip_suffix(')').expect("complete call example"))
                .expect("prompt example uses valid JSON");
        validate(&binding.definition().input_schema, &arguments);
        example_functions.insert(binding.function());
        if binding.function() == ToolApiFunction::Call {
            let name = arguments["tool"].as_str().unwrap();
            let execution = tools
                .iter()
                .find(|tool| compact_tool_call_name(&tool.canonical_name()) == name)
                .cloned()
                .and_then(ExecutionTool::from_registered_tool)
                .expect("prompt calls a real execution tool");
            validate(&execution.input_schema(), &arguments["input"]);
        }
    }
    assert_eq!(
        example_functions,
        BTreeSet::from([ToolApiFunction::Help, ToolApiFunction::Call])
    );

    // These common malformed envelopes must not become accepted examples.
    let schema = ToolApiBinding::call_gateway().definition().input_schema;
    let validator = jsonschema::validator_for(&schema).unwrap();
    for invalid in [
        json!(r#"{"tool":"fs.read","input":{"file_path":"src/main.rs"}}"#),
        json!({"arguments":{"tool":"fs.read","input":{"file_path":"src/main.rs"}}}),
        json!({"tool":"fs.read"}),
        json!({"tool":"fs.read","input":"src/main.rs"}),
    ] {
        assert!(
            !validator.is_valid(&invalid),
            "accepted malformed envelope: {invalid}"
        );
    }
}

#[test]
fn named_core_execution_tools_exist_in_bundled_manifests() {
    let tools = registered_tools()
        .into_iter()
        .filter_map(ExecutionTool::from_registered_tool)
        .map(|tool| compact_tool_call_name(&tool.canonical_name()))
        .collect::<BTreeSet<_>>();
    let prompt = agena_runtime_contracts::identity::system_prompt();
    let mut checked = 0;
    for code in prompt.split('`').skip(1).step_by(2) {
        if !code.contains(['(', ' ', '*'])
            && ["session.", "fs.", "code."]
                .iter()
                .any(|prefix| code.starts_with(prefix))
        {
            assert!(
                tools.contains(code),
                "prompt names a nonexistent tool: {code}"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 10,
        "core prompt tool coverage unexpectedly disappeared"
    );
}

#[test]
fn catalog_and_docs_classify_all_bundled_gateway_handlers_like_runtime() {
    let catalog = crate::bundled_capability_manifest();
    let entries = catalog
        .plugins
        .iter()
        .flat_map(|plugin| &plugin.tools)
        .map(|tool| (tool.canonical_name.as_str(), tool.gateway))
        .collect::<BTreeMap<_, _>>();
    let reference = crate::bundled_tools_markdown_reference();
    let mut functions = BTreeSet::from([ToolApiFunction::Call]);
    let mut execution_count = 0;
    for registered in registered_tools() {
        let canonical = registered.canonical_name();
        let binding = ToolApiBinding::from_registered_tool(registered.clone());
        let gateway = binding.is_some();
        assert_eq!(entries[canonical.as_str()], gateway, "catalog: {canonical}");
        assert_eq!(
            reference.contains(&format!("`{canonical}` · **Tool API gateway handler**")),
            gateway,
            "reference: {canonical}"
        );
        assert_eq!(
            ExecutionTool::from_registered_tool(registered).is_some(),
            !gateway
        );
        if let Some(binding) = binding {
            assert!(functions.insert(binding.function()), "duplicate gateway");
        } else {
            execution_count += 1;
        }
    }
    assert_eq!(functions, BTreeSet::from(ToolApiFunction::ALL));
    assert_eq!(catalog.counts.gateway_tools + 1, functions.len());
    assert_eq!(catalog.counts.execution_tools, execution_count);
}
