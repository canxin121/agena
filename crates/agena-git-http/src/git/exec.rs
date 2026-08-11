use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use tokio::process::Command;
use tokio::sync::Mutex;

const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_GIT_STREAM_BYTES: usize = 16 * 1024 * 1024;

async fn read_git_stream<R>(mut reader: R) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;

    let mut retained = Vec::new();
    let mut truncated = false;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let remaining = MAX_GIT_STREAM_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
        truncated |= read > remaining;
    }
    (retained, truncated)
}

async fn join_git_streams(
    stdout_task: tokio::task::JoinHandle<(Vec<u8>, bool)>,
    stderr_task: tokio::task::JoinHandle<(Vec<u8>, bool)>,
) -> ((Vec<u8>, bool), (Vec<u8>, bool)) {
    let stdout_abort = stdout_task.abort_handle();
    let stderr_abort = stderr_task.abort_handle();
    match tokio::time::timeout(Duration::from_secs(2), async move {
        let (stdout, stderr) = tokio::join!(stdout_task, stderr_task);
        (stdout.unwrap_or_default(), stderr.unwrap_or_default())
    })
    .await
    {
        Ok(output) => output,
        Err(_) => {
            stdout_abort.abort();
            stderr_abort.abort();
            ((Vec::new(), true), (Vec::new(), true))
        }
    }
}

fn git_stream_text(bytes: Vec<u8>, truncated: bool) -> String {
    let mut text = String::from_utf8_lossy(&bytes).to_string();
    if truncated {
        text.push_str("\n... git output truncated at 16 MiB\n");
    }
    text
}

fn parse_git_subcommand<'a>(args: &'a [&'a str]) -> Option<&'a str> {
    let mut i = 0usize;
    while i < args.len() {
        let token = args[i].trim();
        if token.is_empty() {
            i += 1;
            continue;
        }
        // Skip common global options before the subcommand.
        if token == "-c"
            || token == "-C"
            || token == "--git-dir"
            || token == "--work-tree"
            || token == "--namespace"
            || token == "--super-prefix"
            || token == "--config-env"
        {
            i += 2;
            continue;
        }
        if token.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(token);
    }
    None
}

fn operation_from_args(args: &[&str]) -> Option<&'static str> {
    let subcommand = parse_git_subcommand(args)?;
    match subcommand {
        "commit" => Some("commit"),
        "push" => Some("push"),
        "pull" => Some("pull"),
        "fetch" => Some("fetch"),
        "rebase" => Some("rebase"),
        "merge" => Some("merge"),
        "cherry-pick" => Some("cherry-pick"),
        "revert" => Some("revert"),
        "add" => Some("stage"),
        "apply" => {
            let reverse = args.contains(&"--reverse");
            let cached = args.contains(&"--cached");
            if reverse && cached {
                Some("unstage")
            } else if reverse {
                Some("discard")
            } else if cached {
                Some("stage")
            } else {
                Some("patch")
            }
        }
        _ => None,
    }
}

fn emit_git_diagnostics(args: &[&str], code: i32, stdout: &str, stderr: &str, elapsed: Duration) {
    let Some(operation) = operation_from_args(args) else {
        return;
    };

    let latency_ms = elapsed.as_secs_f64() * 1000.0;
    if code == 0 {
        tracing::info!(
            target: "agena.git.metrics",
            git_operation = operation,
            git_success = true,
            git_exit_code = code,
            git_latency_ms = latency_ms,
            "git operation finished"
        );
        return;
    }

    let classification = super::utils::classify_git_failure(code, stdout, stderr);
    let error_code = classification.map(|it| it.code).unwrap_or("git_failed");
    let error_category = classification.map(|it| it.category).unwrap_or("unknown");
    let retryable = classification.map(|it| it.retryable).unwrap_or(false);

    tracing::warn!(
        target: "agena.git.metrics",
        git_operation = operation,
        git_success = false,
        git_exit_code = code,
        git_latency_ms = latency_ms,
        git_error_code = error_code,
        git_error_category = error_category,
        git_retryable = retryable,
        "git operation failed"
    );
}

