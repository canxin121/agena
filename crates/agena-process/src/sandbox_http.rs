//! A single HTTP hop executed inside MXC. DNS authorization and redirects stay
//! with the caller; the worker connects only to the supplied addresses. Request
//! and response bodies stream over anonymous pipes, including multipart and SSE.
use crate::{ManagedChild, sandbox::SandboxPolicy, worker_ipc as ipc};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use reqwest::ResponseBuilderExt;
use serde::{Deserialize, Serialize};
use std::{io, net::SocketAddr, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;
pub use tokio_util::sync::CancellationToken as WorkerCancellation;

mod lifetime;
mod upload;
use lifetime::{RequestLifetime, pump_body};

const ARGUMENT: &str = "--agena-http-worker";
// A 512 MiB upload may additionally contain bounded multipart framing.
const MAX_REQUEST: u64 = 513 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct HttpOptions {
    pub policy: SandboxPolicy,
    pub pinned_addresses: Vec<SocketAddr>,
    pub connect_timeout: Duration,
    pub total_timeout: Option<Duration>,
    pub max_request_bytes: u64,
    pub max_response_bytes: Option<u64>,
    pub cancellation: Option<WorkerCancellation>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            policy: SandboxPolicy::default(),
            pinned_addresses: Vec::new(),
            connect_timeout: Duration::from_secs(30),
            total_timeout: None,
            max_request_bytes: 512 * 1024 * 1024,
            max_response_bytes: None,
            cancellation: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestHead {
    method: String,
    url: String,
    headers: Vec<(String, Vec<u8>)>,
    version: String,
    has_body: bool,
    addresses: Vec<SocketAddr>,
    connect_ms: u64,
    total_ms: Option<u64>,
    max_request: u64,
    max_response: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseHead {
    status: u16,
    url: String,
    version: String,
    headers: Vec<(String, Vec<u8>)>,
    #[serde(default)]
    remote_addr: Option<SocketAddr>,
}

#[derive(Clone, Copy)]
struct ResponsePeer(SocketAddr);

/// The numeric address used by the worker, including sandboxed requests.
pub fn response_peer_addr(response: &reqwest::Response) -> Option<SocketAddr> {
    response
        .extensions()
        .get::<ResponsePeer>()
        .map(|peer| peer.0)
        .or_else(|| response.remote_addr())
}

fn wire_headers(headers: &http::HeaderMap) -> Vec<(String, Vec<u8>)> {
    headers
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), value.as_bytes().to_vec()))
        .collect()
}

fn headers(values: Vec<(String, Vec<u8>)>) -> io::Result<http::HeaderMap> {
    let mut result = http::HeaderMap::new();
    for (key, value) in values {
        let key = http::HeaderName::from_bytes(key.as_bytes()).map_err(io::Error::other)?;
        let value = http::HeaderValue::from_bytes(&value).map_err(io::Error::other)?;
        result.append(key, value);
    }
    Ok(result)
}

fn version(value: &str) -> io::Result<http::Version> {
    match value {
        "HTTP/0.9" => Ok(http::Version::HTTP_09),
        "HTTP/1.0" => Ok(http::Version::HTTP_10),
        "HTTP/1.1" => Ok(http::Version::HTTP_11),
        "HTTP/2.0" => Ok(http::Version::HTTP_2),
        "HTTP/3.0" => Ok(http::Version::HTTP_3),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid worker HTTP version",
        )),
    }
}

fn milliseconds(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn client(
    url: &reqwest::Url,
    addresses: &[SocketAddr],
    connect: Duration,
    total: Option<Duration>,
) -> io::Result<reqwest::Client> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HTTP worker requires an HTTP(S) URL without userinfo",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| io::Error::other("HTTP target has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| io::Error::other("HTTP target has no port"))?;
    if addresses.is_empty()
        || addresses.len() > 128
        || addresses.iter().any(|address| address.port() != port)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "HTTP target requires approved addresses for its exact port",
        ));
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>()
        && addresses.iter().any(|address| address.ip() != ip)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "HTTP IP target differs from its approved addresses",
        ));
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connect)
        .resolve_to_addrs(host, addresses);
    if let Some(timeout) = total {
        builder = builder.timeout(timeout);
    }
    builder
        .build()
        .map_err(|error| io::Error::other(error.without_url()))
}

fn response(head: ResponseHead, body: reqwest::Body) -> io::Result<reqwest::Response> {
    let url = reqwest::Url::parse(&head.url).map_err(io::Error::other)?;
    let mut result = http::Response::builder()
        .status(head.status)
        .version(version(&head.version)?)
        .url(url)
        .body(body)
        .map_err(io::Error::other)?;
    *result.headers_mut() = headers(head.headers)?;
    if let Some(address) = head.remote_addr {
        result.extensions_mut().insert(ResponsePeer(address));
    }
    Ok(reqwest::Response::from(result))
}

