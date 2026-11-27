//! Stdio entry point for a turn-scoped MCP server owned by the Agena process.

use std::{
    io::{self, ErrorKind},
    net::SocketAddrV4,
    time::Duration,
};

#[cfg(unix)]
use std::path::PathBuf;

use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, copy},
    net::TcpStream,
};

pub const WORKER_ARG: &str = "--internal-acp-mcp-stdio";
pub const TOKEN_ENV: &str = "AGENA_ACP_MCP_RELAY_TOKEN";
pub const TOKEN_BYTES: usize = 32;
#[cfg(unix)]
pub const UNIX_PREFIX: &str = "unix:";
#[cfg(windows)]
pub const PIPE_PREFIX: &str = r"\\.\pipe\agena-acp-mcp-";

enum Endpoint {
    Loopback(SocketAddrV4),
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    Pipe(String),
}

impl Endpoint {
    fn parse(value: String) -> Option<Self> {
        #[cfg(windows)]
        if let Some(name) = value.strip_prefix(PIPE_PREFIX) {
            return (name.len() == TOKEN_BYTES && name.bytes().all(|b| b.is_ascii_hexdigit()))
                .then_some(Self::Pipe(value));
        }
        #[cfg(unix)]
        if let Some(path) = value.strip_prefix(UNIX_PREFIX) {
            let path = PathBuf::from(path);
            return path.is_absolute().then_some(Self::Unix(path));
        }
        value
            .parse::<SocketAddrV4>()
            .ok()
            .filter(|address| address.ip().is_loopback() && address.port() != 0)
            .map(Self::Loopback)
    }
}

/// Run the relay before the main executable loads user configuration or plugins.
pub fn try_run_worker() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return None;
    }
    let address = args
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(Endpoint::parse);
    let token = std::env::var(TOKEN_ENV)
        .ok()
        .filter(|value| value.len() == TOKEN_BYTES && value.bytes().all(|b| b.is_ascii_hexdigit()));
    let result = match (address, token, args.next()) {
        (Some(address), Some(token), None) => run(address, token.as_bytes()),
        _ => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "invalid relay arguments",
        )),
    };
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("Agena MCP stdio relay: {error}");
            Some(1)
        }
    }
}

fn run(address: Endpoint, token: &[u8]) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        match address {
            Endpoint::Loopback(address) => relay(TcpStream::connect(address), token).await,
            #[cfg(unix)]
            Endpoint::Unix(path) => relay(tokio::net::UnixStream::connect(path), token).await,
            #[cfg(windows)]
            Endpoint::Pipe(name) => {
                let connect = async {
                    use tokio::net::windows::named_pipe::ClientOptions;

                    loop {
                        match ClientOptions::new().open(&name) {
                            // ERROR_PIPE_BUSY: another client has connected and the next
                            // server instance is about to be created.
                            Err(error) if error.raw_os_error() == Some(231) => {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            result => return result,
                        }
                    }
                };
                relay(connect, token).await
            }
        }
    });
    // Stdin can still be open in the native agent after the grant expires.
    // A pending blocking read must not keep the relay process alive.
    runtime.shutdown_background();
    result
}

async fn relay<T: AsyncRead + AsyncWrite + Unpin>(
    connect: impl Future<Output = io::Result<T>>,
    token: &[u8],
) -> io::Result<()> {
    let socket = tokio::time::timeout(Duration::from_secs(5), async {
        let mut socket = connect.await?;
        socket.write_all(token).await?;
        Ok::<_, io::Error>(socket)
    })
    .await??;
    let (mut read, mut write) = tokio::io::split(socket);
    let upstream = async {
        copy(&mut tokio::io::stdin(), &mut write).await?;
        write.shutdown().await
    };
    let downstream = async {
        let mut stdout = tokio::io::stdout();
        copy(&mut read, &mut stdout).await?;
        stdout.flush().await
    };
    tokio::pin!(downstream);
    tokio::select! {
        result = &mut downstream => result,
        result = upstream => {
            result?;
            downstream.await
        }
    }
}