fn git_timeout() -> Duration {
    if let Ok(v) = std::env::var("AGENA_GIT_TIMEOUT_MS")
        && let Ok(ms) = v.trim().parse::<u64>()
        && ms > 0
    {
        return Duration::from_millis(ms);
    }
    DEFAULT_GIT_TIMEOUT
}

// VS Code queues git operations per repository. Do the same server-side so we don't
// race on the index/worktree (and to reduce index.lock errors under rapid UI clicks).
static REPO_LOCKS: OnceLock<DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();

pub(crate) type RepoLockGuard = tokio::sync::OwnedMutexGuard<()>;

fn repo_lock_key(dir: &Path) -> String {
    dir.to_string_lossy().to_string()
}

pub(crate) async fn lock_repo(dir: &Path) -> Result<RepoLockGuard, Response> {
    let key = repo_lock_key(dir);
    let locks = REPO_LOCKS.get_or_init(DashMap::new);
    let m = if let Some(v) = locks.get(&key) {
        v.value().clone()
    } else {
        let v = Arc::new(Mutex::new(()));
        locks.insert(key.clone(), v.clone());
        v
    };

    match tokio::time::timeout(Duration::from_secs(10), m.clone().lock_owned()).await {
        Ok(g) => Ok(g),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Repository is busy running another git operation",
                "code": "git_busy",
                "hint": "Wait for the current operation to finish, then retry.",
            })),
        )
            .into_response()),
    }
}

pub(crate) fn git_success_response() -> Response {
    Json(serde_json::json!({"success": true})).into_response()
}

fn git_command_error_response_with_status(
    status: StatusCode,
    error: &str,
    code: Option<&str>,
) -> Response {
    match code {
        Some(code) => (
            status,
            Json(serde_json::json!({"error": error.trim(), "code": code})),
        )
            .into_response(),
        None => (status, Json(serde_json::json!({"error": error.trim()}))).into_response(),
    }
}

pub(crate) async fn run_git_checked_with_status(
    directory: &Path,
    args: &[&str],
    failure_status: StatusCode,
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (code, out, err) =
        run_git(directory, args)
            .await
            .unwrap_or((1, String::new(), String::new()));
    if code == 0 {
        return Ok((out, err));
    }
    if let Some(resp) = super::map_git_failure(code, &out, &err) {
        return Err(resp);
    }
    Err(git_command_error_response_with_status(
        failure_status,
        &err,
        failure_code,
    ))
}

pub(crate) async fn run_git_checked(
    directory: &Path,
    args: &[&str],
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    run_git_checked_with_status(
        directory,
        args,
        StatusCode::INTERNAL_SERVER_ERROR,
        failure_code,
    )
    .await
}

pub(crate) async fn run_git_env_checked_with_status(
    directory: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
    failure_status: StatusCode,
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (code, out, err) =
        run_git_env(directory, args, extra_env)
            .await
            .unwrap_or((1, String::new(), String::new()));
    if code == 0 {
        return Ok((out, err));
    }
    if let Some(resp) = super::map_git_failure(code, &out, &err) {
        return Err(resp);
    }
    Err(git_command_error_response_with_status(
        failure_status,
        &err,
        failure_code,
    ))
}

pub(crate) async fn run_git_env_checked(
    directory: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    run_git_env_checked_with_status(
        directory,
        args,
        extra_env,
        StatusCode::INTERNAL_SERVER_ERROR,
        failure_code,
    )
    .await
}

pub(crate) async fn run_locked_git_checked(
    q: &super::DirectoryQuery,
    args: &[&str],
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (dir, _guard) = super::require_locked_directory(q).await?;
    run_git_checked(&dir, args, failure_code).await
}

pub(crate) async fn run_locked_git_checked_with_status(
    q: &super::DirectoryQuery,
    args: &[&str],
    failure_status: StatusCode,
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (dir, _guard) = super::require_locked_directory(q).await?;
    run_git_checked_with_status(&dir, args, failure_status, failure_code).await
}

