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

fn service_name_for(root: &Path, prefix: &str, suffix: &str) -> String {
    let key = agena_storage_layout::workspace_key(root);
    format!("{prefix}{key}{suffix}")
}

#[cfg(target_os = "macos")]
fn service_label() -> String {
    service_name_for(
        agena_storage_layout::StorageLayout::current().root(),
        "com.agena.server.",
        "",
    )
}

#[cfg(target_os = "linux")]
fn systemd_unit_name() -> String {
    service_name_for(
        agena_storage_layout::StorageLayout::current().root(),
        "agena-server-",
        ".service",
    )
}

#[cfg(target_os = "windows")]
fn windows_task_name() -> String {
    service_name_for(
        agena_storage_layout::StorageLayout::current().root(),
        "Agena Server ",
        "",
    )
}

#[cfg(target_os = "windows")]
fn windows_task_exists() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", windows_task_name().as_str()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn is_installed() -> bool {
    service_file_path().is_ok_and(|path| path.is_file())
}

#[cfg(target_os = "windows")]
pub(crate) fn is_installed() -> bool {
    service_file_path().is_ok_and(|path| path.is_file()) && windows_task_exists()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(crate) fn is_installed() -> bool {
    false
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn install(args: &ServerArgs) -> Result<PathBuf> {
    let path = service_file_path()?;
    let executable = std::env::current_exe().context("failed to resolve Agena executable")?;
    let log_path = agena_storage_layout::StorageLayout::current()
        .logs_dir()
        .join("server-service.log");
    if let Some(parent) = log_path.parent() {
        agena_storage_layout::secure_directory(parent)
            .context("failed to create private service log directory")?;
    }
    let arguments = server_arguments(args)?;

    #[cfg(target_os = "macos")]
    let (executable, arguments) = {
        let host = super::macos_permissions::ensure_host()?;
        let mut hosted_arguments = Vec::with_capacity(arguments.len() + 1);
        hosted_arguments.push(executable.into_os_string());
        hosted_arguments.extend(arguments);
        (host, hosted_arguments)
    };

    #[cfg(target_os = "macos")]
    let contents = launchd_plist(
        executable.as_os_str(),
        arguments.as_slice(),
        log_path.as_path(),
        args.ui_password.as_deref(),
    )?;
    #[cfg(target_os = "linux")]
    let contents = systemd_unit(
        executable.as_os_str(),
        arguments.as_slice(),
        args.ui_password.as_deref(),
    )?;
    #[cfg(target_os = "windows")]
    let contents = windows_task_xml(
        executable.as_os_str(),
        arguments.as_slice(),
        args.ui_password.as_deref(),
    )?;

    #[cfg(target_os = "windows")]
    let contents = windows_task_file_bytes(contents.as_str());
    #[cfg(not(target_os = "windows"))]
    let contents = contents.into_bytes();
    write_private_file(path.as_path(), contents.as_slice())?;
    reload_and_start(path.as_path())?;
    Ok(path)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(crate) fn install(_args: &ServerArgs) -> Result<PathBuf> {
    bail!("OS-native Agena user services are supported only on macOS, Linux, and Windows")
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(crate) fn start() -> Result<()> {
    bail!("OS-native Agena user services are supported only on macOS, Linux, and Windows")
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn stop() -> Result<()> {
    let path = service_file_path()?;
    stop_service(path.as_path())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(crate) fn stop() -> Result<()> {
    bail!("OS-native Agena user services are supported only on macOS, Linux, and Windows")
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub(crate) fn uninstall() -> Result<PathBuf> {
    bail!("OS-native Agena user services are supported only on macOS, Linux, and Windows")
}

fn server_arguments(args: &ServerArgs) -> Result<Vec<OsString>> {
    let mut arguments = Vec::new();
    for expression in &args.overrides {
        arguments.push("--set".into());
        arguments.push(expression.into());
    }
    arguments.push("--data-dir".into());
    arguments.push(
        agena_storage_layout::StorageLayout::current()
            .root()
            .as_os_str()
            .to_owned(),
    );
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
            .join(format!("{}.plist", service_label())))
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(home_dir()?.join(".config"));
        Ok(base
            .join("systemd")
            .join("user")
            .join(systemd_unit_name().as_str()))
    }
    #[cfg(target_os = "windows")]
    {
        let record = super::server_record::record_path();
        let base = record
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or(home_dir()?);
        Ok(base.join("agena-server-task.xml"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    bail!("OS-native Agena user services are supported only on macOS, Linux, and Windows")
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
        #[cfg(target_os = "windows")]
        if path.exists() {
            fs::remove_file(path).with_context(|| {
                format!(
                    "failed to replace existing Windows user service file {}",
                    path.display()
                )
            })?;
        }
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
    log_path: &Path,
    ui_password: Option<&str>,
) -> Result<String> {
    let mut program_arguments = Vec::with_capacity(arguments.len() + 1);
    program_arguments.push(os_text(executable, "Agena executable")?);
    for argument in arguments {
        program_arguments.push(os_text(argument.as_os_str(), "service argument")?);
    }
    let mut environment = Vec::new();
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
    let service_label = service_label();
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
  <dict>\n\
    <key>Label</key>\n\
    <string>{service_label}</string>\n\
    <key>AssociatedBundleIdentifiers</key>\n\
    <array><string>{}</string></array>\n\
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
        super::macos_permissions::BUNDLE_ID,
        xml_escape(log_path.as_str()),
        xml_escape(log_path.as_str())
    ))
}

#[cfg(target_os = "windows")]
fn windows_task_file_bytes(contents: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(contents.len().saturating_mul(2) + 2);
    bytes.extend_from_slice(&[0xff, 0xfe]);
    for unit in contents.encode_utf16() {
        bytes.extend_from_slice(unit.to_le_bytes().as_slice());
    }
    bytes
}

#[cfg(target_os = "linux")]
fn systemd_unit(
    executable: &OsStr,
    arguments: &[OsString],
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
    let mut environment = Vec::new();
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn path_text(path: &Path, label: &str) -> Result<String> {
    os_text(path.as_os_str(), label)
}

#[cfg(target_os = "windows")]
fn windows_quote_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || ch == '"')
    {
        return value.to_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
            continue;
        }
        quoted.push_str(&"\\".repeat(backslashes));
        backslashes = 0;
        quoted.push(ch);
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(target_os = "windows")]
fn windows_user_id() -> Result<String> {
    let username = std::env::var("USERNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("cannot resolve the Windows user name"))?;
    Ok(std::env::var("USERDOMAIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|domain| format!("{domain}\\{username}"))
        .unwrap_or(username))
}

#[cfg(target_os = "windows")]
fn windows_task_xml(
    executable: &OsStr,
    arguments: &[OsString],
    ui_password: Option<&str>,
) -> Result<String> {
    windows_task_xml_for_user(
        executable,
        arguments,
        ui_password,
        windows_user_id()?.as_str(),
    )
}

#[cfg(target_os = "windows")]
fn windows_task_xml_for_user(
    executable: &OsStr,
    arguments: &[OsString],
    ui_password: Option<&str>,
    user_id: &str,
) -> Result<String> {
    let executable = os_text(executable, "Agena executable")?;
    let mut task_arguments = arguments
        .iter()
        .map(|argument| os_text(argument.as_os_str(), "service argument"))
        .collect::<Result<Vec<_>>>()?;
    if let Some(password) = ui_password.filter(|password| !password.trim().is_empty()) {
        task_arguments.push("--ui-password".to_owned());
        task_arguments.push(password.to_owned());
    }
    let task_arguments = task_arguments
        .iter()
        .map(|argument| windows_quote_argument(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let working_directory = Path::new(executable.as_str())
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path_text(path, "Agena executable directory"))
        .transpose()?
        .unwrap_or_else(|| ".".to_owned());

    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
  <RegistrationInfo>\n\
    <Description>Agena server</Description>\n\
    <URI>\\{}</URI>\n\
  </RegistrationInfo>\n\
  <Triggers>\n\
    <LogonTrigger>\n\
      <Enabled>true</Enabled>\n\
      <UserId>{}</UserId>\n\
    </LogonTrigger>\n\
  </Triggers>\n\
  <Principals>\n\
    <Principal id=\"Author\">\n\
      <UserId>{}</UserId>\n\
      <LogonType>InteractiveToken</LogonType>\n\
      <RunLevel>LeastPrivilege</RunLevel>\n\
    </Principal>\n\
  </Principals>\n\
  <Settings>\n\
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
    <AllowHardTerminate>true</AllowHardTerminate>\n\
    <StartWhenAvailable>true</StartWhenAvailable>\n\
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\n\
    <AllowStartOnDemand>true</AllowStartOnDemand>\n\
    <Enabled>true</Enabled>\n\
    <Hidden>false</Hidden>\n\
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
    <Priority>7</Priority>\n\
  </Settings>\n\
  <Actions Context=\"Author\">\n\
    <Exec>\n\
      <Command>{}</Command>\n\
      <Arguments>{}</Arguments>\n\
      <WorkingDirectory>{}</WorkingDirectory>\n\
    </Exec>\n\
  </Actions>\n\
</Task>\n",
        xml_escape(windows_task_name().as_str()),
        xml_escape(user_id),
        xml_escape(user_id),
        xml_escape(executable.as_str()),
        xml_escape(task_arguments.as_str()),
        xml_escape(working_directory.as_str()),
    ))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "linux")]
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
        Command::new("launchctl")
            .args(["enable", format!("{domain}/{}", service_label()).as_str()]),
        "launchctl enable",
    )
}

#[cfg(target_os = "macos")]
fn start_service(path: &Path) -> Result<()> {
    let domain = launchd_domain();
    let target = format!("{domain}/{}", service_label());
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
    run_systemctl(["enable", "--now", systemd_unit_name().as_str()])
}

#[cfg(target_os = "linux")]
fn start_service(_path: &Path) -> Result<()> {
    run_systemctl(["start", systemd_unit_name().as_str()])
}

#[cfg(target_os = "linux")]
fn stop_service(_path: &Path) -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "stop", systemd_unit_name().as_str()])
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
        .args(["--user", "reset-failed", systemd_unit_name().as_str()])
        // daemon-reload may already have unloaded this removed unit. Clearing
        // its optional failure state must not print a spurious uninstall error.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
fn disable_before_uninstall() -> Result<()> {
    run_systemctl(["disable", systemd_unit_name().as_str()])
}

#[cfg(target_os = "windows")]
fn reload_and_start(path: &Path) -> Result<()> {
    run_command(
        Command::new("schtasks.exe")
            .args(["/Create", "/TN", windows_task_name().as_str(), "/XML"])
            .arg(path)
            .arg("/F"),
        "schtasks /Create",
    )?;
    start_service(path)
}

#[cfg(target_os = "windows")]
fn start_service(_path: &Path) -> Result<()> {
    run_command(
        Command::new("schtasks.exe").args(["/Run", "/TN", windows_task_name().as_str()]),
        "schtasks /Run",
    )
}

#[cfg(target_os = "windows")]
fn stop_service(_path: &Path) -> Result<()> {
    let status = Command::new("schtasks.exe")
        .args(["/End", "/TN", windows_task_name().as_str()])
        .status()
        .context("failed to execute schtasks /End")?;
    if !status.success() && windows_task_exists() && super::server_record::record_path().exists() {
        bail!("schtasks /End failed with status {status}");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn reload_after_uninstall() -> Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn disable_before_uninstall() -> Result<()> {
    let status = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", windows_task_name().as_str(), "/F"])
        .status()
        .context("failed to execute schtasks /Delete")?;
    if !status.success() && windows_task_exists() {
        bail!("schtasks /Delete failed with status {status}");
    }
    Ok(())
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
