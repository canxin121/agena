//! Request-local query planning and bounded context assembly primitives.
//!
//! A query is a hint for finding evidence, never a new statement of fact. Only
//! visible text participates: attachments, reasoning and tool payloads remain
//! opaque and are preserved by the caller's original message projection.

use agena_plugin_host::{ChatMessage, ChatMessagesTransformInput};

const MAX_QUERY_CHARS: usize = 2048;
const MAX_HISTORY_PART_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RetrievalQuery {
    pub text: String,
    pub used_history: bool,
}

pub(super) fn visible_text(message: &ChatMessage) -> String {
    match &message.content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(super) fn plan_query(
    input: &ChatMessagesTransformInput,
    context_threshold: usize,
) -> Option<RetrievalQuery> {
    let latest = input
        .messages
        .iter()
        .rposition(|message| message.role == "user")?;
    let text = visible_text(&input.messages[latest]);
    if skips_memory(&text) {
        return None;
    }
    let text = text.trim();
    let needs_context = text.chars().count() < context_threshold || is_followup(text);
    let mut parts = vec![bounded_query_text(text, MAX_QUERY_CHARS)];
    let mut used_history = false;
    if needs_context {
        let mut users = 0;
        let mut assistants = 0;
        for message in input.messages[..latest].iter().rev() {
            let relevant = matches!(message.role.as_str(), "user" if users < 2)
                || matches!(message.role.as_str(), "assistant" if assistants < 1);
            if relevant {
                let prior = visible_text(message);
                if !prior.trim().is_empty() && !skips_memory(&prior) {
                    if message.role == "user" {
                        users += 1;
                    } else {
                        assistants += 1;
                    }
                    parts.push(bounded_query_text(prior.trim(), MAX_HISTORY_PART_CHARS));
                    used_history = true;
                }
            }
            if users == 2 && assistants == 1 {
                break;
            }
        }
    }
    Some(RetrievalQuery {
        text: truncate_chars(parts.join("\n").trim(), MAX_QUERY_CHARS),
        used_history,
    })
}

fn bounded_query_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    if limit < 6 {
        return truncate_chars(text, limit);
    }
    let head = (limit - 3) / 3;
    let tail = limit - 3 - head;
    let mut suffix = text.chars().rev().take(tail).collect::<Vec<_>>();
    suffix.reverse();
    format!(
        "{}\n…\n{}",
        truncate_chars(text, head),
        suffix.into_iter().collect::<String>()
    )
}

fn is_followup(text: &str) -> bool {
    if text.chars().count() > 96 {
        return false;
    }
    let lowered = text.to_ascii_lowercase();
    [
        "继续",
        "接着",
        "上次",
        "刚才",
        "之前那个",
        "按那个",
        "那就",
        "这个方案",
        "continue",
        "resume",
        "go on",
        "what about that",
        "as before",
        "same as",
    ]
    .iter()
    .any(|prefix| lowered.starts_with(prefix))
}

pub(super) fn skips_memory(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    [
        "ignore memory",
        "don't use memory",
        "do not use memory",
        "not use memory",
        "without memory",
        "不要用记忆",
        "不要使用记忆",
        "不使用记忆",
        "不用记忆",
        "不要读取记忆",
        "不要调用记忆",
        "忽略记忆",
        "不要参考历史记忆",
    ]
    .iter()
    .any(|phrase| lowered.contains(phrase))
}

pub(super) fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// A model-independent estimate: ASCII word runs use four bytes per token;
/// punctuation and non-ASCII text are budgeted conservatively. Callers also
/// enforce a byte cap, so high-entropy strings cannot grow an unbounded prompt.
pub(super) fn estimated_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            ascii_run += 1;
        } else {
            tokens += ascii_run.div_ceil(4);
            ascii_run = 0;
            tokens += if ch.is_ascii_whitespace() {
                0
            } else {
                ch.len_utf8().min(3)
            };
        }
    }
    tokens + ascii_run.div_ceil(4)
}

