//! Stdio entry point for a turn-scoped MCP server owned by the Agena process.

use std::{
    io::{self, ErrorKind},
    net::SocketAddrV4,
    time::Duration,
};

use tokio::{
    io::{AsyncWriteExt, copy},
    net::TcpStream,
};

pub const WORKER_ARG: &str = "--internal-acp-mcp-stdio";
pub const TOKEN_ENV: &str = "AGENA_ACP_MCP_RELAY_TOKEN";
pub const TOKEN_BYTES: usize = 32;

/// Run the relay before the main executable loads user configuration or plugins.
pub fn try_run_worker() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return None;
    }
    let address = args
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<SocketAddrV4>().ok())
        .filter(|address| address.ip().is_loopback() && address.port() != 0);
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

fn run(address: SocketAddrV4, token: &[u8]) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(relay(address, token));
    // Stdin can still be open in the native agent after the grant expires.
    // A pending blocking read must not keep the relay process alive.
    runtime.shutdown_background();
    result
}

async fn relay(address: SocketAddrV4, token: &[u8]) -> io::Result<()> {
    let socket = tokio::time::timeout(Duration::from_secs(5), async {
        let mut socket = TcpStream::connect(address).await?;
        socket.write_all(token).await?;
        Ok::<_, io::Error>(socket)
    })
    .await??;
    let (mut read, mut write) = socket.into_split();
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
