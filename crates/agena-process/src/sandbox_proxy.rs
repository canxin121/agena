//! Per-launch authenticated HTTP proxy. Destinations are resolved and authorized
//! by the caller; this broker never resolves DNS or accepts a new destination.
//! MXC permits the workload to reach only this listener. Dropping the broker
//! closes every accepted socket, including outstanding CONNECT tunnels.
use crate::sandbox::ProxyTarget;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const HEADER_LIMIT: usize = 32 * 1024;
const CONNECTION_LIMIT: usize = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
type Connections = Arc<Mutex<HashMap<u64, Vec<TcpStream>>>>;
mod tls;

pub(crate) struct ProxyBroker {
    url: String,
    stopped: Arc<AtomicBool>,
    connections: Connections,
    listener: Option<JoinHandle<()>>,
}

impl ProxyBroker {
    pub(crate) fn start(targets: &[ProxyTarget]) -> io::Result<Self> {
        if targets.is_empty()
            || targets.iter().any(|target| {
                target.host.is_empty()
                    || target.addresses.is_empty()
                    || target.port == Some(0)
                    || target
                        .host
                        .chars()
                        .any(|ch| ch.is_control() || ch.is_whitespace())
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "proxy requires concrete authorized destinations",
            ));
        }
        let socket = TcpListener::bind(("127.0.0.1", 0))?;
        socket.set_nonblocking(true)?;
        let address = socket.local_addr()?;
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let authorization = format!("Basic {}", STANDARD.encode(format!("agena:{secret}")));
        let url = format!("http://agena:{secret}@{address}");
        let stopped = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(HashMap::new()));
        let worker_stop = stopped.clone();
        let worker_connections = connections.clone();
        let targets = Arc::new(targets.to_vec());
        let sequence = AtomicU64::new(1);
        let listener = thread::Builder::new().name("agena-sandbox-proxy".into()).spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                let (mut stream, _) = match socket.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(_) => break,
                };
                let id = sequence.fetch_add(1, Ordering::Relaxed);
                let Ok(copy) = stream.try_clone() else { continue };
                let Ok(mut active) = worker_connections.lock() else { break };
                if worker_stop.load(Ordering::Acquire) || active.len() >= CONNECTION_LIMIT {
                    let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                active.insert(id, vec![copy]);
                drop(active);
                let targets = targets.clone();
                let authorization = authorization.clone();
                let active = worker_connections.clone();
                let stop = worker_stop.clone();
                if thread::Builder::new().name("agena-sandbox-proxy-connection".into()).spawn(move || {
                    let _ = serve(&mut stream, &targets, &authorization, id, &active, &stop);
                    let _ = stream.shutdown(Shutdown::Both);
                    if let Ok(mut active) = active.lock() { active.remove(&id); }
                }).is_err() {
                    if let Ok(mut active) = worker_connections.lock()
                        && let Some(streams) = active.remove(&id) {
                        for stream in streams { let _ = stream.shutdown(Shutdown::Both); }
                    }
                }
            }
        })?;
        Ok(Self {
            url,
            stopped,
            connections,
            listener: Some(listener),
        })
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for ProxyBroker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        if let Ok(mut active) = self.connections.lock() {
            for (_, streams) in active.drain() {
                for stream in streams {
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
        }
    }
}

