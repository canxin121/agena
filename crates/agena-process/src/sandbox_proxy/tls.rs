//! Bounded ClientHello inspection for hostname-authorized CONNECT tunnels.
//! Wire formats: RFC 8446 sections 4.1.2/5.1 and RFC 6066 section 3.
//! TLS remains end-to-end; this only checks the initial destination indication.
use std::{
    collections::HashSet,
    io::{self, Read},
    net::TcpStream,
    time::{Duration, Instant},
};

const HELLO_LIMIT: usize = 64 * 1024;
const RECORD_LIMIT: usize = 16 * 1024;

/// Read complete handshake records until the first ClientHello can be validated.
/// No upstream socket may be opened before this succeeds. Preserve original wire
/// bytes, including already-read bytes after the hello, for transparent forwarding.
pub(super) fn read_client_hello(
    stream: &mut TcpStream,
    initial: Vec<u8>,
    approved_host: &str,
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut wire = initial;
    if wire.len() > HELLO_LIMIT {
        return Err(super::invalid("TLS ClientHello exceeds 64 KiB"));
    }
    let mut offset = 0;
    let mut handshake = Vec::new();
    loop {
        ensure_bytes(stream, &mut wire, offset + 5, deadline)?;
        if wire[offset] != 22 || wire[offset + 1] != 3 || !(1..=3).contains(&wire[offset + 2]) {
            return Err(super::invalid(
                "hostname CONNECT requires a TLS ClientHello",
            ));
        }
        let length = usize::from(u16::from_be_bytes([wire[offset + 3], wire[offset + 4]]));
        if length == 0 || length > RECORD_LIMIT {
            return Err(super::invalid("invalid TLS handshake record length"));
        }
        let end = offset + 5 + length;
        ensure_bytes(stream, &mut wire, end, deadline)?;
        handshake.extend_from_slice(&wire[offset + 5..end]);
        offset = end;
        if handshake.len() < 4 {
            continue;
        }
        if handshake[0] != 1 {
            return Err(super::invalid("first TLS handshake must be ClientHello"));
        }
        let length = (usize::from(handshake[1]) << 16)
            | (usize::from(handshake[2]) << 8)
            | usize::from(handshake[3]);
        if length + 4 > HELLO_LIMIT {
            return Err(super::invalid("TLS ClientHello exceeds 64 KiB"));
        }
        if handshake.len() < length + 4 {
            continue;
        }
        if handshake.len() != length + 4 {
            return Err(super::invalid(
                "unexpected handshake after initial ClientHello",
            ));
        }
        validate_client_hello(&handshake[4..], approved_host)?;
        return Ok(wire);
    }
}

fn ensure_bytes(
    stream: &mut TcpStream,
    bytes: &mut Vec<u8>,
    required: usize,
    deadline: Instant,
) -> io::Result<()> {
    if required > HELLO_LIMIT {
        return Err(super::invalid("TLS ClientHello exceeds 64 KiB"));
    }
    while bytes.len() < required {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "TLS ClientHello deadline elapsed")
            })?;
        stream.set_read_timeout(Some(remaining))?;
        let mut buffer = [0; 4096];
        let maximum = buffer.len().min(required - bytes.len());
        let count = stream.read(&mut buffer[..maximum])?;
        if count == 0 {
            return Err(super::invalid("incomplete TLS ClientHello"));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.bytes.len() < count {
            return Err(super::invalid("malformed TLS ClientHello"));
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> io::Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn vector16(&mut self) -> io::Result<&'a [u8]> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }
}

fn validate_client_hello(body: &[u8], approved_host: &str) -> io::Result<()> {
    let mut hello = Cursor { bytes: body };
    let version = hello.take(2)?;
    if version[0] != 3 || !(1..=3).contains(&version[1]) {
        return Err(super::invalid("unsupported TLS ClientHello version"));
    }
    hello.take(32)?; // Random.
    let session_length = usize::from(hello.u8()?);
    if session_length > 32 {
        return Err(super::invalid("invalid TLS session identifier"));
    }
    hello.take(session_length)?;
    let suites = hello.vector16()?;
    if suites.is_empty() || suites.len() % 2 != 0 {
        return Err(super::invalid("invalid TLS cipher suites"));
    }
    let compression_length = usize::from(hello.u8()?);
    if compression_length == 0 {
        return Err(super::invalid("invalid TLS compression methods"));
    }
    hello.take(compression_length)?;
    let mut extensions = Cursor {
        bytes: hello.vector16()?,
    };
    if !hello.bytes.is_empty() {
        return Err(super::invalid("trailing TLS ClientHello bytes"));
    }
    let mut seen = HashSet::new();
    let mut server_name = None;
    while !extensions.bytes.is_empty() {
        let kind = extensions.u16()?;
        let data = extensions.vector16()?;
        if !seen.insert(kind) {
            return Err(super::invalid("duplicate TLS ClientHello extension"));
        }
        match kind {
            0 => {
                server_name = Some(parse_server_name(data)?);
            }
            // RFC 9849: an encrypted inner hello can name a different server.
            0xfe0d => {
                return Err(super::invalid(
                    "encrypted ClientHello is unsupported by hostname-authorized CONNECT",
                ));
            }
            _ => {}
        }
    }
    let name = server_name.ok_or_else(|| super::invalid("hostname CONNECT requires TLS SNI"))?;
    if !name.eq_ignore_ascii_case(approved_host.trim_end_matches('.')) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "TLS SNI does not match the approved CONNECT hostname",
        ));
    }
    Ok(())
}

