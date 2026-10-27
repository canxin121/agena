use super::*;
use serde_json::{Value, json};

fn stdio_args() -> McpAddArgs {
    McpAddArgs {
        server: "fixture".into(),
        url: None,
        command: Some("fixture-mcp".into()),
        args: vec![],
        env: vec![],
        cwd: None,
        headers: vec![],
        auth: McpHttpAuthArg::None,
        scopes: vec![],
        include_tools: vec![],
        exclude_tools: vec![],
        auth_env: None,
        layer: McpConfigLayerArg::Global,
        force: false,
        dry_run: false,
        no_reload: false,
        format: OutputFormat::Json,
    }
}

fn http_args() -> McpAddArgs {
    McpAddArgs {
        url: Some("https://example.test/mcp".into()),
        command: None,
        ..stdio_args()
    }
}

#[test]
fn stdio_accepts_the_same_tool_policy_as_http_and_preserves_exact_arguments() {
    let mut args = stdio_args();
    args.args = vec!["two words".into(), "".into(), "$(literal)".into()];
    args.include_tools = vec!["read_*".into()];
    args.exclude_tools = vec!["*_private".into()];
    let config = mcp_server_config_value(&args).unwrap();
    assert_eq!(
        config["process"]["args"],
        json!(["two words", "", "$(literal)"])
    );
    assert_eq!(
        config["tools"],
        json!({"include":["read_*"],"exclude":["*_private"]})
    );
    assert_eq!(config["enabled"], true);
}

#[test]
fn cli_add_reuses_shared_semantic_validation_without_echoing_secret_values() {
    let mut bad_header = http_args();
    bad_header.headers = vec!["X-Key=private-header\ninjection".into()];
    let mut duplicate_header = http_args();
    duplicate_header.headers = vec!["X-Key=private-first".into(), "x-key=private-second".into()];
    let mut fragment = http_args();
    fragment.url = Some("https://example.test/mcp#private-fragment".into());
    let mut environment = stdio_args();
    environment.env = vec!["TOKEN=private-value\0suffix".into()];
    let mut bearer_env = http_args();
    bearer_env.auth = McpHttpAuthArg::BearerFromEnv;
    bearer_env.auth_env = Some("BAD=private-name".into());
    let mut filter = stdio_args();
    filter.exclude_tools = vec!["".into()];
    for args in [
        bad_header,
        duplicate_header,
        fragment,
        environment,
        bearer_env,
        filter,
    ] {
        let error = mcp_server_config_value(&args).unwrap_err().to_string();
        assert!(!error.contains("private-"), "{error}");
    }
    let error = parse_mcp_key_value_pairs(&["private-accidental-token".into()], "--env")
        .unwrap_err()
        .to_string();
    assert!(!error.contains("private-accidental-token"));
}

#[test]
fn oauth_cli_argument_serializes_to_the_canonical_native_auth_kind() {
    let mut args = http_args();
    args.auth = McpHttpAuthArg::OAuth;
    args.scopes = vec!["read".into(), "write".into()];
    let config = mcp_server_config_value(&args).unwrap();
    assert_eq!(config["auth"]["kind"], "o_auth");
    assert_eq!(config["auth"]["scopes"], json!(["read", "write"]));
    assert!(config["auth"].get("client_secret").is_none());
}

