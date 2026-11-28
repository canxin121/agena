//! An already-authorized, DNS-free TCP connection for full-duplex protocols.

use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    process::Stdio,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, copy, join},
    net::TcpStream,
};

use crate::{ManagedChild, sandbox::SandboxPolicy, worker_ipc as ipc};

const ARGUMENT: &str = "--agena-tcp-worker";

trait SocketIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SocketIo for T {}

pub struct Socket {
    io: Box<dyn SocketIo>,
    peer_addr: SocketAddr,
    _worker: Option<(ManagedChild, tempfile::TempDir)>,
}

impl Socket {
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
}

impl AsyncRead for Socket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_read(context, buffer)
    }
}

impl AsyncWrite for Socket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.io).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.io).poll_shutdown(context)
    }
}

/// Connect only to numeric addresses approved by the caller. Dropping the
/// future or socket closes its pipes and kills any sandboxed connection worker.
pub async fn connect(mut policy: SandboxPolicy, addresses: Vec<SocketAddr>) -> io::Result<Socket> {
    validate(&addresses)?;
    if policy.disabled {
        let stream = TcpStream::connect(addresses.as_slice()).await?;
        stream.set_nodelay(true)?;
        return Ok(Socket {
            peer_addr: stream.peer_addr()?,
            io: Box::new(stream),
            _worker: None,
        });
    }

    policy.readonly_paths.clear();
    policy.readwrite_paths.clear();
    policy.private_paths.clear();
    policy.configured_filesystem = true;
    let (scratch, mut command) = crate::worker::command(ARGUMENT, &policy)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = crate::spawn(command)?;
    let mut input = child
        .stdin()
        .take()
        .ok_or_else(|| io::Error::other("TCP worker stdin is unavailable"))?;
    let mut output = child
        .stdout()
        .take()
        .ok_or_else(|| io::Error::other("TCP worker stdout is unavailable"))?;
    ipc::write_json(&mut input, &addresses).await?;
    let connected: Result<SocketAddr, String> = ipc::read_json(&mut output).await?;
    Ok(Socket {
        peer_addr: connected.map_err(io::Error::other)?,
        io: Box::new(join(output, input)),
        _worker: Some((child, scratch)),
    })
}

fn validate(addresses: &[SocketAddr]) -> io::Result<()> {
    if addresses.is_empty()
        || addresses.len() > 128
        || addresses.iter().any(|address| address.port() == 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TCP worker requires a bounded list of numeric endpoints",
        ));
    }
    Ok(())
}

async fn worker() -> io::Result<()> {
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let addresses: Vec<SocketAddr> = ipc::read_json(&mut input).await?;
    validate(&addresses)?;
    let mut stream = match TcpStream::connect(addresses.as_slice()).await {
        Ok(stream) => stream,
        Err(error) => {
            return ipc::write_json(&mut output, &Err::<SocketAddr, _>(error.to_string())).await;
        }
    };
    stream.set_nodelay(true)?;
    ipc::write_json(&mut output, &Ok::<_, String>(stream.peer_addr()?)).await?;
    let (mut from_peer, mut to_peer) = stream.split();
    let receive = async {
        let mut buffer = [0; 8192];
        loop {
            let count = from_peer.read(&mut buffer).await?;
            if count == 0 {
                return Ok::<_, io::Error>(());
            }
            output.write_all(&buffer[..count]).await?;
            output.flush().await?;
        }
    };
    tokio::select! {
        result = copy(&mut input, &mut to_peer) => { result?; }
        result = receive => result?,
    }
    Ok(())
}

pub fn try_run_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let result = (|| {
        if std::env::args_os().count() != 2 {
            return Err(io::Error::other("invalid TCP worker arguments"));
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(worker());
        // A pending stdin read is blocking. The process exits immediately after
        // this function returns, so avoid waiting for that read during shutdown.
        runtime.shutdown_background();
        result
    })();
    Some(if result.is_ok() { 0 } else { 1 })
}
