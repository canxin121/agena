use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    PluginTransport, http::HttpTransport, inproc::InProcessTransport, quiescent::QuiescentTransport,
};
use crate::sdk::{CancellationToken, Plugin, PluginManifest, ToolInvokeInput, ToolInvokeOutput};

struct ProbePlugin {
    started: mpsc::UnboundedSender<(i64, CancellationToken)>,
    stopped: mpsc::UnboundedSender<i64>,
}

struct Stopped {
    call_id: i64,
    sender: mpsc::UnboundedSender<i64>,
}

impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.sender.send(self.call_id);
    }
}

#[async_trait]
impl Plugin for ProbePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "cancellation", "1.0.0")
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> crate::sdk::Result<ToolInvokeOutput> {
        let _stopped = Stopped {
            call_id: input.call_id,
            sender: self.stopped.clone(),
        };
        let _ = self
            .started
            .send((input.call_id, input.context().cancellation));
        std::future::pending().await
    }
}

fn input(call_id: i64) -> ToolInvokeInput {
    ToolInvokeInput {
        tool_name: "wait".into(),
        session_id: 1,
        call_id,
        workspace_root: ".".into(),
        input: serde_json::json!({}),
    }
}

async fn receive<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cancellation_stops_one_inproc_request_and_keeps_its_peer_running() {
    let (started, mut starts) = mpsc::unbounded_channel();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let transport = Arc::new(InProcessTransport::new(ProbePlugin { started, stopped }));
    let first = transport.clone();
    let first = tokio::spawn(async move {
        first
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(1)).unwrap(),
            )
            .await
    });
    let (id, first_signal) = receive(&mut starts).await;
    assert_eq!(id, 1);
    let second = transport.clone();
    let second = tokio::spawn(async move {
        second
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(2)).unwrap(),
            )
            .await
    });
    let (id, second_signal) = receive(&mut starts).await;
    assert_eq!(id, 2);
    first.abort();
    let _ = first.await;
    assert_eq!(receive(&mut stops).await, 1);
    assert!(first_signal.is_cancelled());
    assert!(!second_signal.is_cancelled());
    assert!(!second.is_finished());
    second.abort();
    let _ = second.await;
    assert_eq!(receive(&mut stops).await, 2);
}

#[tokio::test]
async fn cancellation_of_quiescent_stream_reaches_the_plugin_after_setup() {
    let (started, mut starts) = mpsc::unbounded_channel();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let transport = QuiescentTransport::new(Arc::new(InProcessTransport::new(ProbePlugin {
        started,
        stopped,
    })));
    let stream = transport.invoke_stream(input(7)).await.unwrap().unwrap();
    let (id, signal) = receive(&mut starts).await;
    assert_eq!(id, 7);
    assert!(!signal.is_cancelled());
    drop(stream);
    assert_eq!(receive(&mut stops).await, 7);
    assert!(signal.is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), async {
        while transport.active_calls() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn forced_close_stops_retained_calls_and_streams_while_graceful_shutdown_waits() {
    let (started, mut starts) = mpsc::unbounded_channel();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let transport = Arc::new(QuiescentTransport::new(Arc::new(InProcessTransport::new(
        ProbePlugin { started, stopped },
    ))));
    let calling = transport.clone();
    let call = tokio::spawn(async move {
        calling
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(20)).unwrap(),
            )
            .await
    });
    let (id, call_signal) = receive(&mut starts).await;
    assert_eq!(id, 20);
    let stream = transport.invoke_stream(input(21)).await.unwrap().unwrap();
    let (id, stream_signal) = receive(&mut starts).await;
    assert_eq!(id, 21);

    let shutting_down = transport.clone();
    let graceful = tokio::spawn(async move { shutting_down.shutdown().await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while transport.is_accepting() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!call.is_finished());
    assert!(!graceful.is_finished());
    assert!(!call_signal.is_cancelled());
    assert!(!stream_signal.is_cancelled());

    // Keep both consumers alive: force close must stop the bodies itself and
    // release the graceful waiter that currently owns the lifecycle lock.
    tokio::time::timeout(Duration::from_secs(5), transport.close())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        call.await.unwrap(),
        Err(crate::error::TransportError::Cancelled)
    ));
    assert_eq!(
        stream.end.await.unwrap().unwrap_err().kind,
        crate::sdk::PluginErrorKind::Cancelled
    );
    tokio::time::timeout(Duration::from_secs(5), graceful)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut stopped = [receive(&mut stops).await, receive(&mut stops).await];
    stopped.sort();
    assert_eq!(stopped, [20, 21]);
    assert!(call_signal.is_cancelled());
    assert!(stream_signal.is_cancelled());
    assert_eq!(transport.active_calls(), 0);
}

struct ShutdownProbe(ProbePlugin);

#[async_trait]
impl Plugin for ShutdownProbe {
    fn manifest(&self) -> PluginManifest {
        self.0.manifest()
    }

