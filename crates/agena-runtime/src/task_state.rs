use crate::{RuntimeSchedulingPolicy, WatchPathSet};

/// Immutable inputs consumed by runtime background loops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTaskState {
    watch_paths: WatchPathSet,
    scheduling: RuntimeSchedulingPolicy,
}

impl RuntimeTaskState {
    pub fn new(watch_paths: WatchPathSet) -> Self {
        Self {
            watch_paths,
            scheduling: RuntimeSchedulingPolicy::default(),
        }
    }

    pub fn watch_paths(&self) -> &WatchPathSet {
        &self.watch_paths
    }

    pub fn scheduling(&self) -> RuntimeSchedulingPolicy {
        self.scheduling
    }
}
