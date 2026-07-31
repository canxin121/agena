/// Completion result used when updating a background-task record.
#[derive(Debug, Clone)]
pub(crate) enum RuntimeBackgroundTaskCompletion {
    Succeeded { message: Option<String> },
    Failed { failure: agena_failure::Failure },
    Cancelled { message: Option<String> },
}
