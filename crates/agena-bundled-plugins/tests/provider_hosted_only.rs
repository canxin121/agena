//! Contract and migration tests run through real PluginHost/ToolExecutor routes.
//! Network endpoints are synthetic loopback servers; credentials and HOME live
//! only in child test processes. No real provider model is called.
#[path = "support/hosted_fixture.rs"]
mod fixture;
use agena_domain::{StructuredObject, ToolInvocation};
use agena_tool::provider_tools::{HOSTED_TOOLS, RETIRED_TOOLS};
use fixture::{Fixture, isolate};
use serde_json::{Value, json};

fn input_for(name: &str) -> Value {
    let mut input = json!({"prompt":"fixture","model":"fixture-model"});
    if name.ends_with("image_edit") {
        input["images"] = json!(["fixture.png"]);
    }
    if name == "claude.cloud_advisor" {
        input["tool_options"] = json!({"model":"fixture-advisor"});
    }
    if name == "chatgpt.cloud_file_search" {
        input["tool_options"] = json!({"vector_store_ids":["vs_fixture"]});
    }
    if name == "gemini.cloud_file_search" {
        input["tool_options"] = json!({"file_search_store_names":["fileSearchStores/fixture"]});
    }
    input
}

#[test]
fn manifest_exposes_exactly_32_hosted_tools_and_no_retired_wrapper() {
    let manifest = agena_bundled_plugins::bundled_capability_manifest();
    assert_eq!(manifest.counts.execution_tools, 134);
    assert_eq!(manifest.counts.gateway_tools, 4);
    let all = manifest
        .plugins
        .iter()
        .flat_map(|plugin| plugin.tools.iter())
        .map(|tool| {
            tool.canonical_name
                .strip_prefix("agena.")
                .unwrap_or(&tool.canonical_name)
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    let provider = all
        .iter()
        .filter(|name| {
            name.starts_with("chatgpt.")
                || name.starts_with("claude.")
                || name.starts_with("gemini.")
        })
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(provider, HOSTED_TOOLS.iter().copied().collect());
    for old in RETIRED_TOOLS {
        assert!(
            !all.contains(old.name),
            "retired {} is still discoverable",
            old.name
        );
        for alternative in old.alternatives {
            assert!(
                all.contains(*alternative),
                "migration points to a nonexistent tool: {alternative}"
            );
        }
    }
    for native in [
        "shell.run",
        "shell.write",
        "shell.signal",
        "fs.apply_patch",
        "fs.output_read",
        "mcp.tools.call",
        "memory.write",
        "web.browser_open",
        "tasks.run",
    ] {
        assert!(all.contains(native), "native capability removed: {native}");
    }
}

#[tokio::test]
async fn all_hosted_entries_send_only_their_declared_service_to_a_fake_provider() {
    let name = "all_hosted_entries_send_only_their_declared_service_to_a_fake_provider";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    for name in HOSTED_TOOLS.iter().take(17) {
        let (output, text) = f
            .call(name, input_for(name))
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(output["execution_boundary"], "vendor_hosted_only", "{name}");
        let cloud = agena_tool::provider_tools::cloud_tool(name).unwrap();
        assert_eq!(output["public_tool_name"], *name);
        assert_eq!(
            output["tool"], cloud.operation,
            "vendor accounting identity must not be renamed"
        );
        assert_eq!(output["execution_location"], "vendor_cloud");
        assert_eq!(output["execution_provider"], cloud.provider_label);
        assert_eq!(output["local_workspace_automatically_available"], false);
        assert!(text.contains(name));
        assert!(text.contains(&format!("{} cloud", cloud.provider_label)));

        assert_eq!(output["local_actions_executed"], false);
        assert_eq!(output["client_execution_allowed"], false);
        assert_eq!(output["outcome"], "completed", "{name}: {output}");
        assert_eq!(output["pending_calls"], json!([]));
        assert_eq!(output["continuation_required"], false);
        assert!(
            text.contains("vendor-hosted only"),
            "text result loses boundary: {name}"
        );
        let request = f.requests().pop().unwrap();
        assert_eq!(request.method, "POST");
        assert!(!request.raw.is_empty());
        assert!(request.authenticated);
        if name.ends_with("image_edit") || *name == "gemini.cloud_image_generation" {
            assert!(
                request.path.ends_with("/images/edits")
                    || request.path.ends_with(":generateContent")
            );
        } else {
            assert_eq!(request.body["tools"].as_array().unwrap().len(), 1, "{name}");
            let expected_type = match (cloud.provider, cloud.operation) {
                ("claude", "code_execution") => "code_execution_20260521",
                ("claude", "web_search") => "web_search_20260318",
                ("claude", "web_fetch") => "web_fetch_20260318",
                ("claude", "advisor") => "advisor_20260301",
                (_, operation) => operation,
            };
            assert_eq!(
                request.body["tools"][0]["type"], expected_type,
                "public cloud prefix must never enter vendor tool type"
            );
        }
        if *name == "chatgpt.cloud_shell" {
            assert_eq!(
                request.body["tools"][0]["environment"]["type"],
                "container_auto"
            );
        }
        if *name == "claude.cloud_advisor" {
            assert_eq!(request.beta.as_deref(), Some("advisor-tool-2026-03-01"));
        } else if name.starts_with("claude.") {
            assert!(
                request.beta.is_none(),
                "GA hosted tools should not add obsolete beta headers"
            );
        }
    }
    assert_eq!(f.requests().len(), 17);
}

#[tokio::test]
async fn invalid_modes_and_callback_histories_fail_before_network() {
    let name = "invalid_modes_and_callback_histories_fail_before_network";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    for environment in [
        json!({"type":"local"}),
        json!({"type":"container"}),
        json!({"type":"unknown"}),
        Value::Null,
        json!("local"),
        json!({"type":"container_reference","container_id":""}),
    ] {
        assert!(
            f.call(
                "chatgpt.cloud_shell",
                json!({"prompt":"fixture","tool_options":{"environment":environment}})
            )
            .await
            .is_err()
        );
    }
    for name in HOSTED_TOOLS.iter().take(17) {
        let mut input = input_for(name);
        if *name == "chatgpt.cloud_image_edit" {
            input["options"] = json!({"tools":[{"type":"local_shell"}]});
        } else {
            input["request_options"] = json!({"tools":[{"type":"function","name":"local"}]});
        }
        assert!(
            f.call(name, input).await.is_err(),
            "injected tools accepted: {name}"
        );
    }
    for (name, input) in [
        (
            "claude.cloud_web_search",
            json!({"prompt":"fixture","request_options":{"mcp_servers":[{"url":"https://example.invalid"}]}}),
        ),
        (
            "chatgpt.cloud_web_search",
            json!({"input_items":[{"type":"function_call_output","call_id":"local","output":"done"}]}),
        ),
        (
            "chatgpt.cloud_shell",
            json!({"input_items":[{"type":"shell_call_output","call_id":"local","output":[]}]}),
        ),
        (
            "claude.cloud_code_execution",
            json!({"messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"local","content":"done"}]}]}),
        ),
        (
            "gemini.cloud_code_execution",
            json!({"input_steps":[{"type":"function_result","call_id":"local","result":[]}]}),
        ),
    ] {
        assert!(
            f.call(name, input).await.is_err(),
            "callback accepted: {name}"
        );
    }
    assert!(
        f.requests().is_empty(),
        "invalid boundary performed a provider request"
    );
    assert!(
        !f.dir
            .path()
            .join(".agena/artifacts/provider-tools")
            .exists()
    );
    let (output,_)=f.call("chatgpt.cloud_shell",json!({"prompt":"fixture","tool_options":{"environment":{"type":"container_reference","container_id":"cntr_fixture"}}})).await.unwrap();
    assert_eq!(output["outcome"], "completed");
    assert_eq!(f.requests().len(), 1);
}

#[tokio::test]
async fn retired_names_have_explicit_errors_and_never_execute_local_fallbacks() {
    let name = "retired_names_have_explicit_errors_and_never_execute_local_fallbacks";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    for old in RETIRED_TOOLS {
        for requested in [
            old.name.to_owned(),
            format!("agena.{}", old.name),
            format!("agena_{}", old.name.replacen('.', "_", 1)),
        ] {
            let error=f.call(&requested,json!({"prompt":"fixture","command":"must-not-run","path":"must-not-write","content":"x"})).await.unwrap_err();
            match error {
                agena_runtime_tools::tool::ToolError::ToolUnavailable(error) => {
                    assert_eq!(error.source, "provider_tool_retirement");
                    assert!(!error.retryable);
                    assert!(error.reason.contains("not redirected automatically"));
                }
                error => panic!("wrong retired result for {requested}: {error}"),
            }
            // Direct executor calls must remain blocked even when a caller
            // does not invoke prepare_invocation first.
            let invocation = ToolInvocation::new(requested, StructuredObject::default());
            let error = f
                .executor
                .execute_invocation_detailed(&invocation, 41, 3)
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                agena_runtime_tools::tool::ToolError::ToolUnavailable(_)
            ));
        }
    }
    assert!(f.requests().is_empty());
    assert!(!f.dir.path().join("must-not-write").exists());
    f.call(
        "fs.write",
        json!({"path":"native.txt","content":"native filesystem still works"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("native.txt")).unwrap(),
        "native filesystem still works"
    );
    let (output, _) = f
        .call(
            "shell.run",
            json!({"command":"printf native-shell","reads":[],"writes":[],"network":[]}),
        )
        .await
        .unwrap();
    assert_eq!(output["output"], "native-shell");
    assert!(f.requests().is_empty());
}

#[tokio::test]
async fn client_actions_returned_by_a_provider_are_quarantined_not_replayed() {
    let name = "client_actions_returned_by_a_provider_are_quarantined_not_replayed";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    for (tool, response) in [
        (
            "chatgpt.cloud_web_search",
            json!({"id":"resp_bad","status":"completed","output":[{"type":"custom_tool_call","status":"completed","call_id":"x","name":"local_exec","input":"DO_NOT_EXECUTE_ME"}]}),
        ),
        (
            "claude.cloud_code_execution",
            json!({"id":"msg_bad","stop_reason":"tool_use","content":[{"type":"tool_use","id":"x","name":"bash","input":{"command":"DO_NOT_EXECUTE_ME"}}]}),
        ),
        (
            "gemini.cloud_code_execution",
            json!({"id":"int_bad","status":"requires_action","steps":[{"type":"function_call","id":"x","name":"local","arguments":{"command":"DO_NOT_EXECUTE_ME"}}]}),
        ),
    ] {
        *f.state.response.lock().unwrap() = Some(response);
        let (output, text) = f.call(tool, input_for(tool)).await.unwrap();
        assert_eq!(output["outcome"], "blocked_client_execution");
        if tool == "gemini.cloud_code_execution" {
            assert_eq!(output["provider_reported_outcome"], "requires_action");
        }

        assert_eq!(output["pending_calls"], json!([]));
        assert_eq!(output["local_actions_executed"], false);
        assert_eq!(output["continuation_required"], false);
        assert!(output["response_received"].as_bool().unwrap());
        assert!(output["response_receipt"]["path"].is_string());
        assert!(!text.contains("DO_NOT_EXECUTE_ME"));
        assert!(text.contains("Do not execute"));
    }
    assert_eq!(f.requests().len(), 3);
    assert!(!f.dir.path().join("DO_NOT_EXECUTE_ME").exists());
}

#[tokio::test]
async fn hosted_pause_resumes_server_work_without_inventing_a_local_tool_result() {
    let name = "hosted_pause_resumes_server_work_without_inventing_a_local_tool_result";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    *f.state.response.lock().unwrap() = Some(
        json!({"id":"msg_paused","stop_reason":"pause_turn","content":[{"type":"server_tool_use","id":"srv_1","name":"advisor","input":{"query":"fixture"}}]}),
    );
    let (output, text) = f
        .call("claude.cloud_advisor", input_for("claude.cloud_advisor"))
        .await
        .unwrap();
    assert_eq!(output["outcome"], "provider_pending");
    assert_eq!(output["continuation_required"], true);
    assert_eq!(output["pending_calls"], json!([]));
    assert!(text.contains("do not run its internal server calls"));
    *f.state.response.lock().unwrap() = None;
    let (resumed,_)=f.call("claude.cloud_advisor",json!({"tool_options":{"model":"fixture-advisor"},"messages":[{"role":"assistant","content":output["assistant_content"]}]})).await.unwrap();
    assert_eq!(resumed["outcome"], "completed");
    assert_eq!(resumed["continuation_required"], false);
    let request = f.requests().pop().unwrap();
    assert_eq!(
        request.body["messages"][0]["content"][0]["type"],
        "server_tool_use"
    );
}

#[tokio::test]
async fn cloud_catalogue_and_help_make_execution_and_data_transfer_explicit() {
    let name = "cloud_catalogue_and_help_make_execution_and_data_transfer_explicit";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    let tools = f.executor.available_execution_tools_async().await;
    for cloud in agena_tool::provider_tools::CLOUD_TOOLS {
        let tool = tools
            .iter()
            .find(|tool| tool.canonical_name() == format!("agena.{}", cloud.name))
            .expect("registered cloud tool");
        let summary = tool.summary_text().unwrap();
        let help = tool.help_text().unwrap();
        assert!(summary.contains("cloud"), "{}: {summary}", cloud.name);
        assert!(summary.contains(cloud.provider_label));
        for phrase in [
            "cloud",
            "not on this computer",
            "provider endpoint",
            "local project files are not automatically available",
            "No local execution fallback",
        ] {
            assert!(
                help.contains(phrase),
                "{} missing {phrase}: {help}",
                cloud.name
            );
        }
        assert!(
            !tools
                .iter()
                .any(|tool| tool.canonical_name() == format!("agena.{}", cloud.previous_name))
        );
        if cloud.operation == "image_edit" {
            assert!(help.contains("uploaded"));
            assert!(help.contains("separate local artifacts"));
        }
    }
    assert!(
        f.requests().is_empty(),
        "help discovery must not send provider requests"
    );
}

#[tokio::test]
async fn previous_provider_names_require_explicit_cloud_migration_without_side_effects() {
    let name = "previous_provider_names_require_explicit_cloud_migration_without_side_effects";
    if isolate(name) {
        return;
    }
    let f = Fixture::new().await;
    for cloud in agena_tool::provider_tools::CLOUD_TOOLS {
        for previous in [
            cloud.previous_name.to_owned(),
            format!("agena.{}", cloud.previous_name),
            format!("agena_{}", cloud.previous_name.replacen('.', "_", 1)),
        ] {
            let error = f.call(&previous, input_for(cloud.name)).await.unwrap_err();
            match error {
                agena_runtime_tools::tool::ToolError::ToolUnavailable(error) => {
                    assert_eq!(error.source, "provider_cloud_tool_rename");
                    assert_eq!(error.suggestions, vec![cloud.name]);
                    assert!(!error.retryable);
                    assert!(error.reason.contains("cloud execution"));
                    assert!(error.reason.contains("not redirected automatically"));
                }
                error => panic!("unexpected migration error: {error}"),
            }
            let invocation = ToolInvocation::new(previous, StructuredObject::default());
            let error = f
                .executor
                .execute_invocation_detailed(&invocation, 41, 5)
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                agena_runtime_tools::tool::ToolError::ToolUnavailable(_)
            ));
        }
    }
    assert!(f.requests().is_empty());
    assert!(
        !f.dir
            .path()
            .join(".agena/artifacts/provider-tools")
            .exists()
    );
}

use agena_runtime_config::config::raw::ProviderKind;

const COVERED_ADAPTER_KINDS: [ProviderKind; 7] = [
    ProviderKind::Ollama,
    ProviderKind::OpenAiResponses,
    ProviderKind::OpenAiChatCompletions,
    ProviderKind::Anthropic,
    ProviderKind::Gemini,
    ProviderKind::Gitlab,
    ProviderKind::AmazonBedrock,
];

fn expected_cloud_family(kind: ProviderKind) -> Option<&'static str> {
    // Exhaustive on purpose: every future adapter must get an explicit policy.
    match kind {
        ProviderKind::OpenAiResponses | ProviderKind::OpenAiChatCompletions => Some("chatgpt"),
        ProviderKind::Anthropic => Some("claude"),
        ProviderKind::Gemini => Some("gemini"),
        ProviderKind::Ollama | ProviderKind::Gitlab | ProviderKind::AmazonBedrock => None,
    }
}

