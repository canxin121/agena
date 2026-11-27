use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use agena_domain::ExternalAgent;
use serde_json::{Map, Value, json};

use super::AppError;

const ANTIGRAVITY_PROFILE: &str = "agena-acp";
const ANTIGRAVITY_AGENT_BODY: &str =
    "description: An ACP main agent whose workspace tools are provided by Agena.
mainAgent: true
subagent: false
tools: []
inheritMcp: false
inheritCustomizations: false
---
Use the tools provided by Agena to access the workspace or the web.
";

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
    let directory = profile.join("config/agents");
    fs::create_dir_all(&directory)?;
    let name = antigravity_agent_name(&profile)?;
    let installed = directory.join(format!("{name}.md"));
    let definition = format!("---\nname: {name}\n{ANTIGRAVITY_AGENT_BODY}");
    if fs::read(&installed).as_deref().ok() != Some(definition.as_bytes()) {
        agena_runtime_tools::atomic_write_file(&installed, definition.as_bytes())?;
    }
    let profile = profile.to_str().ok_or_else(|| {
        AppError::Config("Antigravity ACP requires a UTF-8 home directory.".into())
    })?;
    let mut arguments = vec![
        "--gemini-dir".into(),
        profile.into(),
        "--agent".into(),
        name,
        "--include-default-customizations=false".into(),
    ];
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

pub(super) fn native_antigravity_customizations(
    cwd: &Path,
    directories: &[PathBuf],
) -> Result<Vec<PathBuf>, AppError> {
    let mut paths = vec![antigravity_profile()?.join("config/mcp_config.json")];
    for directory in std::iter::once(cwd).chain(directories.iter().map(PathBuf::as_path)) {
        let config = directory.join(".agents");
        paths.push(config.join("mcp_config.json"));
        paths.push(config.join("hooks.json"));
    }
    paths.retain(|path| match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != ErrorKind::NotFound,
    });
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(super) fn install_antigravity_guard() -> Result<bool, AppError> {
    if !cfg!(unix) {
        return Ok(false);
    }
    let path = antigravity_profile()?.join("config/hooks.json");
    let mut hooks: Map<String, Value> = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error.into()),
    };
    let program = std::env::current_exe()?;
    let program = program.to_str().ok_or_else(|| {
        AppError::Config("The Antigravity tool guard needs a UTF-8 Agena path.".into())
    })?;
    let command = format!(
        "'{}' {}",
        program.replace('\'', "'\"'\"'"),
        agena_mcp_server::antigravity_guard::WORKER_ARG,
    );
    let hook = json!({
        "PreToolUse": [{
            "matcher": "*",
            "hooks": [{"type": "command", "command": command, "timeout": 10}]
        }],
    });
    if hooks.get("agena-acp-tools") != Some(&hook) {
        hooks.insert("agena-acp-tools".into(), hook);
        agena_runtime_tools::atomic_write_file(&path, &serde_json::to_vec_pretty(&hooks)?)?;
    }
    Ok(true)
}

pub(super) fn antigravity_guard_supported(initialized: &Value) -> bool {
    // Legacy sessions have a saved tool catalog. Require a release in which
    // PreToolUse denies the native tools and blocks when its command fails.
    initialized["agentInfo"]["name"] == "Antigravity ACP Server"
        && initialized["agentInfo"]["version"] == "1.5.18"
}

pub(super) fn antigravity_agent_name(profile: &Path) -> Result<String, AppError> {
    let path = profile.join("agena-agent-id");
    // Workspace Agent definitions take precedence by name. Keep an
    // instance-specific name stable across native resume and server restarts.
    let id = match fs::read_to_string(&path) {
        Ok(id) => id,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let id = uuid::Uuid::new_v4().simple().to_string();
            match agena_runtime_tools::atomic_create_file(&path, id.as_bytes(), None) {
                Ok(()) => id,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    fs::read_to_string(&path)?
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Config(
            "The Antigravity ACP profile has an invalid agent ID.".into(),
        ));
    }
    Ok(format!("agena-acp-{id}"))
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
            params["_meta"]["claudeCode"]["options"] = json!({
                "tools": [],
                "strictMcpConfig": true,
                "settingSources": [],
                "allowDangerouslySkipPermissions": false,
                "systemPrompt": system_prompt,
                "env": {"ENABLE_TOOL_SEARCH": "false"},
                "extraArgs": {"restricted": null},
                // The bridge enforces Agena's permissions for every invocation.
                "allowedTools": ["mcp__agena__*"]
            });
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