fn response_head(response: &reqwest::Response) -> ResponseHead {
    ResponseHead {
        status: response.status().as_u16(),
        url: response.url().to_string(),
        version: format!("{:?}", response.version()),
        headers: wire_headers(response.headers()),
        remote_addr: response.remote_addr(),
    }
}

fn count_bytes(total: &mut u64, length: usize, maximum: Option<u64>) -> io::Result<()> {
    *total = total
        .checked_add(length as u64)
        .ok_or_else(|| io::Error::other("HTTP body length overflow"))?;
    if maximum.is_some_and(|maximum| *total > maximum) {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "HTTP body exceeds the configured byte limit",
        ));
    }
    Ok(())
}

struct WorkerBody {
    child: std::sync::Arc<tokio::sync::Mutex<ManagedChild>>,
    stdout: tokio::process::ChildStdout,
    upload: tokio::task::JoinHandle<io::Result<()>>,
    diagnostics: tokio::task::JoinHandle<io::Result<(Vec<u8>, bool)>>,
    supervisor: Option<tokio::task::JoinHandle<()>>,
    _scratch: tempfile::TempDir,
    received: u64,
    maximum: Option<u64>,
    lifetime: RequestLifetime,
    upload_failures: upload::Failures,
}

async fn worker_stopped(
    lifetime: &RequestLifetime,
    upload_failures: &mut upload::Failures,
) -> io::Error {
    tokio::select! {
        biased;
        error = lifetime.stopped() => error,
        error = upload::failed(upload_failures) => error,
    }
}

impl WorkerBody {
    async fn finish_upload(&mut self) -> io::Result<()> {
        // A server may finish before consuming the entire upload. Retain any
        // source error already observed, then release an otherwise pending
        // source; a closed input pipe does not erase a valid 413/redirect.
        self.upload.abort();
        let result = (&mut self.upload).await;
        upload::check(&self.upload_failures)?;
        if let Err(error) = result
            && !error.is_cancelled()
        {
            return Err(io::Error::other(format!(
                "HTTP request upload failed: {error}"
            )));
        }
        Ok(())
    }

    async fn header_failure(&mut self, error: io::Error) -> io::Error {
        self.upload.abort();
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.abort();
            let _ = supervisor.await;
        }
        let stop = async {
            let mut child = self.child.lock().await;
            let _ = child.start_kill();
            let _ = child.wait().await;
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), stop).await;
        let diagnostics = tokio::time::timeout(Duration::from_secs(2), &mut self.diagnostics).await;
        match diagnostics {
            Ok(Ok(Ok((bytes, truncated)))) if !bytes.is_empty() => {
                let text = String::from_utf8_lossy(&bytes);
                let suffix = if truncated {
                    " [diagnostics truncated]"
                } else {
                    ""
                };
                io::Error::new(
                    error.kind(),
                    format!(
                        "HTTP worker failed before response headers: {error}; {}{suffix}",
                        text.trim()
                    ),
                )
            }
            _ => io::Error::new(
                error.kind(),
                format!("HTTP worker failed before response headers: {error}"),
            ),
        }
    }
}

