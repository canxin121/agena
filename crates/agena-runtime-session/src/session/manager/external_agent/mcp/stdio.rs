use agena_mcp_server::stdio_relay::{TOKEN_BYTES, TOKEN_ENV, WORKER_ARG};
use futures_util::StreamExt;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::async_rw::JsonRpcMessageCodec,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::TcpStream,
    task::JoinSet,
};
use tokio_util::codec::{FramedRead, FramedWrite};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use super::*;

const MAX_CONNECTIONS: usize = 16;
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

enum Listener {
    Loopback(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
    #[cfg(windows)]
    Pipe {
        name: String,
        server: NamedPipeServer,
    },
}

enum Socket {
    Loopback(TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    Pipe(NamedPipeServer),
}

impl Listener {
    async fn accept(&mut self) -> std::io::Result<Socket> {
        match self {
            Self::Loopback(listener) => Ok(Socket::Loopback(listener.accept().await?.0)),
            #[cfg(unix)]
            Self::Unix(listener) => Ok(Socket::Unix(listener.accept().await?.0)),
            #[cfg(windows)]
            Self::Pipe { name, server } => {
                server.connect().await?;
                let next = ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(name)?;
                Ok(Socket::Pipe(std::mem::replace(server, next)))
            }
        }
    }
}

pub(super) async fn start(
    scope: &Scope,
    backend: Backend,
    typed: bool,
    relay_socket: Option<&std::path::Path>,
) -> Result<Transport, AppError> {
    let command = std::env::current_exe()?
        .into_os_string()
        .into_string()
        .map_err(|_| {
            AppError::Config("The Agena executable path must be UTF-8 to use ACP stdio MCP.".into())
        })?;
    let (listener, address) = match relay_socket {
        #[cfg(unix)]
        Some(path) => {
            let address = path
                .to_str()
                .ok_or_else(|| AppError::Config("The ACP MCP relay path must be UTF-8.".into()))?;
            (
                Listener::Unix(tokio::net::UnixListener::bind(path)?),
                format!("{}{address}", agena_mcp_server::stdio_relay::UNIX_PREFIX),
            )
        }
        #[cfg(windows)]
        Some(path) => {
            let name = path
                .to_str()
                .ok_or_else(|| AppError::Config("The ACP MCP named pipe must be UTF-8.".into()))?;
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(name)?;
            (
                Listener::Pipe {
                    name: name.to_owned(),
                    server,
                },
                name.to_owned(),
            )
        }
        #[cfg(not(any(unix, windows)))]
        Some(_) => {
            return Err(AppError::Config(
                "The ACP MCP path relay needs a supported local IPC transport.".into(),
            ));
        }
        None => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
            let address = listener.local_addr()?.to_string();
            (Listener::Loopback(listener), address)
        }
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    let mut descriptor = json!({
        "name": "agena", "command": command, "args": [WORKER_ARG, address],
        "env": [{"name": TOKEN_ENV, "value": token}]
    });
    if typed {
        descriptor["type"] = json!("stdio");
    }
    Ok(Transport::Local {
        descriptor,
        server: tokio::spawn(serve(listener, backend, token, scope.closing.clone())),
    })
}

async fn serve(
    mut listener: Listener,
    backend: Backend,
    token: String,
    closing: CancellationToken,
) -> std::io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = closing.cancelled() => return Ok(()),
            _ = connections.join_next(), if !connections.is_empty() => {}
            client = listener.accept() => {
                let socket = client?;
                if connections.len() < MAX_CONNECTIONS {
                    let backend = backend.clone();
                    let token = token.clone();
                    let closing = closing.child_token();
                    match socket {
                        Socket::Loopback(socket) => {
                            connections.spawn(connect(socket, backend, token, closing));
                        }
                        #[cfg(unix)]
                        Socket::Unix(socket) => {
                            connections.spawn(connect(socket, backend, token, closing));
                        }
                        #[cfg(windows)]
                        Socket::Pipe(socket) => {
                            connections.spawn(connect(socket, backend, token, closing));
                        }
                    }
                }
            }
        }
    }
}

async fn connect<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    mut socket: T,
    backend: Backend,
    token: String,
    closing: CancellationToken,
) {
    let mut received = [0; TOKEN_BYTES];
    if !matches!(
        tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut received)).await,
        Ok(Ok(_))
    ) || received != token.as_bytes()
    {
        return;
    }
    let (read, write) = tokio::io::split(socket);
    let incoming = FramedRead::new(
        read,
        JsonRpcMessageCodec::<ClientJsonRpcMessage>::new_with_max_length(MAX_MESSAGE_BYTES),
    )
    .filter_map(|message| std::future::ready(message.ok()));
    let outgoing = FramedWrite::new(write, JsonRpcMessageCodec::<ServerJsonRpcMessage>::new());
    if let Err(error) =
        agena_mcp_server::serve_tools_transport(backend, (outgoing, incoming), closing).await
    {
        tracing::debug!(%error, "ACP stdio MCP connection closed");
    }
}
