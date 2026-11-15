//! A persistent macOS app identity for the installed launchd service.
//!
//! Linker/ad-hoc signatures identify one specific build by its cdhash. Replacing
//! that binary invalidates TCC's remembered grants (Apple TN3127). Keep a tiny
//! signed supervisor, including its Info.plist and seal, unchanged in runtime
//! data and spawn the replaceable server as its child. Never exec the server,
//! re-sign an existing host, or invent a weaker designated requirement.

use std::{
    fs,
    io::{BufRead as _, IsTerminal as _, Read as _, Write as _},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};

use super::macos_privacy_probe::{ARGUMENT, PermissionProbe, REPORT_ARGUMENT};

pub(super) const BUNDLE_ID: &str = "com.agena.permission-host";
const HOST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agena-permission-host"));
const INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.agena.permission-host</string>
  <key>CFBundleName</key><string>Agena</string>
  <key>CFBundleDisplayName</key><string>Agena</string>
  <key>CFBundleExecutable</key><string>Agena</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>LSUIElement</key><true/>
  <key>NSScreenCaptureUsageDescription</key><string>Agena uses screen capture for computer tools you authorize.</string>
  <key>NSDesktopFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>Agena accesses your workspaces on external drives.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>Agena accesses your workspaces on network drives.</string>
</dict></plist>
"#;

pub(super) fn bundle_path() -> Result<PathBuf> {
    Ok(agena_storage_layout::StorageLayout::current()
        .root()
        .join("macos")
        .join("Agena.app"))
}

fn executable(bundle: &Path) -> PathBuf {
    bundle.join("Contents/MacOS/Agena")
}

fn validate(bundle: &Path) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(bundle)?.file_type().is_symlink(),
        "the macOS permission host must not be a symlink: {}",
        bundle.display()
    );
    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R"])
        .arg(format!("=identifier \"{BUNDLE_ID}\""))
        .arg(bundle)
        .output()
        .context("failed to verify the macOS permission host signature")?;
    ensure!(
        signature.status.success(),
        "macOS permission host signature is invalid at {}: {}. Restore its original backup; replacing it requires Full Disk Access again",
        bundle.display(),
        String::from_utf8_lossy(&signature.stderr).trim()
    );
    let protocol = Command::new(executable(bundle))
        .arg("--agena-host-protocol")
        .output()
        .context("failed to inspect the macOS permission host protocol")?;
    ensure!(
        protocol.status.success() && protocol.stdout == b"1\n",
        "unsupported macOS permission host at {}; it was preserved to retain its authorization",
        bundle.display()
    );
    Ok(())
}

/// Install only when absent. Even a newly built host with different bytes must
/// reuse the originally authorized app on upgrade, rollback, and reinstall.
fn ensure_bundle_at(bundle: &Path) -> Result<bool> {
    if bundle.try_exists()? {
        validate(bundle)?;
        return Ok(false);
    }
    let parent = bundle.parent().context("permission host has no parent")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    let staged_bundle = staging.path().join("Agena.app");
    let host = executable(&staged_bundle);
    fs::create_dir_all(host.parent().unwrap())?;
    fs::write(&host, HOST)?;
    fs::set_permissions(&host, fs::Permissions::from_mode(0o700))?;
    fs::write(staged_bundle.join("Contents/Info.plist"), INFO)?;
    let signed = Command::new("/usr/bin/codesign")
        .args(["--sign", "-", "--timestamp=none", "--identifier", BUNDLE_ID])
        .arg(&staged_bundle)
        .output()
        .context("failed to sign the macOS permission host")?;
    ensure!(
        signed.status.success(),
        "failed to sign macOS permission host: {}",
        String::from_utf8_lossy(&signed.stderr)
    );
    validate(&staged_bundle)?;
    if let Err(error) = fs::rename(&staged_bundle, bundle) {
        // A concurrent install may have won. Validate and preserve that copy.
        if bundle.try_exists()? {
            validate(bundle)?;
            return Ok(false);
        }
        return Err(error).context("failed to install the macOS permission host");
    }
    Ok(true)
}

