//! Run ViperJS in a managed instance of the Agena executable. The caller
//! retains the scoped tool host and can stop even a long-running JS built-in.

use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    process::Stdio,
    time::Duration,
};

use agena_plugin_sdk::{HostClient, PluginError, Result as SdkResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout, Command},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::{
    CallReceipt, MAX_INTERMEDIATE_BYTES, MAX_REQUESTS, MAX_SOURCE_BYTES, cancelled, encode_error,
    handle_operation,
    vm::{self, Operation},
};

const ARGUMENT: &str = "--agena-tool-script-worker";
const COMPUTE_BUDGET: Duration = Duration::from_secs(2);
const TIMEOUT_MESSAGE: &str = "Tool script JavaScript exceeded its compute budget.";
const REQUEST_LIMIT_MESSAGE: &str = "Tool script reached its limit of 50 tool calls and searches.";

type ProgramResult = Result<(Value, bool), String>;

#[derive(Deserialize, Serialize)]
enum Message {
    Request(Operation),
    Finished(ProgramResult),
}

pub(super) async fn execute(
    host: &dyn HostClient,
    code: String,
    cancellation: &CancellationToken,
    calls: &mut Vec<CallReceipt>,
) -> SdkResult<ProgramResult> {
    let executable = agena_process::sandbox::launcher_executable()
        .map_err(|error| PluginError::internal_error(&error))?;
    let mut command = Command::new(executable);
    command
        .arg(ARGUMENT)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let mut child =
        agena_process::spawn(command).map_err(|error| PluginError::internal_error(&error))?;
    let mut input = child
        .stdin()
        .take()
        .ok_or_else(|| PluginError::internal("Tool script worker stdin is unavailable"))?;
    let mut output = child
        .stdout()
        .take()
        .ok_or_else(|| PluginError::internal("Tool script worker stdout is unavailable"))?;
    let outcome = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(cancelled()),
        result = exchange(host, code, calls, &mut input, &mut output) => Ok(result),
    };
    drop(input);
    drop(output);
    if let Err(error) = child.terminate(Duration::ZERO).await {
        tracing::warn!(%error, "Tool script worker cleanup failed");
    }
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    Ok(match outcome? {
        Ok(result) => result,
        Err(error) => Err(format!("Tool script JavaScript worker stopped: {error}")),
    })
}

async fn exchange(
    host: &dyn HostClient,
    mut reply: String,
    calls: &mut Vec<CallReceipt>,
    input: &mut ChildStdin,
    output: &mut ChildStdout,
) -> io::Result<ProgramResult> {
    let mut remaining_time = COMPUTE_BUDGET;
    let mut remaining_requests = MAX_REQUESTS;
    loop {
        let start = Instant::now();
        let message = tokio::time::timeout(remaining_time, async {
            check_length(reply.len(), MAX_INTERMEDIATE_BYTES)?;
            input.write_u32(reply.len() as u32).await?;
            input.write_all(reply.as_bytes()).await?;
            let length = output.read_u32().await? as usize;
            check_length(length, MAX_INTERMEDIATE_BYTES)?;
            let mut bytes = vec![0; length];
            output.read_exact(&mut bytes).await?;
            serde_json::from_slice(&bytes).map_err(io::Error::other)
        })
        .await;
        remaining_time = remaining_time.saturating_sub(start.elapsed());
        if remaining_time.is_zero() {
            return Ok(Err(TIMEOUT_MESSAGE.to_owned()));
        }
        let message = match message {
            Ok(result) => result?,
            Err(_) => return Ok(Err(TIMEOUT_MESSAGE.to_owned())),
        };
        match message {
            Message::Finished(result) => return Ok(result),
            Message::Request(operation) => {
                if remaining_requests == 0 {
                    return Ok(Err(REQUEST_LIMIT_MESSAGE.to_owned()));
                }
                remaining_requests -= 1;
                // The VM is blocked reading its reply during host I/O and approvals.
                reply = handle_operation(host, operation, calls).await;
            }
        }
    }
}

pub(crate) fn try_run_worker() -> Option<i32> {
    let mut args = std::env::args_os();
    if args.nth(1).as_deref() != Some(OsStr::new(ARGUMENT)) {
        return None;
    }
    if args.next().is_some() {
        return Some(2);
    }
    Some(match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Tool script worker failed: {error}");
            1
        }
    })
}

fn run() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    limit_worker_memory()?;

    let mut input = io::stdin().lock();
    let source = read_string(&mut input, MAX_SOURCE_BYTES)?;
    let mut output = io::stdout().lock();
    let mut remaining_requests = MAX_REQUESTS;
    let result = vm::run(&source, move |operation| {
        if remaining_requests == 0 {
            return Ok(encode_error(
                "tool_script.request_limit",
                REQUEST_LIMIT_MESSAGE,
            ));
        }
        remaining_requests -= 1;
        write_message(&mut output, &Message::Request(operation))?;
        read_string(&mut input, MAX_INTERMEDIATE_BYTES)
    });
    write_message(&mut io::stdout().lock(), &Message::Finished(result))
}

fn check_length(length: usize, limit: usize) -> io::Result<()> {
    if length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Tool script worker frame is too large",
        ));
    }
    Ok(())
}

fn read_string(input: &mut impl Read, limit: usize) -> io::Result<String> {
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    check_length(length, limit)?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(io::Error::other)
}

fn write_message(output: &mut impl Write, message: &Message) -> io::Result<()> {
    let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    check_length(bytes.len(), MAX_INTERMEDIATE_BYTES)?;
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

#[cfg(target_os = "linux")]
fn limit_worker_memory() -> io::Result<()> {
    // ViperJS's heap accounting excludes some native allocations. RLIMIT_DATA
    // also bounds those allocations without counting the executable's text.
    const DATA_LIMIT: libc::rlim_t = 128 * 1024 * 1024;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: Both calls use a live rlimit for this process's data segment.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_DATA, &raw mut limit) != 0 {
            return Err(io::Error::last_os_error());
        }
        limit.rlim_cur = limit.rlim_cur.min(DATA_LIMIT);
        if libc::setrlimit(libc::RLIMIT_DATA, &raw const limit) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
