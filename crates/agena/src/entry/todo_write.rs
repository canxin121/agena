use crate::message::{TodoWriteToolInput, ToolPayloadOutput};

use super::{ToolExecutionView, ToolPayloadExecution};

pub(super) fn execute(input: &TodoWriteToolInput) -> ToolPayloadExecution {
    let output = ToolPayloadOutput::TodoWrite {
        items: input.items.clone(),
    };

    let summary = render_todo_list(input);

    let mut view = ToolExecutionView::simple("Todo write", summary);
    view.metadata
        .insert("todo_count".to_string(), input.items.len().to_string());

    ToolPayloadExecution::new(output, view)
}

fn render_todo_list(input: &TodoWriteToolInput) -> String {
    if input.items.is_empty() {
        return "Todo list is empty.".to_string();
    }

    let mut lines = vec![format!(
        "Updated todo list with {} item(s):",
        input.items.len()
    )];
    for item in &input.items {
        lines.push(format!(
            "- [{}][{}] {}",
            status_label(item.status),
            priority_label(item.priority),
            item.content
        ));
    }
    lines.join("\n")
}

fn status_label(status: crate::message::TodoStatus) -> &'static str {
    match status {
        crate::message::TodoStatus::Pending => "pending",
        crate::message::TodoStatus::InProgress => "in_progress",
        crate::message::TodoStatus::Completed => "completed",
        crate::message::TodoStatus::Cancelled => "cancelled",
    }
}

fn priority_label(priority: crate::message::TodoPriority) -> &'static str {
    match priority {
        crate::message::TodoPriority::High => "high",
        crate::message::TodoPriority::Medium => "medium",
        crate::message::TodoPriority::Low => "low",
    }
}