fn reject(stream: &mut TcpStream, code: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {code}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

struct Request {
    method: String,
    host: String,
    port: u16,
    forwarded: Vec<u8>,
    body_length: u64,
    pending: Vec<u8>,
    tunnel: bool,
}

fn normalize_host(value: &str) -> String {
    value
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn read_request(stream: &mut TcpStream, authorization: &str) -> io::Result<Request> {
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let mut input = Vec::new();
    let boundary = loop {
        if let Some(offset) = input.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
            break offset + 4;
        }
        if input.len() >= HEADER_LIMIT {
            return Err(invalid("proxy header too large"));
        }
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(invalid("incomplete proxy request"));
        }
        input.extend_from_slice(&chunk[..count]);
    };
    if boundary > HEADER_LIMIT {
        return Err(invalid("proxy header too large"));
    }
    let header =
        std::str::from_utf8(&input[..boundary]).map_err(|_| invalid("invalid proxy header"))?;
    let mut lines = header.split("\r\n");
    let first = lines
        .next()
        .ok_or_else(|| invalid("missing proxy request line"))?;
    let parts = first.split(' ').collect::<Vec<_>>();
    if parts.len() != 3
        || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1")
        || parts[0].is_empty()
        || !parts[0].bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(invalid("invalid proxy request line"));
    }
    let tunnel = parts[0] == "CONNECT";
    let uri = if tunnel {
        format!("http://{}", parts[1])
    } else {
        parts[1].to_owned()
    };
    let target = url::Url::parse(&uri).map_err(|_| invalid("invalid proxy destination"))?;
    if target.scheme() != "http"
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
        || (tunnel && (target.path() != "/" || target.query().is_some()))
    {
        return Err(invalid("unsupported proxy destination"));
    }
    let host = normalize_host(
        target
            .host_str()
            .ok_or_else(|| invalid("missing proxy host"))?,
    );
    let port = target
        .port_or_known_default()
        .ok_or_else(|| invalid("missing proxy port"))?;
    if port == 0 {
        return Err(invalid("invalid proxy port"));
    }
    // CONNECT must carry an explicit authority port, including when it is 80.
    if tunnel
        && !parts[1]
            .rsplit_once(':')
            .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
    {
        return Err(invalid("CONNECT requires host:port"));
    }
    let mut authorized = false;
    let mut has_auth = false;
    let mut body_length = None;
    let mut has_host = false;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            return Err(invalid("folded proxy headers are unsupported"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("invalid proxy header"))?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(invalid("invalid proxy header name"));
        }
        let value = value.trim();
        if value.chars().any(|ch| ch.is_control() && ch != '\t') {
            return Err(invalid("invalid proxy header value"));
        }
        match name.to_ascii_lowercase().as_str() {
            "proxy-authorization" => {
                if has_auth {
                    return Err(invalid("duplicate proxy credentials"));
                }
                has_auth = true;
                authorized = value == authorization;
            }
            "host" => {
                if has_host {
                    return Err(invalid("duplicate host header"));
                }
                has_host = true;
                let supplied = url::Url::parse(&format!("http://{value}"))
                    .map_err(|_| invalid("invalid host header"))?;
                if supplied.host_str().map(normalize_host).as_deref() != Some(host.as_str())
                    || supplied.port_or_known_default() != Some(port)
                    || !supplied.username().is_empty()
                    || supplied.password().is_some()
                    || supplied.path() != "/"
                    || supplied.query().is_some()
                {
                    return Err(invalid("proxy authority and host disagree"));
                }
            }
            "content-length" => {
                if body_length.is_some()
                    || value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(invalid("ambiguous proxy content length"));
                }
                body_length = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| invalid("invalid proxy content length"))?,
                );
            }
            "transfer-encoding" | "upgrade" | "expect" => {
                return Err(invalid(
                    "chunked uploads and HTTP upgrades require an approved CONNECT tunnel",
                ));
            }
            "connection" | "proxy-connection" | "keep-alive" | "te" | "trailer" => {}
            _ => headers.push(format!("{name}: {value}\r\n")),
        }
    }
    if !authorized {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "proxy credentials required",
        ));
    }
    let pending = input[boundary..].to_vec();
    let length = body_length.unwrap_or(0);
    if !tunnel && pending.len() as u64 > length {
        return Err(invalid("pipelined proxy requests are unsupported"));
    }
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let path = match target.query() {
        Some(query) => format!("{}?{query}", target.path()),
        None => target.path().to_owned(),
    };
    let mut forwarded = format!(
        "{} {} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n",
        parts[0], path
    )
    .into_bytes();
    for header in headers {
        forwarded.extend_from_slice(header.as_bytes());
    }
    if body_length.is_some() {
        forwarded.extend_from_slice(format!("Content-Length: {length}\r\n").as_bytes());
    }
    forwarded.extend_from_slice(b"\r\n");
    Ok(Request {
        method: parts[0].into(),
        host,
        port,
        forwarded,
        body_length: length,
        pending,
        tunnel,
    })
}

