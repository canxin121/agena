use std::path::PathBuf;

use agena_domain::ExternalAgent;
use serde_json::{Value, json};

use super::AppError;

pub(super) fn native_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub(super) fn antigravity_store() -> Result<PathBuf, AppError> {
    native_home()
        .map(|home| home.join(".gemini"))
        .ok_or_else(|| AppError::Config("Antigravity ACP requires a server home directory.".into()))
}

pub(super) fn apply_to_session(agent: &ExternalAgent, params: &mut Value) {
    match agent {
        ExternalAgent::ClaudeCode => {
            if params["mcpServers"]
                .as_array()
                .is_some_and(|servers| servers.iter().any(|server| server["name"] == "agena"))
            {
                // The injected server applies Agena's own approval rules.
                params["_meta"]["claudeCode"]["options"] = json!({
                    "allowedTools": ["mcp__agena__*"],
                });
            }
        }
        ExternalAgent::Antigravity => {
            if let Some(servers) = params.get_mut("mcpServers").and_then(Value::as_array_mut)
                && !servers.is_empty()
            {
                for server in servers {
                    server["_meta"]["antigravity"]["forceAllToolsEager"] = json!(true);
                }
                params["_meta"]["antigravity"]["eagerMcpServers"] = json!(["agena"]);
            }
        }
        ExternalAgent::Custom(_) => {}
    }
}
