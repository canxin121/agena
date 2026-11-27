use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use agena_domain::ExternalAgent;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::process::Command;
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};

#[cfg(unix)]
use std::os::unix::fs::symlink as link_directory;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir as link_directory;

use super::{AppError, MAX_FRAME_BYTES, PromptIdle, agent_environment, isolation, policy};

pub(super) async fn compact(
    cwd: &Path,
    additional_directories: &[PathBuf],
    native_id: &str,
    previous_history: bool,
    isolated_session: bool,
    cancel: &CancellationToken,
) -> Result<u64, AppError> {
    let profile = policy::antigravity_profile()?;
    let name = policy::antigravity_agent_name(&profile)?;
    // Print mode selects its profile from the home directory. Link the
    // existing store so ACP and the command use the same native conversation.
    let command_home = tempfile::Builder::new()
        .prefix("agena-sandbox-antigravity-")
        .tempdir()?;
    let config = command_home.path().join(".gemini");
    if previous_history {
        std::fs::create_dir(&config)?;
        link_directory(profile.join("config"), config.join("config"))?;
        link_directory(
            policy::previous_antigravity_store()?,
            config.join("antigravity-cli"),
        )?;
    } else {
        link_directory(&profile, &config)?;
    }

    let (program, _) = ExternalAgent::Antigravity.command();
    let mut command = Command::new(program);
    agent_environment(&mut command);
    if !previous_history {
        command.env(
            agena_mcp_server::antigravity_guard::MANAGED_PROFILE_ENV,
            "1",
        );
    }
    command
        .args([
            "--agent",
            &name,
            "--resume",
            native_id,
            "--print",
            "/compact",
            "--output-format",
            "stream-json",
        ])
        .env("HOME", command_home.path())
        .env("USERPROFILE", command_home.path())
        .current_dir(cwd);
    let process_isolation = isolation::wrap(
        &mut command,
        ExternalAgent::Antigravity,
        cwd,
        additional_directories,
        previous_history,
        Some(command_home.path()),
    )?;
    if isolated_session && process_isolation.is_none() {
        return Err(AppError::Config(
            "Antigravity's ACP sandbox policy changed. Reopen the session before compacting."
                .into(),
        ));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = agena_process::spawn(command)?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| AppError::Internal("Antigravity did not provide stdout.".into()))?;
    let mut output = FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_FRAME_BYTES));
    let complete = async {
        let mut checkpoint = None;
        let mut succeeded = false;
        let mut progress = PromptIdle::after_send(None);
        let idle = || {
            AppError::Config("Antigravity stopped reporting compaction progress. Reopen the native conversation to check its checkpoint.".into())
        };
        while let Some(line) = tokio::select! {
            _ = progress.expired(None) => return Err(idle()),
            line = output.next() => line,
        } {
            let line = line.map_err(|_| {
                AppError::Config("Antigravity returned an invalid compaction stream.".into())
            })?;
            let event: Value = serde_json::from_str(&line)?;
            let advanced = match event["event"].as_str() {
                Some("init") if event["conversation_id"].as_str() != Some(native_id) => {
                    return Err(AppError::Config(
                        "Antigravity could not resume the conversation for compaction.".into(),
                    ));
                }
                Some("init") => true,
                Some("step_update") => {
                    let step = &event["step_update"];
                    let checkpoint_done = step["conversation_id"].as_str() == Some(native_id)
                        && step["step_type"] == "checkpoint"
                        && step["state"] == "DONE";
                    if checkpoint_done {
                        checkpoint = step["step_index"].as_u64();
                    }
                    step["conversation_id"].as_str() == Some(native_id)
                }
                Some("result") => {
                    let result = &event["result"];
                    succeeded = result["conversation_id"].as_str() == Some(native_id)
                        && result["status"] == "SUCCESS";
                    succeeded
                }
                _ => false,
            };
            if advanced {
                progress.record();
            }
        }
        let status = tokio::select! {
            _ = progress.expired(None) => return Err(idle()),
            status = child.wait() => status?,
        };
        if !status.success() || !succeeded {
            return Err(AppError::Config(
                "Antigravity could not compact the native conversation.".into(),
            ));
        }
        checkpoint.ok_or_else(|| {
            AppError::Config("Antigravity did not create a compaction checkpoint.".into())
        })
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(AppError::Cancelled),
        result = complete => result,
    };
    if result.is_err() {
        let _ = child.terminate(Duration::from_millis(500)).await;
    }
    result
}
