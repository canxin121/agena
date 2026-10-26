//! One cancellation lifetime for a plugin request, including streamed work.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use tokio_util::sync::CancellationToken;

use crate::error::{PluginError, PluginErrorKind, Result};
use crate::rpc::RequestId;

tokio::task_local! {
    static REQUEST_CANCELLATION: CancellationToken;
}

/// The current invocation's signal. Drivers install it before entering plugin
/// code; manually invoked plugins have no enclosing request.
pub fn current_cancellation_token() -> Option<CancellationToken> {
    REQUEST_CANCELLATION.try_with(Clone::clone).ok()
}

/// Run plugin code with a signal that is cancelled when this future is dropped
/// or finishes. Spawned work can retain the signal and stop with its parent.
pub async fn run_with_cancellation<T>(
    token: CancellationToken,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    let _cancel_on_drop = token.clone().drop_guard();
    REQUEST_CANCELLATION
        .scope(token.clone(), async move {
            tokio::select! {
                biased;
                _ = token.cancelled() => Err(PluginError::from_kind(
                    PluginErrorKind::Cancelled,
                    "plugin request was cancelled",
                )),
                result = future => result,
            }
        })
        .await
}

const EARLY_CANCEL_TTL: Duration = Duration::from_secs(60);
const MAX_EARLY_CANCELLATIONS: usize = 256;

#[derive(Default)]
struct Requests {
    active: HashMap<RequestId, Arc<CancellationToken>>,
    // Separate HTTP requests can arrive out of order. A bounded tombstone also
    // cancels work whose cancellation arrives before its original request.
    early: VecDeque<(RequestId, Instant)>,
}

#[derive(Clone, Default)]
pub struct RequestRegistry {
    requests: Arc<Mutex<Requests>>,
}

impl RequestRegistry {
    pub fn begin(&self, id: RequestId) -> Result<RequestScope> {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        expire_early(&mut requests);
        if requests.active.contains_key(&id) {
            return Err(PluginError::invalid_params(
                "duplicate active plugin request id",
            ));
        }
        let token = Arc::new(CancellationToken::new());
        if let Some(index) = requests
            .early
            .iter()
            .position(|(cancelled, _)| cancelled == &id)
        {
            requests.early.remove(index);
            token.cancel();
        }
        requests.active.insert(id.clone(), token.clone());
        Ok(RequestScope {
            registry: self.clone(),
            id,
            token,
        })
    }

    /// Signal one request. Unknown IDs are retained briefly because HTTP
    /// cancellation can overtake the request it cancels.
    pub fn cancel(&self, id: &RequestId) -> bool {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        expire_early(&mut requests);
        if let Some(token) = requests.active.get(id) {
            token.cancel();
            return true;
        }
        if !requests.early.iter().any(|(cancelled, _)| cancelled == id) {
            if requests.early.len() == MAX_EARLY_CANCELLATIONS {
                requests.early.pop_front();
            }
            requests.early.push_back((id.clone(), Instant::now()));
        }
        false
    }
}

fn expire_early(requests: &mut Requests) {
    while requests
        .early
        .front()
        .is_some_and(|(_, at)| at.elapsed() >= EARLY_CANCEL_TTL)
    {
        requests.early.pop_front();
    }
}

/// Owns admission until ordinary execution or the entire stream has settled.
/// Dropping it also signals any child work that retained the token.
pub struct RequestScope {
    registry: RequestRegistry,
    id: RequestId,
    token: Arc<CancellationToken>,
}

impl RequestScope {
    pub fn token(&self) -> CancellationToken {
        self.token.as_ref().clone()
    }

    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        run_with_cancellation(self.token(), future).await
    }
}

impl Drop for RequestScope {
    fn drop(&mut self) {
        self.token.cancel();
        let mut requests = self
            .registry
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if requests
            .active
            .get(&self.id)
            .is_some_and(|token| Arc::ptr_eq(token, &self.token))
        {
            requests.active.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cancellation_drops_the_running_body_and_signals_child_work() {
        let registry = RequestRegistry::default();
        let id = RequestId::Num(41);
        let request = registry.begin(id.clone()).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let body_dropped = dropped.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            request
                .run(async {
                    let _drop = Dropped(body_dropped);
                    let child = current_cancellation_token().unwrap().child_token();
                    started_tx.send(child).unwrap();
                    std::future::pending::<Result<()>>().await
                })
                .await
        });
        let child = started_rx.await.unwrap();
        assert!(!child.is_cancelled());
        assert!(registry.cancel(&id));
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().kind, PluginErrorKind::Cancelled);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(child.is_cancelled());
        assert!(registry.requests.lock().unwrap().active.is_empty());
    }

    #[tokio::test]
    async fn cancellation_that_overtakes_http_admission_prevents_execution() {
        let registry = RequestRegistry::default();
        let id = RequestId::Str("early-http-request".into());
        assert!(!registry.cancel(&id));
        let request = registry.begin(id).unwrap();
        let executed = AtomicBool::new(false);
        let result = request
            .run(async {
                executed.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(result.unwrap_err().kind, PluginErrorKind::Cancelled);
        assert!(!executed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn parent_completion_cancels_retained_signals() {
        let registry = RequestRegistry::default();
        let request = registry.begin(RequestId::Num(9)).unwrap();
        let child = request
            .run(async { Ok(current_cancellation_token().unwrap().child_token()) })
            .await
            .unwrap();
        assert!(child.is_cancelled());
    }

    #[test]
    fn early_cancellation_memory_is_bounded_and_expires() {
        let registry = RequestRegistry::default();
        for id in 0..1024 {
            registry.cancel(&RequestId::Num(id));
        }
        assert_eq!(
            registry.requests.lock().unwrap().early.len(),
            MAX_EARLY_CANCELLATIONS
        );
        let mut requests = registry.requests.lock().unwrap();
        for (_, at) in &mut requests.early {
            *at = Instant::now() - EARLY_CANCEL_TTL;
        }
        drop(requests);
        let request = registry.begin(RequestId::Num(1023)).unwrap();
        assert!(!request.token().is_cancelled());
        assert!(registry.requests.lock().unwrap().early.is_empty());
    }
}
