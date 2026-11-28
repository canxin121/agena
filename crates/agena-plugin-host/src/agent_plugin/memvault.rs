//! Agena bindings for the DeepSeek MemVault prompt, turn, and disposal services.

use std::path::{Path, PathBuf};

mod outcome;

use agena_plugin_sdk::{
    HookSubscription, HostClient, ImportedTurnCompletedInput, PluginKey, host_api::LogLevel,
};
use serde::Deserialize;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;

use super::{AgentPluginContext, agent_plugin_mcp_server_name, deepseek, mcp};

pub(super) const MODULE: &str = "@dreamor/dsh-memvault";
const MAX_PROMPT_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub(super) enum Backend {
    Attach(String),
    Spawn {
        binary: String,
        database: String,
        embedding_provider: String,
    },
}

#[derive(Clone)]
pub(super) struct MemVault {
    pub server: String,
    pub backend: Backend,
    pub inject: bool,
    pub extract: bool,
    infer_outcome: bool,
    agent_id: String,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Config {
    mode: Mode,
    url: String,
    binary_path: String,
    db: String,
    embedding_provider: String,
    agent_id: String,
    inject_on_assemble: bool,
    extract_on_turn_end: bool,
    infer_outcome_on_session_end: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Spawn,
    Attach,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Spawn,
            url: String::new(),
            binary_path: String::new(),
            db: "~/.memvault/data.db".into(),
            embedding_provider: "native".into(),
            agent_id: "dsh".into(),
            inject_on_assemble: true,
            extract_on_turn_end: true,
            infer_outcome_on_session_end: false,
        }
    }
}

pub(super) fn discover(rows: &[(String, Yaml)], diagnostics: &mut Vec<String>) -> Vec<MemVault> {
    rows.iter()
        .filter(|(_, row)| deepseek::module_name(row) == Some(MODULE))
        .filter_map(|(id, row)| {
            let config: Config = match row.get("config").filter(|value| !value.is_null()).cloned() {
                None => Config::default(),
                Some(value) => match serde_yaml::from_value(value) {
                    Ok(config) => config,
                    Err(error) => {
                        diagnostics
                            .push(format!("DeepSeek Harness MemVault entry {id:?}: {error}"));
                        return None;
                    }
                },
            };
            let backend = match config.mode {
                Mode::Attach => Backend::Attach(config.url),
                Mode::Spawn => Backend::Spawn {
                    binary: if config.binary_path.trim().is_empty() {
                        "memvault-proxy".into()
                    } else {
                        config.binary_path
                    },
                    database: config.db,
                    embedding_provider: config.embedding_provider,
                },
            };
            let server = format!("memvault-{}", &blake3::hash(id.as_bytes()).to_hex()[..16]);
            Some(MemVault {
                server,
                backend,
                inject: config.inject_on_assemble,
                extract: config.extract_on_turn_end,
                infer_outcome: config.infer_outcome_on_session_end,
                agent_id: config.agent_id,
            })
        })
        .collect()
}

pub(super) fn spawn_paths(
    context: &AgentPluginContext<'_>,
    binary: &str,
    database: &str,
) -> Result<(String, PathBuf), String> {
    let binary_path = Path::new(binary);
    let command = if binary_path.is_absolute() {
        mcp::executable_path(binary_path.to_owned())?
    } else if binary.starts_with("./") {
        mcp::executable_path(super::manifest_path(context.root, binary)?)?
    } else if binary.trim().is_empty() || binary.contains(['/', '\\', ':', '\0']) {
        return Err("MemVault binaryPath must be a bare executable or a valid path".into());
    } else {
        binary.to_owned()
    };
    let database = if let Some(suffix) = database.strip_prefix("~/") {
        std::env::home_dir()
            .ok_or("MemVault database requires a home directory")?
            .join(suffix)
    } else if let Some(suffix) = database.strip_prefix("${PLUGIN_DATA}/") {
        super::package_path(context.data, Path::new(suffix))?
    } else {
        context.workspace.join(database)
    };
    if database.file_name().is_none() {
        return Err("MemVault database must name a file".into());
    }
    Ok((command, database))
}

