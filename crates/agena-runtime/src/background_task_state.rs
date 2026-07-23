use std::collections::{HashMap, VecDeque};

use tokio_util::sync::CancellationToken;

use crate::RuntimeBackgroundTask;

/// Mutable record/index state used by a background-task registry.
#[derive(Default)]
pub(crate) struct RuntimeBackgroundTaskState {
    pub(crate) order: VecDeque<String>,
    pub(crate) tasks: HashMap<String, RuntimeBackgroundTask>,
    pub(crate) controls: HashMap<String, CancellationToken>,
    pub(crate) active_by_key: HashMap<String, String>,
    pub(crate) dedupe_keys: HashMap<String, String>,
}

impl RuntimeBackgroundTaskState {
    pub(crate) fn trim_history(&mut self, history_limit: usize) {
        if self.order.len() <= history_limit {
            return;
        }

        let mut index = self.order.len();
        while self.order.len() > history_limit && index > 0 {
            index -= 1;
            let Some(task_id) = self.order.get(index).cloned() else {
                break;
            };
            let should_remove = self
                .tasks
                .get(task_id.as_str())
                .map(|task| !task.is_running())
                .unwrap_or(true);
            if !should_remove {
                continue;
            }

            let _ = self.order.remove(index);
            self.tasks.remove(task_id.as_str());
            self.controls.remove(task_id.as_str());
            if let Some(dedupe_key) = self.dedupe_keys.remove(task_id.as_str())
                && self
                    .active_by_key
                    .get(dedupe_key.as_str())
                    .is_some_and(|id| id == task_id.as_str())
            {
                self.active_by_key.remove(dedupe_key.as_str());
            }
        }
    }
}
