//! Installation-time native sandbox verification without loading application state.
//!
//! The installed executable is its own probe and launcher, so the check exercises
//! the same AppArmor attachment and runtime path as the service that will run.

use crate::sandbox::{SandboxNetwork, SandboxPolicy};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHECK_ARGUMENT: &str = "--agena-sandbox-check";
const PROBE_ARGUMENT: &str = "--agena-sandbox-check-probe";
const ENV_SENTINEL: &str = "AGENA_SANDBOX_CHECK_HOST_SECRET";
const FIXTURE_REQUEST: &[u8] = b"agena-native-network-check\n";
const OUTPUT_LIMIT: usize = 16 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const PARENT_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Unsupported,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: &'static str,
    pub required: bool,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub schema_version: u32,
    pub ok: bool,
    pub platform: &'static str,
    pub duration_ms: u64,
    pub checks: Vec<CheckResult>,
}

impl CheckReport {
    fn finish(started: Instant, checks: Vec<CheckResult>) -> Self {
        Self {
            schema_version: 1,
            ok: checks
                .iter()
                .all(|check| !check.required || check.status == CheckStatus::Passed),
            platform: std::env::consts::OS,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            checks,
        }
    }
}

/// Handle only the private diagnostic arguments, before normal CLI/config setup.
/// The public check writes exactly one JSON report to stdout.
pub fn try_run_cli() -> Option<i32> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next()?;
    if first == CHECK_ARGUMENT {
        let report = if arguments.next().is_none() {
            run()
        } else {
            CheckReport::finish(
                Instant::now(),
                vec![failed(
                    "arguments",
                    "sandbox check accepts no additional arguments",
                )],
            )
        };
        let code = i32::from(!report.ok);
        let mut stdout = io::stdout().lock();
        return Some(
            if serde_json::to_writer(&mut stdout, &report).is_ok()
                && stdout.write_all(b"\n").is_ok()
            {
                code
            } else {
                1
            },
        );
    }
    if first == PROBE_ARGUMENT {
        return Some(match probe(arguments.collect()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("sandbox self-check probe failed: {error}");
                1
            }
        });
    }
    None
}

/// Check real native processes with temporary fixtures and no external network.
pub fn run() -> CheckReport {
    let started = Instant::now();
    let mut checks = vec![result("offline", offline_check())];
    if cfg!(any(target_os = "linux", windows)) {
        checks.push(result("native_network", native_network_check()));
    } else if cfg!(target_os = "macos") {
        checks.push(CheckResult {
            name: "native_network",
            required: false,
            status: CheckStatus::Unsupported,
            detail: "macOS MXC Seatbelt supports egress defaults, but not IP/protocol/port allow and deny rules".into(),
        });
    } else {
        checks.push(CheckResult {
            name: "native_network",
            required: true,
            status: CheckStatus::Unsupported,
            detail: "native MXC networking has no supported backend on this platform".into(),
        });
    }
    CheckReport::finish(started, checks)
}

fn result(name: &'static str, outcome: io::Result<&'static str>) -> CheckResult {
    match outcome {
        Ok(detail) => CheckResult {
            name,
            required: true,
            status: CheckStatus::Passed,
            detail: detail.into(),
        },
        Err(error) => failed(name, &error.to_string()),
    }
}

fn failed(name: &'static str, detail: &str) -> CheckResult {
    CheckResult {
        name,
        required: true,
        status: CheckStatus::Failed,
        detail: detail.chars().take(8_192).collect(),
    }
}

fn fixture() -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("agena-tool-worker-sandbox-check-")
        .tempdir()
}

fn offline_check() -> io::Result<&'static str> {
    let root = fixture()?;
    let input = root.path().join("input");
    let output = root.path().join("output");
    let secret = input.join("secret");
    let outside = root.path().join("ungranted");
    for directory in [&input, &output, &secret, &outside] {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(input.join("data"), b"input-fixture")?;
    std::fs::write(secret.join("key"), b"secret-fixture")?;
    std::fs::write(outside.join("data"), b"outside-fixture")?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let policy = SandboxPolicy {
        readonly_paths: vec![input.clone()],
        readwrite_paths: vec![output.clone()],
        denied_paths: vec![secret],
        private_paths: vec![root.path().to_path_buf()],
        timeout_ms: Some(PROBE_TIMEOUT.as_millis() as u64),
        ..Default::default()
    };
    invoke(
        &[
            "offline".into(),
            path_argument(root.path())?,
            listener.local_addr()?.port().to_string(),
        ],
        &output,
        &policy,
    )?;
    require(
        std::fs::read(input.join("data"))? == b"input-fixture",
        "readonly input changed on the host",
    )?;
    require(
        std::fs::read(output.join("result"))? == b"output-fixture",
        "sandbox output was not written to the authorized directory",
    )?;
    Ok(
        "input read; input writes denied; output write; secret and ungranted paths denied; host loopback denied; environment cleared; private HOME writable",
    )
}

