//! Claude Code's stdio channel notifications and one-time permission relay.

use std::collections::BTreeMap;

use rmcp::model::CustomNotification;
use rmcp::service::{Peer, RoleClient};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::*;

const CAPABILITY: &str = "claude/channel";
const PERMISSION_CAPABILITY: &str = "claude/channel/permission";
const MESSAGE: &str = "notifications/claude/channel";
const PERMISSION: &str = "notifications/claude/channel/permission";
const PERMISSION_REQUEST: &str = "notifications/claude/channel/permission_request";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaudeChannelEvent {
    Message {
        content: String,
        meta: BTreeMap<String, String>,
    },
    Permission {
        request_id: String,
        allowed: bool,
    },
}

pub(super) struct ChannelEvents(broadcast::Sender<ClaudeChannelEvent>);

impl Default for ChannelEvents {
    fn default() -> Self {
        Self(broadcast::channel(64).0)
    }
}

impl ChannelEvents {
    fn subscribe(&self) -> broadcast::Receiver<ClaudeChannelEvent> {
        self.0.subscribe()
    }

    pub(super) fn handle(
        &self,
        notification: &CustomNotification,
        peer: &Peer<RoleClient>,
        redactions: &McpRedactions,
    ) -> bool {
        if !matches!(notification.method.as_str(), MESSAGE | PERMISSION) {
            return false;
        }
        if self.0.receiver_count() == 0 || !has_capability(peer, CAPABILITY) {
            return true;
        }
        if let Some(params) = notification.params.as_ref() {
            let event = match notification.method.as_str() {
                MESSAGE => decode_message(params, redactions),
                PERMISSION if has_capability(peer, PERMISSION_CAPABILITY) => {
                    decode_permission(params)
                }
                _ => None,
            };
            if let Some(event) = event {
                let _ = self.0.send(event);
            }
        }
        true
    }
}

#[derive(Clone)]
pub struct ClaudeChannelPeer(Arc<ConnectedServer>);

impl ClaudeChannelPeer {
    pub fn connected(&self) -> bool {
        !self.0.peer.is_transport_closed() && !self.0.events.shutdown.is_cancelled()
    }

    pub fn supports_permission_relay(&self) -> bool {
        has_capability(&self.0.peer, PERMISSION_CAPABILITY)
    }

    pub fn instructions(&self) -> Option<&str> {
        self.0.instructions.as_deref()
    }

    pub async fn request_permission(
        &self,
        request_id: &str,
        tool_name: &str,
        description: &str,
        input_preview: &str,
    ) -> McpResult<()> {
        if !self.connected() || !self.supports_permission_relay() {
            return Err(McpError::PermissionDenied);
        }
        self.0
            .peer
            .send_notification(
                CustomNotification::new(
                    PERMISSION_REQUEST,
                    Some(json!({
                        "request_id": request_id,
                        "tool_name": tool_name,
                        "description": self.0.redactions.text(description.to_owned()),
                        "input_preview": self.0.redactions.text(input_preview.to_owned()),
                    })),
                )
                .into(),
            )
            .await
            .map_err(McpError::from)
    }
}

pub struct ClaudeChannelStream {
    peer: ClaudeChannelPeer,
    events: broadcast::Receiver<ClaudeChannelEvent>,
}

impl ClaudeChannelStream {
    fn new(
        server: Arc<ConnectedServer>,
        startup_events: Option<broadcast::Receiver<ClaudeChannelEvent>>,
    ) -> McpResult<Self> {
        if !has_capability(&server.peer, CAPABILITY) {
            return Err(McpError::Malformed(
                "MCP server does not advertise the claude/channel capability".into(),
            ));
        }
        let events = startup_events.unwrap_or_else(|| server.events.claude_channel.subscribe());
        Ok(Self {
            peer: ClaudeChannelPeer(server),
            events,
        })
    }

    pub fn peer(&self) -> &ClaudeChannelPeer {
        &self.peer
    }

    pub async fn next(&mut self) -> Option<ClaudeChannelEvent> {
        loop {
            tokio::select! {
                biased;
                _ = self.peer.0.events.shutdown.cancelled() => return None,
                event = self.events.recv() => match event {
                    Ok(event) => return Some(event),
                    Err(broadcast::error::RecvError::Closed) => return None,
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(server = %self.peer.0.name, count, "MCP channel consumer lagged");
                    }
                },
                _ = self.peer.0.peer.transport_closed() => return None,
            }
        }
    }
}

impl McpConnectionManager {
    /// Listen on an authorized scope. New transports can send events as soon
    /// as they receive `initialized`, so subscribe before opening one.
    pub(super) async fn open_claude_channel(&self, name: &str) -> McpResult<ClaudeChannelStream> {
        if let Ok(server) = self.get(name).await {
            return ClaudeChannelStream::new(server, None);
        }
        let channel = ChannelEvents::default();
        let events = channel.subscribe();
        self.reconnect_with_channel_events(name, channel).await?;
        ClaudeChannelStream::new(self.get(name).await?, Some(events))
    }
}

fn has_capability(peer: &Peer<RoleClient>, capability: &str) -> bool {
    peer.peer_info().is_some_and(|info| {
        info.capabilities
            .experimental
            .as_ref()
            .is_some_and(|caps| caps.contains_key(capability))
    })
}

fn decode_message(params: &Value, redactions: &McpRedactions) -> Option<ClaudeChannelEvent> {
    let content = params.get("content")?.as_str()?;
    if content.is_empty() || content.len() > 64 * 1024 {
        return None;
    }
    let mut meta = BTreeMap::new();
    if let Some(values) = params.get("meta").and_then(Value::as_object) {
        for (key, value) in values.iter().take(32) {
            if key != "source"
                && !key.is_empty()
                && key.len() <= 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && let Some(value) = value.as_str().filter(|value| value.len() <= 1024)
            {
                meta.insert(key.clone(), redactions.text(value.to_owned()));
            }
        }
    }
    Some(ClaudeChannelEvent::Message {
        content: redactions.text(content.to_owned()),
        meta,
    })
}

