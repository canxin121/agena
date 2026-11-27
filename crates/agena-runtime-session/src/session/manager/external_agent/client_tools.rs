use std::{collections::HashMap, path::Path, time::Duration};

use agena_domain::{ContentFormat, ProcessStatus, RawOutput, StructuredObject, ToolInvocation};
use agena_runtime_contracts::part::ShellSignal;
use agena_runtime_tools::{TerminalOwner, TerminalRead, TerminalRegistry};
use tokio::sync::Mutex;

use super::*;

const FILE_BYTES: usize = 1024 * 1024;
const TERMINAL_BYTES: usize = 1024 * 1024;
const MAX_HOSTED_TERMINALS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Operation {
    ReadFile,
    WriteFile,
    CreateTerminal,
    TerminalOutput,
    WaitTerminal,
    KillTerminal,
    ReleaseTerminal,
}

impl Operation {
    pub(super) fn parse(method: &str) -> Option<Self> {
        Some(match method {
            "fs/read_text_file" => Self::ReadFile,
            "fs/write_text_file" => Self::WriteFile,
            "terminal/create" => Self::CreateTerminal,
            "terminal/output" => Self::TerminalOutput,
            "terminal/wait_for_exit" => Self::WaitTerminal,
            "terminal/kill" => Self::KillTerminal,
            "terminal/release" => Self::ReleaseTerminal,
            _ => return None,
        })
    }