pub(super) fn ensure_host() -> Result<PathBuf> {
    let bundle = bundle_path()?;
    if ensure_bundle_at(&bundle)? {
        print_instructions(&bundle);
        // Interactive first install only. Service restarts/updates never open
        // settings or probe protected data to guess the current TCC state.
        if std::io::stdout().is_terminal()
            && graphical_session()
            && std::env::var_os("AGENA_MACOS_OPEN_PERMISSIONS").as_deref()
                != Some(std::ffi::OsStr::new("0"))
            && let Err(error) = open_settings(&bundle)
        {
            eprintln!(
                "Could not open macOS settings: {error}. Run `agena server permissions` later."
            );
        }
    }
    Ok(executable(&bundle))
}

fn print_instructions(bundle: &Path) {
    println!(
        "macOS privacy setup for Agena:\n\
         Enable Agena in System Settings > Privacy & Security > Full Disk Access.\n\
         Add this app with the + button (Cmd+Shift+G accepts its path): {}\n\
         Accessibility and Screen Recording requests are checked during this setup.\n\
         This signed app is preserved across upgrades to retain its authorization.\n\
         Repeat setup at any time: agena server permissions",
        bundle.display()
    );
}

fn open_settings(bundle: &Path) -> Result<()> {
    for args in [
        vec![std::ffi::OsStr::new("-R"), bundle.as_os_str()],
        vec![std::ffi::OsStr::new(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        )],
    ] {
        let status = Command::new("/usr/bin/open")
            .args(args)
            .stdin(Stdio::null())
            .status()?;
        ensure!(status.success(), "macOS open failed with {status}");
    }
    Ok(())
}

