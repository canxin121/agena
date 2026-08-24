//! OS-native user-service installation for the server.
//!
//! launchd and systemd receive an argument vector directly. No generated
//! service definition invokes a shell, and the definition is written with
//! user-only permissions because it may contain the configured UI password.

use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::{BufWriter, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

use agena_cli::ServerArgs;
use anyhow::{Context, Result, bail};

#[cfg(target_os = "macos")]
const SERVICE_LABEL: &str = "com.agena.server";
#[cfg(target_os = "linux")]
const SYSTEMD_UNIT_NAME: &str = "agena-server.service";

pub(crate) fn is_installed() -> bool {
    service_file_path().is_ok_and(|path| path.is_file())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn install(args: &ServerArgs) -> Result<PathBuf> {
    let path = service_file_path()?;
    let executable = std::env::current_exe().context("failed to resolve Agena executable")?;
    let record_path = super::server_record::record_path();
    let state_dir = record_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let log_path = state_dir.join("logs").join("server-service.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create service log directory {}",
                parent.display()
            )
        })?;
    }
    let arguments = server_arguments(args)?;

    #[cfg(target_os = "macos")]
    let contents = launchd_plist(
        executable.as_os_str(),
        arguments.as_slice(),
        record_path.as_path(),
        log_path.as_path(),
        args.ui_password.as_deref(),
    )?;
    #[cfg(target_os = "linux")]
    let contents = systemd_unit(
        executable.as_os_str(),
        arguments.as_slice(),
        record_path.as_path(),
        args.ui_password.as_deref(),
    )?;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("OS-native Agena user services are supported only on macOS and Linux");

    write_private_file(path.as_path(), contents.as_bytes())?;
    reload_and_start(path.as_path())?;
    Ok(path)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn install(_args: &ServerArgs) -> Result<PathBuf> {
    bail!("OS-native Agena user services are supported only on macOS and Linux")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn start() -> Result<()> {
    let path = service_file_path()?;
    if !path.is_file() {
        bail!(
            "Agena user service is not installed at {}; run `agena server install` first",
            path.display()
        );
    }
    start_service(path.as_path())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn start() -> Result<()> {
    bail!("OS-native Agena user services are supported only on macOS and Linux")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn stop() -> Result<()> {
    let path = service_file_path()?;
    stop_service(path.as_path())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn stop() -> Result<()> {
    bail!("OS-native Agena user services are supported only on macOS and Linux")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn uninstall() -> Result<PathBuf> {
    let path = service_file_path()?;
    stop_service(path.as_path())?;
    disable_before_uninstall()?;
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("failed to remove user service {}", path.display()))?;
    }
    reload_after_uninstall()?;
    Ok(path)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn uninstall() -> Result<PathBuf> {
    bail!("OS-native Agena user services are supported only on macOS and Linux")
}

fn server_arguments(args: &ServerArgs) -> Result<Vec<OsString>> {
    let mut arguments = Vec::new();
    for expression in &args.overrides {
        arguments.push("--set".into());
        arguments.push(expression.into());
    }
    if let Some(database_url) = &args.database_url {
        arguments.push("--database-url".into());
        arguments.push(database_url.into());
    }
    if let Some(database_path) = &args.database_path {
        arguments.push("--database-path".into());
        arguments.push(database_path.as_os_str().to_owned());
    }
    arguments.push("server".into());
    arguments.push("--host".into());
    arguments.push(args.host.as_str().into());
    arguments.push("--port".into());
    arguments.push(args.port.to_string().into());
    if let Some(enabled) = args.mcp_enabled {
        arguments.push("--mcp-enabled".into());
        arguments.push(enabled.to_string().into());
    }
    if let Some(mcp_public_url) = &args.mcp_public_url {
        arguments.push("--mcp-public-url".into());
        arguments.push(mcp_public_url.into());
    }
    if let Some(mcp_oauth_issuer_url) = &args.mcp_oauth_issuer_url {
        arguments.push("--mcp-oauth-issuer-url".into());
        arguments.push(mcp_oauth_issuer_url.into());
    }
    if let Some(mode) = args.mcp_auth_mode {
        arguments.push("--mcp-auth-mode".into());
        arguments.push(mode.as_str().into());
    }
    if let Some(access) = args.mcp_anonymous_access {
        arguments.push("--mcp-anonymous-access".into());
        arguments.push(access.as_str().into());
    }
    if let Some(registration) = args.mcp_client_registration {
        arguments.push("--mcp-client-registration".into());
        arguments.push(registration.as_str().into());
    }

    let workspace = args
        .workspace_root
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    let workspace = fs::canonicalize(&workspace).with_context(|| {
        format!(
            "failed to canonicalize the user-service workspace {}",
            workspace.display()
        )
    })?;
    arguments.push("--workspace".into());
    arguments.push(workspace.into_os_string());
    if let Some(ui_dir) = &args.ui_dir {
        arguments.push("--ui-dir".into());
        arguments.push(ui_dir.as_os_str().to_owned());
    }

    Ok(arguments)
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("cannot resolve the user home directory"))
}

fn service_file_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(home_dir()?
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{SERVICE_LABEL}.plist")))
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(home_dir()?.join(".config"));
        Ok(base.join("systemd").join("user").join(SYSTEMD_UNIT_NAME))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    bail!("OS-native Agena user services are supported only on macOS and Linux")
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("service path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create service directory {}", parent.display()))?;
    let temporary = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(&temporary)
        .with_context(|| format!("failed to create service file {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        let mut writer = BufWriter::new(file);
        writer.write_all(contents)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to install service file {}", path.display()))?;
        Ok(())
    })();
    if result.is_err()
        && let Err(cleanup_error) = fs::remove_file(&temporary)
        && cleanup_error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            path = %temporary.display(),
            diagnostic = %agena_failure::diagnostic::format_error_chain(&cleanup_error),
            "failed to remove a temporary service file after installation failed"
        );
    }
    result
}

