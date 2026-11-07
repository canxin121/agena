//! Terminal workbench coverage using actual PTY subprocesses, authenticated
//! HTTP operations and replayed production SSE, not a mock terminal service.

use super::fixture;
use agena_testkit::TestResult;
use anyhow::ensure;
use reqwest::Method;
use serde_json::{Value, json};

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_workbench_pty_input_sse_resize_stop_start_and_restart() -> TestResult {
    fixture().env("SHELL", "/bin/sh").run(|case| async move {
        let terminal = case.terminal().create(case.workspace()).await?;
        let info = terminal.info().await?;
        ensure!(info["sessionId"] == terminal.id() && info["running"] == true, "{info}");
        ensure!(info["cwd"] == case.workspace().to_string_lossy().as_ref());

        let dimensions = terminal.resize(112, 31).await?;
        ensure!(dimensions["success"] == true && dimensions["cols"] == 112
            && dimensions["rows"] == 31, "{dimensions}");

        // PTY input is sent as real keystrokes. A filesystem side effect proves
        // the shell executed the command rather than just echoing input bytes.
        let checkpoint = terminal.checkpoint().await?;
        // The expected output must not occur literally in the input:
        // otherwise the PTY's echoed command could satisfy the assertion.
        terminal.input(b"printf 'from-real-pty' > terminal-proof.txt; printf 'shell-%s\\n' 'finished-marker'\n".to_vec())
            .await?;
        let transcript = terminal.wait_output_after(&checkpoint, "shell-finished-marker").await?;
        ensure!(transcript.contains("shell-finished-marker"), "{transcript:?}");
        let later = terminal.checkpoint().await?;
        ensure!(
            tokio::time::timeout(
                std::time::Duration::from_millis(250),
                terminal.wait_output_after(&later, "shell-finished-marker")
            ).await.is_err(),
            "old terminal SSE output was replayed as newly produced stdout"
        );
        let proof_path = case.wait_file_bytes(
            "workspace/terminal-proof.txt", b"from-real-pty"
        ).await?;
        let proof = tokio::fs::read(proof_path).await?;
        ensure!(proof == b"from-real-pty", "PTY input was echoed but not executed");

        // Multiple rapid stop/restore cycles exercise stale PTY exit tasks.
        // They must not erase the newly restored instance sharing this ID.
        for _ in 0..3 {
            terminal.stop().await?;
            let stopped = terminal.info().await?;
            ensure!(stopped["running"] == false, "stop did not persist stopped state: {stopped}");
            let resumed = terminal.start().await?;
            ensure!(resumed["sessionId"] == terminal.id() && resumed["running"] == true,
                "terminal resume failed: {resumed}");
        }
        terminal.input(b"printf 'after-restore' > terminal-after-restore.txt\n".to_vec()).await?;
        case.wait_file_bytes("workspace/terminal-after-restore.txt", b"after-restore").await?;
        let next = terminal.restart(case.workspace()).await?;
        ensure!(next.id() != terminal.id());
        ensure!(
            next.wait_output_after(&checkpoint, "shell-finished-marker").await.is_err(),
            "checkpoint from the old PTY session was accepted by a new session"
        );
        let old = case.http().await.get(&format!("/api/v1/workbench/terminal/{}", terminal.id())).await?;
        ensure!(old.status.as_u16() == 404, "restarted terminal retained old session: {old:?}");
        next.delete().await?;
        let deleted = case.http().await.get(&format!("/api/v1/workbench/terminal/{}", next.id())).await?;
        ensure!(deleted.status.as_u16() == 404, "deleted PTY remained visible: {deleted:?}");
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_workbench_rejects_invalid_directories_resize_and_oversized_input() -> TestResult {
    fixture()
        .run(|case| async move {
            let http = case.http().await;
            let unauthenticated = http
                .anonymous()
                .send_json(
                    Method::POST,
                    "/api/v1/workbench/terminal/create",
                    &json!({"cwd": case.workspace()}),
                )
                .await?;
            ensure!(
                unauthenticated.status.as_u16() == 401,
                "terminal creation bypassed auth: {unauthenticated:?}"
            );

            let missing = http
                .send_json(
                    Method::POST,
                    "/api/v1/workbench/terminal/create",
                    &json!({}),
                )
                .await?;
            ensure!(
                missing.status.as_u16() == 400,
                "terminal creation accepted missing cwd: {missing:?}"
            );
            let not_directory = case
                .seed_file("workspace/not-a-directory.txt", b"not a directory")
                .await?;
            let rejected = http
                .send_json(
                    Method::POST,
                    "/api/v1/workbench/terminal/create",
                    &json!({"cwd": not_directory}),
                )
                .await?;
            ensure!(
                rejected.status.as_u16() == 400,
                "terminal created inside regular file: {rejected:?}"
            );
            ensure!(
                case.terminal()
                    .create(&case.paths().root.join("not-present"))
                    .await
                    .is_err()
            );

            let terminal = case.terminal().create(case.workspace()).await?;
            let endpoint = format!("/api/v1/workbench/terminal/{}", terminal.id());
            let incomplete = http
                .send_json(
                    Method::POST,
                    &format!("{endpoint}/resize"),
                    &json!({"cols": 88}),
                )
                .await?;
            ensure!(
                incomplete.status.as_u16() == 400,
                "terminal resize accepted missing rows: {incomplete:?}"
            );

            let oversized = http
                .send_bytes(
                    Method::POST,
                    &format!("{endpoint}/input"),
                    Some("application/octet-stream"),
                    vec![b'X'; 1024 * 1024 + 1],
                )
                .await?;
            ensure!(
                oversized.status.as_u16() == 413,
                "PTY input size cap was not enforced: {oversized:?}"
            );
            let info: Value = terminal.info().await?;
            ensure!(
                info["running"] == true,
                "oversized input killed the legitimate PTY: {info}"
            );
            terminal.delete().await?;
            Ok(())
        })
        .await
}