/// Return a UTF-8-safe prefix fitting both the estimated token and byte budgets.
pub(super) fn fit_to_budget(text: &str, tokens: usize) -> String {
    if tokens == 0 {
        return String::new();
    }
    let byte_cap = tokens.saturating_mul(4);
    let mut end = text.len().min(byte_cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if estimated_tokens(&text[..end]) <= tokens {
        return text[..end].to_string();
    }
    let boundaries = text[..end]
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(end))
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len() - 1;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if estimated_tokens(&text[..boundaries[mid]]) <= tokens {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    text[..boundaries[low]].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(messages: Vec<ChatMessage>) -> ChatMessagesTransformInput {
        ChatMessagesTransformInput {
            session_id: 42,
            workspace_root: None,
            messages,
        }
    }

    #[test]
    fn multipart_text_is_recalled_without_attachment_or_reasoning_payloads() {
        let message = ChatMessage {
            role: "user".into(),
            content: json!([
                {"type":"text","text":"我们之前数据库"},
                {"type":"attachment","attachment":{"data":"PRIVATE_BASE64","text":"not query text"}},
                {"type":"reasoning","text":"private reasoning"},
                {"type":"text","text":"选型是什么"}
            ]),
            provider_state: Some(json!({"opaque":"preserve"})),
        };
        let original = message.clone();
        let query = plan_query(&input(vec![message.clone()]), 8).unwrap();
        assert_eq!(query.text, "我们之前数据库\n选型是什么");
        assert_eq!(message.content, original.content);
        assert_eq!(message.provider_state, original.provider_state);
    }

    #[test]
    fn continuation_reuses_recent_task_context_and_short_specific_queries_still_search() {
        let query = plan_query(
            &input(vec![
                ChatMessage::system("system memory must not enter query"),
                ChatMessage::user("修复 PostgreSQL 的连接池"),
                ChatMessage::assistant("下一步检查 pool.rs 中的 idle_timeout"),
                ChatMessage::user("继续"),
            ]),
            8,
        )
        .unwrap();
        assert!(query.used_history);
        assert!(query.text.starts_with("继续"));
        assert!(query.text.contains("PostgreSQL"));
        assert!(query.text.contains("pool.rs"));
        assert!(!query.text.contains("system memory"));
        assert_eq!(
            plan_query(&input(vec![ChatMessage::user("数据库")]), 8)
                .unwrap()
                .text,
            "数据库"
        );
    }

    #[test]
    fn a_new_specific_request_does_not_drag_in_unrelated_history() {
        let query = plan_query(
            &input(vec![
                ChatMessage::user("Unrelated old task"),
                ChatMessage::user("解释数据库连接池配置"),
            ]),
            8,
        )
        .unwrap();
        assert!(!query.used_history);
        assert_eq!(query.text, "解释数据库连接池配置");
    }

    #[test]
    fn explicit_memory_opt_out_works_in_chinese_english_and_multipart() {
        for text in [
            "这次不要使用记忆，重新分析",
            "Do not use memory for this task",
        ] {
            assert!(plan_query(&input(vec![ChatMessage::user(text)]), 8).is_none());
        }
        let message = ChatMessage {
            role: "user".into(),
            content: json!([
                {"type":"text","text":"忽略记忆"}, {"type":"attachment","attachment":{}}
            ]),
            provider_state: None,
        };
        assert!(plan_query(&input(vec![message]), 8).is_none());
    }

    #[test]
    fn image_only_input_keeps_core_recall_and_query_work_is_bounded() {
        let message = ChatMessage {
            role: "user".into(),
            content: json!([
                {"type":"attachment","attachment":{"data":"opaque"}}
            ]),
            provider_state: None,
        };
        assert_eq!(plan_query(&input(vec![message]), 8).unwrap().text, "");
        let long = "任务内容".repeat(10_000);
        assert_eq!(
            plan_query(&input(vec![ChatMessage::user(long)]), 8)
                .unwrap()
                .text
                .chars()
                .count(),
            MAX_QUERY_CHARS
        );
    }

    #[test]
    fn tool_only_assistant_messages_do_not_hide_task_context() {
        let tool_only = ChatMessage {
            role: "assistant".into(),
            content: json!([
                {"type":"tool_call","id":"call-1","arguments_json":"{}"}
            ]),
            provider_state: None,
        };
        let query = plan_query(
            &input(vec![
                ChatMessage::user("修复数据库连接池"),
                ChatMessage::assistant("下一步核对 pool.rs 的 acquire_timeout"),
                tool_only,
                ChatMessage::user("继续"),
            ]),
            8,
        )
        .unwrap();
        assert!(query.text.contains("acquire_timeout"));
    }

    #[test]
    fn long_pasted_logs_do_not_erase_the_question_at_the_end() {
        let message = format!(
            "报错背景开始\n{}\n请查找之前 database-decision 的决定",
            "重复日志 ".repeat(2000)
        );
        let query = plan_query(&input(vec![ChatMessage::user(message)]), 8).unwrap();
        assert!(query.text.starts_with("报错背景开始"));
        assert!(query.text.ends_with("请查找之前 database-decision 的决定"));
        assert_eq!(query.text.chars().count(), MAX_QUERY_CHARS);
    }

    #[test]
    fn budget_handles_cjk_emoji_and_ascii_without_broken_utf8() {
        for text in [
            "中文记忆🙂数据库选型".repeat(100),
            "abcdef0123456789 ".repeat(100),
        ] {
            for budget in [0, 1, 2, 3, 8, 31, 128] {
                let fitted = fit_to_budget(&text, budget);
                assert!(estimated_tokens(&fitted) <= budget);
                assert!(fitted.len() <= budget * 4);
                assert!(text.starts_with(&fitted));
            }
        }
    }
}
