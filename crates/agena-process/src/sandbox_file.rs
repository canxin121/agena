//! Bounded native file reads for provider uploads. Only raw bytes cross the
//! worker pipe; archive/media parsing belongs to separate offline workers.
use crate::sandbox::{SandboxNetwork, SandboxPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const ARGUMENT: &str = "--agena-file-worker";
const MAXIMUM: u64 = 512 * 1024 * 1024;
const MAX_HEADER: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    path: PathBuf,
    maximum: u64,
    expected: Option<String>,
}

fn check_request(request: &Request) -> io::Result<()> {
    if !request.path.is_absolute() || request.maximum > MAXIMUM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file worker requires an absolute path and a limit at most 512 MiB",
        ));
    }
    if let Some(expected) = &request.expected
        && (expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected_sha256 must contain 64 hexadecimal characters",
        ));
    }
    Ok(())
}

fn check_revision(bytes: &[u8], expected: Option<&str>) -> io::Result<()> {
    if let Some(expected) = expected {
        let digest = hex::encode(Sha256::digest(bytes));
        if !digest.eq_ignore_ascii_case(expected) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file changed: expected_sha256 does not match",
            ));
        }
    }
    Ok(())
}

fn read_file(request: &Request) -> io::Result<Vec<u8>> {
    check_request(request)?;
    if !std::fs::symlink_metadata(&request.path)?
        .file_type()
        .is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "upload input must be a regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(&request.path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "upload input must be a regular file",
        ));
    }
    if metadata.len() > request.maximum {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "file exceeds the upload byte limit",
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(request.maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > request.maximum {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "file exceeds the upload byte limit",
        ));
    }
    let after = file.metadata()?;
    if metadata.len() != after.len()
        || metadata.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file changed while it was being read",
        ));
    }
    check_revision(&bytes, request.expected.as_deref())?;
    Ok(bytes)
}

pub async fn read(
    path: &Path,
    maximum: u64,
    expected: Option<&str>,
    policy: &SandboxPolicy,
) -> io::Result<Vec<u8>> {
    let mut request = Request {
        path: path.to_owned(),
        maximum,
        expected: expected.map(str::to_owned),
    };
    check_request(&request)?;
    if policy.disabled {
        request.path = std::fs::canonicalize(&request.path)?;
        return tokio::task::spawn_blocking(move || read_file(&request))
            .await
            .map_err(io::Error::other)?;
    }
    let mut policy = crate::worker::restrict_read(policy, path)?;
    request.path = policy.readonly_paths[0].clone();
    policy.network = SandboxNetwork::Disabled;
    policy.readwrite_paths.clear();
    policy.private_paths.clear();
    policy.configured_filesystem = true;
    let (_scratch, command) = crate::worker::command(ARGUMENT, &policy)?;
    let input = serde_json::to_vec(&request).map_err(io::Error::other)?;
    if input.len() as u64 > MAX_HEADER {
        return Err(io::Error::other("file worker request too large"));
    }
    let output = crate::output_with_input(
        command,
        &input,
        Duration::from_secs(120),
        maximum as usize + MAX_HEADER as usize + 1,
    )
    .await?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "MXC file worker failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut bytes = output.stdout;
    let (tag, payload) = bytes.split_first().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing file worker response")
    })?;
    match tag {
        0 if payload.len() as u64 <= maximum => {
            check_revision(payload, expected)?;
            bytes.remove(0);
            Ok(bytes)
        }
        1 if payload.len() as u64 <= MAX_HEADER => Err(io::Error::other(
            String::from_utf8_lossy(payload).into_owned(),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid file worker response",
        )),
    }
}

pub fn try_run_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let result = (|| {
        if std::env::args_os().count() != 2 {
            return Err(io::Error::other("invalid file worker arguments"));
        }
        let mut input = Vec::new();
        std::io::stdin()
            .take(MAX_HEADER + 1)
            .read_to_end(&mut input)?;
        if input.len() as u64 > MAX_HEADER {
            return Err(io::Error::other("file worker request too large"));
        }
        let request: Request = serde_json::from_slice(&input).map_err(io::Error::other)?;
        read_file(&request)
    })();
    let mut output = std::io::stdout().lock();
    let result = match result {
        Ok(bytes) => output
            .write_all(&[0])
            .and_then(|()| output.write_all(&bytes)),
        Err(error) => output
            .write_all(&[1])
            .and_then(|()| output.write_all(error.to_string().as_bytes())),
    }
    .and_then(|()| output.flush());
    Some(if result.is_ok() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_enforce_size_regular_files_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"content").unwrap();
        let mut request = Request {
            path,
            maximum: 7,
            expected: None,
        };
        assert_eq!(read_file(&request).unwrap(), b"content");
        request.maximum = 6;
        assert_eq!(
            read_file(&request).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        request.maximum = 7;
        request.expected = Some("0".repeat(64));
        assert!(
            read_file(&request)
                .unwrap_err()
                .to_string()
                .contains("file changed")
        );
        request.expected = Some(hex::encode(Sha256::digest(b"content")));
        assert_eq!(read_file(&request).unwrap(), b"content");
        request.path = dir.path().to_owned();
        assert!(read_file(&request).is_err());
    }
}
