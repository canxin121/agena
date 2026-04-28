//! Bridge between agena's domain event bus and the plugin host. Spawns a
//! task that subscribes globally and forwards every event as an
//! `EventEnvelope` to plugins that subscribed via `HookSubscription::EVENT`.
//!
//! Returns a `JoinHandle` so the caller can abort the bridge on shutdown.

use std::sync::Arc;

use crate::event::{EventBus, EventFilter as BusEventFilter, Scope, bus::SubscriptionItem};
use tokio::task::JoinHandle;

use crate::event::EventKind;
use crate::plugin::{EventEnvelope, PluginHost};

/// Subscribes to every event on the unified bus and pushes each one to the
/// plugin host (which fans out to plugins that subscribed to `EVENT`). The
/// returned task ends only when the bus closes; abort it via the handle to
/// stop the bridge early.
pub fn spawn_event_bridge(
    bus: Arc<dyn EventBus<EventKind>>,
    plugins: Arc<PluginHost>,
) -> JoinHandle<()> {
    let mut sub = bus.subscribe(BusEventFilter::new(Scope::Global));
    tokio::spawn(async move {
        while let Some(item) = sub.recv().await {
            let event = match item {
                SubscriptionItem::Event(e) => e,
                SubscriptionItem::Lagged(n) => {
                    tracing::warn!(target: "agena_plugin_host::events", lagged = n, "plugin event bridge lagged");
                    continue;
                }
            };
            let payload = match serde_json::to_value(event.kind.clone()) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!(
                        target: "agena_plugin_host::events",
                        "failed to serialize event for plugin: {err}"
                    );
                    continue;
                }
            };
            let envelope = EventEnvelope {
                kind: event.kind.tag_str().to_string(),
                timestamp_ms: event.meta.created_at.timestamp_millis(),
                session_id: event.meta.session_id,
                payload,
            };
            plugins.broadcast_event(envelope).await;
        }
    })
}
