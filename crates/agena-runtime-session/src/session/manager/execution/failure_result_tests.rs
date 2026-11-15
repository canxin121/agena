use super::*;

fn network_denial() -> agena_failure::Failure {
    let denial = serde_json::from_value(json!({
        "action": {
            "kind": "network_access",
            "target": "https://192.0.2.7/",
            "host": "192.0.2.7",
            "port": 443,
        },
        "reason": "no native MXC network allow rule matches this destination",
        "authority": "static_policy",
    }))
    .unwrap();
    AppError::PolicyDenied(Box::new(denial)).failure()
}

#[test]
fn provider_failure_results_hide_query_values_without_changing_execution_input() {
    let denied = network_denial();
    let invalid = *agena_plugin_host::sdk::PluginError::invalid_params(
        "credentials must come from the current provider",
    )
    .failure;
    for (name, plugin_name) in [
        ("chatgpt.files", None),
        ("agena.chatgpt.files", None),
        ("agena_chatgpt_files", None),
        ("files", Some("agena.chatgpt")),
        ("claude.files", None),
        ("gemini.files", None),
    ] {
        let input = json!({
            "action": "list",
            "id": "resource_1",
            "query": {"after": "query-secret-canary", "key": "credential-secret-canary"},
            "path": "input.txt",
        });
        let invocation = ToolInvocation {
            name: name.into(),
            plugin_name: plugin_name.map(str::to_owned),
            input: input.clone().try_into().unwrap(),
        };
        for failure in [&denied, &invalid] {
            // Exercise the complete persisted failure result, including its
            // invocation metadata, rather than only the already-safe error.
            let result = ToolResult {
                phase: ToolPhase::Failed,
                value: failure_result_value(failure, &invocation),
            };
            let public = serde_json::to_string(&result).unwrap();
            assert!(!public.contains("query-secret-canary"), "{name}: {public}");
            assert!(
                !public.contains("credential-secret-canary"),
                "{name}: {public}"
            );
            assert_eq!(
                result.value["failure"],
                serde_json::to_value(failure).unwrap()
            );
            assert_eq!(
                result.value["error"],
                serde_json::to_value(agena_failure::UserProblem::from(failure)).unwrap()
            );
            let displayed: ToolInvocation =
                serde_json::from_value(result.value["invocation"].clone()).unwrap();
            assert_eq!(displayed.name, invocation.name);
            assert_eq!(displayed.plugin_name, invocation.plugin_name);
            assert_eq!(
                Value::from(displayed.input),
                json!({
                    "action": "list", "id": "resource_1", "path": "input.txt"
                })
            );
            assert_eq!(Value::from(invocation.input.clone()), input);
        }
    }
    assert_eq!(denied.code.as_str(), "permission.denied");
    assert_eq!(
        denied.category,
        agena_failure::FailureCategory::PermissionDenied
    );
    assert!(
        denied
            .user
            .fallback
            .contains("no native MXC network allow rule")
    );
}

#[test]
fn non_provider_failure_results_keep_the_existing_invocation_projection() {
    let invocation = ToolInvocation::new(
        "agena.web.search",
        json!({"query": "public search terms"}).try_into().unwrap(),
    );
    let value = failure_result_value(&network_denial(), &invocation);
    assert_eq!(
        value["invocation"],
        serde_json::to_value(&invocation).unwrap()
    );
}
