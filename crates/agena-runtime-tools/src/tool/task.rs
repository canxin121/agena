use crate::message::TaskToolInput;

use super::{ToolExecutor, ToolPayloadExecution};

pub(super) fn execute(
    _executor: &ToolExecutor,
    _input: &TaskToolInput,
) -> Result<ToolPayloadExecution, super::ToolError> {
    Err(super::ToolError::Plugin(
        "task must be invoked through the agena.tasks host bridge".to_string(),
    ))
}