fn probe_command(bundle: &Path, binary: &Path, request: bool, report: &Path) -> Command {
    // LaunchServices starts the stable application in its own responsibility
    // context. Directly spawning its executable from a terminal may attribute
    // the privacy decision to Terminal instead of the installed service.
    // The original protocol-1 host still forwards these arguments unchanged.
    let mut command = Command::new("/usr/bin/open");
    command
        .args(["-W", "-n", "-a"])
        .arg(bundle)
        .arg("--args")
        .arg(binary)
        .arg(ARGUMENT)
        .arg(if request { "request" } else { "check" })
        .arg(REPORT_ARGUMENT)
        .arg(report)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

fn read_probe_report(path: &Path) -> Result<PermissionProbe> {
    const MAX_REPORT_BYTES: u64 = 4096;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("the macOS privacy probe did not publish its report")?;
    ensure!(
        file.metadata()?.is_file(),
        "macOS privacy report is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_REPORT_BYTES,
        "macOS privacy report exceeds the size limit"
    );
    let report: PermissionProbe =
        serde_json::from_slice(&bytes).context("invalid macOS privacy check report")?;
    ensure!(
        report.schema_version == 1
            && report.accessibility.is_some()
            && report.screen_recording.is_some(),
        "unsupported or incomplete macOS privacy report"
    );
    Ok(report)
}

fn probe(bundle: &Path, request: bool) -> Result<PermissionProbe> {
    let directory = tempfile::Builder::new()
        .prefix("agena-macos-permissions-")
        .tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    let report_path = directory.path().join("report.json");
    let status = probe_command(bundle, &std::env::current_exe()?, request, &report_path)
        .status()
        .context("failed to launch Agena.app through macOS LaunchServices")?;
    ensure!(
        status.success(),
        "macOS LaunchServices could not run the privacy probe: {status}"
    );
    // open's exit status describes the launch, so the bounded child result is
    // mandatory even when LaunchServices reports success.
    read_probe_report(&report_path)
}

fn graphical_session() -> bool {
    std::env::var_os("SSH_CONNECTION").is_none()
        && std::env::var_os("SSH_TTY").is_none()
        && Command::new("/bin/launchctl")
            // SAFETY: geteuid has no preconditions and does not mutate state.
            .args(["print", &format!("gui/{}", unsafe { libc::geteuid() })])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

fn save_report(bundle: &Path, report: &PermissionProbe) -> Result<()> {
    let parent = bundle.parent().context("permission host has no parent")?;
    let path = parent.join("permissions.json");
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(
        &mut file,
        &serde_json::json!({
            "schema_version": 1,
            "permission_host": bundle,
            "accessibility": report.accessibility,
            "screen_recording": report.screen_recording,
            "inspection": if report.accessibility.is_some() && report.screen_recording.is_some() {
                "launch_services"
            } else {
                "unavailable_no_local_desktop"
            },
            // macOS has no public API that verifies Full Disk Access. Do not
            // infer it by touching private databases or protected user data.
            "full_disk_access": "confirm_in_system_settings",
        }),
    )?;
    file.write_all(b"\n")?;
    file.persist(&path)
        .map_err(|error| error.error)
        .context("failed to save the macOS permission report")?;
    println!("macOS permission report: {}", path.display());
    Ok(())
}

fn print_report(report: &PermissionProbe) {
    for (name, granted) in [
        ("Accessibility", report.accessibility),
        ("Screen Recording", report.screen_recording),
    ] {
        println!(
            "  {name}: {}",
            match granted {
                Some(true) => "granted",
                Some(false) => "pending in System Settings",
                None => "unknown (requires a local macOS desktop session)",
            }
        );
    }
    println!("  Full Disk Access: confirm Agena is enabled in System Settings.");
}

pub(super) fn setup() -> Result<()> {
    let bundle = bundle_path()?;
    ensure_bundle_at(&bundle)?;
    let graphical = graphical_session();
    let mut report = if graphical {
        probe(&bundle, false)?
    } else {
        PermissionProbe::unknown()
    };
    let auto_install = std::env::var_os("AGENA_MACOS_PERMISSION_INSTALL").as_deref()
        == Some(std::ffi::OsStr::new("1"));
    let prior_setup = bundle.parent().unwrap().join("permissions.json").is_file();
    let requests_enabled = std::env::var_os("AGENA_MACOS_OPEN_PERMISSIONS").as_deref()
        != Some(std::ffi::OsStr::new("0"));
    let needs_setup = !auto_install || !prior_setup || !report.desktop_permissions_granted();
    if requests_enabled && needs_setup {
        print_instructions(&bundle);
        let terminal = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty");
        // A service/SSH installation must not leave hidden dialogs waiting on
        // another desktop. The printed command resumes setup locally.
        if let Ok(mut terminal) = terminal
            && terminal.is_terminal()
            && graphical
        {
            report = probe(&bundle, true)?;
            open_settings(&bundle)?;
            print_report(&report);
            write!(
                terminal,
                "Grant the permissions above, then press Enter to recheck: "
            )?;
            terminal.flush()?;
            let mut answer = String::new();
            std::io::BufReader::new(&terminal).read_line(&mut answer)?;
            report = probe(&bundle, false)?;
        } else {
            println!(
                "Complete privacy authorization in a macOS desktop terminal: agena server permissions"
            );
        }
    }
    print_report(&report);
    save_report(&bundle, &report)?;
    if !report.desktop_permissions_granted() {
        println!("Pending permissions can be completed with: agena server permissions");
    }
    if !auto_install {
        println!(
            "After changing permissions, restart the service: agena server stop && agena server start"
        );
    }
    Ok(())
}

#[cfg(test)]
mod privacy_report_tests {
    use super::*;
    use std::{ffi::OsStr, os::unix::fs::symlink};

    #[test]
    fn privacy_check_launches_a_new_stable_app_with_a_private_report_argument() {
        let command = probe_command(
            Path::new("/private data/Agena.app"),
            Path::new("/installed program/bin/agena"),
            false,
            Path::new("/private report/report.json"),
        );
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/open"));
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>(),
            [
                "-W",
                "-n",
                "-a",
                "/private data/Agena.app",
                "--args",
                "/installed program/bin/agena",
                ARGUMENT,
                "check",
                REPORT_ARGUMENT,
                "/private report/report.json",
            ],
        );
    }

    #[test]
    fn probe_reader_requires_bounded_complete_regular_file_reports() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("report.json");
        fs::write(
            &path,
            br#"{"schema_version":1,"accessibility":true,"screen_recording":false}"#,
        )
        .unwrap();
        let report = read_probe_report(&path).unwrap();
        assert_eq!(report.accessibility, Some(true));
        assert_eq!(report.screen_recording, Some(false));
        assert!(!report.desktop_permissions_granted());

        for invalid in [
            br#"{"schema_version":1,"accessibility":true}"#.as_slice(),
            br#"{"schema_version":1,"accessibility":null,"screen_recording":true}"#.as_slice(),
            br#"{"schema_version":2,"accessibility":true,"screen_recording":true}"#.as_slice(),
            b"not-json".as_slice(),
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(read_probe_report(&path).is_err());
        }
        fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(read_probe_report(&path).is_err());
        let link = directory.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(read_probe_report(&link).is_err());
        assert!(read_probe_report(directory.path()).is_err());
    }
}