#[cfg(target_os = "macos")]
fn launchd_plist(
    executable: &OsStr,
    arguments: &[OsString],
    record_path: &Path,
    log_path: &Path,
    ui_password: Option<&str>,
) -> Result<String> {
    let mut program_arguments = Vec::with_capacity(arguments.len() + 1);
    program_arguments.push(os_text(executable, "Agena executable")?);
    for argument in arguments {
        program_arguments.push(os_text(argument.as_os_str(), "service argument")?);
    }
    let mut environment = vec![(
        "AGENA_SERVER_RECORD",
        path_text(record_path, "server record path")?,
    )];
    if let Some(password) = ui_password.filter(|password| !password.trim().is_empty()) {
        environment.push(("AGENA_SERVER_UI_PASSWORD", password.to_owned()));
    }
    let argument_xml = program_arguments
        .iter()
        .map(|argument| format!("      <string>{}</string>", xml_escape(argument)))
        .collect::<Vec<_>>()
        .join("\n");
    let environment_xml = environment
        .iter()
        .map(|(key, value)| {
            format!(
                "      <key>{}</key>\n      <string>{}</string>",
                xml_escape(key),
                xml_escape(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let log_path = path_text(log_path, "service log path")?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
  <dict>\n\
    <key>Label</key>\n\
    <string>{SERVICE_LABEL}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n{argument_xml}\n    </array>\n\
    <key>EnvironmentVariables</key>\n\
    <dict>\n{environment_xml}\n    </dict>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>KeepAlive</key>\n\
    <dict>\n\
      <key>SuccessfulExit</key>\n\
      <false/>\n\
    </dict>\n\
    <key>ThrottleInterval</key>\n\
    <integer>2</integer>\n\
    <key>ProcessType</key>\n\
    <string>Background</string>\n\
    <key>StandardOutPath</key>\n\
    <string>{}</string>\n\
    <key>StandardErrorPath</key>\n\
    <string>{}</string>\n\
  </dict>\n\
</plist>\n",
        xml_escape(log_path.as_str()),
        xml_escape(log_path.as_str())
    ))
}

#[cfg(any(target_os = "linux", test))]
fn systemd_unit(
    executable: &OsStr,
    arguments: &[OsString],
    record_path: &Path,
    ui_password: Option<&str>,
) -> Result<String> {
    let mut command = vec![systemd_quote(
        os_text(executable, "Agena executable")?.as_str(),
    )?];
    for argument in arguments {
        command.push(systemd_quote(
            os_text(argument.as_os_str(), "service argument")?.as_str(),
        )?);
    }
    let mut environment = vec![format!(
        "Environment={}",
        systemd_quote(
            format!(
                "AGENA_SERVER_RECORD={}",
                path_text(record_path, "server record path")?
            )
            .as_str()
        )?
    )];
    if let Some(password) = ui_password.filter(|password| !password.trim().is_empty()) {
        environment.push(format!(
            "Environment={}",
            systemd_quote(format!("AGENA_SERVER_UI_PASSWORD={password}").as_str())?
        ));
    }
    Ok(format!(
        "[Unit]\n\
Description=Agena server\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=simple\n\
{}\n\
ExecStart={}\n\
Restart=on-failure\n\
RestartSec=2\n\n\
[Install]\n\
WantedBy=default.target\n",
        environment.join("\n"),
        command.join(" ")
    ))
}

fn os_text(value: &OsStr, label: &str) -> Result<String> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{label} is not valid UTF-8"))
}

fn path_text(path: &Path, label: &str) -> Result<String> {
    os_text(path.as_os_str(), label)
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(any(target_os = "linux", test))]
fn systemd_quote(value: &str) -> Result<String> {
    if value.contains(['\n', '\r', '\0']) {
        bail!("systemd service values must not contain control characters");
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('%', "%%")
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    ))
}

