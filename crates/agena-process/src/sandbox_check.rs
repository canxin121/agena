//! Installation-time native sandbox verification without loading application state.
//!
//! The installed executable is its own probe and launcher, so the check exercises
//! the same AppArmor attachment and runtime path as the service that will run.

use crate::sandbox::{ProxyTarget, SandboxNetwork, SandboxPolicy};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
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
const FIXTURE_HOST: &str = "agena-sandbox-check.invalid";
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
    if cfg!(windows) {
        checks.push(CheckResult {
            name: "http_proxy",
            required: false,
            status: CheckStatus::Unsupported,
            detail: "Windows identity-bound AppContainer HTTP proxy support is not implemented; offline isolation remains required".into(),
        });
    } else if cfg!(any(target_os = "linux", target_os = "macos")) {
        checks.push(result("http_proxy", proxy_check()));
    } else {
        checks.push(CheckResult {
            name: "http_proxy",
            required: true,
            status: CheckStatus::Unsupported,
            detail: "HTTP proxy isolation has no supported backend on this platform".into(),
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
    let [mode, first, second] = arguments.as_slice() else {
        return Err(io::Error::other("invalid private sandbox check arguments"));
    };
    match mode.to_str() {
        Some("offline") => {
            let port = parse_port(second)?;
            offline_probe(Path::new(first), port)
        }
        Some("http_proxy") => {
            let port = parse_port(first)?;
            let expected = second
                .to_str()
                .ok_or_else(|| io::Error::other("invalid fixture response"))?;
            let proxy = std::env::var("HTTP_PROXY")
                .map_err(|_| io::Error::other("MXC did not provide an HTTP proxy endpoint"))?;
            verify_proxy(&proxy, port, expected)
        }
        _ => Err(io::Error::other("unknown private sandbox check")),
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

fn proxy_check() -> io::Result<&'static str> {
    let root = fixture()?;
    let endpoint = LocalEndpoint::start()?;
    let policy = SandboxPolicy {
        readonly_paths: vec![root.path().to_path_buf()],
        private_paths: vec![root.path().to_path_buf()],
        network: SandboxNetwork::HttpProxy {
            targets: vec![endpoint.target()],
        },
        timeout_ms: Some(PROBE_TIMEOUT.as_millis() as u64),
        ..Default::default()
    };
    invoke(
        &[
            "http_proxy".into(),
            endpoint.port.to_string(),
            endpoint.response.clone(),
        ],
        root.path(),
        &policy,
    )?;
    Ok(
        "authenticated MXC HTTP proxy reached the pinned local fixture; unapproved destination denied; no external network used",
    )
}

fn verify_proxy(proxy: &str, port: u16, expected: &str) -> io::Result<()> {
    let response = proxy_request(proxy, FIXTURE_HOST, port)?;
    require(
        response.starts_with("HTTP/1.1 200 ")
            && response
                .split_once("\r\n\r\n")
                .is_some_and(|(_, body)| body == expected),
        "approved local HTTP fixture was not reached through the MXC proxy",
    )?;
    let denied = proxy_request(proxy, "unapproved-sandbox-check.invalid", port)?;
    require(
        denied.starts_with("HTTP/1.1 403 "),
        "HTTP proxy did not reject an unapproved destination",
    )
}

fn proxy_request(proxy: &str, host: &str, port: u16) -> io::Result<String> {
    let proxy =
        url::Url::parse(proxy).map_err(|_| io::Error::other("invalid MXC proxy endpoint"))?;
    let address = match proxy.host() {
        Some(url::Host::Ipv4(address)) => IpAddr::V4(address),
        Some(url::Host::Ipv6(address)) => IpAddr::V6(address),
        _ => {
            return Err(io::Error::other(
                "MXC proxy endpoint must have a numeric address",
            ));
        }
    };
    let proxy_port = proxy
        .port_or_known_default()
        .ok_or_else(|| io::Error::other("MXC proxy endpoint has no port"))?;
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(address, proxy_port),
        Duration::from_secs(3),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let authorization = STANDARD.encode(format!(
        "{}:{}",
        proxy.username(),
        proxy.password().unwrap_or_default()
    ));
    write!(
        stream,
        "GET http://{host}:{port}/probe HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {authorization}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = Vec::new();
    stream
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut response)?;
    require(
        response.len() <= OUTPUT_LIMIT,
        "HTTP fixture response exceeded its limit",
    )?;
    String::from_utf8(response).map_err(|_| io::Error::other("invalid HTTP fixture response"))
}

struct LocalEndpoint {
    port: u16,
    response: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LocalEndpoint {
    fn start() -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let response = uuid::Uuid::new_v4().simple().to_string();
        let body = response.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("agena-sandbox-check-http".into())
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

    fn target(&self) -> ProxyTarget {
        ProxyTarget {
            host: FIXTURE_HOST.into(),
            port: Some(self.port),
            addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        }
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
    let mut header = Vec::new();
    while header.len() < 4096 && Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        let mut buffer = [0; 512];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                header.extend_from_slice(&buffer[..count]);
                if header.windows(4).any(|part| part == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&header);
                    require(
                        header.starts_with("GET /probe HTTP/1.1\r\n")
                            && !header.to_ascii_lowercase().contains("proxy-authorization:"),
                        "proxy fixture received an invalid request or leaked authorization",
                    )?;
                    return write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "HTTP fixture request did not complete",
    ))
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
                name: "http_proxy",
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
            name: "http_proxy",
            required: true,
            status: CheckStatus::Unsupported,
            detail: "required backend missing".into(),
        };
        assert!(!CheckReport::finish(Instant::now(), vec![unsupported]).ok);
    }

    #[test]
    fn local_http_fixture_checks_authentication_pinning_and_denied_destinations() {
        let endpoint = LocalEndpoint::start().unwrap();
        let proxy = crate::sandbox_proxy::ProxyBroker::start(&[endpoint.target()]).unwrap();
        verify_proxy(proxy.url(), endpoint.port, &endpoint.response).unwrap();
        let mut bad = url::Url::parse(proxy.url()).unwrap();
        bad.set_password(Some("incorrect-fixture-password"))
            .unwrap();
        let response = proxy_request(bad.as_str(), FIXTURE_HOST, endpoint.port).unwrap();
        assert!(response.starts_with("HTTP/1.1 407 "));
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
    fn incomplete_http_fixture_clients_do_not_delay_cleanup() {
        let endpoint = LocalEndpoint::start().unwrap();
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        client.write_all(b"GET /probe").unwrap();
        thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        drop(endpoint);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
