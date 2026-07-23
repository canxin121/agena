use std::collections::BTreeMap;

use crate::message::AskUserToolInput;

use super::{ToolError, ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput};

pub(crate) fn execute(input: &AskUserToolInput) -> Result<super::ToolPayloadExecution, ToolError> {
    let normalized = normalize(input)?;
    Err(ToolError::UserInputRequired(Box::new(normalized)))
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

    ToolPayloadExecution::new(
        ToolPayloadOutput::AskUser {
            answers,
            timed_out: false,
        },
        view,
    )
}

pub(crate) fn execution_from_timeout(input: &AskUserToolInput) -> ToolPayloadExecution {
    let mut view = ToolExecutionView::simple(
        "Ask user",
        "No user response before the deadline. Continue with best judgment.",
    );
    view.metadata
        .insert("timed_out".to_string(), "true".to_string());
    view.metadata.insert(
        "question_count".to_string(),
        input.questions.len().to_string(),
    );
    ToolPayloadExecution::new(
        ToolPayloadOutput::AskUser {
            answers: BTreeMap::new(),
            timed_out: true,
        },
        view,
    )
}

#[cfg(test)]
mod tests {
    use crate::message::AskUserToolInput;
    use agena_domain::UserInputQuestion;

    #[test]
    fn timeout_is_a_successful_structured_result() {
        let input = AskUserToolInput {
            title: "Decision".to_string(),
            body_markdown: String::new(),
            kind: String::new(),
            submit_label: String::new(),
            cancel_label: String::new(),
            auto_resolution_ms: Some(60_000),
            questions: vec![UserInputQuestion {
                id: "decision".to_string(),
                header: String::new(),
                question: "Choose".to_string(),
                options: Vec::new(),
                multiple: false,
                allow_custom: true,
            }],
        };
        let execution = super::execution_from_timeout(&input);
        assert!(matches!(
            execution.output,
            super::ToolPayloadOutput::AskUser {
                timed_out: true,
                ref answers
            } if answers.is_empty()
        ));
        assert_eq!(
            execution
                .summary()
                .metadata
                .get("timed_out")
                .map(String::as_str),
            Some("true")
        );
    }
}
