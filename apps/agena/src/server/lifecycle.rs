use std::{
    fs::{self, OpenOptions},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use agena_api::resource::{CenterEndpointRecord, CenterIdentityResource};
use agena_cli::{ServerArgs, UiCookieSameSite};
use agena_client::AgenaClient;
use anyhow::{Context, Result, anyhow, bail};

use super::center_record;

pub(crate) async fn start(mut args: ServerArgs) -> Result<()> {
    args.action = None;
    if super::user_service::is_installed() {
        if !args.overrides.is_empty()
            || args.database_url.is_some()
            || args.database_path.is_some()
            || args.workspace_root.is_some()
            || args.ui_password.is_some()
        {
            bail!(
                "the Agena user service is already installed; run `agena center install` with the new options to update its definition"
            );
        }
        let record_path = center_record::record_path();
        if let Ok(record) = center_record::read_record(record_path.as_path())
            && let Ok(client) = AgenaClient::new(record.url.as_str())
            && let Ok(identity) = client.center_identity().await
            && ensure_record_matches(&record, &identity).is_ok()
        {
            println!(
                "Installed Agena processing center is already running at {} (pid {}, id {}).",
                record.url, identity.pid, identity.id
            );
            return Ok(());
        }
        super::user_service::start()?;
        let (record, identity) = wait_for_installed_service().await?;
        println!(
            "Started installed Agena processing center at {} (pid {}, id {}).",
            record.url, identity.pid, identity.id
        );
        return Ok(());
    }
    let intended_url = intended_url(&args)?;
    if let Ok(identity) = AgenaClient::new(intended_url.as_str())?
        .center_identity()
        .await
    {
        println!(
            "Agena processing center is already running at {intended_url} (pid {}, id {}).",
            identity.pid, identity.id
        );
        return Ok(());
    }

    let executable = std::env::current_exe().context("failed to resolve the Agena executable")?;
    let record_path = center_record::record_path();
    let state_dir = record_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let log_dir = state_dir.join("logs");
    fs::create_dir_all(&log_dir).with_context(|| {
        format!(
            "failed to create center log directory {}",
            log_dir.display()
        )
    })?;
    let log_path = log_dir.join("center.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open center log {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .context("failed to clone the center log handle")?;

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
        .arg("center")
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
    for origin in &args.cors_origin {
        command.arg("--cors-origin").arg(origin);
    }
    if args.cors_allow_all {
        command.arg("--cors-allow-all");
    }
    command
        .arg("--ui-cookie-samesite")
        .arg(cookie_same_site_name(&args.ui_cookie_samesite));
    if let Some(password) = &args.ui_password {
        command.env("AGENA_SERVER_UI_PASSWORD", password);
    }
    command
        .env("AGENA_CENTER_RECORD", &record_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .context("failed to spawn the processing center")?;
    let child_pid = child.id();
    drop(child);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(identity) = AgenaClient::new(intended_url.as_str())?
            .center_identity()
            .await
            && identity.pid == child_pid
        {
            let record = center_record::read_record(&record_path)?;
            ensure_record_matches(&record, &identity)?;
            println!(
                "Started Agena processing center at {} (pid {}, id {}). Logs: {}",
                record.url,
                identity.pid,
                identity.id,
                log_path.display()
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "processing center pid {child_pid} did not become ready at {intended_url}; inspect {}",
                log_path.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn status() -> Result<()> {
    let path = center_record::record_path();
    let record = match center_record::read_record(&path) {
        Ok(record) => record,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            if super::user_service::is_installed() {
                println!(
                    "Agena processing center user service is installed but not running (no record at {}).",
                    path.display()
                );
            } else {
                println!(
                    "Agena processing center is not running (no record at {}).",
                    path.display()
                );
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let identity = AgenaClient::new(record.url.as_str())?
        .center_identity()
        .await
        .with_context(|| format!("center record at {} is stale", path.display()))?;
    ensure_record_matches(&record, &identity)?;
    println!(
        "Agena processing center is running at {} (pid {}, id {}, started {}).",
        record.url, record.pid, record.center_id, record.started_at
    );
    Ok(())
}

pub(crate) async fn stop() -> Result<()> {
    if super::user_service::is_installed() {
        let record = center_record::read_record(center_record::record_path().as_path()).ok();
        super::user_service::stop()?;
        if let Some(record) = record {
            wait_until_identity_stops(&record).await?;
            println!(
                "Stopped installed Agena processing center pid {} (id {}).",
                record.pid, record.center_id
            );
        } else {
            println!("Stopped installed Agena processing center user service.");
        }
        return Ok(());
    }
    let path = center_record::record_path();
    let record = center_record::read_record(&path)?;
    let identity = AgenaClient::new(record.url.as_str())?
        .center_identity()
        .await
        .with_context(|| format!("refusing to stop stale center record {}", path.display()))?;
    ensure_record_matches(&record, &identity)?;
    send_interrupt(identity.pid)?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let still_same = AgenaClient::new(record.url.as_str())?
            .center_identity()
            .await
            .is_ok_and(|current| current.id == identity.id && current.pid == identity.pid);
        if !still_same {
            println!(
                "Stopped Agena processing center pid {} (id {}).",
                identity.pid, identity.id
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "processing center pid {} did not stop after SIGINT",
                identity.pid
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn install(mut args: ServerArgs) -> Result<()> {
    args.action = None;
    let intended_url = intended_url(&args)?;
    if !super::user_service::is_installed()
        && let Ok(identity) = AgenaClient::new(intended_url.as_str())?
            .center_identity()
            .await
    {
        bail!(
            "a detached processing center is already running at {intended_url} (pid {}, id {}); stop it before installing the user service",
            identity.pid,
            identity.id
        );
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
    let record = center_record::read_record(center_record::record_path().as_path()).ok();
    let path = super::user_service::uninstall()?;
    if let Some(record) = record {
        wait_until_identity_stops(&record).await?;
    }
    println!(
        "Uninstalled Agena user service definition {}. Agena configuration, sessions, and logs were preserved.",
        path.display()
    );
    Ok(())
}

async fn wait_for_installed_service() -> Result<(CenterEndpointRecord, CenterIdentityResource)> {
    let path = center_record::record_path();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(record) = center_record::read_record(&path)
            && let Ok(client) = AgenaClient::new(record.url.as_str())
            && let Ok(identity) = client.center_identity().await
            && ensure_record_matches(&record, &identity).is_ok()
        {
            return Ok((record, identity));
        }
        if Instant::now() >= deadline {
            bail!(
                "installed processing center did not become ready; inspect the user service and {}",
                path.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_until_identity_stops(record: &CenterEndpointRecord) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let still_same = AgenaClient::new(record.url.as_str())?
            .center_identity()
            .await
            .is_ok_and(|current| current.id == record.center_id && current.pid == record.pid);
        if !still_same {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "processing center pid {} did not stop after the user service was stopped",
                record.pid
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn ensure_record_matches(
    record: &CenterEndpointRecord,
    identity: &CenterIdentityResource,
) -> Result<()> {
    if !record.matches(identity) {
        bail!(
            "center identity mismatch: record has pid {} / id {}, endpoint has pid {} / id {}; refusing lifecycle action",
            record.pid,
            record.center_id,
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
        .with_context(|| format!("invalid center host {}", args.host))?;
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

fn cookie_same_site_name(value: &UiCookieSameSite) -> &'static str {
    match value {
        UiCookieSameSite::Auto => "auto",
        UiCookieSameSite::Strict => "strict",
        UiCookieSameSite::Lax => "lax",
        UiCookieSameSite::None => "none",
    }
}

#[cfg(unix)]
fn send_interrupt(pid: u32) -> Result<()> {
    let pid = i32::try_from(pid).map_err(|_| anyhow!("center pid {pid} is out of range"))?;
    // SAFETY: `kill` is called with a positive, identity-validated PID and a
    // non-destructive SIGINT so the center can run its graceful shutdown path.
    let result = unsafe { libc::kill(pid, libc::SIGINT) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context("failed to signal the processing center")
    }
}

#[cfg(not(unix))]
fn send_interrupt(_pid: u32) -> Result<()> {
    bail!("`agena center stop` is not implemented on this platform")
}
