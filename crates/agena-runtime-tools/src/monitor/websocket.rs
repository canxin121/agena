//! WebSocket socket I/O for monitors, including the MXC worker boundary.
use std::{
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr},
    process::Stdio,
    time::Duration,
};

use agena_process::{ManagedChild, sandbox::SandboxPolicy};
use futures_util::{SinkExt, StreamExt, stream::BoxStream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

use super::MonitorWsParams;

const ARGUMENT: &str = "--agena-monitor-worker";
const MAX_REQUEST: usize = 128 * 1024;
const MAX_FRAME: usize = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    url: String,
    protocols: Vec<String>,
    addresses: Vec<SocketAddr>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Text(String),
    Binary(usize),
    Close(String),
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub(super) type EventStream = BoxStream<'static, io::Result<Event>>;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn validate(request: &Request) -> io::Result<()> {
    let url = url::Url::parse(&request.url).map_err(|_| invalid("invalid WebSocket URL"))?;
    if !matches!(url.scheme(), "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
    {
        return Err(invalid(
            "WebSocket URL must be ws/wss without embedded credentials",
        ));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid("WebSocket port is missing"))?;
    if request.addresses.is_empty() || request.addresses.len() > 128 {
        return Err(invalid("WebSocket requires 1–128 approved addresses"));
    }
    let literal = match url.host() {
        Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        _ => None,
    };
    if request
        .addresses
        .iter()
        .any(|address| address.port() != port || literal.is_some_and(|ip| ip != address.ip()))
    {
        return Err(invalid("WebSocket approved address does not match its URL"));
    }
    if request.protocols.len() > 64 {
        return Err(invalid("too many WebSocket protocols"));
    }
    if !request.protocols.is_empty() {
        request
            .protocols
            .join(", ")
            .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()
            .map_err(|_| invalid("invalid WebSocket protocol header"))?;
    }
    Ok(())
}

/// Connect only to the addresses authorized by the parent. The request URI is
/// unchanged so the HTTP Host header and TLS SNI still use the original name.
async fn connect_pinned(request: &Request) -> io::Result<Socket> {
    validate(request)?;
    let mut handshake = request
        .url
        .clone()
        .into_client_request()
        .map_err(|_| invalid("invalid WebSocket request"))?;
    if !request.protocols.is_empty() {
        handshake.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL,
            request
                .protocols
                .join(", ")
                .parse()
                .map_err(|_| invalid("invalid WebSocket protocols"))?,
        );
    }
    tokio::time::timeout(CONNECT_TIMEOUT, async {
        let mut last = None;
        for address in &request.addresses {
            let socket = match TcpStream::connect(address).await {
                Ok(socket) => socket,
                Err(error) => {
                    last = Some(error);
                    continue;
                }
            };
            match tokio_tungstenite::client_async_tls_with_config(
                handshake.clone(),
                socket,
                None,
                None,
            )
            .await
            {
                Ok((stream, _)) => return Ok(stream),
                Err(error) => {
                    last = Some(io::Error::other(format!(
                        "WebSocket handshake failed: {error}"
                    )))
                }
            }
        }
        Err(last.unwrap_or_else(|| invalid("no approved WebSocket address")))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket connect timed out"))?
}

fn events(socket: Socket) -> EventStream {
    futures_util::stream::try_unfold(socket, |mut socket| async move {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Ok(Some((Event::Text(text.to_string()), socket)));
                }
                Some(Ok(Message::Binary(data))) => {
                    return Ok(Some((Event::Binary(data.len()), socket)));
                }
                Some(Ok(Message::Close(frame))) => {
                    let reason = frame
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_default();
                    let _ = socket.flush().await;
                    return Ok(Some((Event::Close(reason), socket)));
                }
                Some(Ok(Message::Ping(_))) => {
                    socket.flush().await.map_err(io::Error::other)?;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Err(error)) => return Err(io::Error::other(error)),
                None => return Ok(None),
            }
        }
    })
    .boxed()
}

