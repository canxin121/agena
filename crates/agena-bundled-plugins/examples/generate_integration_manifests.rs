//! Generate transport-neutral reference plugin manifests from the Rust SDK.
#[derive(Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct GithubSettings {
    api_base: String,
    token_env: String,
    allow_loopback_http: bool,
}
#[derive(Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ChatSettings {
    database_path: String,
    secret_env: String,
    listen_host: String,
    listen_port: u16,
    status_path: Option<String>,
    reply_url: String,
    allow_loopback_http: bool,
    remote_idempotent: bool,
    spool_only: bool,
}

fn main() {
    use agena_plugin_host::sdk::{PluginManifest, integration::integration_export};
    let mut github = PluginManifest::new("example", "github_issues", "1.0.0");
    github.summary =
        Some("Read-only GitHub issue event source; no I/O without an enabled grant.".into());
    github.settings =
        Some(agena_plugin_host::sdk::settings_contract_for::<GithubSettings>().unwrap());
    github.services.exports.push(integration_export(false));
    let mut chat = PluginManifest::new("example", "webhook_chat", "1.0.0");
    chat.summary = Some(
        "Signed webhook chat adapter with durable inbox and explicit outbound receipts.".into(),
    );
    chat.settings = Some(agena_plugin_host::sdk::settings_contract_for::<ChatSettings>().unwrap());
    chat.services.exports.push(integration_export(true));
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"protocol_version":agena_plugin_host::sdk::rpc::PROTOCOL_VERSION,"github_issues":github,"webhook_chat":chat})).unwrap());
}
