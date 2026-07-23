//! Adapter from the core event bus to the runtime-owned plugin forwarder.

use std::sync::Arc;

use crate::event::EventKind;
use crate::event::{
    EventBus,
    bus::{Subscription, SubscriptionItem},
};
use agena_domain::{EventFilter as BusEventFilter, EventScope};
use agena_plugin_host::{EventEnvelope, PluginHost};

/// Subscribes to every event on the unified bus and pushes each one to the
/// plugin host. Runtime owns the receive loop and lifecycle guard; this module
/// owns only the core event-bus subscription and envelope projection.
pub(crate) fn spawn_event_bridge(
    bus: Arc<dyn EventBus<EventKind>>,
    plugins: Arc<PluginHost>,
) -> agena_runtime::AbortOnDrop {
    let sub = bus.subscribe(BusEventFilter::new(EventScope::Global));
    agena_runtime::spawn_event_forwarder(sub, plugins, |event| {
        let payload = match serde_json::to_value(event.kind.clone()) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    target: "agena_plugin_host::events",
                    "failed to serialize event for plugin: {error}"
                );
                return None;
            }
        };
        Some(EventEnvelope {
            kind: event.kind.tag_str().to_owned(),
            timestamp_ms: event.meta.created_at.timestamp_millis(),
            session_id: event.meta.session_id,
            payload,
        })
    })
}

impl agena_runtime::RuntimeEventSubscription for Subscription<EventKind> {
    type Event = Arc<agena_domain::EventEnvelope<EventKind>>;

    fn recv<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Option<agena_runtime::RuntimeEventSubscriptionItem<Self::Event>>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.recv().await.map(|item| match item {
                SubscriptionItem::Event(event) => {
                    agena_runtime::RuntimeEventSubscriptionItem::Event(event)
                }
                SubscriptionItem::Lagged(count) => {
                    agena_runtime::RuntimeEventSubscriptionItem::Lagged(count)
                }
            })
        })
    }
}
