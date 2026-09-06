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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk::host_api::{NoopHostClient, current_host_callback_context};
    use futures_util::poll;
    use std::sync::{Arc, Mutex};

    fn host() -> Arc<HostHandle> {
        let host = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        host.begin_plugin_instance("test.outer".parse().unwrap());
        host.begin_plugin_instance("test.inner".parse().unwrap());
        host
    }

    #[tokio::test]
    async fn callback_task_and_nested_call_share_only_the_originating_completion() {
        let host = host();
        let outer = host
            .run_in_authorized_callback_context(
                &"test.outer".parse().unwrap(),
                HostCallbackContext::default(),
                async {
                    let outer = current_plugin_call_completion().unwrap();
                    let context = current_host_callback_context().unwrap();
                    let callback_host = host.clone();
                    let expected = outer.id().to_owned();
                    tokio::spawn(async move {
                        assert!(current_plugin_call_completion().is_none());
                        let validated = callback_host
                            .validated_callback_context(Some("test.outer".into()), Some(context))
                            .unwrap();
                        callback_host
                            .run_in_callback_completion(&validated, async {
                                assert_eq!(
                                    current_plugin_call_completion().unwrap().id(),
                                    expected
                                );
                                callback_host
                                    .run_in_authorized_callback_context(
                                        &"test.inner".parse().unwrap(),
                                        HostCallbackContext::default(),
                                        async {
                                            assert_eq!(
                                                current_plugin_call_completion().unwrap().id(),
                                                expected
                                            );
                                        },
                                    )
                                    .await;
                            })
                            .await;
                    })
                    .await
                    .unwrap();
                    assert!(
                        poll!(std::pin::pin!(outer.wait())).is_pending(),
                        "returning a nested call must not finish its parent"
                    );
                    outer
                },
            )
            .await;
        assert!(poll!(std::pin::pin!(outer.wait())).is_ready());
        assert!(current_plugin_call_completion().is_none());
    }

    #[tokio::test]
    async fn independent_calls_do_not_share_completion_and_cancellation_releases_ownership() {
        let host = host();
        let weak = Arc::downgrade(&host);
        let captured = Mutex::new(None);
        let key = "test.outer".parse().unwrap();
        let mut first = Box::pin(host.run_in_authorized_callback_context(
            &key,
            HostCallbackContext::default(),
            async {
                *captured.lock().unwrap() = current_plugin_call_completion();
                std::future::pending::<()>().await;
            },
        ));
        assert!(poll!(&mut first).is_pending());
        let first_completion = captured.lock().unwrap().clone().unwrap();
        let second = host
            .run_in_authorized_callback_context(&key, HostCallbackContext::default(), async {
                current_plugin_call_completion().unwrap()
            })
            .await;
        assert_ne!(first_completion.id(), second.id());
        assert!(poll!(std::pin::pin!(second.wait())).is_ready());
        assert!(poll!(std::pin::pin!(first_completion.wait())).is_pending());
        drop(first);
        assert!(poll!(std::pin::pin!(first_completion.wait())).is_ready());
        drop(host);
        assert!(
            weak.upgrade().is_none(),
            "completion signals must not own hosts"
        );
    }
}