pub(super) struct MemVaultBridge {
    server: String,
    inject: bool,
    extract: bool,
    infer_outcome: bool,
    agent_id: String,
}

impl MemVaultBridge {
    pub(super) fn bind(definition: &MemVault, key: &PluginKey) -> Self {
        Self {
            server: agent_plugin_mcp_server_name(&key.to_string(), &definition.server),
            inject: definition.inject,
            extract: definition.extract,
            infer_outcome: definition.infer_outcome,
            agent_id: definition.agent_id.clone(),
        }
    }

    pub(super) fn subscriptions(&self) -> HookSubscription {
        let mut hooks = HookSubscription::empty();
        hooks.set(HookSubscription::CHAT_SYSTEM_TRANSFORM, self.inject);
        hooks.set(HookSubscription::IMPORTED_TURN_COMPLETED, self.extract);
        hooks.set(HookSubscription::SESSION_END, self.infer_outcome);
        hooks
    }

    pub(super) async fn prompt_section(&self, host: &dyn HostClient) -> Option<String> {
        if !self.inject {
            return None;
        }
        let response = host
            .read_imported_plugin_mcp(self.server.clone(), "memory://session-inject".into())
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                host.log(
                    LogLevel::Warn,
                    error.diagnostic_message().into(),
                    json!({"component":"MemVault", "operation":"read memories"}),
                )
                .await;
                return None;
            }
        };
        let mut section = String::new();
        let payload = response.payload?;
        for text in payload
            .get("contents")?
            .as_array()?
            .iter()
            .filter_map(|entry| entry["text"].as_str())
        {
            if !section.is_empty() {
                section.push('\n');
            }
            let remaining = MAX_PROMPT_BYTES.saturating_sub(section.len());
            let length = text.floor_char_boundary(remaining.min(text.len()));
            section.push_str(&text[..length]);
            if length < text.len() || section.len() == MAX_PROMPT_BYTES {
                break;
            }
        }
        (!section.trim().is_empty()).then_some(section)
    }

    pub(super) async fn completed_turn(
        &self,
        host: &dyn HostClient,
        turn: &ImportedTurnCompletedInput,
    ) {
        if !self.extract {
            return;
        }
        let mut arguments = json!({
            "response_text": turn.assistant_text,
            "agent_id": self.agent_id,
        });
        if !turn.user_text.trim().is_empty() {
            arguments["user_text"] = Value::String(turn.user_text.clone());
        }
        if let Err(error) = host
            .call_imported_plugin_mcp(self.server.clone(), "notify_response".into(), arguments)
            .await
        {
            host.log(
                LogLevel::Warn,
                error.diagnostic_message().into(),
                json!({"component":"MemVault", "operation":"record completed turn"}),
            )
            .await;
        }
    }
}

pub(super) async fn infer_session_outcome(bridges: &[MemVaultBridge], host: &dyn HostClient) {
    if !bridges.iter().any(|bridge| bridge.infer_outcome) {
        return;
    }
    let mut evidence = outcome::OutcomeEvidence::default();
    let observed = host
        .visit_ending_session_history(&mut |event| evidence.observe(event))
        .await;
    if let Err(error) = observed {
        host.log(
            LogLevel::Warn,
            error.diagnostic_message().into(),
            json!({"component":"MemVault", "operation":"read session outcome evidence"}),
        )
        .await;
        return;
    }
    for bridge in bridges.iter().filter(|bridge| bridge.infer_outcome) {
        let Some(arguments) = evidence.arguments(&bridge.agent_id) else {
            return;
        };
        if let Err(error) = host
            .call_imported_plugin_mcp(bridge.server.clone(), "infer_outcome".into(), arguments)
            .await
        {
            host.log(
                LogLevel::Warn,
                error.diagnostic_message().into(),
                json!({"component":"MemVault", "operation":"infer session outcome"}),
            )
            .await;
        }
    }
}