struct Worker {
    // ManagedChild kills its entire process tree on drop, including a response
    // that is dropped while the peer is quiet or during connection establishment.
    child: ManagedChild,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::task::JoinHandle<io::Result<Vec<u8>>>,
    _scratch: tempfile::TempDir,
}

async fn drain_stderr(mut stderr: tokio::process::ChildStderr) -> io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            return Ok(retained);
        }
        let keep = count.min((16 * 1024usize).saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
    }
}

async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<(u8, Vec<u8>)>> {
    let kind = match reader.read_u8().await {
        Ok(kind) => kind,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = reader.read_u32().await? as usize;
    if length > MAX_FRAME {
        return Err(invalid("WebSocket worker frame exceeded 64 MiB"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    Ok(Some((kind, bytes)))
}

fn write_frame(writer: &mut impl Write, kind: u8, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(invalid("WebSocket worker frame exceeded 64 MiB"));
    }
    writer.write_all(&[kind])?;
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

fn decode_event(kind: u8, bytes: Vec<u8>) -> io::Result<Event> {
    match kind {
        1 => String::from_utf8(bytes)
            .map(Event::Text)
            .map_err(|_| invalid("invalid WebSocket text")),
        2 if bytes.len() == 8 => {
            let count = u64::from_be_bytes(bytes.try_into().expect("eight bytes"));
            usize::try_from(count)
                .map(Event::Binary)
                .map_err(|_| invalid("WebSocket frame size overflow"))
        }
        3 => String::from_utf8(bytes)
            .map(Event::Close)
            .map_err(|_| invalid("invalid WebSocket close")),
        4 => Err(io::Error::other(
            String::from_utf8_lossy(&bytes).into_owned(),
        )),
        _ => Err(invalid("invalid WebSocket worker event")),
    }
}

async fn worker_failure(worker: &mut Worker) -> io::Error {
    let _ = worker.child.terminate(Duration::from_secs(2)).await;
    let diagnostic = match (&mut worker.stderr).await {
        Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).trim().to_owned(),
        _ => String::new(),
    };
    io::Error::other(if diagnostic.is_empty() {
        "WebSocket worker ended before connection readiness".to_owned()
    } else {
        format!("WebSocket worker could not connect: {diagnostic}")
    })
}

async fn isolated(request: Request, policy: &SandboxPolicy) -> io::Result<EventStream> {
    let bytes = serde_json::to_vec(&request).map_err(io::Error::other)?;
    if bytes.len() > MAX_REQUEST {
        return Err(invalid("WebSocket worker request is too large"));
    }
    let (scratch, mut command) = agena_process::worker::command(ARGUMENT, policy)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = agena_process::spawn(command)?;
    let mut input = child
        .stdin()
        .take()
        .ok_or_else(|| invalid("worker stdin unavailable"))?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| invalid("worker stdout unavailable"))?;
    let stderr = child
        .stderr()
        .take()
        .ok_or_else(|| invalid("worker stderr unavailable"))?;
    let mut worker = Worker {
        child,
        stdout,
        stderr: tokio::spawn(drain_stderr(stderr)),
        _scratch: scratch,
    };
    input.write_all(&bytes).await?;
    input.shutdown().await?;
    drop(input);
    match read_frame(&mut worker.stdout).await {
        Ok(Some((0, bytes))) if bytes.is_empty() => {}
        Ok(Some((kind, bytes))) => {
            return Err(decode_event(kind, bytes)
                .err()
                .unwrap_or_else(|| invalid("worker sent an event before readiness")));
        }
        _ => return Err(worker_failure(&mut worker).await),
    }
    Ok(
        futures_util::stream::try_unfold(worker, |mut worker| async move {
            match read_frame(&mut worker.stdout).await? {
                Some((kind, bytes)) => Ok(Some((decode_event(kind, bytes)?, worker))),
                None => {
                    let status = worker.child.wait().await?;
                    if status.success() {
                        Ok(None)
                    } else {
                        Err(io::Error::other(format!(
                            "WebSocket worker exited with {status}"
                        )))
                    }
                }
            }
        })
        .boxed(),
    )
}

pub(super) async fn connect(params: MonitorWsParams) -> io::Result<EventStream> {
    let request = Request {
        url: params.url,
        protocols: params.protocols,
        addresses: params.addresses,
    };
    validate(&request)?;
    if params.policy.disabled {
        return Ok(events(connect_pinned(&request).await?));
    }
    isolated(request, &params.policy).await
}

/// Early executable dispatch. No app configuration, plugin bootstrap or secrets
/// are loaded in the socket worker.
pub fn try_run_monitor_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    let result = (|| -> io::Result<()> {
        if std::env::args_os().count() != 2 {
            return Err(invalid("invalid WebSocket worker arguments"));
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_REQUEST as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_REQUEST {
            return Err(invalid("WebSocket worker request is too large"));
        }
        let request: Request = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate(&request)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let socket = connect_pinned(&request).await?;
            write_frame(&mut stdout, 0, &[])?;
            let mut stream = events(socket);
            while let Some(event) = stream.next().await {
                match event? {
                    Event::Text(text) => write_frame(&mut stdout, 1, text.as_bytes())?,
                    Event::Binary(count) => {
                        write_frame(&mut stdout, 2, &(count as u64).to_be_bytes())?
                    }
                    Event::Close(reason) => {
                        write_frame(&mut stdout, 3, reason.as_bytes())?;
                        return Ok(());
                    }
                }
            }
            Ok(())
        })
    })();
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            let _ = write_frame(&mut stdout, 4, error.to_string().as_bytes());
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn literal_targets_cannot_rebind_to_another_address_or_port() {
        let mut request = Request {
            url: "ws://127.0.0.1:32123/events".into(),
            protocols: vec![],
            addresses: vec!["127.0.0.2:32123".parse().unwrap()],
        };
        assert!(validate(&request).is_err());
        request.addresses = vec!["127.0.0.1:32124".parse().unwrap()];
        assert!(validate(&request).is_err());
        request.addresses = vec!["127.0.0.1:32123".parse().unwrap()];
        validate(&request).unwrap();
        request.protocols = vec!["unsafe\r\ninjected: header".into()];
        assert!(validate(&request).is_err());
    }

    #[tokio::test]
    async fn pinned_connection_keeps_the_original_host_without_second_dns_lookup() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.headers().get("host").unwrap().to_str().unwrap(),
                        format!("pinned-monitor.invalid:{}", address.port())
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            stream
                .send(Message::Text("first event".into()))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("dropping the subscription must close the socket")
        });
        let request = Request {
            url: format!("ws://pinned-monitor.invalid:{}/events", address.port()),
            protocols: vec![],
            addresses: vec![address],
        };
        let mut stream = events(connect_pinned(&request).await.unwrap());
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            Event::Text("first event".into())
        );
        drop(stream);
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn wire_frames_keep_text_binary_lengths_and_close_reasons() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, 1, "内容".as_bytes()).unwrap();
        write_frame(&mut bytes, 2, &123u64.to_be_bytes()).unwrap();
        write_frame(&mut bytes, 3, b"finished").unwrap();
        let mut reader = bytes.as_slice();
        for expected in [
            Event::Text("内容".into()),
            Event::Binary(123),
            Event::Close("finished".into()),
        ] {
            let (kind, body) = read_frame(&mut reader).await.unwrap().unwrap();
            assert_eq!(decode_event(kind, body).unwrap(), expected);
        }
        assert!(read_frame(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalid_worker_frames_fail_before_allocating_the_payload() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&((MAX_FRAME as u32) + 1).to_be_bytes());
        assert!(read_frame(&mut bytes.as_slice()).await.is_err());
        assert!(decode_event(2, vec![0; 7]).is_err());
        assert!(decode_event(4, b"backend denied".to_vec()).is_err());
    }
}