#[cfg(target_os = "macos")]
fn launchd_domain() -> String {
    // SAFETY: getuid has no preconditions and does not retain pointers.
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn reload_and_start(path: &Path) -> Result<()> {
    let domain = launchd_domain();
    let _ = Command::new("launchctl")
        .args(["bootout", domain.as_str()])
        .arg(path)
        .status();
    run_command(
        Command::new("launchctl")
            .args(["bootstrap", domain.as_str()])
            .arg(path),
        "launchctl bootstrap",
    )?;
    run_command(
        Command::new("launchctl").args(["enable", format!("{domain}/{SERVICE_LABEL}").as_str()]),
        "launchctl enable",
    )
}

#[cfg(target_os = "macos")]
fn start_service(path: &Path) -> Result<()> {
    let domain = launchd_domain();
    let target = format!("{domain}/{SERVICE_LABEL}");
    let bootstrap = Command::new("launchctl")
        .args(["bootstrap", domain.as_str()])
        .arg(path)
        .status();
    if bootstrap.as_ref().is_err() || bootstrap.is_ok_and(|status| !status.success()) {
        run_command(
            Command::new("launchctl").args(["kickstart", "-k", target.as_str()]),
            "launchctl kickstart",
        )?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn stop_service(path: &Path) -> Result<()> {
    let domain = launchd_domain();
    let status = Command::new("launchctl")
        .args(["bootout", domain.as_str()])
        .arg(path)
        .status()
        .context("failed to execute launchctl bootout")?;
    if !status.success() && super::server_record::record_path().exists() {
        bail!("launchctl bootout failed with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn reload_after_uninstall() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn disable_before_uninstall() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn reload_and_start(_path: &Path) -> Result<()> {
    run_systemctl(["daemon-reload"])?;
    run_systemctl(["enable", "--now", SYSTEMD_UNIT_NAME])
}

#[cfg(target_os = "linux")]
fn start_service(_path: &Path) -> Result<()> {
    run_systemctl(["start", SYSTEMD_UNIT_NAME])
}

#[cfg(target_os = "linux")]
fn stop_service(_path: &Path) -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "stop", SYSTEMD_UNIT_NAME])
        .status()
        .context("failed to execute systemctl --user stop")?;
    if !status.success() && super::server_record::record_path().exists() {
        bail!("systemctl --user stop failed with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reload_after_uninstall() -> Result<()> {
    run_systemctl(["daemon-reload"])?;
    let _ = Command::new("systemctl")
        .args(["--user", "reset-failed", SYSTEMD_UNIT_NAME])
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
fn disable_before_uninstall() -> Result<()> {
    run_systemctl(["disable", SYSTEMD_UNIT_NAME])
}

#[cfg(target_os = "linux")]
fn run_systemctl<const N: usize>(arguments: [&str; N]) -> Result<()> {
    run_command(
        Command::new("systemctl").arg("--user").args(arguments),
        "systemctl --user",
    )
}

fn run_command(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to execute {description}"))?;
    if !status.success() {
        bail!("{description} failed with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_arguments_preserve_the_web_frontend_directory() {
        let workspace = tempfile::tempdir().expect("temporary Agena workspace");
        let args = ServerArgs {
            action: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            host: "127.0.0.1".to_owned(),
            port: 3210,
            ui_password: None,
            mcp_enabled: Some(true),
            mcp_public_url: Some("https://mcp.example.test/mcp".to_owned()),
            mcp_oauth_issuer_url: Some("https://auth.example.test".to_owned()),
            mcp_auth_mode: Some(agena_cli::McpAuthModeArg::Mixed),
            mcp_anonymous_access: Some(agena_cli::McpAnonymousAccessArg::ReadOnly),
            mcp_client_registration: Some(agena_cli::McpClientRegistrationArg::CimdOnly),
            workspace_root: Some(workspace.path().to_path_buf()),
            ui_dir: Some(PathBuf::from("/opt/agena/web-dist")),
        };
        let arguments = server_arguments(&args).expect("render server arguments");
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--ui-dir", "/opt/agena/web-dist"])
        );
        for expected in [
            ["--mcp-enabled", "true"],
            ["--mcp-public-url", "https://mcp.example.test/mcp"],
            ["--mcp-oauth-issuer-url", "https://auth.example.test"],
            ["--mcp-auth-mode", "mixed"],
            ["--mcp-anonymous-access", "read-only"],
            ["--mcp-client-registration", "cimd-only"],
        ] {
            assert!(
                arguments.windows(2).any(|pair| pair == expected),
                "installed service arguments must preserve {expected:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_definition_is_shell_free_private_service_contract() {
        let plist = launchd_plist(
            OsStr::new("/Applications/Agena & Tools/agena"),
            &[OsString::from("server"), OsString::from("--workspace=a&b")],
            Path::new("/tmp/agena & state/server.json"),
            Path::new("/tmp/agena.log"),
            Some("p<&>\"'"),
        )
        .expect("render launchd plist");
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>SuccessfulExit</key>"));
        assert!(plist.contains("/Applications/Agena &amp; Tools/agena"));
        assert!(plist.contains("p&lt;&amp;&gt;&quot;&apos;"));
        assert!(!plist.contains("/bin/sh"));

        let directory = tempfile::tempdir().expect("temporary launchd directory");
        let path = directory.path().join("com.agena.server.plist");
        write_private_file(path.as_path(), plist.as_bytes()).expect("write private plist");
        let permissions = fs::metadata(&path).expect("plist metadata").permissions();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(permissions.mode() & 0o777, 0o600);
        let status = Command::new("/usr/bin/plutil")
            .args(["-lint", "--"])
            .arg(&path)
            .status()
            .expect("run plutil");
        assert!(status.success(), "generated launchd plist must be valid");
    }

    #[test]
    fn systemd_definition_is_shell_free_restart_contract() {
        let unit = systemd_unit(
            OsStr::new("/opt/Agena Tools/agena"),
            &[OsString::from("server"), OsString::from("100%")],
            Path::new("/tmp/agena state/server.json"),
            Some("secret value"),
        )
        .expect("render systemd unit");
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(unit.contains("100%%"));
        assert!(unit.contains("AGENA_SERVER_UI_PASSWORD=secret value"));
        assert!(!unit.contains("/bin/sh"));
    }
}