fn decode_permission(params: &Value) -> Option<ClaudeChannelEvent> {
    let request_id = params.get("request_id")?.as_str()?;
    if request_id.len() != 5
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() && byte != b'l')
    {
        return None;
    }
    let allowed = match params.get("behavior")?.as_str()? {
        "allow" => true,
        "deny" => false,
        _ => return None,
    };
    Some(ClaudeChannelEvent::Permission {
        request_id: request_id.to_owned(),
        allowed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::oneshot;

    #[test]
    fn claude_channel_explicitly_disabled_relay_keeps_message_capability() {
        let capabilities: rmcp::model::ServerCapabilities = serde_json::from_value(json!({
            "experimental": {(CAPABILITY): {}, (PERMISSION_CAPABILITY): false}
        }))
        .unwrap();
        let experimental = capabilities.experimental.unwrap();
        assert!(experimental.contains_key(CAPABILITY));
        assert!(!experimental.contains_key(PERMISSION_CAPABILITY));
    }

    #[tokio::test]
    async fn claude_channel_captures_startup_messages_in_order_and_relays_permission() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let manager = McpConnectionManager::default();
            let channel = ChannelEvents::default();
            let buffered_events = channel.subscribe();
            let events = Arc::new(ServerEventState {
                claude_channel: channel,
                ..Default::default()
            });
            let handler = manager.client_handler(
                "fixture",
                events.clone(),
                McpToolPolicy::default(),
                Duration::from_secs(1),
                McpRedactions::default(),
            );
            let (server, client) = tokio::io::duplex(64 * 1024);
            let (processed, startup_complete) = oneshot::channel();
            let peer = tokio::spawn(async move {
                let (input, mut output) = tokio::io::split(server);
                let mut lines = BufReader::new(input).lines();
                let discovery: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(discovery["method"], "server/discover");
                let unsupported = json!({
                    "jsonrpc":"2.0", "id":discovery["id"],
                    "error":{"code":-32601,"message":"Unsupported fixture method"},
                });
                output.write_all(format!("{unsupported}\n").as_bytes()).await.unwrap();
                let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(request["method"], "initialize");
                let initialize = json!({
                    "jsonrpc":"2.0", "id":request["id"],
                    "result": {
                        "protocolVersion": "2025-11-25",
                        "serverInfo": {"name":"fixture", "version":"1.0"},
                        "capabilities": {"experimental": {(CAPABILITY): {}, (PERMISSION_CAPABILITY): {}}},
                        "instructions": "Reply to the fixture channel.",
                    }
                });
                output.write_all(format!("{initialize}\n").as_bytes()).await.unwrap();
                let _: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                for body in ["first", "second"] {
                    let message = json!({
                        "jsonrpc":"2.0", "method":MESSAGE,
                        "params":{"content":body,"meta":{"source":"spoof","chat_id":"room","invalid-key":"ignored"}},
                    });
                    output.write_all(format!("{message}\n").as_bytes()).await.unwrap();
                }
                // A response proves the ordered startup notifications reached the
                // client before the completed stream is handed to its caller.
                let ping = json!({"jsonrpc":"2.0", "id":"startup", "method":"ping"});
                output.write_all(format!("{ping}\n").as_bytes()).await.unwrap();
                let pong: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(pong["id"], "startup");
                assert!(pong.get("result").is_some());
                processed.send(()).unwrap();
                let approval: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(approval["method"], PERMISSION_REQUEST);
                assert_eq!(approval["params"]["request_id"], "abcde");
                assert_eq!(approval["params"]["tool_name"], "Bash");
                let verdict = json!({
                    "jsonrpc":"2.0", "method":PERMISSION,
                    "params":{"request_id":"abcde","behavior":"allow"},
                });
                output.write_all(format!("{verdict}\n").as_bytes()).await.unwrap();
                let _ = lines.next_line().await;
            });
            let running = handler.serve_with_lifecycle(client, client_lifecycle()).await.unwrap();
            startup_complete.await.unwrap();
            let connection = Arc::new(ConnectedServer {
                peer_id: Uuid::new_v4(),
                name: "fixture".into(),
                peer: running.peer().clone(),
                running: Mutex::new(Some(running)),
                events,
                network_target: None,
                instructions: Some("Reply to the fixture channel.".into()),
                tool_policy: McpToolPolicy::default(),
                request_timeout: Duration::from_secs(1),
                tool_timeout: Duration::from_secs(1),
                close_timeout: Duration::from_secs(1),
                connected_at_ms: 0,
                redactions: McpRedactions::default(),
            });
            let mut stream = ClaudeChannelStream::new(connection.clone(), Some(buffered_events)).unwrap();
            manager.inner.write().await.servers.insert("fixture".into(), connection);
            assert!(stream.peer().supports_permission_relay());
            for body in ["first", "second"] {
                assert_eq!(stream.next().await, Some(ClaudeChannelEvent::Message {
                    content: body.into(),
                    meta: BTreeMap::from([("chat_id".into(), "room".into())]),
                }));
            }
            stream.peer().request_permission("abcde", "Bash", "Run shell command", "{}").await.unwrap();
            assert_eq!(stream.next().await, Some(ClaudeChannelEvent::Permission {
                request_id: "abcde".into(),
                allowed: true,
            }));
            manager.shutdown_all().await;
            peer.await.unwrap();
        }).await.unwrap();
    }
}
