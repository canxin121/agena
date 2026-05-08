use crate::message::TaskToolInput;

use super::{FirstPartyExecution, ToolExecutor};

pub(super) fn execute(
    _executor: &ToolExecutor,
    _input: &TaskToolInput,
) -> Result<FirstPartyExecution, super::ToolError> {
    Err(super::ToolError::Plugin(
        "task must be invoked through the agena.workflow host bridge".to_string(),
    ))
}