fn serve(
    client: &mut TcpStream,
    targets: &[ProxyTarget],
    authorization: &str,
    id: u64,
    connections: &Connections,
    stopped: &AtomicBool,
) -> io::Result<()> {
    let mut request = match read_request(client, authorization) {
        Ok(request) => request,
        Err(error) => {
            let status = if error.kind() == io::ErrorKind::PermissionDenied {
                "407 Proxy Authentication Required"
            } else {
                "400 Bad Request"
            };
            let _ = reject(client, status);
            return Err(error);
        }
    };
    let Some(destination) = targets.iter().find(|target| {
        normalize_host(&target.host) == request.host
            && target.port.is_none_or(|port| port == request.port)
    }) else {
        reject(client, "403 Forbidden")?;
        return Ok(());
    };
    let hostname_tunnel = request.tunnel && request.host.parse::<std::net::IpAddr>().is_err();
    if hostname_tunnel {
        // Clients wait for CONNECT success before sending TLS. No upstream
        // connection exists yet: the approved name must also match ClientHello
        // SNI before any traffic reaches the destination. Literal-IP grants
        // deliberately authorize the TCP endpoint without hostname semantics.
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        request.pending = tls::read_client_hello(
            client,
            std::mem::take(&mut request.pending),
            &request.host,
            CONNECT_TIMEOUT,
        )?;
    }
    let mut upstream = None;
    for address in &destination.addresses {
        if stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "proxy stopped"));
        }
        if let Ok(stream) =
            TcpStream::connect_timeout(&SocketAddr::new(*address, request.port), CONNECT_TIMEOUT)
        {
            upstream = Some(stream);
            break;
        }
    }
    let Some(mut upstream) = upstream else {
        if !hostname_tunnel {
            reject(client, "502 Bad Gateway")?;
        }
        return Ok(());
    };
    {
        let mut active = connections
            .lock()
            .map_err(|_| io::Error::other("proxy state unavailable"))?;
        if stopped.load(Ordering::Acquire) || !active.contains_key(&id) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "proxy stopped"));
        }
        active
            .get_mut(&id)
            .expect("connection was checked")
            .push(upstream.try_clone()?);
    }
    upstream.set_read_timeout(Some(Duration::from_secs(120)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(120)))?;
    client.set_read_timeout(Some(Duration::from_secs(120)))?;
    client.set_write_timeout(Some(Duration::from_secs(120)))?;
    if request.tunnel {
        if !hostname_tunnel {
            client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        }
        upstream.write_all(&request.pending)?;
        let mut client_read = client.try_clone()?;
        let mut upstream_write = upstream.try_clone()?;
        thread::scope(|scope| {
            let upload = scope.spawn(move || {
                let result = io::copy(&mut client_read, &mut upstream_write);
                let _ = upstream_write.shutdown(Shutdown::Write);
                result
            });
            let download = io::copy(&mut upstream, client);
            let _ = client.shutdown(Shutdown::Both);
            let _ = upstream.shutdown(Shutdown::Both);
            let uploaded = upload
                .join()
                .map_err(|_| io::Error::other("proxy upload worker failed"))?;
            download?;
            uploaded?;
            Ok::<_, io::Error>(())
        })?;
    } else {
        upstream.write_all(&request.forwarded)?;
        upstream.write_all(&request.pending)?;
        let remaining = request
            .body_length
            .saturating_sub(request.pending.len() as u64);
        if remaining != 0 {
            let copied = io::copy(&mut client.take(remaining), &mut upstream)?;
            if copied != remaining {
                return Err(invalid("incomplete proxy request body"));
            }
        }
        // Only one HTTP request is forwarded. The socket cannot be reused to
        // send a different Host header to another tenant on the approved IP.
        let _ = upstream.shutdown(Shutdown::Write);
        io::copy(&mut upstream, client)?;
    }
    let _ = request.method;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connect(broker: &ProxyBroker) -> (TcpStream, String) {
        let url = url::Url::parse(broker.url()).unwrap();
        let auth = format!(
            "Basic {}",
            STANDARD.encode(format!("{}:{}", url.username(), url.password().unwrap()))
        );
        (
            TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap(),
            auth,
        )
    }
    fn target(port: u16) -> ProxyTarget {
        ProxyTarget {
            host: "allowed.example".into(),
            port: Some(port),
            addresses: vec!["127.0.0.1".parse().unwrap()],
        }
    }
    #[test]
    fn proxy_auth_and_destination_are_enforced_before_connecting() {
        let broker = ProxyBroker::start(&[target(9)]).unwrap();
        let (mut stream, _) = connect(&broker);
        stream
            .write_all(b"CONNECT allowed.example:9 HTTP/1.1\r\nHost: allowed.example:9\r\n\r\n")
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 407"));
        let (mut stream, auth) = connect(&broker);
        write!(stream, "CONNECT other.example:9 HTTP/1.1\r\nHost: other.example:9\r\nProxy-Authorization: {auth}\r\n\r\n").unwrap();
        answer.clear();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 403"));
    }
    #[test]
    fn proxy_connects_only_to_pinned_address_and_strips_credentials() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            stream.read_to_string(&mut request).unwrap();
            assert!(request.starts_with("GET /test HTTP/1.1"));
            assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let broker = ProxyBroker::start(&[target(port)]).unwrap();
        let (mut stream, auth) = connect(&broker);
        write!(stream, "GET http://allowed.example:{port}/test HTTP/1.1\r\nHost: allowed.example:{port}\r\nProxy-Authorization: {auth}\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("ok"));
        server.join().unwrap();
    }
    #[test]
    fn proxy_rejects_authority_confusion_and_request_smuggling() {
        let broker = ProxyBroker::start(&[target(80)]).unwrap();
        for headers in [
            "Host: other.example\r\n",
            "Host: allowed.example\r\nContent-Length: 0\r\nContent-Length: 0\r\n",
            "Host: allowed.example\r\nTransfer-Encoding: chunked\r\n",
        ] {
            let (mut stream, auth) = connect(&broker);
            write!(stream, "GET http://allowed.example/ HTTP/1.1\r\n{headers}Proxy-Authorization: {auth}\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        }
    }
    #[test]
    fn dropping_proxy_closes_incomplete_clients_promptly() {
        let broker = ProxyBroker::start(&[target(80)]).unwrap();
        let (mut stream, _) = connect(&broker);
        stream.write_all(b"GET ").unwrap();
        thread::sleep(Duration::from_millis(50));
        drop(broker);
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut data = Vec::new();
        let result = stream.read_to_end(&mut data);
        assert!(result.is_ok() || result.unwrap_err().kind() == io::ErrorKind::ConnectionReset);
    }

    fn begin_tunnel(broker: &ProxyBroker, host: &str, port: u16) -> TcpStream {
        let (mut stream, auth) = connect(broker);
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(stream, "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: {auth}\r\n\r\n").unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
            assert!(header.len() < 512);
        }
        assert!(header.starts_with(b"HTTP/1.1 200"));
        stream
    }

    #[test]
    fn hostname_connect_rejects_plaintext_wrong_missing_duplicate_and_encrypted_sni_before_upstream()
     {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let broker = ProxyBroker::start(&[target(port)]).unwrap();
        let mut cases = vec![
            b"GET / HTTP/1.1\r\nHost: other.example\r\n\r\n".to_vec(),
            vec![22, 3, 1, 255, 255],
            vec![22, 3, 1, 0, 4, 1, 1, 0, 0],
        ];
        for (names, duplicate, ech) in [
            (vec!["other.example"], false, false),
            (vec![], false, false),
            (vec!["allowed.example", "allowed.example"], false, false),
            (vec!["allowed.example"], true, false),
            (vec!["allowed.example"], false, true),
        ] {
            cases.push(tls::test_records(
                &tls::test_client_hello(&names, duplicate, ech),
                16_384,
            ));
        }
        for bytes in cases {
            let mut stream = begin_tunnel(&broker, "allowed.example", port);
            stream.write_all(&bytes).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            let mut answer = Vec::new();
            let result = stream.read_to_end(&mut answer);
            assert!(result.is_ok() || result.unwrap_err().kind() == io::ErrorKind::ConnectionReset);
            assert!(answer.is_empty());
            assert!(
                matches!(upstream.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
                "rejected TLS traffic reached an upstream socket"
            );
        }
    }

    #[test]
    fn hostname_connect_forwards_fragmented_matching_client_hello_unchanged() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        upstream.set_nonblocking(true).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let wire = tls::test_records(
            &tls::test_client_hello(&["ALLOWED.EXAMPLE"], false, false),
            9,
        );
        let expected = wire.clone();
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match upstream.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("expected an approved TLS connection: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut actual = vec![0; expected.len()];
            stream.read_exact(&mut actual).unwrap();
            assert_eq!(actual, expected);
            stream.write_all(b"forwarded").unwrap();
        });
        let broker = ProxyBroker::start(&[target(port)]).unwrap();
        let mut stream = begin_tunnel(&broker, "allowed.example", port);
        stream.write_all(&wire).unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, b"forwarded");
        server.join().unwrap();
    }

    #[test]
    fn explicitly_approved_ip_keeps_tcp_endpoint_tunnel_semantics() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut input = [0; 5];
            stream.read_exact(&mut input).unwrap();
            assert_eq!(&input, b"plain");
            stream.write_all(b"ok").unwrap();
        });
        let mut destination = target(port);
        destination.host = "127.0.0.1".into();
        let broker = ProxyBroker::start(&[destination]).unwrap();
        let mut stream = begin_tunnel(&broker, "127.0.0.1", port);
        stream.write_all(b"plain").unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, b"ok");
        server.join().unwrap();
    }
}