impl Drop for WorkerBody {
    fn drop(&mut self) {
        self.upload.abort();
        self.diagnostics.abort();
        if let Some(supervisor) = &self.supervisor {
            supervisor.abort();
        }
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

/// Execute exactly one already-authorized hop. Dropping the future before
/// headers, or dropping the response body, terminates the worker process tree.
pub async fn execute(
    mut request: reqwest::Request,
    options: HttpOptions,
) -> io::Result<reqwest::Response> {
    let started = tokio::time::Instant::now();
    let total_timeout = match (options.total_timeout, request.timeout().copied()) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let lifetime = RequestLifetime::new(started, total_timeout, options.cancellation)?;
    lifetime.check()?;
    let max_request = options.max_request_bytes.min(MAX_REQUEST);
    if let Some(length) = request
        .body()
        .and_then(|body| http_body::Body::size_hint(body).exact())
    {
        if length > max_request {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "HTTP request exceeds the configured byte limit",
            ));
        }
        if !request.headers().contains_key(http::header::CONTENT_LENGTH)
            && !request
                .headers()
                .contains_key(http::header::TRANSFER_ENCODING)
        {
            request
                .headers_mut()
                .insert(http::header::CONTENT_LENGTH, length.into());
        }
    }
    // Validate routing before spawning. This builder never connects or resolves.
    let direct = client(
        request.url(),
        &options.pinned_addresses,
        options.connect_timeout,
        total_timeout,
    )?;
    if options.policy.disabled {
        if let Some(body) = request.body_mut().take() {
            let stream = futures_util::stream::try_unfold(
                (body.into_data_stream(), 0_u64),
                move |(mut body, mut total)| async move {
                    match body.next().await {
                        Some(Ok(chunk)) => {
                            count_bytes(&mut total, chunk.len(), Some(max_request))?;
                            Ok(Some((chunk, (body, total))))
                        }
                        Some(Err(error)) => Err(io::Error::other(error)),
                        None => Ok(None),
                    }
                },
            );
            *request.body_mut() = Some(reqwest::Body::wrap_stream(stream));
        }
        let result = tokio::select! {
            biased;
            error = lifetime.stopped() => return Err(error),
            result = direct.execute(request) => result
                .map_err(|error| io::Error::other(error.without_url()))?,
        };
        let head = response_head(&result);
        let stream = pump_body(
            result
                .bytes_stream()
                .map(|result| result.map_err(|error| io::Error::other(error.without_url()))),
            options.max_response_bytes,
            lifetime,
        );
        return response(head, reqwest::Body::wrap_stream(stream));
    }
    drop(direct);
    let body = request.body_mut().take();
    let head = RequestHead {
        method: request.method().to_string(),
        url: request.url().to_string(),
        headers: wire_headers(request.headers()),
        version: format!("{:?}", request.version()),
        has_body: body.is_some(),
        addresses: options.pinned_addresses,
        connect_ms: milliseconds(options.connect_timeout),
        total_ms: total_timeout.map(milliseconds),
        max_request,
        max_response: options.max_response_bytes,
    };
    let mut policy = options.policy;
    // Network workers have no reason to inspect the user's filesystem. Runtime
    // libraries and certificate roots are supplied by the native launcher.
    policy.readonly_paths.clear();
    policy.readwrite_paths.clear();
    policy.private_paths.clear();
    policy.configured_filesystem = true;
    lifetime.check()?;
    let (scratch, mut command) = crate::worker::command(ARGUMENT, &policy)?;
    lifetime.check()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::spawn(command)?;
    let mut stdin = child
        .stdin()
        .take()
        .ok_or_else(|| io::Error::other("HTTP worker stdin is unavailable"))?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| io::Error::other("HTTP worker stdout is unavailable"))?;
    let stderr = child
        .stderr()
        .take()
        .ok_or_else(|| io::Error::other("HTTP worker stderr is unavailable"))?;
    let (upload_errors, upload_failures) = tokio::sync::watch::channel(None);
    let upload = tokio::spawn(async move {
        ipc::write_json(&mut stdin, &head).await?;
        if let Some(body) = body {
            let source = body
                .into_data_stream()
                .map(|result| result.map_err(|error| io::Error::other(error.without_url())));
            upload::write_chunks(&mut stdin, source, max_request)
                .await
                .map_err(|error| error.report(&upload_errors))?;
        }
        ipc::finish(&mut stdin).await?;
        stdin.shutdown().await
    });
    let child = std::sync::Arc::new(tokio::sync::Mutex::new(child));
    let supervisor = Some({
        let child = child.clone();
        let lifetime = lifetime.clone();
        let mut upload_failures = upload_failures.clone();
        let upload = upload.abort_handle();
        tokio::spawn(async move {
            worker_stopped(&lifetime, &mut upload_failures).await;
            upload.abort();
            let mut child = child.lock().await;
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        })
    });
    let mut state = WorkerBody {
        child,
        stdout,
        upload,
        diagnostics: tokio::spawn(crate::read_bounded(stderr, 16 * 1024)),
        supervisor,
        _scratch: scratch,
        received: 0,
        maximum: options.max_response_bytes,
        lifetime,
        upload_failures,
    };
    let receive_head = async {
        let result: Result<ResponseHead, String> = ipc::read_json(&mut state.stdout).await?;
        result.map_err(io::Error::other)
    };
    let head = tokio::select! {
        biased;
        error = worker_stopped(&state.lifetime, &mut state.upload_failures) => return Err(error),
        result = receive_head => result,
    };
    let head = match head {
        Ok(head) => head,
        Err(error) => return Err(state.header_failure(error).await),
    };
    let stream = futures_util::stream::try_unfold(state, |mut state| async move {
        let chunk = tokio::select! {
            biased;
            error = worker_stopped(&state.lifetime, &mut state.upload_failures) => return Err(error),
            chunk = ipc::read_chunk(&mut state.stdout) => chunk?,
        };
        match chunk {
            Some(chunk) => {
                count_bytes(&mut state.received, chunk.len(), state.maximum)?;
                Ok(Some((chunk, state)))
            }
            None => {
                let wait = async {
                    let mut child = state.child.lock().await;
                    tokio::time::timeout(Duration::from_secs(10), child.wait())
                        .await
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::TimedOut,
                                "HTTP worker did not exit after response",
                            )
                        })?
                };
                let status = tokio::select! {
                    biased;
                    error = worker_stopped(&state.lifetime, &mut state.upload_failures) => return Err(error),
                    status = wait => status?,
                };
                if !status.success() {
                    return Err(io::Error::other(format!(
                        "HTTP worker exited with {status}"
                    )));
                }
                state.finish_upload().await?;
                Ok(None)
            }
        }
    });
    response(head, reqwest::Body::wrap_stream(stream))
}

