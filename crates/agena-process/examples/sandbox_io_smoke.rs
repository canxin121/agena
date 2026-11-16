//! Real native I/O checks: cargo run -p agena-process --example sandbox_io_smoke
//! Defaults to a local interface fixture. --files-only uses no network;
//! --https-only uses a read-only public GitHub endpoint without credentials.
use agena_process::{
    sandbox::{SandboxNetwork, SandboxPolicy},
    sandbox_http::{self, HttpOptions},
};
use futures_util::StreamExt;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(code) = agena_process::sandbox::try_run_launcher() {
        std::process::exit(code);
    }
    if let Some(code) = sandbox_http::try_run_worker() {
        std::process::exit(code);
    }
    if let Some(code) = agena_process::sandbox_file::try_run_worker() {
        std::process::exit(code);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if std::env::args().any(|argument| argument == "--https-only") {
        runtime.block_on(https_checks())
    } else {
        runtime.block_on(checks())
    }
}

/// A read-only public fixture for environments that cannot authorize a local
/// listener. This covers native HTTPS and offline denial; the local fixture
/// separately exercises controlled SSE timing and server-observed cancellation.
async fn https_checks() -> Result<(), Box<dyn std::error::Error>> {
    let addresses = tokio::net::lookup_host(("github.com", 443))
        .await?
        .filter(|address| address.is_ipv4())
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("GitHub fixture has no IPv4 address".into());
    }
    let allow = addresses
        .iter()
        .map(|address| {
            serde_json::json!({
                "to": address.ip().to_string(), "protocol": "tcp", "ports": [443]
            })
        })
        .collect::<Vec<_>>();
    let options = HttpOptions {
        policy: SandboxPolicy {
            configured_filesystem: true,
            network: SandboxNetwork::Native {
                policy: serde_json::from_value(
                    serde_json::json!({"default": "deny", "allow": allow}),
                )?,
            },
            ..Default::default()
        },
        pinned_addresses: addresses,
        connect_timeout: Duration::from_secs(10),
        total_timeout: Some(Duration::from_secs(30)),
        max_request_bytes: 1024,
        max_response_bytes: Some(1024 * 1024),
        cancellation: None,
    };
    let request = || {
        let mut request = reqwest::Request::new(
            reqwest::Method::GET,
            "https://github.com/robots.txt".parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("user-agent", "Agena-Sandbox-Smoke".parse().unwrap());
        request
    };
    let response = sandbox_http::execute(request(), options.clone()).await?;
    assert!(
        response.status().is_success(),
        "public HTTPS fixture returned {}",
        response.status()
    );
    assert_eq!(response.url().as_str(), "https://github.com/robots.txt");
    let bytes = response.bytes().await?;
    assert!(!bytes.is_empty());
    println!(
        "PASS: native pinned HTTPS, verified TLS, response URL and complete framed body ({} bytes)",
        bytes.len()
    );
    let mut offline = options;
    offline.policy.network = SandboxNetwork::Disabled;
    offline.connect_timeout = Duration::from_secs(1);
    offline.total_timeout = Some(Duration::from_secs(5));
    assert!(
        sandbox_http::execute(request(), offline).await.is_err(),
        "offline worker reached public HTTPS"
    );
    println!("PASS: offline MXC denies the same HTTPS request without host fallback");
    Ok(())
}

fn fixture_address() -> io::Result<IpAddr> {
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    route.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    let ip = route.local_addr()?.ip();
    if ip.is_unspecified() {
        return Err(io::Error::other(
            "no local interface for native network fixture",
        ));
    }
    Ok(ip)
}

