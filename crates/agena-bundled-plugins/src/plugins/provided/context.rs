//! Safe model-context budget, model identity, compaction status, and session
//! environment facts.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostContextStatusRequest, HostGetSessionRequest,
};
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
use process_control::{ChildExt as _, Control as _};

pub(crate) const CONTEXT_PLUGIN_ID: &str = "agena.context";

pub(crate) struct ContextPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Default)]
struct GitFacts {
    branch: Option<String>,
    short_sha: Option<String>,
    dirty: bool,
}

fn run_git(workspace: &Path, args: &[&str]) -> Option<String> {
    const MAX_GIT_FACT_BYTES: usize = 4 * 1024 * 1024;
    let child = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("GPG_TTY", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Git facts are best-effort and never surface stderr. Discard it so a
        // broken hook or wrapper cannot make this probe retain unbounded data.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut retained = 0_usize;
    let output = child
        .controlled_with_output()
        .stdout_filter(move |chunk: &[u8]| {
            if retained.saturating_add(chunk.len()) <= MAX_GIT_FACT_BYTES {
                retained += chunk.len();
                Ok(true)
            } else {
                retained = MAX_GIT_FACT_BYTES;
                Ok(false)
            }
        })
        .time_limit(Duration::from_secs(15))
        .terminate_for_timeout()
        .wait()
        .ok()??;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Fresh git facts for a workspace. Deliberately uncached: the environment can
/// change mid-session, so callers query on demand for current values.
fn git_facts(workspace: &Path) -> Option<GitFacts> {
    let branch = run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let short_sha = run_git(workspace, &["rev-parse", "--short", "HEAD"]);
    let dirty = run_git(workspace, &["status", "--porcelain"])
        .map(|status| !status.trim().is_empty())
        .unwrap_or(false);
    Some(GitFacts {
        branch: Some(branch),
        short_sha,
        dirty,
    })
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "context",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Safe context-window budget, model identity, and compaction status.",
)]
impl ContextPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("context plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect remaining context budget, model identity, and compaction health without exposing prompts.",
        read_only,
        concurrency_safe
    )]
    async fn status(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("context plugin invoked before init"))?
            .get_context_status(HostContextStatusRequest {
                session_id: Some(context.session_id),
            })
            .await?;
        let ratio = status
            .limit_tokens
            .and_then(|limit| (limit > 0).then_some(status.current_tokens as f64 / limit as f64));
        let model_identity = match (
            status.model_provider_id.as_deref(),
            status.model_adapter_id.as_deref(),
            status.model_id.as_deref(),
        ) {
            (Some(provider), Some(adapter), Some(model)) => {
                format!("{provider}/{adapter}/{model}")
            }
            (Some(provider), None, Some(model)) => format!("{provider}/{model}"),
            _ => "unknown".to_string(),
        };
        let mut model_detail = vec![format!("Model: {model_identity}")];
        if let Some(thinking) = status.thinking_mode.as_deref() {
            model_detail.push(format!("thinking: {thinking}"));
        }
        if let Some(speed) = status.speed_mode.as_deref() {
            model_detail.push(format!("speed: {speed}"));
        }
        if let Some(verbosity) = status.verbosity.as_deref() {
            model_detail.push(format!("verbosity: {verbosity}"));
        }
        let text = format!(
            "Context: {} token(s) used; limit {}; remaining {}; generation {}; compacted={}; auto_compaction_disabled={}. {}.",
            status.current_tokens,
            status
                .limit_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .remaining_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status.prompt_window_generation,
            status.compacted,
            status.auto_compaction_disabled,
            model_detail.join("; "),
        );
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "model_provider_id": status.model_provider_id,
            "model_adapter_id": status.model_adapter_id,
            "model_id": status.model_id,
            "thinking_mode": status.thinking_mode,
            "speed_mode": status.speed_mode,
            "verbosity": status.verbosity,
            "current_tokens": status.current_tokens,
            "measured_prompt_tokens": status.measured_prompt_tokens,
            "projected_tokens": status.projected_tokens,
            "limit_tokens": status.limit_tokens,
            "remaining_tokens": status.remaining_tokens,
            "usage_ratio": ratio,
            "reserved_tokens": status.reserved_tokens,
            "model_context_window_tokens": status.model_context_window_tokens,
            "model_max_input_tokens": status.model_max_input_tokens,
            "model_max_output_tokens": status.model_max_output_tokens,
            "prompt_window_generation": status.prompt_window_generation,
            "compacted": status.compacted,
            "last_compaction_before_tokens": status.last_compaction_before_tokens,
            "last_compaction_after_tokens": status.last_compaction_after_tokens,
            "auto_compaction_disabled": status.auto_compaction_disabled,
            "consecutive_compaction_failures": status.consecutive_compaction_failures,
        });
        Ok(ToolInvokeOutput::from_parts(
            "context status",
            status.remaining_tokens.map_or_else(
                || format!("{} tokens used", status.current_tokens),
                |remaining| format!("{} used · {remaining} remaining", status.current_tokens),
            ),
            text,
            Some(payload),
            std::collections::BTreeMap::from([
                (
                    "current_tokens".to_string(),
                    status.current_tokens.to_string(),
                ),
                (
                    "remaining_tokens".to_string(),
                    status
                        .remaining_tokens
                        .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
                ),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect the current session environment: working directory, git state, shell, OS, and session identity.",
        read_only,
        concurrency_safe
    )]
    async fn environment(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("context plugin invoked before init"))?;
        let workspace_root = context.workspace_root.to_string();
        let mut lines = vec![format!("Working directory: {}", workspace_root)];
        let mut git_branch = None::<String>;
        let mut git_short_sha = None::<String>;
        let mut git_dirty = false;
        let git_workspace = workspace_root.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|_| PluginError::internal("context worker pool is unavailable"))?;
        let facts = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            git_facts(Path::new(&git_workspace))
        })
        .await
        .map_err(|error| PluginError::internal(format!("git inspection failed: {error}")))?;
        if let Some(facts) = facts {
            git_branch = facts.branch;
            git_short_sha = facts.short_sha;
            git_dirty = facts.dirty;
            if let (Some(branch), Some(short_sha)) =
                (git_branch.as_deref(), git_short_sha.as_deref())
            {
                let dirty = if git_dirty { " (dirty)" } else { "" };
                lines.push(format!("Git: {branch} @ {short_sha}{dirty}"));
            } else if let Some(branch) = git_branch.as_deref() {
                lines.push(format!("Git branch: {branch}"));
            }
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell".to_string()
                } else {
                    "/bin/bash".to_string()
                }
            });
        lines.push(format!("Shell: {shell}"));
        lines.push(format!(
            "OS: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        let session = host
            .get_session(HostGetSessionRequest {
                session_id: Some(context.session_id),
            })
            .await;
        let is_subagent = session
            .as_ref()
            .is_ok_and(|response| response.session.is_subagent);
        if is_subagent {
            lines.push(format!("Session: subagent {}", context.session_id));
        } else {
            lines.push(format!("Session: {}", context.session_id));
        }
        let text = lines.join("\n");
        let payload = serde_json::json!({
            "workspace_root": workspace_root,
            "git_branch": git_branch,
            "git_short_sha": git_short_sha,
            "git_dirty": git_dirty,
            "shell": shell,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "session_id": context.session_id,
            "is_subagent": is_subagent,
        });
        Ok(ToolInvokeOutput::from_parts(
            "session environment",
            "environment facts",
            text,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::ContextPlugin;

    #[test]
    fn manifest_exposes_safe_context_status_and_environment() {
        let manifest = ContextPlugin::new().manifest();
        assert!(manifest.tools.iter().any(|tool| tool.name == "status"));
        assert!(manifest.tools.iter().any(|tool| tool.name == "environment"));
    }
}
