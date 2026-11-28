use std::sync::{
    Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::*;

/// The native controls cover the package. Custom controllers cover the
/// project session that owns their module instance.
#[derive(Default)]
pub(super) struct JobControllers {
    native: AtomicBool,
    modules: Mutex<Vec<Weak<Program>>>,
    sessions: Mutex<BTreeMap<i64, Vec<ControllerScope>>>,
}

struct ControllerScope {
    count: Weak<AtomicUsize>,
    lifetime: CancellationToken,
}

impl ControllerScope {
    fn live(&self) -> bool {
        !self.lifetime.is_cancelled() && self.count.strong_count() > 0
    }
}

impl JobControllers {
    pub(super) fn register_native(&self) {
        self.native.store(true, Ordering::Release);
    }

    pub(super) fn has_native(&self) -> bool {
        self.native.load(Ordering::Acquire)
    }

    pub(super) fn register_module(&self, program: &Arc<Program>) {
        self.modules
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Arc::downgrade(program));
    }

    pub(super) fn has_modules(&self) -> bool {
        !self
            .modules
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    }

    pub(super) fn bind(&self, session: i64, count: &Arc<AtomicUsize>, lifetime: CancellationToken) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        sessions.retain(|_, scopes| {
            scopes.retain(ControllerScope::live);
            !scopes.is_empty()
        });
        sessions.entry(session).or_default().push(ControllerScope {
            count: Arc::downgrade(count),
            lifetime,
        });
    }

    pub(super) fn serves(&self, session: i64) -> bool {
        self.has_native()
            || self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(&session)
                .is_some_and(|scopes| {
                    scopes.iter().any(|scope| {
                        scope.live()
                            && scope
                                .count
                                .upgrade()
                                .is_some_and(|count| count.load(Ordering::Acquire) > 0)
                    })
                })
    }

    pub(super) async fn start_modules(
        &self,
        session: i64,
        host: &dyn HostClient,
    ) -> agena_plugin_sdk::Result<()> {
        if !self.has_native() {
            let modules = self
                .modules
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>();
            for program in modules {
                program.start_session(session, host).await?;
            }
        }
        Ok(())
    }
}