fn options(address: SocketAddr) -> HttpOptions {
    HttpOptions {
        policy: SandboxPolicy {
            configured_filesystem: true,
            network: SandboxNetwork::Native {
                policy: serde_json::from_value(serde_json::json!({
                    "default": "deny", "allow": [{"to": address.ip().to_string(), "protocol": "tcp", "ports": [address.port()]}]
                })).unwrap(),
            },
            ..Default::default()
        },
        pinned_addresses: vec![address],
        connect_timeout: Duration::from_secs(4),
        total_timeout: Some(Duration::from_secs(15)),
        max_request_bytes: 1024 * 1024,
        max_response_bytes: Some(1024 * 1024),
        cancellation: None,
    }
}

fn request(address: SocketAddr, path: &str) -> reqwest::Request {
    // This name has no usable DNS record. Successful delivery proves that the
    // worker retained the approved address while preserving the original Host.
    reqwest::Request::new(
        reqwest::Method::GET,
        format!("http://worker-fixture.invalid:{}/{path}", address.port())
            .parse()
            .unwrap(),
    )
}

async fn checks() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let readable = directory.path().join("input");
    let secret = directory.path().join("secret");
    std::fs::write(&readable, b"approved-data")?;
    std::fs::write(&secret, b"secret-sentinel")?;
    let file_policy = SandboxPolicy {
        readonly_paths: vec![readable.clone()],
        configured_filesystem: true,
        ..Default::default()
    };
    assert_eq!(
        agena_process::sandbox_file::read(&readable, 32, None, &file_policy).await?,
        b"approved-data"
    );
    assert!(
        agena_process::sandbox_file::read(&secret, 32, None, &file_policy)
            .await
            .is_err()
    );
    // Also probe the OS boundary directly, bypassing the parent helper's policy
    // membership check: the worker must still be unable to open the secret.
    let (_scratch, command) = agena_process::worker::command("--agena-file-worker", &file_policy)?;
    let input =
        serde_json::to_vec(&serde_json::json!({"path": secret, "maximum": 32, "expected": null}))?;
    let result =
        agena_process::output_with_input(command, &input, Duration::from_secs(15), 64 * 1024)
            .await?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout.first(),
        Some(&1),
        "native worker exposed an ungranted file"
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("secret-sentinel"));
    println!("PASS: MXC file read, parent scope check, native denial of ungranted file");
    if std::env::args().any(|argument| argument == "--files-only") {
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind((fixture_address()?, 0)).await?;
    let address = listener.local_addr()?;
    let release = Arc::new(tokio::sync::Notify::new());
    let (events, mut observed) = tokio::sync::mpsc::unbounded_channel::<String>();
    let server_release = release.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await?;
            let release = server_release.clone();
            let events = events.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let header_end = loop {
                    if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                    let mut chunk = [0; 4096];
                    let length = socket.read(&mut chunk).await?;
                    if length == 0 || buffer.len() > 64 * 1024 {
                        return Err(io::Error::other("invalid fixture request"));
                    }
                    buffer.extend_from_slice(&chunk[..length]);
                };
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                let path = header.split_whitespace().nth(1).unwrap_or("").to_owned();
                assert!(
                    header
                        .to_ascii_lowercase()
                        .contains("host: worker-fixture.invalid:")
                );
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while buffer.len() - header_end < content_length {
                    let mut chunk = [0; 8192];
                    let length = socket.read(&mut chunk).await?;
                    if length == 0 {
                        return Err(io::Error::other("incomplete upload"));
                    }
                    buffer.extend_from_slice(&chunk[..length]);
                }
                match path.as_str() {
                    "/upload" => {
                        assert_eq!(content_length, 256 * 1024);
                        assert!(buffer[header_end..].iter().all(|byte| *byte == b'x'));
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\nSet-Cookie: first=1\r\nSet-Cookie: second=2\r\n\r\nOK").await?;
                    }
                    "/sse" => {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nb\r\ndata: one\n\n\r\n").await?;
                        release.notified().await;
                        socket.write_all(b"b\r\ndata: two\n\n\r\n0\r\n\r\n").await?;
                    }
                    "/cancel" | "/lease" | "/waiting" => {
                        if path != "/waiting" {
                            socket
                                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                                .await?;
                        }
                        let _ = events.send(format!("opened:{path}"));
                        let mut probe = [0; 1];
                        match socket.read(&mut probe).await {
                            Ok(0) | Err(_) => {
                                let _ = events.send(format!("closed:{path}"));
                            }
                            Ok(_) => {
                                return Err(io::Error::other("unexpected data after HTTP request"));
                            }
                        }
                    }
                    "/denied" => {
                        let _ = events.send("denied-reached".into());
                    }
                    _ => return Err(io::Error::other("unexpected fixture path")),
                }
                Ok::<_, io::Error>(())
            });
        }
        #[allow(unreachable_code)]
        Ok::<_, io::Error>(())
    });
    let mut upload = request(address, "upload");
    *upload.method_mut() = reqwest::Method::POST;
    upload
        .headers_mut()
        .insert("content-length", (256 * 1024_u64).into());
    *upload.body_mut() = Some(reqwest::Body::wrap_stream(futures_util::stream::iter(
        (0..4).map(|_| Ok::<_, io::Error>(vec![b'x'; 64 * 1024])),
    )));
    let result = sandbox_http::execute(upload, options(address)).await?;
    assert_eq!(result.url().host_str(), Some("worker-fixture.invalid"));
    assert_eq!(result.headers().get_all("set-cookie").iter().count(), 2);
    assert_eq!(result.text().await?, "OK");
    println!(
        "PASS: native pinned HTTP upload, original URL, duplicate headers, no workspace read grant"
    );

    let response = sandbox_http::execute(request(address, "sse"), options(address)).await?;
    let mut chunks = response.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(5), chunks.next())
        .await?
        .ok_or("missing SSE event")??;
    assert_eq!(first.as_ref(), b"data: one\n\n");
    release.notify_one();
    let mut tail = Vec::new();
    while let Some(chunk) = chunks.next().await {
        tail.extend_from_slice(&chunk?);
    }
    assert_eq!(tail, b"data: two\n\n");
    println!("PASS: streaming headers and SSE arrive before the response completes");

    let response = sandbox_http::execute(request(address, "cancel"), options(address)).await?;
    drop(response);
    let wait_closed = async {
        while let Some(event) = observed.recv().await {
            if event == "closed:/cancel" {
                return Ok::<_, io::Error>(());
            }
        }
        Err(io::Error::other("fixture event channel closed"))
    };
    tokio::time::timeout(Duration::from_secs(8), wait_closed).await??;
    let cancellation = sandbox_http::WorkerCancellation::new();
    let mut scoped = options(address);
    scoped.cancellation = Some(cancellation.clone());
    let retained = sandbox_http::execute(request(address, "lease"), scoped).await?;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(8), async {
        while observed.recv().await.as_deref() != Some("closed:/lease") {}
    })
    .await?;
    drop(retained);
    let waiting = tokio::spawn(sandbox_http::execute(
        request(address, "waiting"),
        options(address),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        while observed.recv().await.as_deref() != Some("opened:/waiting") {}
    })
    .await?;
    waiting.abort();
    let _ = waiting.await;
    tokio::time::timeout(Duration::from_secs(8), async {
        while observed.recv().await.as_deref() != Some("closed:/waiting") {}
    })
    .await?;
    println!(
        "PASS: drop, origin cancellation of an unpolled body, and cancellation before headers close native connections"
    );

    let mut offline = options(address);
    offline.policy.network = SandboxNetwork::Disabled;
    offline.connect_timeout = Duration::from_secs(1);
    offline.total_timeout = Some(Duration::from_secs(4));
    assert!(
        sandbox_http::execute(request(address, "denied"), offline)
            .await
            .is_err()
    );
    while let Ok(event) = observed.try_recv() {
        assert_ne!(event, "denied-reached");
    }
    server.abort();
    println!("PASS: offline MXC HTTP failure does not fall back to host networking");
    Ok(())
}