fn invoke(arguments: &[String], cwd: &Path, policy: &SandboxPolicy) -> io::Result<()> {
    let mut argv = vec![
        path_argument(&crate::sandbox::launcher_executable()?)?,
        PROBE_ARGUMENT.into(),
    ];
    argv.extend_from_slice(arguments);
    // Read only runtime routing variables. No application configuration or
    // credential store is opened, and the only secret-like value is a fixture.
    let mut environment: HashMap<_, _> = [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
    .collect();
    environment.insert(ENV_SENTINEL.into(), "diagnostic-sentinel".into());
    let argv = crate::sandbox::wrap_argv(&argv, cwd, &mut environment, policy)?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null());
    let output = crate::blocking::output(command, PARENT_TIMEOUT, OUTPUT_LIMIT)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "native probe exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn probe(arguments: Vec<OsString>) -> io::Result<()> {
    let (mode, arguments) = arguments
        .split_first()
        .ok_or_else(|| io::Error::other("missing private sandbox check mode"))?;
    match (mode.to_str(), arguments) {
        (Some("offline"), [root, port]) => offline_probe(Path::new(root), parse_port(port)?),
        (Some("native_network"), [address, denied_port, expected]) => {
            let address = address
                .to_str()
                .and_then(|value| value.parse::<SocketAddr>().ok())
                .ok_or_else(|| io::Error::other("invalid local fixture address"))?;
            let expected = expected
                .to_str()
                .ok_or_else(|| io::Error::other("invalid fixture response"))?;
            verify_native_network(address, parse_port(denied_port)?, expected)
        }
        _ => Err(io::Error::other("invalid private sandbox check arguments")),
    }
}

fn parse_port(value: &OsString) -> io::Result<u16> {
    value
        .to_str()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| io::Error::other("invalid fixture port"))
}

fn offline_probe(root: &Path, port: u16) -> io::Result<()> {
    require(
        std::fs::read(root.join("input/data"))? == b"input-fixture",
        "authorized input could not be read",
    )?;
    require(
        std::fs::write(root.join("input/data"), b"modified").is_err(),
        "readonly input remained writable",
    )?;
    std::fs::write(root.join("output/result"), b"output-fixture")?;
    require(
        std::fs::read(root.join("input/secret/key")).is_err(),
        "explicitly denied secret remained readable",
    )?;
    require(
        std::fs::read(root.join("ungranted/data")).is_err(),
        "ungranted file remained readable",
    )?;
    require(
        TcpStream::connect_timeout(
            &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            Duration::from_millis(400),
        )
        .is_err(),
        "offline sandbox could reach a host loopback listener",
    )?;
    require(
        std::env::var_os(ENV_SENTINEL).is_none(),
        "parent environment sentinel leaked",
    )?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("private HOME is missing"))?;
    require(home.is_absolute(), "private HOME is not absolute")?;
    require(!home.starts_with(root), "HOME overlaps the caller fixture")?;
    std::fs::write(home.join("sandbox-check-cache"), b"private-fixture")?;
    Ok(())
}

fn native_network_check() -> io::Result<&'static str> {
    let root = fixture()?;
    let address = local_fixture_address()?;
    let allowed = LocalEndpoint::start(address)?;
    let denied = LocalEndpoint::start(address)?;
    let policy = SandboxPolicy {
        readonly_paths: vec![root.path().to_path_buf()],
        private_paths: vec![root.path().to_path_buf()],
        network: SandboxNetwork::Native {
            policy: serde_json::from_value(serde_json::json!({
                "default": "deny",
                "allow": [{
                    "to": address.to_string(),
                    "protocol": "tcp",
                    "ports": [allowed.port, denied.port]
                }],
                "deny": [{
                    "to": address.to_string(),
                    "protocol": "tcp",
                    "ports": [denied.port]
                }]
            }))
            .map_err(io::Error::other)?,
        },
        timeout_ms: Some(PROBE_TIMEOUT.as_millis() as u64),
        ..Default::default()
    };
    invoke(
        &[
            "native_network".into(),
            SocketAddr::new(IpAddr::V4(address), allowed.port).to_string(),
            denied.port.to_string(),
            allowed.response.clone(),
        ],
        root.path(),
        &policy,
    )?;
    Ok(
        "native MXC TCP allow reached a local interface fixture; explicit deny overrode allow; no proxy or external network used",
    )
}

