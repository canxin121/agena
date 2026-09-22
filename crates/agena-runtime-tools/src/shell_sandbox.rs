//! Opt-in, fail-closed offline OS sandbox for shell subprocesses on macOS.
//! This is deliberately NOT a claim of cross-platform or per-domain network
//! isolation: network-capable requests and unsupported platforms are refused.
//! Trusted process configuration selects the mode; model input cannot disable it.
use crate::{
    part::ShellCommandInput,
    tool::{ToolError, ToolExecutor},
};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::{collections::HashMap, sync::LazyLock};
static POLICY: LazyLock<Result<bool, String>> =
    LazyLock::new(|| match std::env::var("AGENA_SHELL_SANDBOX") {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Ok(value) if value == "disabled" => Ok(false),
        Ok(value) if matches!(value.as_str(), "offline" | "required") => Ok(true),
        Ok(_) => Err(
            "AGENA_SHELL_SANDBOX must be disabled or offline/required; invalid policy fails closed"
                .into(),
        ),
        Err(_) => Err("AGENA_SHELL_SANDBOX is not valid UTF-8".into()),
    });
pub fn enabled() -> Result<bool, ToolError> {
    POLICY.clone().map_err(ToolError::invalid_input)
}
pub(crate) fn protect(
    executor: &ToolExecutor,
    command: Vec<String>,
    input: &ShellCommandInput,
    env: &mut HashMap<String, String>,
) -> Result<Vec<String>, ToolError> {
    if !enabled()? {
        return Ok(command);
    }
    if !input.network.is_empty() {
        return Err(ToolError::invalid_input(
            "offline OS sandbox denies network requests; a domain-aware network proxy is not implemented, so this request cannot be run in offline mode",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (executor, command, env);
        Err(ToolError::invalid_input(
            "required shell OS sandbox is currently implemented only on macOS; no unconfined fallback was used",
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let reads = input
            .reads
            .iter()
            .map(|path| executor.resolve_target_path(path))
            .collect::<Vec<_>>();
        let writes = input
            .writes
            .iter()
            .map(|path| executor.resolve_target_path(path))
            .collect::<Vec<_>>();
        let wrapped = macos_command(command, &reads, &writes)?;
        // A filesystem sandbox cannot hide API keys inherited in environment.
        env.retain(|key, _| {
            matches!(
                key.as_str(),
                "PATH" | "LANG" | "HOME" | "USER" | "LOGNAME" | "TERM" | "COLORTERM" | "TMPDIR"
            ) || key.starts_with("LC_")
        });
        env.insert("PYTHONDONTWRITEBYTECODE".into(), "1".into());
        Ok(wrapped)
    }
}
#[cfg(target_os = "macos")]
fn quoted(path: &Path) -> Result<String, ToolError> {
    let path = path
        .to_str()
        .ok_or_else(|| ToolError::invalid_input("sandbox paths must be UTF-8"))?;
    if path.chars().any(char::is_control) {
        return Err(ToolError::invalid_input(
            "sandbox paths cannot contain control characters",
        ));
    }
    serde_json::to_string(path).map_err(|error| ToolError::invalid_input_error(&error))
}
#[cfg(target_os = "macos")]
fn macos_command(
    command: Vec<String>,
    reads: &[std::path::PathBuf],
    writes: &[std::path::PathBuf],
) -> Result<Vec<String>, ToolError> {
    if !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err(ToolError::invalid_input(
            "required OS sandbox launcher is unavailable; command was not started",
        ));
    }
    let mut policy = String::from(
        r#"(version 1)
(deny default)
(allow process-exec process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow sysctl-read)
(allow file-read-metadata)
(allow system-mac-syscall (mac-policy-name "vnguard"))
(allow system-mac-syscall (require-all (mac-policy-name "Sandbox") (mac-syscall-number 67)))
(allow file-read* (literal "/"))
(allow file-read* file-map-executable
 (subpath "/bin") (subpath "/sbin") (subpath "/usr/bin") (subpath "/usr/sbin")
 (subpath "/usr/lib") (subpath "/usr/share") (subpath "/System/Library")
 (subpath "/Library/Apple") (subpath "/Library/Developer/CommandLineTools")
 (subpath "/Applications/Xcode.app/Contents/Developer")
 (subpath "/private/var/db/timezone")
 (literal "/private/etc/localtime") (literal "/private/etc/passwd")
 (literal "/dev/null") (literal "/dev/zero") (literal "/dev/random") (literal "/dev/urandom")
 (subpath "/dev/fd"))
(allow file-write-data (literal "/dev/null") (literal "/dev/zero") (subpath "/dev/fd"))
(allow pseudo-tty)
(allow file-ioctl (regex #"^/dev/ttys[0-9]+"))
(allow file-read* file-write* file-ioctl (literal "/dev/ptmx"))
(allow mach-lookup (global-name "com.apple.system.opendirectoryd.libinfo"))
"#,
    );
    for path in reads.iter().chain(writes) {
        let actual = crate::canonicalize_mutation_path(path);
        policy.push_str(&format!(
            "(allow file-read* file-map-executable ({} {}))\n",
            if actual.is_dir() {
                "subpath"
            } else {
                "literal"
            },
            quoted(&actual)?
        ));
    }
    for path in writes {
        let actual = crate::canonicalize_mutation_path(path);
        policy.push_str(&format!(
            "(allow file-write* ({} {}))\n",
            if actual.is_dir() {
                "subpath"
            } else {
                "literal"
            },
            quoted(&actual)?
        ));
    }
    let mut wrapped = vec!["/usr/bin/sandbox-exec".into(), "-p".into(), policy];
    wrapped.extend(command);
    Ok(wrapped)
}
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn run(
        script: &str,
        reads: &[std::path::PathBuf],
        writes: &[std::path::PathBuf],
    ) -> std::process::Output {
        let command = macos_command(
            vec!["/bin/sh".into(), "-c".into(), script.into()],
            reads,
            writes,
        )
        .unwrap();
        let result = std::process::Command::new(&command[0])
            .args(&command[1..])
            .current_dir("/")
            .output()
            .unwrap();
        eprintln!(
            "sandbox fixture exit={:?}, stdout={:?}, stderr={:?}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }
    #[test]
    fn os_denies_undeclared_contents_and_writes_even_for_descendants() {
        let root = tempfile::tempdir().unwrap();
        let allowed = root.path().join("allowed");
        let denied = root.path().join("denied");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&denied).unwrap();
        std::fs::write(denied.join("secret.txt"), "SECRET_FIXTURE").unwrap();
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
        let q = |path: &Path| quote(path.to_str().unwrap());
        let ok = run(
            &format!("printf allowed > {}", q(&allowed.join("output.txt"))),
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed),
        );
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        let fail = run(
            &format!(
                "/bin/sh -c {}",
                quote(&format!("cat {}", q(&denied.join("secret.txt"))))
            ),
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed),
        );
        assert!(!fail.status.success());
        assert!(!String::from_utf8_lossy(&fail.stdout).contains("SECRET_FIXTURE"));
        let fail = run(
            &format!("printf prohibited > {}", q(&denied.join("output.txt"))),
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed),
        );
        assert!(!fail.status.success());
        assert!(!denied.join("output.txt").exists());
    }
    #[test]
    fn os_denies_loopback_network_access() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        let python = [
            "/Library/Developer/CommandLineTools/usr/bin/python3",
            "/Applications/Xcode.app/Contents/Developer/usr/bin/python3",
        ]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .expect("a real system Python is required for the network sandbox test");
        let script = format!(
            "import socket,sys\nprint(\"SOCKET_ATTEMPT\",flush=True)\ntry: socket.create_connection((\"127.0.0.1\",{port}),1)\nexcept PermissionError: print(\"NETWORK_DENIED\",flush=True);sys.exit(0)\nelse: sys.exit(2)"
        );
        let command = macos_command(vec![python.into(), "-c".into(), script], &[], &[]).unwrap();
        let result = std::process::Command::new(&command[0])
            .args(&command[1..])
            .current_dir("/")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{:?}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8_lossy(&result.stdout);
        assert!(output.contains("SOCKET_ATTEMPT"));
        assert!(output.contains("NETWORK_DENIED"));
    }
}