async fn worker() -> io::Result<()> {
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let initial = async {
        let head: RequestHead = ipc::read_json(&mut input).await?;
        let url = reqwest::Url::parse(&head.url).map_err(io::Error::other)?;
        let client = client(
            &url,
            &head.addresses,
            Duration::from_millis(head.connect_ms),
            head.total_ms.map(Duration::from_millis),
        )?;
        let mut request =
            reqwest::Request::new(head.method.parse().map_err(io::Error::other)?, url);
        *request.headers_mut() = headers(head.headers)?;
        *request.version_mut() = version(&head.version)?;
        if head.has_body {
            let maximum = head.max_request.min(MAX_REQUEST);
            let stream = futures_util::stream::try_unfold(
                (input, 0_u64),
                move |(mut input, mut total)| async move {
                    match ipc::read_chunk(&mut input).await? {
                        Some(chunk) => {
                            count_bytes(&mut total, chunk.len(), Some(maximum))?;
                            Ok::<_, io::Error>(Some((chunk, (input, total))))
                        }
                        None => Ok(None),
                    }
                },
            );
            *request.body_mut() = Some(reqwest::Body::wrap_stream(stream));
        }
        let response = client
            .execute(request)
            .await
            .map_err(|error| io::Error::other(error.without_url()))?;
        Ok::<_, io::Error>((response, head.max_response))
    }
    .await;
    let (result, maximum) = match initial {
        Ok(result) => result,
        Err(error) => {
            ipc::write_json(&mut output, &Err::<ResponseHead, _>(error.to_string())).await?;
            return Ok(());
        }
    };
    ipc::write_json(&mut output, &Ok::<_, String>(response_head(&result))).await?;
    let send = async {
        let mut body = result.bytes_stream();
        let mut total = 0;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| io::Error::other(error.without_url()))?;
            count_bytes(&mut total, chunk.len(), maximum)?;
            ipc::write_chunk(&mut output, &chunk).await?;
        }
        ipc::finish(&mut output).await
    }
    .await;
    if let Err(error) = send {
        ipc::fail(&mut output, &error).await?;
    }
    Ok(())
}

pub fn try_run_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let result = (|| {
        if std::env::args_os().count() != 2 {
            return Err(io::Error::other("invalid HTTP worker arguments"));
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(worker())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("HTTP worker failed: {error}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinning_rejects_missing_addresses_and_wrong_ports() {
        let url = reqwest::Url::parse("https://example.com/path").unwrap();
        assert!(client(&url, &[], Duration::from_secs(1), None).is_err());
        assert!(
            client(
                &url,
                &["127.0.0.1:80".parse().unwrap()],
                Duration::from_secs(1),
                None
            )
            .is_err()
        );
        let url = reqwest::Url::parse("http://127.0.0.1/").unwrap();
        assert!(
            client(
                &url,
                &["127.0.0.2:80".parse().unwrap()],
                Duration::from_secs(1),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn response_envelope_preserves_url_and_duplicate_binary_headers() {
        let head = ResponseHead {
            status: 206,
            url: "https://example.com/a/b".into(),
            version: "HTTP/1.1".into(),
            headers: vec![
                ("set-cookie".into(), b"one=1".to_vec()),
                ("set-cookie".into(), b"two=2".to_vec()),
                ("x-binary".into(), vec![0x80]),
            ],
            remote_addr: Some("192.0.2.5:443".parse().unwrap()),
        };
        let result = response(head, reqwest::Body::from("body")).unwrap();
        assert_eq!(result.url().as_str(), "https://example.com/a/b");
        assert_eq!(result.headers().get_all("set-cookie").iter().count(), 2);
        assert_eq!(result.headers()["x-binary"].as_bytes(), &[0x80]);
        assert_eq!(
            response_peer_addr(&result),
            Some("192.0.2.5:443".parse().unwrap())
        );
    }
}
