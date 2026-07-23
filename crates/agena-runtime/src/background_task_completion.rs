/// Completion result used when updating a background-task record.
#[derive(Debug, Clone)]
pub(crate) enum RuntimeBackgroundTaskCompletion {
    Succeeded { message: Option<String> },
    Failed { error_message: String },
    Cancelled { message: Option<String> },
}
