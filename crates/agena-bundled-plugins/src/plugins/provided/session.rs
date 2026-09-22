use sha2::{Digest, Sha256};
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
    dirty: Option<bool>,
}

fn run_git(workspace: &Path, args: &[&str]) -> SdkResult<Option<String>> {
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
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "start Git while inspecting the session environment",
                &error,
            ))
        })?;
    let mut retained = 0_usize;
    let truncated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture_truncated = truncated.clone();
    let output = child
        .controlled_with_output()
        .stdout_filter(move |chunk: &[u8]| {
            if retained.saturating_add(chunk.len()) <= MAX_GIT_FACT_BYTES {
                retained += chunk.len();
                Ok(true)
            } else {
                retained = MAX_GIT_FACT_BYTES;
                capture_truncated.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(false)
            }
        })
        .time_limit(Duration::from_secs(15))
        .terminate_for_timeout()
        .wait()
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "wait for Git while inspecting the session environment",
                &error,
            ))
        })?
        .ok_or_else(|| {
            PluginError::timeout_with_public_detail(
                "Git session environment inspection timed out after 15 seconds",
                "Git inspection timed out after 15 seconds.",
            )
        })?;
    if truncated.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(arguments = ?args, "Git inspection output exceeded its budget; facts remain unknown");
        return Ok(None);
    }
    if !output.status.success() {
        tracing::debug!(
            arguments = ?args,
            status = %output.status,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "Git session environment probe exited unsuccessfully"
        );
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "decode Git session environment output as UTF-8",
            &error,
        ))
    })?;
    Ok(Some(stdout.trim().to_string()))
}

fn git_dirty_from_status(status: Option<&str>) -> Option<bool> {
    status.map(|status| !status.trim().is_empty())
}

fn git_facts(workspace: &Path) -> SdkResult<Option<GitFacts>> {
    let Some(branch) = run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])? else {
        return Ok(None);
    };
    let short_sha = run_git(workspace, &["rev-parse", "--short", "HEAD"])?;
    let status = run_git(workspace, &["status", "--porcelain"])?;
    let dirty = git_dirty_from_status(status.as_deref());
    Ok(Some(GitFacts {
        branch: Some(branch),
        short_sha,
        dirty,
    }))
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
        let mut git_dirty = None;
        let git_workspace = workspace_root.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire a session environment worker",
                    &error,
                ))
            })?;
        let facts = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            git_facts(Path::new(&git_workspace))
        })
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "Git session environment inspection task failed",
                &error,
            ))
        })??;
        if let Some(facts) = facts {
            git_branch = facts.branch;
            git_short_sha = facts.short_sha;
            git_dirty = facts.dirty;
            if let (Some(branch), Some(short_sha)) =
                (git_branch.as_deref(), git_short_sha.as_deref())
            {
                let dirty = match git_dirty {
                    Some(true) => " (dirty)",
                    Some(false) => " (clean)",
                    None => " (status unknown)",
                };
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
        let catalog = match self.inner.host() {
            Ok(host) => match host.list_tools().await {
                Ok(mut tools) => {
                    tools.sort_by(|a, b| a.name.cmp(&b.name));
                    let bytes = serde_json::to_vec(&tools)
                        .map_err(|error| PluginError::internal_error(&error))?;
                    Some(
                        serde_json::json!({"status":"available","count":tools.len(),"sha256":hex::encode(Sha256::digest(&bytes)),
                        "interactive_shell":tools.iter().any(|tool|tool.name.ends_with("shell.write")),"output_recovery":tools.iter().any(|tool|tool.name.ends_with("fs.output_read"))}),
                    )
                }
                Err(_) => None,
            },
            Err(_) => None,
        };
        let build = serde_json::json!({"package_version":env!("CARGO_PKG_VERSION"),"target":env!("AGENA_TOOL_BUILD_TARGET"),"source_fingerprint":env!("AGENA_TOOL_SOURCE_FINGERPRINT"),"source_scope":env!("AGENA_TOOL_SOURCE_SCOPE")});
        lines.push(format!(
            "Compiled tool-runtime source: {} ({})",
            env!("AGENA_TOOL_SOURCE_FINGERPRINT"),
            env!("AGENA_TOOL_BUILD_TARGET")
        ));
        lines.push(format!(
            "Visible tool catalogue: {}",
            catalog
                .as_ref()
                .map(|catalog| catalog.to_string())
                .unwrap_or_else(
                    || "unavailable; capabilities were not inferred from the checkout".into()
                )
        ));
        let payload = serde_json::json!({
            "tool_runtime_build": build,
            "tool_catalog": catalog,
            "workspace_root": workspace_root,
            "git_branch": git_branch,
            "git_short_sha": git_short_sha,
            "git_dirty": git_dirty,
            "git_status_known": git_dirty.is_some(),
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

#[cfg(test)]
mod audit_fact_tests {
    #[test]
    fn unavailable_status_is_not_reported_as_clean() {
        assert_eq!(super::git_dirty_from_status(None), None);
        assert_eq!(super::git_dirty_from_status(Some("")), Some(false));
        assert_eq!(
            super::git_dirty_from_status(Some(" M src/lib.rs\n")),
            Some(true)
        );
    }
    #[tokio::test]
    async fn nongit_workspace_has_explicit_unknown_git_status() {
        let directory = tempfile::tempdir().unwrap();
        let context = agena_plugin_host::sdk::ToolInvokeContext {
            tool_name: "environment",
            session_id: 41,
            call_id: 1,
            workspace_root: directory.path().to_str().unwrap(),
        };
        let output = super::SessionPlugin::new()
            .environment(&context)
            .await
            .unwrap()
            .payload
            .unwrap();
        assert!(output["git_dirty"].is_null());
        assert_eq!(output["git_status_known"], false);
    }
}