fn local_fixture_address() -> io::Result<Ipv4Addr> {
    // UDP connect only selects a route and local interface; no packet is sent.
    // The fixture then binds and connects exclusively to that local interface.
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    route.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    match route.local_addr()?.ip() {
        IpAddr::V4(address) if !address.is_loopback() && !address.is_unspecified() => Ok(address),
        _ => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "native network self-check requires a local non-loopback IPv4 interface",
        )),
    }
}

fn verify_native_network(allowed: SocketAddr, denied_port: u16, expected: &str) -> io::Result<()> {
    require(
        fixture_request(allowed)? == expected,
        "native MXC allow rule did not reach the local fixture",
    )?;
    require(
        TcpStream::connect_timeout(
            &SocketAddr::new(allowed.ip(), denied_port),
            Duration::from_millis(700),
        )
        .is_err(),
        "native MXC deny rule did not block a destination also covered by allow",
    )?;
    require(
        !std::env::vars_os().any(|(name, _)| {
            matches!(
                name.to_string_lossy().to_ascii_uppercase().as_str(),
                "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "FTP_PROXY"
            )
        }),
        "native sandbox unexpectedly inherited proxy configuration",
    )
}

fn fixture_request(address: SocketAddr) -> io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(FIXTURE_REQUEST)?;
    let mut response = Vec::new();
    stream
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut response)?;
    require(
        response.len() <= OUTPUT_LIMIT,
        "native network fixture response exceeded its limit",
    )?;
    String::from_utf8(response).map_err(|_| io::Error::other("invalid fixture response"))
}

struct LocalEndpoint {
    port: u16,
    response: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LocalEndpoint {
    fn start(address: Ipv4Addr) -> io::Result<Self> {
        let listener = TcpListener::bind((address, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let response = uuid::Uuid::new_v4().simple().to_string();
        let body = response.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("agena-sandbox-check-native-network".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = serve_fixture(&mut stream, &body, &worker_stop);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            port,
            response,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for LocalEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_fixture(stream: &mut TcpStream, body: &str, stop: &AtomicBool) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(Duration::from_millis(200)))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut request = Vec::new();
    while request.len() < FIXTURE_REQUEST.len()
        && Instant::now() < deadline
        && !stop.load(Ordering::Acquire)
    {
        let mut buffer = [0; 64];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => request.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error),
        }
    }
    require(
        request == FIXTURE_REQUEST,
        "native network fixture received an invalid request",
    )?;
    stream.write_all(body.as_bytes())
}

fn path_argument(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("sandbox check requires a UTF-8 installation/temp path"))
}

fn require(condition: bool, message: &'static str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contract_requires_offline_but_allows_explicit_optional_unsupported() {
        let mut checks = vec![
            result("offline", Ok("fixture passed")),
            CheckResult {
                name: "native_network",
                required: false,
                status: CheckStatus::Unsupported,
                detail: "platform limitation".into(),
            },
        ];
        let successful = CheckReport::finish(Instant::now(), std::mem::take(&mut checks));
        assert!(successful.ok);
        let json = serde_json::to_value(&successful).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["checks"][0]["status"], "passed");
        assert_eq!(json["checks"][1]["status"], "unsupported");
        assert_eq!(json["checks"][1]["required"], false);
        assert!(!CheckReport::finish(Instant::now(), vec![failed("offline", "blocked")]).ok);
        let unsupported = CheckResult {
            name: "native_network",
            required: true,
            status: CheckStatus::Unsupported,
            detail: "required backend missing".into(),
        };
        assert!(!CheckReport::finish(Instant::now(), vec![unsupported]).ok);
    }

    #[test]
    fn native_network_fixture_detects_an_unrestricted_process() {
        let allowed = LocalEndpoint::start(Ipv4Addr::LOCALHOST).unwrap();
        let denied = LocalEndpoint::start(Ipv4Addr::LOCALHOST).unwrap();
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allowed.port);
        assert_eq!(fixture_request(address).unwrap(), allowed.response);
        let error = verify_native_network(address, denied.port, &allowed.response).unwrap_err();
        assert!(error.to_string().contains("deny rule did not block"));
    }

    #[test]
    fn offline_probe_detects_an_unisolated_process() {
        let root = fixture().unwrap();
        std::fs::create_dir(root.path().join("input")).unwrap();
        std::fs::write(root.path().join("input/data"), b"input-fixture").unwrap();
        let error = offline_probe(root.path(), 1).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("readonly input remained writable")
        );
    }

    #[test]
    fn incomplete_native_fixture_clients_do_not_delay_cleanup() {
        let endpoint = LocalEndpoint::start(Ipv4Addr::LOCALHOST).unwrap();
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        client.write_all(b"agena-native").unwrap();
        thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        drop(endpoint);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
