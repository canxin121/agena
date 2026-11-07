//! Real Agena CLI commands run under the same isolated environment as the
//! system-test server. Useful for plugin scaffolding, validation and packaging
//! which intentionally do not require an AI model request.

use std::{
    ffi::OsString,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use tokio::process::Command;

use crate::TestCase;

#[derive(Debug)]
pub struct CliOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    pub fn require_success(&self) -> Result<&Self> {
        ensure!(
            self.status.success(),
            "Agena CLI exited as {}:\nstdout: {}\nstderr: {}",
            self.status,
            self.stdout,
            self.stderr
        );
        Ok(self)
    }

    pub fn require_failure(&self) -> Result<&Self> {
        ensure!(
            !self.status.success(),
            "Agena CLI unexpectedly succeeded:\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        Ok(self)
    }
}

impl TestCase {
    /// Execute the *real* product executable as a separate managed process.
    /// No test runner is introduced, and no user home, credentials or current
    /// directory are inherited. CLI stderr and stdout are bounded to 256 KiB.
    /// The returned nonzero exit status is inspectable for negative scenarios.
    pub async fn run_cli<I, S>(&self, args: I) -> Result<CliOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.run_cli_with_timeout(args, self.timeout()).await
    }

    pub async fn run_cli_with_timeout<I, S>(&self, args: I, timeout: Duration) -> Result<CliOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        ensure!(
            !args.iter().any(|arg| {
                let arg = arg.to_string_lossy();
                arg == "--data-dir" || arg.starts_with("--data-dir=")
            }),
            "CLI fixture owns the data directory; custom --data-dir is not allowed"
        );
        ensure!(
            !args.first().is_some_and(|value| {
                matches!(
                    value.to_string_lossy().as_ref(),
                    "server" | "start" | "stop" | "restart"
                )
            }),
            "use TestCase lifecycle methods to control the owned Agena server"
        );
        let spec = self.cli_process_spec().await;
        let mut command = Command::new(&spec.binary);
        command
            .arg("--data-dir")
            .arg(&self.paths().data_dir)
            .args(&args)
            .current_dir(&self.paths().cwd)
            .env_clear()
            .envs(&spec.environment)
            .stdin(Stdio::null());
        let cancel = self.cancellation_token();
        let output = tokio::select! {
            () = cancel.cancelled() => anyhow::bail!("test case ended before Agena CLI command completed"),
            result = agena_process::output(command, timeout, 256 * 1024) => result,
        }.with_context(|| format!("run owned Agena CLI command {:?}",
            args.iter().map(|arg| arg.to_string_lossy().chars().take(120).collect::<String>()).collect::<Vec<_>>()))?;
        Ok(CliOutput {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