#[test]
fn new_workspace_record_inherits_disabled_bridge_and_replacement_is_atomic_per_name() {
    let mut record = mcp_plugin_record(Value::Null, false).unwrap();
    assert_eq!(record["enabled"], false);
    assert_eq!(record["settings"], json!({}));
    apply_mcp_config_mutation(&mut record, McpConfigMutation::Add {
        server: "shared".into(),
        config: json!({"transport":"http","endpoint":{"url":"https://example.test/mcp"},"auth":{"kind":"bearer","token":"old-token"}}),
        force: false,
    }).unwrap();
    apply_mcp_config_mutation(
        &mut record,
        McpConfigMutation::Add {
            server: "sibling".into(),
            config: json!({"transport":"stdio","process":{"command":"sibling-mcp"}}),
            force: false,
        },
    )
    .unwrap();
    let sibling = record["settings"]["servers"]["sibling"].clone();
    apply_mcp_config_mutation(&mut record, McpConfigMutation::Add {
        server: "shared".into(), config: json!({"transport":"stdio","process":{"command":"replacement-mcp"},"enabled":false}), force:true,
    }).unwrap();
    assert!(
        record["settings"]["servers"]["shared"]
            .get("auth")
            .is_none()
    );
    assert!(
        record["settings"]["servers"]["shared"]
            .get("endpoint")
            .is_none()
    );
    assert_eq!(record["settings"]["servers"]["sibling"], sibling);
    assert_eq!(record["enabled"], false);
    apply_mcp_config_mutation(
        &mut record,
        McpConfigMutation::Remove {
            server: "shared".into(),
        },
    )
    .unwrap();
    assert_eq!(record["settings"]["servers"], json!({"sibling":sibling}));
}

#[test]
fn status_and_get_select_the_effective_disabled_workspace_copy_without_hiding_global_provenance() {
    let catalog:McpConnectionCatalog = serde_json::from_value(json!({
        "workspace_root":"/server/project","global_path":"/global/config.json","workspace_path":"/server/project/.agena/config.json","bridge_enabled":true,
        "connections":[
            {"id":"global-id","name":"shared","scope":"global","transport":"http","enabled":true,"effective":false,"state":"shadowed","tool_count":0,"revision":"first","error":null},
            {"id":"workspace-id","name":"shared","scope":"workspace","transport":"stdio","enabled":false,"effective":true,"state":"disabled","tool_count":0,"revision":"second","error":null}
        ]
    })).unwrap();
    let selected = effective_mcp_connection(&catalog, "shared").unwrap();
    assert_eq!(selected.id, "workspace-id");
    assert_eq!(selected.state, "disabled");
    let detail = McpConnectionDetail {
        summary: selected.clone(),
        config: json!({"transport":"stdio","process":{"command":"paused-mcp"}}),
        secret_paths: vec![],
        protocol_version: None,
        server_info: None,
        capabilities: None,
        oauth_health: None,
    };
    let detail = mcp_detail_output(detail).unwrap();
    assert_eq!(detail["connected"], false);
    assert_eq!(detail["state"], "disabled");
    let output = mcp_catalog_output(catalog).unwrap();
    assert_eq!(output["servers"].as_array().unwrap().len(), 2);
    assert_eq!(output["servers"][0]["state"], "shadowed");
    assert_eq!(output["servers"][1]["connected"], false);
    assert_eq!(output["workspace_root"], "/server/project");
}

#[test]
fn mutation_and_dry_run_output_redact_both_previous_and_current_credentials() {
    let mut response = json!({
        "dry_run":true,"changed":true,"previous":{
            "settings":{"servers":{"local":{"transport":"stdio","process":{"command":"fixture-mcp","env":{"TOKEN":"previous-private-env"}}}}}
        },
        "current":{
            "settings":{"servers":{"remote":{
                "transport":"http","endpoint":{"url":"https://example.test/mcp?key=current-private-url","headers":{"X-Key":"current-private-header"}},
                "auth":{"kind":"o_auth","client_id":"public-client-id","client_secret":"current-private-client-secret"}
            }}}
        }
    });
    redact_mcp_settings_edit(&mut response);
    let encoded = response.to_string();
    for secret in [
        "previous-private-env",
        "current-private-url",
        "current-private-header",
        "current-private-client-secret",
    ] {
        assert!(!encoded.contains(secret));
    }
    assert_eq!(response["dry_run"], true);
    assert_eq!(
        response["current"]["settings"]["servers"]["remote"]["auth"]["client_id"],
        "public-client-id"
    );
    assert_eq!(
        response["current"]["settings"]["servers"]["remote"]["endpoint"]["url"],
        "[REDACTED]"
    );
}
