//! Verification uses the actual foreground shell results of one attempt.
use super::*;
use agena_domain::ContentRef;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub(super) fn validate_commands(commands: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut unique = std::collections::BTreeSet::new();
    if commands.len() > 8
        || commands.iter().any(|command| {
            command.is_empty()
                || command.trim() != command
                || command.len() > 1024
                || command.contains('\0')
                || !unique.insert(command)
        })
    {
        return Err(AppError::Config(
            "verification_commands accepts up to eight unique commands of 1–1024 bytes without leading or trailing whitespace".into(),
        ));
    }
    Ok(commands)
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Verdict {
    Pass,
    Fail,
    Blocked,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Blocked => "BLOCKED",
        }
    }
}

#[derive(Serialize)]
pub(super) struct Check {
    command: String,
    status: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_id: Option<ToolId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_resource: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

impl Check {
    fn blocked(command: &str, reason: &'static str) -> Self {
        Self {
            command: command.to_owned(),
            status: Verdict::Blocked,
            exit_code: None,
            tool_id: None,
            output_resource: None,
            reason: Some(reason),
        }
    }

    fn record(&mut self, tool: &ToolView, payload: &Value, worktree: Option<&Path>) {
        self.tool_id = Some(tool.id);
        self.output_resource = payload
            .get("output_resource")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        if "timed_out" == payload["status"].as_str().unwrap_or_default() {
            self.reason = Some("command timed out");
        } else if tool.phase != ToolPhase::Succeeded || payload["status"] != "exited" {
            self.reason = Some("shell did not record a completed exit");
        } else if worktree.is_some_and(|root| {
            payload["metadata"]["workdir"]
                .as_str()
                .is_none_or(|cwd| !Path::new(cwd).starts_with(root))
        }) {
            self.reason = Some("command ran outside the assigned checkout");
        } else if let Some(code) = payload["exit_code"].as_i64() {
            self.exit_code = Some(code);
            self.status = if code == 0 {
                Verdict::Pass
            } else {
                Verdict::Fail
            };
            self.reason = None;
        }
    }

    pub(super) fn display(&self) -> String {
        let detail = if let Some(code) = self.exit_code {
            format!("exit {code}")
        } else {
            self.reason.unwrap_or("exit status unavailable").to_owned()
        };
        let evidence = self.output_resource.as_ref().map_or_else(
            || {
                self.tool_id
                    .map_or_else(String::new, |id| format!(", tool call {}", id.get()))
            },
            |output| format!(", output resource {}", output.resource_id),
        );
        format!(
            "- {} `{}` ({detail}{evidence})",
            self.status.as_str(),
            self.command
        )
    }
}

pub(super) fn blocked(assignment: &AgentAssignment, reason: &'static str) -> Vec<Check> {
    assignment
        .verification_commands
        .iter()
        .map(|command| Check::blocked(command, reason))
        .collect()
}

impl SessionManager {
    pub(super) async fn verification_checks(
        &self,
        session: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
        assignment: &AgentAssignment,
    ) -> Result<Vec<Check>, AppError> {
        let mut checks = blocked(assignment, "no matching shell.exec result");
        if checks.is_empty() {
            return Ok(checks);
        }
        let reader = &self.kernel().reader;
        let run = reader
            .execution_attempt(session, execution, generation)
            .await
            .map_err(kernel_error)?;
        let worktree = assignment.worktree.as_ref().map(|info| info.path.as_path());
        let mut before = None;
        loop {
            let page = reader
                .history(session, before, 128)
                .await
                .map_err(kernel_error)?;
            for entry in page.entries.iter().rev() {
                if entry.id < run.message_id {
                    return Ok(checks);
                }
                if entry.origin_session_id != session
                    || entry.execution_id != execution
                    || entry.generation != generation
                {
                    continue;
                }
                let EntryPayload::Tool { tool_id } = entry.payload else {
                    continue;
                };
                let tool = reader.tool(session, tool_id).await.map_err(kernel_error)?;
                if tool.execution_kind != ToolExecutionKind::Runtime {
                    continue;
                }
                let Some(result) = tool.result.as_ref() else {
                    continue;
                };
                let name = result["invocation"]["name"].as_str().unwrap_or_default();
                if !matches!(name, "agena.shell.exec" | "shell.exec") {
                    continue;
                }
                let Ok(invocation) = serde_json::from_value::<agena_domain::ToolInvocation>(
                    result["invocation"].clone(),
                ) else {
                    continue;
                };
                let payload = &result["raw_output"]["payload"];
                let command = payload["metadata"]["final_command"].as_str().or_else(|| {
                    invocation
                        .input
                        .get("command")
                        .and_then(|value| value.as_text())
                });
                if let Some(check) = checks.iter_mut().find(|check| {
                    check.tool_id.is_none() && command == Some(check.command.as_str())
                }) {
                    // A failed invocation can prove a blocked check. A pass
                    // or fail requires an actual exit from our shell plugin.
                    if invocation.plugin_name.as_deref() == Some("agena.shell")
                        || tool.phase != ToolPhase::Succeeded
                    {
                        check.record(&tool, payload, worktree);
                    }
                }
                if checks.iter().all(|check| check.tool_id.is_some()) {
                    return Ok(checks);
                }
            }
            before = page.next;
            if !page.has_more || before.is_none() {
                return Ok(checks);
            }
        }
    }

    pub(in crate::session::manager) async fn remind_agent_verification(
        &self,
        attempt: &AttemptHandle,
        control: &RunControl,
        assignment: &AgentAssignment,
    ) -> Result<bool, AppError> {
        let checks = self
            .verification_checks(
                control.session,
                control.execution,
                control.generation,
                assignment,
            )
            .await?;
        let missing = checks
            .iter()
            .filter(|check| check.tool_id.is_none())
            .map(|check| check.command.as_str())
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(false);
        }
        let notice = format!(
            "Agena has no recorded shell.exec results for these requested checks in this execution. If permitted, run the exact commands now from the assigned workspace. Otherwise report why the checks are blocked.\n{}",
            serde_json::to_string_pretty(&missing)?,
        );
        let command = RequestId::new();
        persist(|| attempt.notice(command, notice.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(true)
    }
}
