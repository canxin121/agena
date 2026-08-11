use std::collections::BTreeMap;

use crate::part::AskUserToolInput;

use super::{ToolError, ToolExecutionView, ToolPayloadExecution, ToolPayloadOutput};

pub fn execute(input: &AskUserToolInput) -> Result<super::ToolPayloadExecution, ToolError> {
    let normalized = normalize(input)?;
    Err(ToolError::UserInputRequired(Box::new(normalized)))
}

pub fn validate(input: &AskUserToolInput) -> Result<(), ToolError> {
    normalize(input).map(|_| ())
}

fn normalize(input: &AskUserToolInput) -> Result<AskUserToolInput, ToolError> {
    AskUserToolInput::parse_input(
        serde_json::to_value(input).map_err(|err| ToolError::invalid_input(err.to_string()))?,
    )
    .map_err(|err| ToolError::invalid_input(err.to_string()))
}

pub fn execution_from_answers(
    input: &AskUserToolInput,
    answers: BTreeMap<String, Vec<String>>,
) -> ToolPayloadExecution {
    let mut lines = vec!["Answers:".to_string()];
    for (index, _question) in input.questions.iter().enumerate() {
        if let Some(answer) = answers.get(index.to_string().as_str()) {
            lines.push(format!("- {index}: {}", answer.join(", ")));
        }
    }

    let selection_count: usize = answers.values().map(Vec::len).sum();
    let mut view = ToolExecutionView::simple(
        "Ask user",
        format!("{selection_count} answers"),
        lines.join("\n"),
    );
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

pub fn execution_from_timeout(input: &AskUserToolInput) -> ToolPayloadExecution {
    let mut view = ToolExecutionView::simple(
        "Ask user",
        "Timed out",
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
    use crate::part::AskUserToolInput;
    use agena_domain::UserInputQuestion;

    #[test]
    fn timeout_is_a_successful_structured_result() {
        let input = AskUserToolInput {
            title: "Decision".to_string(),
            kind: String::new(),
            body_markdown: String::new(),
            auto_resolution_ms: Some(60_000),
            questions: vec![UserInputQuestion {
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
