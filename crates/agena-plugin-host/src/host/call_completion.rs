//! Completion of an originating Host→Plugin call, shared by nested callbacks.
//! Control work that can retire that plugin must wait outside its call stack.

use std::future::Future;

use tokio_util::sync::{CancellationToken, DropGuard};

use super::{HostCallbackContext, HostHandle};

tokio::task_local! {
    static CURRENT_CALL: PluginCallCompletion;
}

/// A completion signal, without ownership of the originating host or plugin.
#[derive(Debug, Clone)]
pub struct PluginCallCompletion {
    id: String,
    finished: CancellationToken,
}

impl PluginCallCompletion {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn wait(&self) {
        self.finished.cancelled().await;
    }

    pub(super) async fn scope<T>(&self, future: impl Future<Output = T>) -> T {
        CURRENT_CALL.scope(self.clone(), future).await
    }
}

/// Available only inside an admitted plugin call/callback. Ingress restores
/// the completion from a validated authority; callers cannot supply a signal.
pub fn current_plugin_call_completion() -> Option<PluginCallCompletion> {
    CURRENT_CALL.try_with(Clone::clone).ok()
}

pub(super) struct CallCompletionLease {
    pub(super) completion: PluginCallCompletion,
    _finish: Option<DropGuard>,
}

impl CallCompletionLease {
    fn new(inherited: Option<PluginCallCompletion>) -> Self {
        match inherited {
            Some(completion) => Self {
                completion,
                _finish: None,
            },
            None => {
                let finished = CancellationToken::new();
                Self {
                    completion: PluginCallCompletion {
                        id: uuid::Uuid::new_v4().to_string(),
                        finished: finished.clone(),
                    },
                    _finish: Some(finished.drop_guard()),
                }
            }
        }
    }

    pub(super) fn for_outbound_call() -> Self {
        Self::new(current_plugin_call_completion())
    }
}

impl HostHandle {
    /// `context` has already passed this handle's authority validation.
    pub(super) fn run_in_callback_completion<T>(
        &self,
        context: &HostCallbackContext,
        future: impl Future<Output = crate::sdk::Result<T>>,
    ) -> impl Future<Output = crate::sdk::Result<T>> {
        // Box before creating the async cancellation/scope layers. The RPC
        // handler is large; retaining it inline through every nested scope
        // multiplies its poll stack, especially for callback -> service calls.
        let future = Box::pin(future);
        // Retain the exact authority's signal before the first await. A
        // revocation racing with admission must never detach a callback from
        // its originating call and give it a fresh independent lifetime.
        let authority = context
            .authority_token
            .as_ref()
            .map(|token| {
                super::host_handle::recover_mutex(
                    &self.callback_authorities,
                    "plugin callback authority registry",
                )
                .get(token)
                .cloned()
                .ok_or_else(|| {
                    crate::sdk::PluginError::from_kind(
                        crate::sdk::PluginErrorKind::Cancelled,
                        "originating plugin call has ended",
                    )
                })
            })
            .transpose();
        async move {
            let authority = authority?;
            let lease = CallCompletionLease::new(
                authority.as_ref().map(|record| record.completion.clone()),
            );
            lease.completion.scope(async move {
                let Some(authority) = authority else { return future.await; };
                tokio::select! {
                    biased;
                    _ = authority.cancellation.cancelled() => Err(crate::sdk::PluginError::from_kind(
                        crate::sdk::PluginErrorKind::Cancelled,
                        "originating plugin call has ended",
                    )),
                    result = future => result,
                }
            }).await
        }
    }
}
