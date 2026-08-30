use std::{
    fs::{self, OpenOptions},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use agena_api::resource::{ServerEndpointRecord, ServerIdentityResource};
use agena_cli::ServerArgs;
use agena_client::AgenaClient;
use anyhow::{Context, Result, anyhow, bail};

use super::server_record;

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn remove_record_if_matches(record: &ServerEndpointRecord) -> Result<()> {
    let path = server_record::record_path();
    let current = match server_record::read_record(path.as_path()) {
        Ok(current) => current,
        Err(error) if is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if current.server_id != record.server_id || current.pid != record.pid {
        return Ok(());
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove stopped server record {}", path.display())),
    }
}

fn read_optional_server_record() -> Result<Option<ServerEndpointRecord>> {
    let path = server_record::record_path();
    match server_record::read_record(&path) {
        Ok(record) => Ok(Some(record)),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect the existing Agena server record at {}",
                path.display()
            )
        }),
    }
}

pub(crate) async fn start(mut args: ServerArgs) -> Result<()> {
    args.action = None;
    if super::user_service::is_installed() {
        if !args.overrides.is_empty()
            || args.database_url.is_some()
            || args.database_path.is_some()
            || args.workspace_root.is_some()
            || args.ui_password.is_some()
            || args.mcp_enabled.is_some()
            || args.mcp_public_url.is_some()
            || args.mcp_oauth_issuer_url.is_some()
            || args.mcp_auth_mode.is_some()
            || args.mcp_anonymous_access.is_some()
            || args.mcp_client_registration.is_some()
            || args.ui_dir.is_some()
        {
            bail!(
                "the Agena user service is already installed; run `agena server install` with the new options to update its definition"
            );
        }
        let record_path = server_record::record_path();
        match server_record::read_record(record_path.as_path()) {
            Ok(record) => match AgenaClient::new(record.url.as_str()) {
                Ok(client) => match client.server_identity().await {
                    Ok(identity) => match ensure_record_matches(&record, &identity) {
                        Ok(()) => {
                            println!(
                                "Installed Agena server is already running at {} (pid {}, id {}).",
                                record.url, identity.pid, identity.id
                            );
                            return Ok(());
                        }
                        Err(error) => tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                            "installed server record did not match the live endpoint; starting the service"
                        ),
                    },
                    Err(error) => tracing::debug!(
                        diagnostic = %error.operator_diagnostic(),
                        "installed server record endpoint is not currently ready"
                    ),
                },
                Err(error) => tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "installed server record URL is invalid; starting the service"
                ),
            },
            Err(error) if is_not_found(&error) => {}
            Err(error) => tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(error.as_ref()),
                "installed server record could not be read; starting the service and waiting for a replacement"
            ),
        }
        super::user_service::start()?;
        let (record, identity) = wait_for_installed_service().await?;
        println!(
            "Started installed Agena server at {} (pid {}, id {}).",
            record.url, identity.pid, identity.id
        );
        return Ok(());
    }
    let intended_url = intended_url(&args)?;
    match AgenaClient::new(intended_url.as_str())?
        .server_identity()
        .await
    {
        Ok(identity) => {
            println!(
                "Agena server is already running at {intended_url} (pid {}, id {}).",
                identity.pid, identity.id
            );
            return Ok(());
        }
        Err(error) => tracing::debug!(
            diagnostic = %error.operator_diagnostic(),
            "no detached Agena server identity was available before start"
        ),
    }

    let executable = std::env::current_exe().context("failed to resolve the Agena executable")?;
    let record_path = server_record::record_path();
    let state_dir = record_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let log_dir = state_dir.join("logs");
    fs::create_dir_all(&log_dir).with_context(|| {
        format!(
            "failed to create server log directory {}",
            log_dir.display()
        )
    })?;
    let log_path = log_dir.join("server.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open server log {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .context("failed to clone the server log handle")?;

    let mut command = Command::new(executable);
    for expression in &args.overrides {
        command.arg("--set").arg(expression);
    }
    if let Some(database_url) = &args.database_url {
        command.arg("--database-url").arg(database_url);
    }
    if let Some(database_path) = &args.database_path {
        command.arg("--database-path").arg(database_path);
    }
    command
        .arg("server")
        .arg("--host")
        .arg(&args.host)
        .arg("--port")
        .arg(args.port.to_string());
    if let Some(workspace_root) = &args.workspace_root {
        command.arg("--workspace").arg(workspace_root);
    }
    if let Some(ui_dir) = &args.ui_dir {
        command.arg("--ui-dir").arg(ui_dir);
    }
    if let Some(enabled) = args.mcp_enabled {
        command.arg("--mcp-enabled").arg(enabled.to_string());
    }
    if let Some(mcp_public_url) = &args.mcp_public_url {
        command.arg("--mcp-public-url").arg(mcp_public_url);
    }
    if let Some(mcp_oauth_issuer_url) = &args.mcp_oauth_issuer_url {
        command
            .arg("--mcp-oauth-issuer-url")
            .arg(mcp_oauth_issuer_url);
    }
    if let Some(mode) = args.mcp_auth_mode {
        command.arg("--mcp-auth-mode").arg(mode.as_str());
    }
    if let Some(access) = args.mcp_anonymous_access {
        command.arg("--mcp-anonymous-access").arg(access.as_str());
    }
    if let Some(registration) = args.mcp_client_registration {
        command
            .arg("--mcp-client-registration")
            .arg(registration.as_str());
    }
    if let Some(password) = &args.ui_password {
        command.env("AGENA_SERVER_UI_PASSWORD", password);
    }
    command
        .env("AGENA_SERVER_RECORD", &record_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        // `server start` must outlive the invoking PowerShell/terminal. Without
        // Windows creation flags the child remains attached to the caller's
        // console/process group, which can keep CI shells and real installer
        // invocations open even though Agena redirected its stdio to a log.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let child = command.spawn().context("failed to spawn the server")?;
    let child_pid = child.id();
    drop(child);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(identity) = AgenaClient::new(intended_url.as_str())?
            .server_identity()
            .await
            && identity.pid == child_pid
        {
            let record = server_record::read_record(&record_path)?;
            ensure_record_matches(&record, &identity)?;
            println!(
                "Started Agena server at {} (pid {}, id {}). Logs: {}",
                record.url,
                identity.pid,
                identity.id,
                log_path.display()
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "server pid {child_pid} did not become ready at {intended_url}; inspect {}",
                log_path.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn status() -> Result<()> {
    let path = server_record::record_path();
    let record = match server_record::read_record(&path) {
        Ok(record) => record,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            if super::user_service::is_installed() {
                println!(
                    "Agena server user service is installed but not running (no record at {}).",
                    path.display()
                );
            } else {
                println!(
                    "Agena server is not running (no record at {}).",
                    path.display()
                );
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let identity = AgenaClient::new(record.url.as_str())?
        .server_identity()
        .await
        .with_context(|| format!("server record at {} is stale", path.display()))?;
    ensure_record_matches(&record, &identity)?;
    println!(
        "Agena server is running at {} (pid {}, id {}, started {}).",
        record.url, record.pid, record.server_id, record.started_at
    );
    Ok(())
}

pub(crate) async fn stop() -> Result<()> {
    if super::user_service::is_installed() {
        let record = read_optional_server_record();
        if let Err(primary) = super::user_service::stop() {
            if let Err(secondary) = record {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(secondary.as_ref()),
                    "server record inspection also failed before the user service stop failed"
                );
            }
            return Err(primary);
        }
        let record = record?;
        if let Some(record) = record {
            wait_until_identity_stops(&record).await?;
            remove_record_if_matches(&record)?;
            println!(
                "Stopped installed Agena server pid {} (id {}).",
                record.pid, record.server_id
            );
        } else {
            println!("Stopped installed Agena server user service.");
        }
        return Ok(());
    }
    let path = server_record::record_path();
    let record = server_record::read_record(&path)?;
    let identity = AgenaClient::new(record.url.as_str())?
        .server_identity()
        .await
        .with_context(|| format!("refusing to stop stale server record {}", path.display()))?;
    ensure_record_matches(&record, &identity)?;
    send_interrupt(identity.pid)?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let still_same = match AgenaClient::new(record.url.as_str())?
            .server_identity()
            .await
        {
            Ok(current) => current.id == identity.id && current.pid == identity.pid,
            Err(error) => {
                tracing::debug!(
                    diagnostic = %error.operator_diagnostic(),
                    "server identity endpoint became unavailable after SIGINT"
                );
                false
            }
        };
        if !still_same {
            remove_record_if_matches(&record)?;
            println!(
                "Stopped Agena server pid {} (id {}).",
                identity.pid, identity.id
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("server pid {} did not stop after SIGINT", identity.pid);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn install(mut args: ServerArgs) -> Result<()> {
    args.action = None;
    let intended_url = intended_url(&args)?;
    if !super::user_service::is_installed() {
        match AgenaClient::new(intended_url.as_str())?
            .server_identity()
            .await
        {
            Ok(identity) => bail!(
                "a detached server is already running at {intended_url} (pid {}, id {}); stop it before installing the user service",
                identity.pid,
                identity.id
            ),
            Err(error) => tracing::debug!(
                diagnostic = %error.operator_diagnostic(),
                "no detached Agena server identity was available before service installation"
            ),
        }
    }
    let path = super::user_service::install(&args)?;
    let (record, identity) = wait_for_installed_service().await?;
    anyhow::ensure!(
        record.url == intended_url,
        "installed service published {}, expected {intended_url}",
        record.url
    );
    println!(
        "Installed and started Agena user service at {} (pid {}, id {}). Definition: {}",
        record.url,
        identity.pid,
        identity.id,
        path.display()
    );
    Ok(())
}

pub(crate) async fn uninstall() -> Result<()> {
    let record = read_optional_server_record();
    let path = match super::user_service::uninstall() {
        Ok(path) => path,
        Err(primary) => {
            if let Err(secondary) = record {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(secondary.as_ref()),
                    "server record inspection also failed before user service uninstall failed"
                );
            }
            return Err(primary);
        }
    };
    let record = record?;
    if let Some(record) = record {
        wait_until_identity_stops(&record).await?;
        remove_record_if_matches(&record)?;
    }
    println!(
        "Uninstalled Agena user service definition {}. Agena configuration, sessions, and logs were preserved.",
        path.display()
    );
    Ok(())
}

async fn wait_for_installed_service() -> Result<(ServerEndpointRecord, ServerIdentityResource)> {
    let path = server_record::record_path();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let last_diagnostic = match server_record::read_record(&path) {
            Ok(record) => match AgenaClient::new(record.url.as_str()) {
                Ok(client) => match client.server_identity().await {
                    Ok(identity) => match ensure_record_matches(&record, &identity) {
                        Ok(()) => return Ok((record, identity)),
                        Err(error) => agena_failure::diagnostic::format_error_chain(error.as_ref()),
                    },
                    Err(error) => error.operator_diagnostic(),
                },
                Err(error) => agena_failure::diagnostic::format_error_chain(&error),
            },
            Err(error) => agena_failure::diagnostic::format_error_chain(error.as_ref()),
        };
        if Instant::now() >= deadline {
            bail!(
                "installed server did not become ready; last readiness failure: {last_diagnostic}; inspect the user service and {}",
                path.display(),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_until_identity_stops(record: &ServerEndpointRecord) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let still_same = match AgenaClient::new(record.url.as_str())?
            .server_identity()
            .await
        {
            Ok(current) => current.id == record.server_id && current.pid == record.pid,
            Err(error) => {
                tracing::debug!(
                    diagnostic = %error.operator_diagnostic(),
                    "installed server identity endpoint became unavailable after service stop"
                );
                false
            }
        };
        if !still_same {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "server pid {} did not stop after the user service was stopped",
                record.pid
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn ensure_record_matches(
    record: &ServerEndpointRecord,
    identity: &ServerIdentityResource,
) -> Result<()> {
    if !record.matches(identity) {
        bail!(
            "server identity mismatch: record has pid {} / id {}, endpoint has pid {} / id {}; refusing lifecycle action",
            record.pid,
            record.server_id,
            identity.pid,
            identity.id
        );
    }
    Ok(())
}

fn intended_url(args: &ServerArgs) -> Result<String> {
    let ip = args
        .host
        .parse::<std::net::IpAddr>()
        .with_context(|| format!("invalid server host {}", args.host))?;
    let advertised = if ip.is_unspecified() {
        if ip.is_ipv6() {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
    } else {
        ip
    };
    Ok(format!(
        "http://{}",
        std::net::SocketAddr::new(advertised, args.port)
    ))
}

#[cfg(unix)]
fn send_interrupt(pid: u32) -> Result<()> {
    let pid = i32::try_from(pid).map_err(|_| anyhow!("server pid {pid} is out of range"))?;
    // SAFETY: `kill` is called with a positive, identity-validated PID and a
    // non-destructive SIGINT so the server can run its graceful shutdown path.
    let result = unsafe { libc::kill(pid, libc::SIGINT) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context("failed to signal the server")
    }
}

#[cfg(target_os = "windows")]
fn send_interrupt(pid: u32) -> Result<()> {
    let pid = pid.to_string();
    let status = Command::new("taskkill.exe")
        .args(["/PID", pid.as_str(), "/T", "/F"])
        .status()
        .context("failed to execute taskkill")?;
    if status.success() {
        Ok(())
    } else {
        bail!("taskkill failed with status {status}")
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
fn send_interrupt(_pid: u32) -> Result<()> {
    bail!("`agena server stop` is not implemented on this platform")
}