fn adapter_id(kind: ProviderKind) -> String {
    serde_json::to_value(kind)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn valid_gate_input(cloud: &agena_tool::provider_tools::CloudProviderTool) -> Value {
    match cloud.operation {
        "image_understanding" | "document_understanding" => {
            json!({"inputs":[{"source":"local","path":"fixture.png"}],"prompt":"fixture","model":"fixture-model"})
        }
        "file_upload" => json!({"path":"fixture.png"}),
        "file_status" | "file_delete" => json!({"handle":"media_00000000000000000000000000000000"}),
        _ => input_for(cloud.name),
    }
}

fn assert_adapter_gate(error: agena_runtime_tools::tool::ToolError, expected: &str, actual: &str) {
    match error {
        agena_runtime_tools::tool::ToolError::CapabilityUnavailable(error) => {
            assert_eq!(error.capability, "provider_tool_adapter");
            assert_eq!(
                error.source,
                agena_domain::CapabilitySourceKind::RuntimeConfiguration
            );
            assert!(!error.retryable);
            assert!(error.reason.contains(expected));
            assert!(error.reason.contains(actual));
            assert!(error.reason.contains("No provider request was sent"));
        }
        other => panic!("expected execution-time adapter rejection, got {other}"),
    }
}

#[tokio::test]
async fn ai_adapter_gate_blocks_cross_vendor_calls_and_accepts_matching_calls() {
    if isolate("ai_adapter_gate_blocks_cross_vendor_calls_and_accepts_matching_calls") {
        return;
    }
    let mut f = Fixture::new().await;
    for (adapter, family, target) in [
        ("openai_responses", "chatgpt", "chatgpt.cloud_web_search"),
        (
            "openai_chat_completions",
            "chatgpt",
            "chatgpt.cloud_web_search",
        ),
        ("anthropic", "claude", "claude.cloud_web_search"),
        ("gemini", "gemini", "gemini.cloud_google_search"),
    ] {
        f.executor = f
            .executor
            .clone()
            .with_cloud_tool_adapter(Some(agena_domain::AdapterId::new(adapter)));
        let count = f.requests().len();
        for cloud in agena_tool::provider_tools::CLOUD_TOOLS
            .iter()
            .filter(|tool| tool.provider != family)
        {
            let required = match cloud.provider {
                "chatgpt" => "openai_responses",
                "claude" => "anthropic",
                _ => "gemini",
            };
            for name in [
                cloud.name.to_owned(),
                format!("agena.{}", cloud.name),
                format!("agena_{}", cloud.name.replacen('.', "_", 1)),
            ] {
                // Even a forged input adapter cannot override trusted runtime state.
                let error=f.call(&name,json!({"adapter":required,"provider":cloud.provider,"prompt":"must not send"})).await.unwrap_err();
                let wire_name = name.starts_with("agena_");
                if wire_name {
                    // Wire names are display/history identities, not registered
                    // invocation aliases. Preserve the pre-existing refusal.
                    assert!(
                        matches!(
                            error,
                            agena_runtime_tools::tool::ToolError::ToolUnavailable(_)
                        ),
                        "unregistered wire name was unexpectedly made executable"
                    );
                } else {
                    assert_adapter_gate(error, required, adapter);
                }
                let invocation = ToolInvocation::new(name, StructuredObject::default());
                let error = f
                    .executor
                    .clone()
                    .execute_invocation_detailed(&invocation, 41, 41)
                    .await
                    .unwrap_err();
                if wire_name {
                    assert!(matches!(
                        error,
                        agena_runtime_tools::tool::ToolError::ToolUnavailable(_)
                    ));
                } else {
                    assert_adapter_gate(error, required, adapter);
                }
            }
        }
        assert_eq!(
            f.requests().len(),
            count,
            "rejected calls sent a provider request"
        );
        let (result, _) = f.call(target, input_for(target)).await.unwrap();
        assert_eq!(result["outcome"], "completed");
        assert_eq!(f.requests().len(), count + 1);
    }
    // A prepared invocation cannot bypass a later changed execution scope.
    f.executor = f
        .executor
        .clone()
        .with_cloud_tool_adapter(Some(agena_domain::AdapterId::new("anthropic")));
    let invocation = ToolInvocation::new(
        "claude.cloud_web_search",
        StructuredObject::try_from(input_for("claude.cloud_web_search")).unwrap(),
    );
    let prepared = f
        .executor
        .prepare_invocation(&invocation, 41, 50)
        .await
        .unwrap();
    let before = f.requests().len();
    let changed = f
        .executor
        .clone()
        .with_cloud_tool_adapter(Some(agena_domain::AdapterId::new("openai_responses")));
    assert_adapter_gate(
        changed
            .execute_invocation_detailed(&prepared.invocation, 41, 50)
            .await
            .unwrap_err(),
        "anthropic",
        "openai_responses",
    );
    assert_eq!(f.requests().len(), before);
}

#[tokio::test]
async fn ai_adapter_gate_keeps_discovery_help_and_native_tools_unchanged() {
    if isolate("ai_adapter_gate_keeps_discovery_help_and_native_tools_unchanged") {
        return;
    }
    let mut f = Fixture::new().await;
    async fn catalogue(executor: &agena_runtime_tools::tool::ToolExecutor) -> Value {
        let mut tools = executor.available_execution_tools_async().await;
        tools.sort_by_key(|tool| tool.canonical_name());
        json!(
            tools
                .iter()
                .map(|tool| (tool.canonical_name(), tool.definition.clone()))
                .collect::<Vec<_>>()
        )
    }
    let before = catalogue(&f.executor).await;
    for adapter in COVERED_ADAPTER_KINDS
        .into_iter()
        .map(|kind| Some(adapter_id(kind)))
        .chain(std::iter::once(None))
    {
        let guarded = f
            .executor
            .clone()
            .with_cloud_tool_adapter(adapter.map(agena_domain::AdapterId::new));
        assert_eq!(
            catalogue(&guarded).await,
            before,
            "invocation gate must not filter definitions or help"
        );
    }
    assert!(f.requests().is_empty());
    f.executor = f.executor.clone().with_cloud_tool_adapter(None);
    f.call(
        "fs.write",
        json!({"path":"local.txt","content":"still local"}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.dir.path().join("local.txt")).unwrap(),
        "still local"
    );
    let (output, _) = f
        .call(
            "shell.run",
            json!({"command":"printf native-ok","reads":[],"writes":[],"network":[]}),
        )
        .await
        .unwrap();
    assert_eq!(output["output"], "native-ok");
    assert!(f.requests().is_empty());
}

#[tokio::test]
async fn ai_adapter_gate_rejects_unresolved_and_nonmatching_protocols_without_uploads() {
    if isolate("ai_adapter_gate_rejects_unresolved_and_nonmatching_protocols_without_uploads") {
        return;
    }
    let mut f = Fixture::new().await;
    for adapter in [
        None,
        Some("amazon_bedrock"),
        Some("ollama"),
        Some("gitlab"),
        Some("openai"),
        Some("OpenAI_responses"),
        Some("openai_responses_misspelled"),
    ] {
        f.executor = f
            .executor
            .clone()
            .with_cloud_tool_adapter(adapter.map(agena_domain::AdapterId::new));
        for cloud in agena_tool::provider_tools::CLOUD_TOOLS {
            let error=f.call(cloud.name,json!({"path":"fixture.png","inputs":[{"source":"local","path":"fixture.png"}],"prompt":"must not send"})).await.unwrap_err();
            let required = match cloud.provider {
                "chatgpt" => "openai_responses",
                "claude" => "anthropic",
                _ => "gemini",
            };
            assert_adapter_gate(error, required, adapter.unwrap_or("unresolved"));
        }
    }
    assert!(f.requests().is_empty());
    assert!(
        !f.dir
            .path()
            .join(".agena/artifacts/provider-tools")
            .exists(),
        "blocked upload must not create provider artifacts"
    );
}

#[tokio::test]
async fn ai_adapter_gate_covers_every_configured_adapter_against_all_cloud_tools() {
    if isolate("ai_adapter_gate_covers_every_configured_adapter_against_all_cloud_tools") {
        return;
    }
    let f = Fixture::new().await;
    let mut ids = std::collections::BTreeSet::new();
    let mut checked = 0usize;
    for kind in COVERED_ADAPTER_KINDS {
        let adapter = adapter_id(kind);
        assert!(ids.insert(adapter.clone()));
        assert_eq!(adapter.parse::<ProviderKind>().unwrap(), kind);
        let expected = expected_cloud_family(kind);
        let executor = f
            .executor
            .clone()
            .with_cloud_tool_adapter(Some(agena_domain::AdapterId::new(&adapter)));
        for cloud in agena_tool::provider_tools::CLOUD_TOOLS {
            let invocation = ToolInvocation::new(
                cloud.name,
                StructuredObject::try_from(valid_gate_input(cloud)).unwrap(),
            );
            let result = executor
                .prepare_invocation(&invocation, 41, checked as i64 + 100)
                .await;
            if expected == Some(cloud.provider) {
                assert!(
                    result.is_ok(),
                    "{adapter} should allow {} through the adapter gate: {:?}",
                    cloud.name,
                    result.err()
                );
            } else {
                let required = match cloud.provider {
                    "chatgpt" => "openai_responses",
                    "claude" => "anthropic",
                    _ => "gemini",
                };
                assert_adapter_gate(result.unwrap_err(), required, &adapter);
                assert_adapter_gate(
                    executor
                        .execute_invocation_detailed(&invocation, 41, checked as i64 + 100)
                        .await
                        .unwrap_err(),
                    required,
                    &adapter,
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 7 * 32);
    assert!(
        f.requests().is_empty(),
        "the coverage matrix must not send requests or upload inputs"
    );
    assert!(
        !f.dir
            .path()
            .join(".agena/artifacts/provider-tools")
            .exists()
    );
}
