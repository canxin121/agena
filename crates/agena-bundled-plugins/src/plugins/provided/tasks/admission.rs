//! A pending task has no authority to run until its durable state is written.
//! Dropping/cancelling the admission future restores the previous live entry.
use super::*;

type TaskMap = BTreeMap<String, Arc<AsyncTaskEntry>>;
pub(super) struct Reservation {
    registry: Arc<Mutex<TaskMap>>,
    key: String,
    previous: Option<Arc<AsyncTaskEntry>>,
    entry: Arc<AsyncTaskEntry>,
    committed: bool,
}
impl Reservation {
    pub(super) fn commit(mut self) -> Arc<AsyncTaskEntry> {
        self.committed = true;
        Arc::clone(&self.entry)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry
            .get(&self.key)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
        {
            if let Some(previous) = self.previous.take() {
                registry.insert(self.key.clone(), previous);
            } else {
                registry.remove(&self.key);
            }
        }
    }
}
impl TasksPlugin {
    pub(super) fn reserve_task(
        &self,
        state: AsyncTaskState,
        followup: bool,
    ) -> SdkResult<Reservation> {
        let mut registry = self
            .tasks
            .lock()
            .map_err(|_| agena_plugin_host::PluginError::internal("task registry poisoned"))?;
        let key = task_storage_key(&state);
        let previous = registry.get(&key).cloned();
        if previous
            .as_ref()
            .is_some_and(|entry| !followup || !is_terminal(&recover_task_state(entry).status))
        {
            return Err(agena_plugin_host::PluginError::invalid_params(
                "task id already exists; use tasks.followup for a terminal task",
            ));
        }
        let active = registry
            .values()
            .filter(|entry| {
                let item = recover_task_state(entry);
                item.parent_session_id == state.parent_session_id && !is_terminal(&item.status)
            })
            .count();
        if active >= MAX_ACTIVE_TASKS_PER_PARENT {
            return Err(agena_plugin_host::PluginError::invalid_params(
                "at most 8 delegated tasks may run concurrently for one parent session",
            ));
        }
        let entry = Arc::new(AsyncTaskEntry {
            state: Mutex::new(state),
            notify: Arc::new(Notify::new()),
        });
        registry.insert(key.clone(), Arc::clone(&entry));
        Ok(Reservation {
            registry: Arc::clone(&self.tasks),
            key,
            previous,
            entry,
            committed: false,
        })
    }
}
