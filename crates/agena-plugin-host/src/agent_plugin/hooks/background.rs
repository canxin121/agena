use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
};

use tokio::{sync::Semaphore, task::AbortHandle};
use tokio_util::sync::CancellationToken;

pub(super) struct BackgroundJobs {
    sessions: Mutex<BTreeMap<i64, Vec<Job>>>,
    slots: Arc<Semaphore>,
}

struct Job {
    name: Option<String>,
    abort: AbortHandle,
}

impl Default for BackgroundJobs {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
            slots: Arc::new(Semaphore::new(32)),
        }
    }
}

impl BackgroundJobs {
    pub(super) fn started(&self, session: i64, name: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&session)
            .is_some_and(|jobs| jobs.iter().any(|job| job.name.as_deref() == Some(name)))
    }

    pub(super) fn start(
        &self,
        session: i64,
        name: Option<String>,
        cancellation: CancellationToken,
        run: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        sessions.retain(|_, jobs| {
            jobs.retain(|job| job.name.is_some() || !job.abort.is_finished());
            !jobs.is_empty()
        });
        if let Some(name) = name.as_ref()
            && sessions
                .get(&session)
                .is_some_and(|jobs| jobs.iter().any(|job| job.name.as_ref() == Some(name)))
        {
            return Ok(());
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "plugin background process capacity exhausted")?;
        let task = tokio::spawn(async move {
            let _permit = permit;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {},
                _ = run => {},
            }
        });
        sessions.entry(session).or_default().push(Job {
            name,
            abort: task.abort_handle(),
        });
        Ok(())
    }

    pub(super) fn end_session(&self, session: i64) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for job in sessions.remove(&session).into_iter().flatten() {
            job.abort.abort();
        }
    }

    pub(super) fn stop_all(&self) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for job in std::mem::take(&mut *sessions).into_values().flatten() {
            job.abort.abort();
        }
    }
}

impl Drop for BackgroundJobs {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
#[tokio::test]
async fn closing_one_session_cancels_its_jobs_and_releases_the_slots() {
    use tokio::sync::oneshot;

    struct Done(Option<oneshot::Sender<()>>);
    impl Drop for Done {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let jobs = BackgroundJobs::default();
    let mut completion = Vec::new();
    for session in [1, 2] {
        let (send, recv) = oneshot::channel();
        let done = Done(Some(send));
        jobs.start(
            session,
            Some("watch".into()),
            CancellationToken::new(),
            async move {
                let _done = done;
                std::future::pending::<()>().await;
            },
        )
        .unwrap();
        completion.push(recv);
    }
    let mut other = completion.pop().unwrap();
    jobs.end_session(1);
    tokio::time::timeout(std::time::Duration::from_secs(1), completion.pop().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!jobs.started(1, "watch"));
    assert!(jobs.started(2, "watch"));
    assert!(matches!(
        other.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    jobs.stop_all();
    tokio::time::timeout(std::time::Duration::from_secs(1), other)
        .await
        .unwrap()
        .unwrap();
}
