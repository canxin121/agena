//! Ephemeral runtime live signals (v2, design 14.3).
//!
//! Session data live updates are [`SessionChange`](agena_storage::SessionChange)
//! part patches delivered by the sealed facade's notification bus (14.3) —
//! never an event log. This module is the parallel, much smaller surface for
//! runtime signals that are not session parts: background-activity changes,
//! plugin-published events, and tool-registry changes. They are observer
//! notification only: never persisted, never replayed, no causality chain.
//! A consumer must never rely on receiving every signal (the channel is
//! bounded; overflow surfaces as [`RuntimeLiveSignalItem::Lagged`]).

use std::future::Future;
use std::pin::Pin;

use agena_domain::BackgroundActivityChangedEvent;
use agena_plugin_host::PluginKey;
use tokio::sync::broadcast;

/// One ephemeral runtime signal for in-process presentation consumers.
#[derive(Debug, Clone)]
pub enum RuntimeLiveSignal {
    /// A background activity started, updated, or finished.
    Activity(Box<BackgroundActivityChangedEvent>),
    /// A plugin published an event for presentation consumers.
    Plugin {
        session_id: Option<i64>,
        plugin_id: PluginKey,
        kind_label: String,
        payload: serde_json::Value,
    },
    /// The plugin tool registry changed.
    ToolRegistryChanged(Box<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>),
}

/// Item received on a live signal subscription.
#[derive(Debug, Clone)]
pub enum RuntimeLiveSignalItem {
    Signal(RuntimeLiveSignal),
    /// The signal channel overflowed; consumers must re-read state (14.4).
    Lagged(u64),
}

/// A live subscription to runtime signals.
pub trait RuntimeLiveSignalSubscription: Send {
    fn recv<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Option<RuntimeLiveSignalItem>> + Send + 'a>>;
}

/// Stable live-signal port replacing the v1 runtime event stream for
/// ephemeral signals (14.3, D11). Session data does not flow through here —
/// it is `SessionChange` on the facade.
pub trait RuntimeLiveSignalService: Send + Sync {
    fn subscribe(&self) -> Box<dyn RuntimeLiveSignalSubscription>;
}

/// A `tokio::sync::broadcast`-backed live signal stream.
#[derive(Debug, Clone)]
pub(crate) struct LiveSignalHub {
    tx: broadcast::Sender<RuntimeLiveSignal>,
}

impl LiveSignalHub {
    pub(crate) fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish one signal to every subscriber. If the channel is full the
    /// oldest unread signals are dropped and subscribers observe `Lagged`
    /// (14.4); the signal itself is never persisted.
    pub(crate) fn emit(&self, signal: RuntimeLiveSignal) {
        let _ = self.tx.send(signal);
    }

    /// Subscribe to the stream. Returns `None` when no Tokio runtime is
    /// available (CLI one-shot compositions stay safe).
    pub(crate) fn subscribe(&self) -> Option<Box<dyn RuntimeLiveSignalSubscription + Send>> {
        tokio::runtime::Handle::try_current().ok()?;
        Some(Box::new(LiveSignalSubscription {
            rx: self.tx.subscribe(),
        }))
    }
}

struct LiveSignalSubscription {
    rx: broadcast::Receiver<RuntimeLiveSignal>,
}

impl RuntimeLiveSignalSubscription for LiveSignalSubscription {
    fn recv<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Option<RuntimeLiveSignalItem>> + Send + 'a>> {
        Box::pin(async move {
            match self.rx.recv().await {
                Ok(signal) => Some(RuntimeLiveSignalItem::Signal(signal)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    Some(RuntimeLiveSignalItem::Lagged(skipped))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
    }
}