    pub(super) async fn request(
        self,
        turn: &ExternalTurn,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Value, AppError> {
        if withdrawal.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        match self {
            Self::ReadFile => turn.read_native_file(params, withdrawal).await,
            Self::WriteFile => turn.write_native_file(params, withdrawal).await,
            Self::CreateTerminal => turn.create_native_terminal(params, withdrawal).await,
            operation => turn.native_terminal(operation, params, withdrawal).await,
        }
    }
}

#[derive(Default)]
pub(super) struct Terminals {
    hosted: Mutex<HashMap<String, HeldTerminal>>,
}

#[derive(Clone)]
struct HeldTerminal {
    process: String,
    owner: TerminalOwner,
    output_limit: usize,
}

impl ExternalTurn {
    async fn invoke_client_tool_outcome(
        &self,
        name: &str,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<(ToolPhase, Option<Value>, Option<String>), AppError> {
        let arguments = StructuredObject::try_from(input.clone()).map_err(AppError::Config)?;
        let invocation = ToolInvocation::new(name.to_owned(), arguments);
        let (_, result) = self.invoke_tool(invocation, input, cancel).await?;
        if result.phase != ToolPhase::Succeeded {
            let code = result.value["error"]["failure"]["code"]
                .as_str()
                .or_else(|| result.value["error"]["code"].as_str())
                .map(str::to_owned);
            return Ok((result.phase, None, code));
        }
        let raw: RawOutput = serde_json::from_value(result.value["raw_output"].clone())?;
        if let Some(payload) = raw.payload {
            return Ok((result.phase, Some(payload), None));
        }
        let Some(field) = raw
            .fields
            .iter()
            .find(|field| field.pointer.is_empty() && field.format == ContentFormat::Json)
        else {
            return Ok((result.phase, Some(Value::Null), None));
        };
        let session = self.control.session;
        let reader = &self.manager.kernel().reader;
        let head = read_with_busy_retry(session, "external_agent_client_tool", || {
            reader.text_range(session, field.resource.resource_id, None, 1)
        })
        .await?
        .stream
        .clone();
        let mut budget = 8 * FILE_BYTES;
        let text = read_text(reader, session, &head, &mut budget, None)
            .await
            .map_err(kernel_error)?;
        Ok((result.phase, Some(serde_json::from_str(&text)?), None))
    }

    async fn invoke_client_tool(
        &self,
        name: &str,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        Ok(self
            .invoke_client_tool_outcome(name, input, cancel)
            .await?
            .1)
    }

    async fn read_native_file(
        &self,
        params: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let path = absolute_path(&params["path"])?;
        let line = optional_usize(params, "line")?.unwrap_or(1);
        let limit = optional_usize(params, "limit")?;
        if line == 0 {
            return Err(AppError::Config("ACP file lines start at one.".into()));
        }
        let result = self
            .invoke_client_tool(
                "fs.read_many",
                json!({"paths": [path], "max_total_bytes": FILE_BYTES}),
                cancel,
            )
            .await?
            .ok_or_else(denied)?;
        let result = &result["files"][0];
        let content = result["content"].as_str().ok_or_else(denied)?;
        if result["truncated"] == true
            && !limit.is_some_and(|count| {
                content.bytes().filter(|byte| *byte == b'\n').count()
                    >= (line - 1).saturating_add(count)
            })
        {
            return Err(AppError::Config(
                "The ACP file exceeds the text read budget. Request a smaller line range.".into(),
            ));
        }
        let content: String = content
            .split_inclusive('\n')
            .skip(line - 1)
            .take(limit.unwrap_or(usize::MAX))
            .collect();
        Ok(json!({"content": content}))
    }

    async fn write_native_file(
        &self,
        params: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        let path = absolute_path(&params["path"])?;
        let text = params["content"].as_str().ok_or_else(invalid)?;
        // A revision-checked fs.write can create a missing file without a
        // hash. Replacing a file requires an authorized and observed revision.
        // If fs.stat was denied or cancelled by permission policy, do not fall
        // through to fs.write as if the file were missing.
        let (stat_phase, stat, stat_code) = self
            .invoke_client_tool_outcome("fs.stat", json!({"path": path}), cancel)
            .await?;
        if stat_phase == ToolPhase::Cancelled
            || stat_code
                .as_deref()
                .is_some_and(|code| code.contains("permission") || code.contains("denied"))
        {
            return Err(denied());
        }
        let input = json!({
            "path": path, "content": text, "create_parents": true,
            "expected_sha256": stat.as_ref().and_then(|stat| stat["sha256"].as_str()),
        });
        self.invoke_client_tool("fs.write", input, cancel)
            .await?
            .ok_or_else(denied)?;
        Ok(json!({}))
    }

    async fn create_native_terminal(
        &self,
        params: &Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        if self.terminals.hosted.lock().await.len() >= MAX_HOSTED_TERMINALS {
            return Err(AppError::Config(
                "Too many ACP client terminals are open in this turn.".into(),
            ));
        }
        let command = terminal_command(params)?;
        let cwd = match params.get("cwd").filter(|value| !value.is_null()) {
            Some(cwd) => absolute_path(cwd)?,
            None => self
                .executor
                .workspace_root()
                .to_str()
                .ok_or_else(invalid)?,
        };
        let output_limit = optional_usize(params, "outputByteLimit")?
            .unwrap_or(TERMINAL_BYTES)
            .min(TERMINAL_BYTES);
        let shell = if cfg!(windows) { "powershell" } else { "bash" };
        let owner = TerminalOwner {
            workspace: tokio::fs::canonicalize(self.executor.workspace_root()).await?,
            session_id: Some(self.control.session.get()),
        };
        let result = self
            .invoke_client_tool(
                "shell.open",
                json!({
                    "command": command, "shell": shell, "workdir": cwd,
                    "description": "ACP terminal", "yield_time_ms": 0,
                }),
                cancel,
            )
            .await?
            .ok_or_else(denied)?;
        let process = result["process_id"].as_str().ok_or_else(denied)?.to_owned();
        let id = uuid::Uuid::new_v4().to_string();
        self.terminals.hosted.lock().await.insert(
            id.clone(),
            HeldTerminal {
                process,
                owner,
                output_limit,
            },
        );
        Ok(json!({"terminalId": id}))
    }

    async fn native_terminal(
        &self,
        operation: Operation,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Value, AppError> {
        let id = params["terminalId"].as_str().ok_or_else(invalid)?;
        let terminal = self
            .terminals
            .hosted
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(invalid)?;
        match operation {
            Operation::TerminalOutput => {
                let read = self.read_terminal(terminal.clone(), false).await?;
                let (output, truncated) = terminal_output(&read, terminal.output_limit);
                Ok(
                    json!({"output": output, "truncated": truncated, "exitStatus": exit_status(&read)}),
                )
            }
            Operation::WaitTerminal => loop {
                let read = self.read_terminal(terminal.clone(), false).await?;
                if let Some(status) = exit_status(&read) {
                    break Ok(status);
                }
                tokio::select! {
                    _ = withdrawal.cancelled() => break Err(AppError::Cancelled),
                    _ = self.control.cancel.cancelled() => break Err(AppError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            },
            Operation::KillTerminal => {
                self.read_terminal(terminal, true).await?;
                Ok(json!({}))
            }
            Operation::ReleaseTerminal => {
                self.terminals.hosted.lock().await.remove(id);
                self.read_terminal(terminal, true).await?;
                Ok(json!({}))
            }
            _ => Err(invalid()),
        }
    }

    async fn read_terminal(
        &self,
        terminal: HeldTerminal,
        stop: bool,
    ) -> Result<TerminalRead, AppError> {
        let monitor = self
            .executor
            .monitor_registry()
            .cloned()
            .ok_or_else(denied)?;
        tokio::task::spawn_blocking(move || {
            let registry: &TerminalRegistry = monitor.terminals().ok_or_else(denied)?;
            let result = if stop {
                registry.signal(&terminal.process, &terminal.owner, ShellSignal::Kill)
            } else {
                registry.read(&terminal.process, &terminal.owner, Some(0), 0, None)
            };
            result.map_err(|error| AppError::Config(error.to_string()))
        })
        .await
        .map_err(|error| AppError::Internal(format!("ACP terminal task stopped: {error}")))?
    }

    pub(super) async fn native_terminal_output(&self, id: &str) -> Option<(String, Option<i32>)> {
        let terminal = self.terminals.hosted.lock().await.get(id).cloned()?;
        let read = self.read_terminal(terminal.clone(), false).await.ok()?;
        Some((
            terminal_output(&read, terminal.output_limit.min(32 * 1024)).0,
            read.summary.exit_code,
        ))
    }

    pub(super) async fn close_native_terminals(&self) {
        let terminals = std::mem::take(&mut *self.terminals.hosted.lock().await);
        for terminal in terminals.into_values() {
            if let Err(error) = self.read_terminal(terminal, true).await {
                tracing::debug!(%error, "could not close an ACP client terminal");
            }
        }
    }
}

fn absolute_path(value: &Value) -> Result<&str, AppError> {
    value
        .as_str()
        .filter(|path| Path::new(path).is_absolute() && !path.contains('\0'))
        .ok_or_else(invalid)
}

fn optional_usize(params: &Value, key: &str) -> Result<Option<usize>, AppError> {
    params
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_u64()
                .and_then(|number| usize::try_from(number).ok())
                .ok_or_else(invalid)
        })
        .transpose()
}

fn terminal_output(read: &TerminalRead, limit: usize) -> (String, bool) {
    let start = read
        .output
        .ceil_char_boundary(read.output.len().saturating_sub(limit));
    (
        read.output[start..].to_owned(),
        start > 0 || read.dropped_bytes > 0 || read.has_more,
    )
}

fn exit_status(read: &TerminalRead) -> Option<Value> {
    (read.summary.status != ProcessStatus::Running)
        .then(|| json!({"exitCode": read.summary.exit_code}))
}

fn terminal_command(params: &Value) -> Result<String, AppError> {
    let command = params["command"]
        .as_str()
        .filter(|command| !command.is_empty())
        .ok_or_else(invalid)?;
    let args = match params.get("args") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(items)) => items.as_slice(),
        Some(_) => return Err(invalid()),
    };
    let argv = std::iter::once(Ok(command))
        .chain(args.iter().map(|arg| arg.as_str().ok_or_else(invalid)))
        .collect::<Result<Vec<_>, _>>()?;
    let env_entries = match params.get("env") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(items)) => items.as_slice(),
        Some(_) => return Err(invalid()),
    };
    let env = env_entries
        .iter()
        .map(|entry| {
            let name = entry["name"]
                .as_str()
                .filter(|name| {
                    let mut bytes = name.bytes();
                    bytes
                        .next()
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                        && agena_domain::valid_terminal_environment_name(name)
                })
                .ok_or_else(invalid)?;
            let value = entry["value"].as_str().ok_or_else(invalid)?;
            Ok((name, value))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    if argv.iter().any(|arg| arg.contains('\0'))
        || env.iter().any(|(_, value)| value.contains('\0'))
    {
        return Err(invalid());
    }
    let quote = |argument: &str| {
        if cfg!(windows) {
            // PowerShell treats ASCII single quote and curly single quotes
            // (U+2018..=U+201B) as string delimiters; double each to escape.
            let escaped = argument
                .replace('\'', "''")
                .replace('\u{2018}', "\u{2018}\u{2018}")
                .replace('\u{2019}', "\u{2019}\u{2019}")
                .replace('\u{201A}', "\u{201A}\u{201A}")
                .replace('\u{201B}', "\u{201B}\u{201B}");
            format!("'{escaped}'")
        } else {
            format!("'{}'", argument.replace('\'', "'\\''"))
        }
    };
    let mut invocation = String::new();
    for (name, value) in env {
        if cfg!(windows) {
            invocation.push_str(&format!("$env:{name}={}; ", quote(value)));
        } else {
            invocation.push_str(&format!("{name}={} ", quote(value)));
        }
    }
    if cfg!(windows) {
        invocation.push_str("& ");
    }
    invocation.push_str(&argv.into_iter().map(quote).collect::<Vec<_>>().join(" "));
    Ok(invocation)
}

fn invalid() -> AppError {
    AppError::Config("Invalid ACP file or terminal parameters.".into())
}

fn denied() -> AppError {
    AppError::Config(
        "The ACP file or terminal operation was not completed by Agena's tools.".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_command_rejects_dangerous_env_non_array_args_and_nul_bytes() {
        assert!(terminal_command(&json!({
            "command": "echo",
            "args": "not-an-array"
        }))
        .is_err());
        assert!(terminal_command(&json!({
            "command": "echo",
            "env": {"FOO": "bar"}
        }))
        .is_err());
        assert!(terminal_command(&json!({
            "command": "echo",
            "args": ["ok\0bad"]
        }))
        .is_err());
        for blocked in ["PATH", "LD_PRELOAD", "NODE_OPTIONS", "AGENA_DATA_DIR", "HTTP_PROXY"] {
            assert!(
                terminal_command(&json!({
                    "command": "echo",
                    "env": [{"name": blocked, "value": "x"}]
                }))
                .is_err(),
                "{blocked} should be rejected"
            );
        }
        let ok = terminal_command(&json!({
            "command": "echo",
            "args": ["hello'world"],
            "env": [{"name": "APP_MODE", "value": "test"}]
        }))
        .expect("safe command");
        assert!(ok.contains("APP_MODE"));
        assert!(ok.contains("echo"));
    }
}
