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

use super::{AppError, MAX_FRAME_BYTES, agent_environment, isolation, policy};

pub(super) async fn compact(
    cwd: &Path,
    additional_directories: &[PathBuf],
    native_id: &str,
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
    #[cfg(unix)]
    std::os::unix::fs::symlink(&profile, &config)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&profile, &config)?;

    let (program, _) = ExternalAgent::Antigravity.command();
    let mut command = Command::new(program);
    agent_environment(&mut command);
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
        false,
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
        while let Some(line) = output.next().await {
            let line = line.map_err(|_| {
                AppError::Config("Antigravity returned an invalid compaction stream.".into())
            })?;
            let event: Value = serde_json::from_str(&line)?;
            match event["event"].as_str() {
                Some("init") if event["conversation_id"].as_str() != Some(native_id) => {
                    return Err(AppError::Config(
                        "Antigravity could not resume the conversation for compaction.".into(),
                    ));
                }
                Some("step_update") => {
                    let step = &event["step_update"];
                    if step["conversation_id"].as_str() == Some(native_id)
                        && step["step_type"] == "checkpoint"
                        && step["state"] == "DONE"
                    {
                        checkpoint = step["step_index"].as_u64();
                    }
                }
                Some("result") => {
                    let result = &event["result"];
                    succeeded = result["conversation_id"].as_str() == Some(native_id)
                        && result["status"] == "SUCCESS";
                }
                _ => {}
            }
        }
        if !child.wait().await?.success() || !succeeded {
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
