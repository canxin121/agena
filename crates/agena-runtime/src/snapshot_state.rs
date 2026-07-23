use crate::{RuntimeTaskState, SnapshotMetadata};

/// Generic immutable state container for one runtime snapshot.
///
/// The resolution and service types remain generic so the runtime crate does
/// not depend on core configuration or concrete adapters.
pub(crate) struct RuntimeSnapshotState<Resolution, Services> {
    pub(crate) metadata: SnapshotMetadata,
    pub(crate) resolution: Resolution,
    pub(crate) services: Services,
    pub(crate) tasks: RuntimeTaskState,
}

impl<Resolution, Services> RuntimeSnapshotState<Resolution, Services> {
    pub(crate) fn new(
        generation: u64,
        resolution: Resolution,
        services: Services,
        tasks: RuntimeTaskState,
    ) -> Self {
        Self {
            metadata: SnapshotMetadata::new(generation),
            resolution,
            services,
            tasks,
        }
    }
}

impl<Resolution, Services> RuntimeSnapshotState<Resolution, Services> {
    pub(crate) fn metadata(&self) -> &SnapshotMetadata {
        &self.metadata
    }

    pub(crate) fn resolution(&self) -> &Resolution {
        &self.resolution
    }

    pub(crate) fn services(&self) -> &Services {
        &self.services
    }

    pub(crate) fn tasks(&self) -> &RuntimeTaskState {
        &self.tasks
    }
}