pub(crate) async fn run_locked_git_env_checked(
    q: &super::DirectoryQuery,
    args: &[&str],
    extra_env: &[(&str, &str)],
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (dir, _guard) = super::require_locked_directory(q).await?;
    run_git_env_checked(&dir, args, extra_env, failure_code).await
}

pub(crate) async fn run_locked_git_env_checked_with_status(
    q: &super::DirectoryQuery,
    args: &[&str],
    extra_env: &[(&str, &str)],
    failure_status: StatusCode,
    failure_code: Option<&str>,
) -> Result<(String, String), Response> {
    let (dir, _guard) = super::require_locked_directory(q).await?;
    run_git_env_checked_with_status(&dir, args, extra_env, failure_status, failure_code).await
}

pub(crate) async fn run_git_env(
    directory: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<(i32, String, String), String> {
    let started_at = Instant::now();

    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(directory)
        // Prevent hanging on interactive credential prompts.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        // Prevent spawning an interactive editor in server mode.
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        // Ensure that non-interactive git operations never steal the server's TTY
        // (e.g. pinentry-tty printing a passphrase prompt in the server console).
        .env("GPG_TTY", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let mut child = agena_process::spawn(cmd).map_err(|e| e.to_string())?;

    let mut stdout = child.stdout().take();
    let mut stderr = child.stderr().take();

    let stdout_task = tokio::spawn(async move {
        match stdout.take() {
            Some(stream) => read_git_stream(stream).await,
            None => (Vec::new(), false),
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr.take() {
            Some(stream) => read_git_stream(stream).await,
            None => (Vec::new(), false),
        }
    });

    let timeout = git_timeout();
    let mut timed_out = false;
    let status = tokio::select! {
        status = child.wait() => status,
        _ = tokio::time::sleep(timeout) => {
            timed_out = true;
            child.terminate(Duration::from_millis(150)).await
        }
    };

    let _ = child.start_kill();

    let ((stdout_bytes, stdout_truncated), (stderr_bytes, stderr_truncated)) =
        join_git_streams(stdout_task, stderr_task).await;
    let stdout_text = git_stream_text(stdout_bytes, stdout_truncated);
    let mut stderr_text = git_stream_text(stderr_bytes, stderr_truncated);

    let mut code = status.ok().and_then(|s| s.code()).unwrap_or(1);
    if timed_out {
        // Use a conventional timeout exit code so callers can classify.
        code = 124;
        let prefix = format!("git command timed out after {}ms\n", timeout.as_millis());
        if stderr_text.trim().is_empty() {
            stderr_text = prefix;
        } else {
            stderr_text = format!("{}{}", prefix, stderr_text);
        }
    }

    emit_git_diagnostics(args, code, &stdout_text, &stderr_text, started_at.elapsed());
    Ok((code, stdout_text, stderr_text))
}

pub(crate) async fn run_git_input(
    directory: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
    input: &str,
) -> Result<(i32, String, String), String> {
    use tokio::io::AsyncWriteExt;
    let started_at = Instant::now();

    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(directory)
        // Prevent hanging on interactive credential prompts.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        // Prevent spawning an interactive editor in server mode.
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        // Ensure that non-interactive git operations never steal the server's TTY
        // (e.g. pinentry-tty printing a passphrase prompt in the server console).
        .env("GPG_TTY", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let mut child = agena_process::spawn(cmd).map_err(|e| e.to_string())?;

    // Drain both output pipes while stdin is written. Writing first can
    // deadlock when git emits enough diagnostics to fill stdout/stderr before
    // it has consumed the complete patch from stdin.
    let input = input.as_bytes().to_vec();
    let stdin_task = child.stdin().take().map(|mut stdin| {
        tokio::spawn(async move {
            let result = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
            result
        })
    });

    let mut stdout = child.stdout().take();
    let mut stderr = child.stderr().take();

    let stdout_task = tokio::spawn(async move {
        match stdout.take() {
            Some(stream) => read_git_stream(stream).await,
            None => (Vec::new(), false),
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr.take() {
            Some(stream) => read_git_stream(stream).await,
            None => (Vec::new(), false),
        }
    });

    let timeout = git_timeout();
    let mut timed_out = false;
    let status = tokio::select! {
        status = child.wait() => status,
        _ = tokio::time::sleep(timeout) => {
            timed_out = true;
            child.terminate(Duration::from_millis(150)).await
        }
    };

    let _ = child.start_kill();

    if let Some(stdin_task) = stdin_task {
        let stdin_abort = stdin_task.abort_handle();
        if tokio::time::timeout(Duration::from_secs(2), stdin_task)
            .await
            .is_err()
        {
            stdin_abort.abort();
        }
    }

    let ((stdout_bytes, stdout_truncated), (stderr_bytes, stderr_truncated)) =
        join_git_streams(stdout_task, stderr_task).await;
    let stdout_text = git_stream_text(stdout_bytes, stdout_truncated);
    let mut stderr_text = git_stream_text(stderr_bytes, stderr_truncated);

    let mut code = status.ok().and_then(|s| s.code()).unwrap_or(1);
    if timed_out {
        code = 124;
        let prefix = format!("git command timed out after {}ms\n", timeout.as_millis());
        if stderr_text.trim().is_empty() {
            stderr_text = prefix;
        } else {
            stderr_text = format!("{}{}", prefix, stderr_text);
        }
    }

    emit_git_diagnostics(args, code, &stdout_text, &stderr_text, started_at.elapsed());
    Ok((code, stdout_text, stderr_text))
}

pub(crate) async fn run_git_with_input(
    directory: &Path,
    args: &[&str],
    input: &str,
) -> Result<(i32, String, String), String> {
    run_git_input(directory, args, &[], input).await
}

pub(crate) async fn run_git(
    directory: &Path,
    args: &[&str],
) -> Result<(i32, String, String), String> {
    run_git_env(directory, args, &[]).await
}

#[cfg(test)]
mod tests {
    use super::{MAX_GIT_STREAM_BYTES, read_git_stream, run_git, run_git_with_input};
    use tokio::io::AsyncWriteExt as _;

    #[tokio::test]
    async fn stream_reader_keeps_draining_after_retained_output_limit() {
        let (reader, mut writer) = tokio::io::duplex(64 * 1024);
        let writer_task = tokio::spawn(async move {
            let chunk = vec![b'x'; 64 * 1024];
            let mut remaining = MAX_GIT_STREAM_BYTES + chunk.len();
            while remaining > 0 {
                let write = remaining.min(chunk.len());
                writer
                    .write_all(&chunk[..write])
                    .await
                    .expect("write git stream fixture");
                remaining -= write;
            }
        });

        let (retained, truncated) = read_git_stream(reader).await;
        writer_task.await.expect("writer task");

        assert_eq!(retained.len(), MAX_GIT_STREAM_BYTES);
        assert!(truncated);
    }

    #[tokio::test]
    async fn git_patch_stdin_is_written_while_output_pipes_are_drained() {
        let workspace = tempfile::tempdir().expect("git workspace");
        let (code, _, stderr) = run_git(workspace.path(), &["init", "--quiet"])
            .await
            .expect("git init");
        assert_eq!(code, 0, "git init: {stderr}");
        std::fs::write(workspace.path().join("demo.txt"), "one\n").expect("write fixture");
        let patch = "--- a/demo.txt\n+++ b/demo.txt\n@@ -1 +1 @@\n-one\n+two\n";

        let (code, _, stderr) = run_git_with_input(workspace.path(), &["apply", "-"], patch)
            .await
            .expect("git apply");

        assert_eq!(code, 0, "git apply: {stderr}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("demo.txt")).expect("read result"),
            "two\n"
        );
    }
}
