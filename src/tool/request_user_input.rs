use std::collections::HashSet;

use crate::message::RequestUserInputToolInput;

use super::ToolError;

const MAX_QUESTIONS: usize = 3;
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 3;
const MAX_HEADER_CHARS: usize = 12;

pub(super) fn execute(
    input: &RequestUserInputToolInput,
) -> Result<super::BuiltinExecution, ToolError> {
    validate(input)?;
    Err(ToolError::UserInputRequired(input.clone()))
}

fn validate(input: &RequestUserInputToolInput) -> Result<(), ToolError> {
    if input.questions.is_empty() {
        return Err(ToolError::InvalidInput(
            "request_user_input requires at least one question".to_string(),
        ));
    }
    if input.questions.len() > MAX_QUESTIONS {
        return Err(ToolError::InvalidInput(format!(
            "request_user_input accepts at most {MAX_QUESTIONS} questions"
        )));
    }

    let mut ids = HashSet::new();
    for question in &input.questions {
        let id = question.id.trim();
        if id.is_empty() {
            return Err(ToolError::InvalidInput(
                "request_user_input question id must not be empty".to_string(),
            ));
        }
        if !ids.insert(id.to_string()) {
            return Err(ToolError::InvalidInput(format!(
                "duplicate request_user_input question id: {id}"
            )));
        }

        if question.header.trim().is_empty() {
            return Err(ToolError::InvalidInput(format!(
                "request_user_input header must not be empty for question {id}"
            )));
        }
        if question.header.chars().count() > MAX_HEADER_CHARS {
            return Err(ToolError::InvalidInput(format!(
                "request_user_input header must be at most {MAX_HEADER_CHARS} characters for question {id}"
            )));
        }
        if question.question.trim().is_empty() {
            return Err(ToolError::InvalidInput(format!(
                "request_user_input question text must not be empty for question {id}"
            )));
        }
        if question.options.len() < MIN_OPTIONS || question.options.len() > MAX_OPTIONS {
            return Err(ToolError::InvalidInput(format!(
                "request_user_input question {id} must provide {MIN_OPTIONS}-{MAX_OPTIONS} options"
            )));
        }

        let mut option_labels = HashSet::new();
        for option in &question.options {
            let label = option.label.trim();
            if label.is_empty() {
                return Err(ToolError::InvalidInput(format!(
                    "request_user_input option label must not be empty for question {id}"
                )));
            }
            if !option_labels.insert(label.to_string()) {
                return Err(ToolError::InvalidInput(format!(
                    "duplicate request_user_input option label '{label}' for question {id}"
                )));
            }
            if option.description.trim().is_empty() {
                return Err(ToolError::InvalidInput(format!(
                    "request_user_input option description must not be empty for question {id}"
                )));
            }
        }
    }

    Ok(())
}
