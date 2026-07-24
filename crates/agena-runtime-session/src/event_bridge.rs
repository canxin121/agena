//! Generic event-to-plugin forwarding owned by the runtime layer.
//!
//! Concrete event buses stay in their domain/application crates. They adapt
//! their subscription to [`RuntimeEventSubscription`] and provide the small
//! event-to-plugin-envelope projection; the receive loop and lifecycle guard
//! remain runtime-owned.

use std::{future::Future, pin::Pin, sync::Arc};

use agena_plugin_host::{EventEnvelope, PluginHost};

/// Item delivered by a runtime event subscription.
#[derive(Debug, Clone)]
pub enum RuntimeEventSubscriptionItem<E> {
    Event(E),
    Lagged(u64),
}

/// Minimal adapter required for a concrete event bus subscription.
pub trait RuntimeEventSubscription: Send + 'static {
    type Event: Send + 'static;

    fn recv<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Option<RuntimeEventSubscriptionItem<Self::Event>>> + Send + 'a>>;
}

/// Forward mapped subscription events to plugins until the subscription closes
/// or the returned snapshot-scoped guard is dropped.
pub fn spawn_event_forwarder<S, F>(
    mut subscription: S,
    plugins: Arc<PluginHost>,
    mut map_event: F,
) -> crate::AbortOnDrop
where
    S: RuntimeEventSubscription,
    F: FnMut(S::Event) -> Option<EventEnvelope> + Send + 'static,
{
    crate::spawn_abortable(async move {
        while let Some(item) = subscription.recv().await {
            let event = match item {
                RuntimeEventSubscriptionItem::Event(event) => event,
                RuntimeEventSubscriptionItem::Lagged(count) => {
                    tracing::warn!(
                        target: "agena_plugin_host::events",
                        lagged = count,
                        "plugin event bridge lagged"
                    );
                    continue;
                }
            };
            let Some(envelope) = map_event(event) else {
                continue;
            };
            plugins.broadcast_event(envelope).await;
        }
    })
}
