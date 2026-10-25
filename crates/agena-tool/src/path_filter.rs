//! Permission predicates for synchronous directory walkers. The caller owns
//! authorization; the walker must prune a rejected directory before descending.
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct PathPredicate {
    check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    waited: Arc<Mutex<Duration>>,
}

impl PathPredicate {
    pub fn allows(&self, path: &Path) -> bool {
        let start = Instant::now();
        let allowed = (self.check)(path);
        *self.waited.lock().unwrap_or_else(|e| e.into_inner()) += start.elapsed();
        allowed
    }

    /// Synchronous scanners exclude time spent in the authorization callback.
    /// Each traversal uses this predicate serially, before parallel file reads.
    pub fn waited(&self) -> Duration {
        *self.waited.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Keep the original typed rejection across a walker's boolean filter API.
pub struct PathFilter<E> {
    predicate: PathPredicate,
    failure: Arc<Mutex<Option<E>>>,
}

impl<E: Send + 'static> PathFilter<E> {
    pub fn new(check: impl Fn(&Path) -> Result<(), E> + Send + Sync + 'static) -> Self {
        let failure = Arc::new(Mutex::new(None));
        let rejected = failure.clone();
        let predicate = Arc::new(move |path: &Path| {
            let mut failure = rejected.lock().unwrap_or_else(|e| e.into_inner());
            if failure.is_some() {
                return false;
            }
            match check(path) {
                Ok(()) => true,
                Err(error) => {
                    *failure = Some(error);
                    false
                }
            }
        });
        Self {
            predicate: PathPredicate {
                check: predicate,
                waited: Default::default(),
            },
            failure,
        }
    }

    pub fn predicate(&self) -> PathPredicate {
        self.predicate.clone()
    }

    /// Call after traversal, even if the underlying scanner returned an error.
    pub fn finish<T>(self, result: Result<T, E>) -> Result<T, E> {
        match self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            Some(error) => Err(error),
            None => result,
        }
    }
}
