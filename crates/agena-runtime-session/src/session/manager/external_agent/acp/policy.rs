use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use agena_domain::ExternalAgent;
use serde_json::{Map, Value, json};

use super::AppError;

const ANTIGRAVITY_PROFILE: &str = "agena-acp";

pub(super) fn native_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn antigravity_home() -> Option<PathBuf> {
    native_home().map(|home| home.join(".gemini"))
}

pub(super) fn previous_antigravity_store() -> Result<PathBuf, AppError> {
    antigravity_home()
        .map(|home| home.join("antigravity-cli"))
        .ok_or_else(|| AppError::Config("Antigravity ACP requires a server home directory.".into()))
}

pub(super) fn previous_antigravity_history_available() -> bool {
    previous_antigravity_store().is_ok_and(|root| root.join("conversations").is_dir())
}

pub(super) fn antigravity_arguments(previous_history: bool) -> Result<Vec<String>, AppError> {
    let profile = antigravity_profile()?;
    remove_legacy_antigravity_guard(&profile)?;
    let profile = profile.to_str().ok_or_else(|| {
        AppError::Config("Antigravity ACP requires a UTF-8 home directory.".into())
    })?;
    let mut arguments = vec!["--gemini-dir".into(), profile.into()];
    if previous_history {
        // Point the same isolated configuration at the previous native CLI store.
        arguments.extend(["--app-data-dir".into(), "../antigravity-cli".into()]);
    }
    Ok(arguments)
}

pub(super) fn antigravity_profile() -> Result<PathBuf, AppError> {
    antigravity_home()
        .map(|directory| directory.join(ANTIGRAVITY_PROFILE))
        .ok_or_else(|| AppError::Config("Antigravity ACP requires a server home directory.".into()))
}

fn remove_legacy_antigravity_guard(profile: &Path) -> Result<(), AppError> {
    let path = profile.join("config/hooks.json");
    let original = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let Ok(mut hooks) = serde_json::from_slice::<Map<String, Value>>(&original) else {
        return Ok(());
    };
    let installed_by_agena = hooks
        .get("agena-acp-tools")
        .and_then(|hook| hook.pointer("/PreToolUse/0/hooks/0/command"))
        .and_then(Value::as_str)
        .is_some_and(|command| command.ends_with(" --internal-acp-antigravity-tool-guard"));
    if installed_by_agena {
        hooks.remove("agena-acp-tools");
        let updated = serde_json::to_vec_pretty(&hooks)?;
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            if fs::read(&path)? == updated {
                return Ok(());
            }
            agena_runtime_tools::atomic_replace_file_with_check(&path, &updated, || {
                agena_runtime_tools::verify_file_contents(&path, &original)
            })
        })??;
    }
    Ok(())
}

pub(super) fn apply_to_session(agent: ExternalAgent, params: &mut Value) {
    match agent {
        ExternalAgent::ClaudeCode => {
            // The ACP adapter forwards typed SDK options. Rebuild the prompt
            // on resume so current Agena instructions take effect immediately.
            let mut system_prompt = json!({
                "type": "preset", "preset": "claude_code", "snapshot": false
            });
            if let Some(append) = params.pointer("/systemPrompt/content/0/text") {
                system_prompt["append"] = append.clone();
            }
            let mut options = json!({
                "allowDangerouslySkipPermissions": false,
                "systemPrompt": system_prompt,
            });
            if params["mcpServers"]
                .as_array()
                .is_some_and(|servers| servers.iter().any(|server| server["name"] == "agena"))
            {
                // The injected server applies Agena's own approval rules.
                options["allowedTools"] = json!(["mcp__agena__*"]);
            }
            params["_meta"]["claudeCode"]["options"] = options;
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
    }
}
