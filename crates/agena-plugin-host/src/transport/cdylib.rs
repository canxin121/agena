//! Cdylib transport — loads an `abi_stable` shared library and isolates all
//! calls to it on one dedicated, bounded actor thread.
//!
//! Native code cannot be force-cancelled safely. Running it on Tokio's global
//! blocking pool would let a wedged plugin permanently consume shared pool
//! capacity after the async caller times out. A per-plugin actor contains that
//! failure to one OS thread and bounds queued work.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use abi_stable::library::RootModule;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::cdylib_abi::AgenaPluginCdylib_Ref;
use crate::transport::PluginTransport;

const ACTOR_QUEUE_CAPACITY: usize = 64;
const ACTOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Plugin transport over a cdylib ABI.
pub struct CdylibTransport {
    commands: mpsc::Sender<ActorCommand>,
    actor: Mutex<Option<std::thread::JoinHandle<()>>>,
    closed: AtomicBool,
}

enum ActorCommand {
    Dispatch {
        method: String,
        params: String,
        response: oneshot::Sender<Result<serde_json::Value, TransportError>>,
    },
    Shutdown {
        complete: oneshot::Sender<()>,
    },
}

impl CdylibTransport {
    pub fn load(path: &Path) -> Result<Self, TransportError> {
        let module = AgenaPluginCdylib_Ref::load_from_file(path).map_err(|error| {
            TransportError::Io(format!("load cdylib `{}`: {error}", path.display()))
        })?;
        let (commands, receiver) = mpsc::channel(ACTOR_QUEUE_CAPACITY);
        let actor = std::thread::Builder::new()
            .name("agena-cdylib-plugin".to_string())
            .spawn(move || run_actor(module, receiver))
            .map_err(TransportError::from)?;
        Ok(Self {
            commands,
            actor: Mutex::new(Some(actor)),
            closed: AtomicBool::new(false),
        })
    }
}

fn run_actor(module: AgenaPluginCdylib_Ref, mut receiver: mpsc::Receiver<ActorCommand>) {
    while let Some(command) = receiver.blocking_recv() {
        match command {
            ActorCommand::Dispatch {
                method,
                params,
                response,
            } => {
                // A timed-out/cancelled caller may leave a command in the
                // bounded mailbox. Skip it before entering native code.
                if response.is_closed() {
                    continue;
                }
                if response
                    .send(dispatch_native(module, method, params))
                    .is_err()
                {
                    tracing::debug!(
                        "native plugin dispatch result receiver was dropped before completion"
                    );
                }
            }
            ActorCommand::Shutdown { complete } => {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (module.shutdown())()))
                    .is_err()
                {
                    tracing::error!("native plugin panicked during shutdown");
                }
                if complete.send(()).is_err() {
                    tracing::debug!("native plugin shutdown waiter was already dropped");
                }
                return;
            }
        }
    }
}

fn dispatch_native(
    module: AgenaPluginCdylib_Ref,
    method: String,
    params: String,
) -> Result<serde_json::Value, TransportError> {
    let dispatch = module.dispatch();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(method.into(), params.into())
    }));
    match result {
        Ok(result) => match result.into_result() {
            Ok(value) => {
                let value: String = value.into();
                serde_json::from_str(&value).map_err(TransportError::from)
            }
            Err(error) => {
                let error: String = error.into();
                let plugin_error = serde_json::from_str::<PluginError>(&error)
                    .unwrap_or_else(|_| PluginError::internal(error));
                Err(TransportError::Plugin(plugin_error))
            }
        },
        Err(payload) => Err(TransportError::panicked(payload)),
    }
}

#[async_trait]
impl PluginTransport for CdylibTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(TransportError::disconnected(
                "cdylib plugin actor is already closed",
            ));
        }
        let params = serde_json::to_string(&params)?;
        let (response, result) = oneshot::channel();
        self.commands
            .send(ActorCommand::Dispatch {
                method: method.to_string(),
                params,
                response,
            })
            .await
            .map_err(|error| {
                TransportError::disconnected_error(
                    "cdylib plugin actor command channel closed before dispatch",
                    &error,
                )
            })?;
        result.await.map_err(|error| {
            TransportError::disconnected_error(
                "cdylib plugin actor exited before returning the dispatch result",
                &error,
            )
        })?
    }

    async fn close(&self) -> Result<(), TransportError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let (complete, completed) = oneshot::channel();
        tokio::time::timeout(
            ACTOR_SHUTDOWN_TIMEOUT,
            self.commands.send(ActorCommand::Shutdown { complete }),
        )
        .await
        .map_err(|error| {
            TransportError::timeout_error(
                format!(
                    "cdylib plugin actor shutdown command was not accepted within {}ms",
                    ACTOR_SHUTDOWN_TIMEOUT.as_millis()
                ),
                &error,
            )
        })?
        .map_err(|error| {
            TransportError::disconnected_error(
                "cdylib plugin actor command channel closed during shutdown",
                &error,
            )
        })?;
        tokio::time::timeout(ACTOR_SHUTDOWN_TIMEOUT, completed)
            .await
            .map_err(|error| {
                TransportError::timeout_error(
                    format!(
                        "cdylib plugin actor did not acknowledge shutdown within {}ms",
                        ACTOR_SHUTDOWN_TIMEOUT.as_millis()
                    ),
                    &error,
                )
            })?
            .map_err(|error| {
                TransportError::disconnected_error(
                    "cdylib plugin actor exited without acknowledging shutdown",
                    &error,
                )
            })?;

        let actor = self
            .actor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(actor) = actor {
            tokio::task::spawn_blocking(move || actor.join())
                .await
                .map_err(|error| {
                    TransportError::Rpc(agena_failure::diagnostic::format_error_chain_with_context(
                        "cdylib plugin actor join task failed",
                        &error,
                    ))
                })?
                .map_err(TransportError::panicked)?;
        }
        Ok(())
    }
}

impl Drop for CdylibTransport {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let (complete, _completed) = oneshot::channel();
        if let Err(error) = self.commands.try_send(ActorCommand::Shutdown { complete }) {
            tracing::warn!(
                diagnostic = %error,
                "native plugin shutdown command could not be queued; the isolated actor may outlive the transport"
            );
        }
        // Never join here: native code may be permanently wedged. The actor
        // is isolated and may outlive this value without blocking teardown.
        let _ = self
            .actor
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}
