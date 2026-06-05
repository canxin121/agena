use std::collections::BTreeMap;

use crate::message::AskUserToolInput;

use super::{ToolError, ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput};

pub(crate) fn execute(input: &AskUserToolInput) -> Result<super::ToolPayloadExecution, ToolError> {
    let normalized = normalize(input)?;
    Err(ToolError::UserInputRequired(normalized))
}

pub(crate) fn validate(input: &AskUserToolInput) -> Result<(), ToolError> {
    normalize(input).map(|_| ())
}

fn normalize(input: &AskUserToolInput) -> Result<AskUserToolInput, ToolError> {
    AskUserToolInput::parse_input(
        serde_json::to_value(input).map_err(|err| ToolError::InvalidInput(err.to_string()))?,
    )
    .map_err(|err| ToolError::InvalidInput(err.to_string()))
}

pub(crate) fn execution_from_answers(
    input: &AskUserToolInput,
    answers: BTreeMap<String, Vec<String>>,
) -> ToolPayloadExecution {
    let mut lines = vec!["Answers:".to_string()];
    for question in &input.questions {
        if let Some(answer) = answers.get(question.id.as_str()) {
            lines.push(format!("- {}: {}", question.id, answer.join(", ")));
        }
    }

    let mut view = ToolExecutionView::simple("Ask user", lines.join("\n"));
    let selection_count: usize = answers.values().map(Vec::len).sum();
    view.metadata
        .insert("answer_count".to_string(), selection_count.to_string());
    view.metadata.insert(
        "question_count".to_string(),
        input.questions.len().to_string(),
    );

    ToolPayloadExecution::new(ToolPayloadOutput::AskUser { answers }, view)
}
