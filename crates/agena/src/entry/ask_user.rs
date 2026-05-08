use std::collections::{BTreeMap, HashSet};

use crate::message::{AskUserToolInput, FirstPartyToolOutput};

use super::{FirstPartyExecution, ToolError, ToolExecutionView};

const MAX_QUESTIONS: usize = 3;
const MAX_OPTIONS: usize = 8;
const MAX_HEADER_CHARS: usize = 12;

pub(crate) fn execute(input: &AskUserToolInput) -> Result<super::FirstPartyExecution, ToolError> {
    validate(input)?;
    Err(ToolError::UserInputRequired(input.clone()))
}

pub(crate) fn validate(input: &AskUserToolInput) -> Result<(), ToolError> {
    if input.questions.is_empty() {
        return Err(ToolError::InvalidInput(
            "ask_user requires at least one question".to_string(),
        ));
    }
    if input.questions.len() > MAX_QUESTIONS {
        return Err(ToolError::InvalidInput(format!(
            "ask_user accepts at most {MAX_QUESTIONS} questions"
        )));
    }

    let mut ids = HashSet::new();
    for question in &input.questions {
        let id = question.id.trim();
        if id.is_empty() {
            return Err(ToolError::InvalidInput(
                "ask_user question id must not be empty".to_string(),
            ));
        }
        if !ids.insert(id.to_string()) {
            return Err(ToolError::InvalidInput(format!(
                "duplicate ask_user question id: {id}"
            )));
        }

        if question.header.chars().count() > MAX_HEADER_CHARS {
            return Err(ToolError::InvalidInput(format!(
                "ask_user header must be at most {MAX_HEADER_CHARS} characters for question {id}"
            )));
        }
        if question.question.trim().is_empty() {
            return Err(ToolError::InvalidInput(format!(
                "ask_user question text must not be empty for question {id}"
            )));
        }
        if question.options.len() > MAX_OPTIONS {
            return Err(ToolError::InvalidInput(format!(
                "ask_user question {id} accepts at most {MAX_OPTIONS} options"
            )));
        }
        if question.options.is_empty() && !question.allow_custom {
            return Err(ToolError::InvalidInput(format!(
                "ask_user question {id} must provide options or allow_custom"
            )));
        }

        let mut option_labels = HashSet::new();
        for option in &question.options {
            let label = option.label.trim();
            if label.is_empty() {
                return Err(ToolError::InvalidInput(format!(
                    "ask_user option label must not be empty for question {id}"
                )));
            }
            if !option_labels.insert(label.to_string()) {
                return Err(ToolError::InvalidInput(format!(
                    "duplicate ask_user option label '{label}' for question {id}"
                )));
            }
        }
    }

    Ok(())
}

pub(crate) fn execution_from_answers(
    input: &AskUserToolInput,
    answers: BTreeMap<String, Vec<String>>,
) -> FirstPartyExecution {
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

    FirstPartyExecution::new(FirstPartyToolOutput::AskUser { answers }, view)
}

#[cfg(test)]
mod tests {
    use crate::message::{AskUserToolInput, UserInputOption, UserInputQuestion};

    use super::validate;

    #[test]
    fn accepts_multi_select_with_custom_answer() {
        let input = AskUserToolInput {
            questions: vec![UserInputQuestion {
                id: "stack".to_string(),
                header: "Stack".to_string(),
                question: "Which stacks should we support?".to_string(),
                options: vec![
                    UserInputOption {
                        label: "rust".to_string(),
                        description: String::new(),
                    },
                    UserInputOption {
                        label: "go".to_string(),
                        description: String::new(),
                    },
                ],
                multiple: true,
                allow_custom: true,
            }],
        };

        validate(&input).expect("multi-select question should validate");
    }

    #[test]
    fn rejects_question_without_options_or_custom() {
        let input = AskUserToolInput {
            questions: vec![UserInputQuestion {
                id: "empty".to_string(),
                header: String::new(),
                question: "Need an answer".to_string(),
                options: Vec::new(),
                multiple: false,
                allow_custom: false,
            }],
        };

        let err = validate(&input).expect_err("question should be rejected");
        assert!(err.to_string().contains("options or allow_custom"));
    }
}
