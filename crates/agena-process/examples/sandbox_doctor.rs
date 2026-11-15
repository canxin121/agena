//! Native sandbox diagnostics: cargo run -p agena-process --example sandbox_doctor
use agena_process::sandbox::{SandboxPolicy, try_run_launcher, wrap_argv};
use std::collections::HashMap;
use std::process::ExitCode;

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    if let Some(code) = try_run_launcher() {
        std::process::exit(code);
    }
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "MXC discovered backends:\n{}",
        serde_json::to_string_pretty(&mxc_sdk::v1::available_backends())?
    );
    #[cfg(target_os = "linux")]
    {
        for program in [
            "bwrap",
            "slirp4netns",
            "ip",
            "iptables",
            "ip6tables",
            "unshare",
            "nsenter",
        ] {
            match which::which(program) {
                Ok(path) => println!("{program}: {}", path.display()),
                Err(_) => println!("{program}: unavailable"),
            }
        }
        for path in [
            "/proc/sys/kernel/unprivileged_userns_clone",
            "/proc/sys/user/max_user_namespaces",
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
            "/proc/self/attr/current",
        ] {
            if let Ok(value) = std::fs::read_to_string(path) {
                println!("{path}: {}", value.trim());
            }
        }
    }
    let directory = tempfile::tempdir()?;
    let policy = SandboxPolicy {
        readonly_paths: vec![directory.path().to_path_buf()],
        timeout_ms: Some(5_000),
        ..Default::default()
    };
    let mut environment: HashMap<_, _> = std::env::vars().collect();
    let argv = wrap_argv(
        &[
            std::env::current_exe()?.display().to_string(),
            "--probe".to_owned(),
        ],
        directory.path(),
        &mut environment,
        &policy,
    )?;
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(directory.path())
        .env_clear()
        .envs(environment)
        .output()?;
    if output.status.success() {
        println!("PASS: an offline process executed inside the native MXC backend.");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!(
            "FAIL: native isolation could not execute the probe ({}).\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(target_os = "linux")]
        eprintln!(
            "For user-namespace EPERM on Ubuntu, check the AppArmor profile for the installed bwrap executable. Keep global user-namespace protections enabled; an administrator can install/load the distribution's specific Bubblewrap profile. Network-enabled execution also requires slirp4netns."
        );
        Ok(ExitCode::FAILURE)
    }
}
