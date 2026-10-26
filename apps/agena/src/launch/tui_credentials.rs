//! Read the installer's saved client environment as data, never as shell code.

use std::{env, fs::OpenOptions, io::Read, path::Path};

use agena_api::resource::ServerEndpointRecord;
use agena_client::AgenaClient;
use agena_storage_layout::StorageLayout;
use url::{Host, Url};

pub(super) async fn local_server_password(server_url: &str) -> Option<String> {
    let endpoint = Url::parse(server_url).ok()?;
    if !is_local_endpoint(&endpoint) {
        return None;
    }
    let layout = StorageLayout::current();
    let record: ServerEndpointRecord =
        serde_json::from_slice(&std::fs::read(layout.run_dir().join("server.json")).ok()?).ok()?;
    let published = Url::parse(&record.url).ok()?;
    if !is_local_endpoint(&published)
        || endpoint.scheme() != published.scheme()
        || endpoint.port_or_known_default() != published.port_or_known_default()
        || endpoint.path().trim_end_matches('/') != published.path().trim_end_matches('/')
    {
        return None;
    }

    let mut installations = Vec::new();
    if let Some(root) = env::var_os("AGENA_INSTALL_DIR").filter(|root| !root.is_empty()) {
        installations.push(root.into());
    }
    if let Ok(executable) = env::current_exe()
        && let Some(bin) = executable.parent()
        && bin.file_name().is_some_and(|name| name == "bin")
        && let Some(root) = bin.parent()
    {
        installations.push(root.to_path_buf());
    }
    if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
        installations.push(Path::new(&home).join(".local/share/agena"));
    }
    let password = installations
        .into_iter()
        .find_map(|root| saved_password(&root.join("client-env.sh"), &layout))?;

    // Discovery files alone do not establish endpoint ownership. Validate the
    // live process identity before sending any automatically loaded secret.
    let identity = AgenaClient::new(server_url)
        .ok()?
        .server_identity()
        .await
        .ok()?;
    record.matches(&identity).then_some(password)
}

fn is_local_endpoint(endpoint: &Url) -> bool {
    matches!(endpoint.scheme(), "http" | "https")
        && endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.query().is_none()
        && endpoint.fragment().is_none()
        && match endpoint.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain("localhost")) => true,
            _ => false,
        }
}

fn saved_password(path: &Path, layout: &StorageLayout) -> Option<String> {
    let path = agena_storage_layout::resolve_private_path(path).ok()?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // This file already contains the installer's password; never accept
        // a replacement owned by another account or readable by other users.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return None;
        }
    }
    const MAX_CLIENT_ENV_BYTES: u64 = 64 * 1024;
    let mut contents = String::new();
    file.take(MAX_CLIENT_ENV_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() as u64 > MAX_CLIENT_ENV_BYTES {
        return None;
    }
    let mut data_dir = None;
    let mut password = None;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("export AGENA_DATA_DIR=") {
            data_dir = Some(shell_value(value)?);
        } else if let Some(value) = line.strip_prefix("export AGENA_SERVER_PASSWORD=") {
            password = Some(shell_value(value)?);
        }
    }
    let data_dir = data_dir?;
    if data_dir.is_empty() || StorageLayout::new(data_dir) != *layout {
        return None;
    }
    password.filter(|password| !password.trim().is_empty())
}

/// Decode the literal forms emitted by Bash's `printf %q`. No expansion,
/// substitution, sourcing, or process execution takes place here.
fn shell_value(value: &str) -> Option<String> {
    let Some(quoted) = value.strip_prefix("$'") else {
        let mut words = shlex::split(value)?.into_iter();
        let word = words.next()?;
        return words.next().is_none().then_some(word);
    };
    let mut bytes = quoted.strip_suffix('\'')?.bytes().peekable();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let escaped = bytes.next()?;
        decoded.push(match escaped {
            b'a' => 7,
            b'b' => 8,
            b'e' | b'E' => 27,
            b'f' => 12,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 11,
            b'\\' | b'\'' | b'"' | b'?' => escaped,
            b'0'..=b'7' => {
                let mut octal = u16::from(escaped - b'0');
                for _ in 0..2 {
                    let Some(digit @ b'0'..=b'7') = bytes.peek().copied() else {
                        break;
                    };
                    bytes.next();
                    octal = octal * 8 + u16::from(digit - b'0');
                }
                u8::try_from(octal).ok()?
            }
            b'x' | b'u' | b'U' => {
                let limit = match escaped {
                    b'x' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                let mut hex = String::new();
                for _ in 0..limit {
                    let Some(digit) = bytes.peek().copied().filter(u8::is_ascii_hexdigit) else {
                        break;
                    };
                    bytes.next();
                    hex.push(char::from(digit));
                }
                let code = u32::from_str_radix(&hex, 16).ok()?;
                if escaped != b'x' {
                    let character = char::from_u32(code)?;
                    decoded.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                    continue;
                }
                u8::try_from(code).ok()?
            }
            _ => return None,
        });
    }
    String::from_utf8(decoded).ok()
}