fn parse_server_name(extension: &[u8]) -> io::Result<&str> {
    let mut extension = Cursor { bytes: extension };
    let mut names = Cursor {
        bytes: extension.vector16()?,
    };
    if !extension.bytes.is_empty() {
        return Err(super::invalid("trailing TLS server-name data"));
    }
    let mut hostname = None;
    while !names.bytes.is_empty() {
        let name_type = names.u8()?;
        let name = names.vector16()?;
        if name_type != 0 || hostname.is_some() {
            return Err(super::invalid("unsupported or duplicate TLS server name"));
        }
        let name =
            std::str::from_utf8(name).map_err(|_| super::invalid("invalid TLS server name"))?;
        if name.is_empty()
            || name.len() > 253
            || !name.is_ascii()
            || name.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(super::invalid("invalid TLS DNS server name"));
        }
        hostname = Some(name);
    }
    hostname.ok_or_else(|| super::invalid("missing TLS server name"))
}

#[cfg(test)]
pub(super) fn test_client_hello(names: &[&str], duplicate_extension: bool, ech: bool) -> Vec<u8> {
    let mut name_list = Vec::new();
    for name in names {
        name_list.push(0);
        name_list.extend_from_slice(&(name.len() as u16).to_be_bytes());
        name_list.extend_from_slice(name.as_bytes());
    }
    let mut name_extension = Vec::new();
    name_extension.extend_from_slice(&(name_list.len() as u16).to_be_bytes());
    name_extension.extend_from_slice(&name_list);
    let mut extensions = Vec::new();
    if !names.is_empty() {
        for _ in 0..if duplicate_extension { 2 } else { 1 } {
            extensions.extend_from_slice(&0_u16.to_be_bytes());
            extensions.extend_from_slice(&(name_extension.len() as u16).to_be_bytes());
            extensions.extend_from_slice(&name_extension);
        }
    }
    if ech {
        extensions.extend_from_slice(&0xfe0d_u16.to_be_bytes());
        extensions.extend_from_slice(&0_u16.to_be_bytes());
    }
    let mut body = vec![3, 3];
    body.extend_from_slice(&[0; 32]);
    body.push(0); // Empty session identifier.
    body.extend_from_slice(&[0, 2, 0x13, 1]); // One cipher suite.
    body.extend_from_slice(&[1, 0]); // Null compression.
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);
    let mut handshake = vec![
        1,
        0,
        ((body.len() >> 8) & 255) as u8,
        (body.len() & 255) as u8,
    ];
    handshake.extend_from_slice(&body);
    handshake
}

#[cfg(test)]
pub(super) fn test_records(handshake: &[u8], fragment: usize) -> Vec<u8> {
    let mut wire = Vec::new();
    for part in handshake.chunks(fragment) {
        wire.extend_from_slice(&[22, 3, 1]);
        wire.extend_from_slice(&(part.len() as u16).to_be_bytes());
        wire.extend_from_slice(part);
    }
    wire
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_deadline_bounds_a_slow_stream() {
        use std::{io::Write, net::TcpListener, thread};
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut reader, _) = listener.accept().unwrap();
        writer.write_all(&[22, 3, 1, 0, 64]).unwrap();
        let sender = thread::spawn(move || {
            for _ in 0..64 {
                if writer.write_all(&[1]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(8));
            }
        });
        let started = Instant::now();
        assert!(
            read_client_hello(
                &mut reader,
                Vec::new(),
                "allowed.example",
                Duration::from_millis(40)
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(300));
        drop(reader);
        sender.join().unwrap();
    }

    #[test]
    fn client_hello_rejects_missing_duplicate_mismatched_and_encrypted_names() {
        for (names, duplicate, ech) in [
            (vec![], false, false),
            (vec!["other.example"], false, false),
            (vec!["allowed.example", "allowed.example"], false, false),
            (vec!["allowed.example"], true, false),
            (vec!["allowed.example"], false, true),
        ] {
            let hello = test_client_hello(&names, duplicate, ech);
            assert!(validate_client_hello(&hello[4..], "allowed.example").is_err());
        }
        let hello = test_client_hello(&["ALLOWED.EXAMPLE"], false, false);
        assert!(validate_client_hello(&hello[4..], "allowed.example").is_ok());
        for length in 0..hello.len() - 4 {
            assert!(validate_client_hello(&hello[4..4 + length], "allowed.example").is_err());
        }
    }
}
