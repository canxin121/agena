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
    pub(super) async fn run_in_callback_completion<T>(
        &self,
        context: &HostCallbackContext,
        future: impl Future<Output = T>,
    ) -> T {
        let inherited = context.authority_token.as_ref().and_then(|token| {
            super::host_handle::recover_mutex(
                &self.callback_authorities,
                "plugin callback authority registry",
            )
            .get(token)
            .map(|record| record.completion.clone())
        });
        let lease = CallCompletionLease::new(inherited);
        lease.completion.scope(future).await
    }
}