    async fn shutdown(&self) -> crate::sdk::Result<()> {
        self.0.tool_invoke(input(-1)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn forced_close_interrupts_a_shutdown_hook_holding_the_lifecycle_lock() {
    let (started, mut starts) = mpsc::unbounded_channel();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let transport = Arc::new(QuiescentTransport::new(Arc::new(InProcessTransport::new(
        ShutdownProbe(ProbePlugin { started, stopped }),
    ))));
    let shutting_down = transport.clone();
    let graceful = tokio::spawn(async move { shutting_down.shutdown().await });
    let (id, signal) = receive(&mut starts).await;
    assert_eq!(id, -1);
    assert!(!signal.is_cancelled());
    assert!(!graceful.is_finished());
    tokio::time::timeout(Duration::from_secs(5), transport.close())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), graceful)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(receive(&mut stops).await, -1);
    assert!(signal.is_cancelled());
}

#[tokio::test]
async fn cancellation_of_http_request_stops_the_sdk_handler_without_closing_its_instance() {
    let (started, mut starts) = mpsc::unbounded_channel();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let router = crate::sdk::drivers::http::router(
        move || ProbePlugin {
            started: started.clone(),
            stopped: stopped.clone(),
        },
        Arc::new(crate::sdk::host_api::NoopHostClient),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let transport = Arc::new(HttpTransport::new(
        format!("http://{address}/rpc").parse().unwrap(),
        crate::config::HttpAuth::None,
        &|_| None,
        false,
    ));
    let context = crate::sdk::InitContext {
        agena_version: "test".into(),
        workspace_root: ".".into(),
        plugin_id: crate::sdk::PluginKey::new("test", "cancellation").unwrap(),
        host_callback_url: None,
        host_callback_token: None,
        settings: serde_json::json!({}),
        protocol_version: crate::sdk::rpc::PROTOCOL_VERSION,
    };
    transport
        .dispatch(
            crate::sdk::rpc::method::META_INIT,
            serde_json::to_value(context).unwrap(),
        )
        .await
        .unwrap();
    let task_transport = transport.clone();
    let task = tokio::spawn(async move {
        task_transport
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(9)).unwrap(),
            )
            .await
    });
    let (id, signal) = receive(&mut starts).await;
    assert_eq!(id, 9);
    task.abort();
    let _ = task.await;
    assert_eq!(receive(&mut stops).await, 9);
    assert!(signal.is_cancelled());
    transport
        .dispatch(crate::sdk::rpc::method::META_PING, serde_json::json!({}))
        .await
        .unwrap();
    transport
        .dispatch(
            crate::sdk::rpc::method::META_SHUTDOWN,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    server.abort();
}

#[cfg(unix)]
const STDIO_PROBE: &str = r#"
import json, sys
active, cancelled = {}, []
def send(value):
    data = json.dumps(value).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(data)).encode() + data)
    sys.stdout.buffer.flush()
while True:
    line = sys.stdin.buffer.readline()
    if not line:
        break
    length = int(line.split(b':', 1)[1])
    while sys.stdin.buffer.readline().strip():
        pass
    req = json.loads(sys.stdin.buffer.read(length))
    method = req['method']
    if method == '$/cancelRequest':
        call = active.pop(req['params']['id'], None)
        if call is not None:
            cancelled.append(call)
        continue
    if method in ('hooks/tool.invoke', 'hooks/tool.invoke.stream'):
        active[req['id']] = req['params']['call_id']
        if method.endswith('.stream'):
            send({'jsonrpc':'2.0', 'id':req['id'], 'result':{'stream_id':'stream-%s' % req['id']}})
        continue
    send({'jsonrpc':'2.0', 'id':req['id'], 'result':{'active':list(active.values()), 'cancelled':cancelled}})
"#;

#[cfg(unix)]
async fn stdio_probe() -> Arc<super::stdio::StdioTransport> {
    Arc::new(
        super::stdio::StdioTransport::spawn(
            "python3",
            &["-u".into(), "-c".into(), STDIO_PROBE.into()],
            &std::collections::HashMap::new(),
            None,
            None,
        )
        .await
        .unwrap(),
    )
}

#[cfg(unix)]
async fn wait_stdio_state(transport: &dyn PluginTransport, expected: serde_json::Value) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let observed = transport
                .dispatch("inspect", serde_json::json!({}))
                .await
                .unwrap();
            if observed == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_of_stdio_request_reaches_its_process_without_cancelling_a_peer() {
    let transport = stdio_probe().await;
    let first_transport = transport.clone();
    let first = tokio::spawn(async move {
        first_transport
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(1)).unwrap(),
            )
            .await
    });
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[1], "cancelled":[]}),
    )
    .await;
    let second_transport = transport.clone();
    let second = tokio::spawn(async move {
        second_transport
            .dispatch(
                crate::sdk::rpc::method::HOOK_TOOL_INVOKE,
                serde_json::to_value(input(2)).unwrap(),
            )
            .await
    });
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[1,2], "cancelled":[]}),
    )
    .await;
    first.abort();
    let _ = first.await;
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[2], "cancelled":[1]}),
    )
    .await;
    assert!(!second.is_finished());
    second.abort();
    let _ = second.await;
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[], "cancelled":[1,2]}),
    )
    .await;
    transport.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_of_stdio_stream_uses_the_originating_request_after_setup() {
    let transport = stdio_probe().await;
    let wrapped = QuiescentTransport::new(transport.clone());
    let stream = wrapped.invoke_stream(input(7)).await.unwrap().unwrap();
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[7], "cancelled":[]}),
    )
    .await;
    drop(stream);
    wait_stdio_state(
        transport.as_ref(),
        serde_json::json!({"active":[], "cancelled":[7]}),
    )
    .await;
    transport.close().await.unwrap();
}
