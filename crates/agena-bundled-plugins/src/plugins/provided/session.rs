use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::plugins::provided::workflow::{
    SessionRenameToolInput, WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostContextStatusRequest, HostContextStatusResponse,
};
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};
use process_control::{ChildExt as _, Control as _};

pub(crate) const SESSION_PLUGIN_ID: &str = "agena.session";

pub(crate) struct SessionPlugin {
    inner: WorkflowPlugin,
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
    name = "session",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Inspect and manage the current runtime session and its environment, model, and token state.",
)]
impl SessionPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
            host: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(Arc::clone(&host))
            .map_err(|_| PluginError::internal("session plugin initialized more than once"))?;
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect the current session metadata.",
        read_only,
        concurrency_safe
    )]
    async fn get(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_get_session().await
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect the current runtime environment: working directory, git state, shell, OS, and architecture.",
        read_only,
        concurrency_safe
    )]
    async fn environment(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let mut lines = vec![format!("Working directory: {workspace_root}")];
        let mut git_branch = None::<String>;
        let mut git_short_sha = None::<String>;
        let mut git_dirty = false;
        let git_workspace = workspace_root.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|_| PluginError::internal("session worker pool is unavailable"))?;
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
        let payload = serde_json::json!({
            "workspace_root": workspace_root,
            "git_branch": git_branch,
            "git_short_sha": git_short_sha,
            "git_dirty": git_dirty,
            "shell": shell,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        Ok(ToolInvokeOutput::from_parts(
            "session environment",
            "environment facts",
            lines.join("\n"),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    async fn execution_snapshot(
        &self,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<HostContextStatusResponse> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("session plugin invoked before init"))?
            .get_context_status(HostContextStatusRequest {
                session_id: Some(context.session_id),
            })
            .await
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect the current session model identity, runtime modes, and model token limits.",
        read_only,
        concurrency_safe
    )]
    async fn model(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self.execution_snapshot(context).await?;
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
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "model_provider_id": status.model_provider_id,
            "model_adapter_id": status.model_adapter_id,
            "model_id": status.model_id,
            "thinking_mode": status.thinking_mode,
            "speed_mode": status.speed_mode,
            "verbosity": status.verbosity,
            "model_context_window_tokens": status.model_context_window_tokens,
            "model_max_input_tokens": status.model_max_input_tokens,
            "model_max_output_tokens": status.model_max_output_tokens,
        });
        Ok(ToolInvokeOutput::from_parts(
            "session model",
            model_identity,
            model_detail.join("; "),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect current and projected token use, effective limits, and remaining session budget.",
        read_only,
        concurrency_safe
    )]
    async fn tokens(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self.execution_snapshot(context).await?;
        let ratio = status
            .limit_tokens
            .and_then(|limit| (limit > 0).then_some(status.current_tokens as f64 / limit as f64));
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "current_tokens": status.current_tokens,
            "measured_prompt_tokens": status.measured_prompt_tokens,
            "projected_tokens": status.projected_tokens,
            "limit_tokens": status.limit_tokens,
            "remaining_tokens": status.remaining_tokens,
            "usage_ratio": ratio,
            "reserved_tokens": status.reserved_tokens,
        });
        let text = format!(
            "Tokens: {} used; measured {}; projected {}; limit {}; remaining {}; reserved {}.",
            status.current_tokens,
            status
                .measured_prompt_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .projected_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .limit_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .remaining_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status.reserved_tokens,
        );
        Ok(ToolInvokeOutput::from_parts(
            "session tokens",
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

    #[tool(tags(mutate), summary = "Rename the current session.", mutating)]
    async fn rename(&self, input: &SessionRenameToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_rename_session(input).await
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::SessionPlugin;

    #[test]
    fn manifest_contains_split_session_tools() {
        let manifest = SessionPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "session");
        assert_eq!(
            tool_names,
            ["get", "environment", "model", "tokens", "rename"]
        );
    }
}
