//! Antigravity's PreToolUse gate for the Agena ACP profile.

use std::io::{self, Read};

use serde_json::{Value, json};

pub const WORKER_ARG: &str = "--internal-acp-antigravity-tool-guard";
pub const TOOL: &str = "tool";
pub const MANAGED_PROFILE_ENV: &str = "AGENA_ACP_ANTIGRAVITY_MANAGED_PROFILE";
const MAX_EVENT_BYTES: u64 = 4 * 1024 * 1024;

/// Run before loading Agena configuration, plugins or credentials.
pub fn try_run_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return None;
    }
    let mut input = Vec::new();
    let valid_event = io::stdin()
        .take(MAX_EVENT_BYTES + 1)
        .read_to_end(&mut input)
        .is_ok_and(|length| (length as u64) <= MAX_EVENT_BYTES);
    // An Agena-created conversation already has its built-in tools removed
    // by the custom Agent definition. A missing marker uses the strict gate.
    let managed = std::env::var(MANAGED_PROFILE_ENV).as_deref() == Ok("1");
    let allowed = valid_event && (managed || allows_tool(&input));
    let response = if allowed {
        json!({"decision": "allow"})
    } else {
        json!({"decision": "deny", "reason": "Use the Agena MCP tool for project and network actions."})
    };
    Some(i32::from(
        serde_json::to_writer(io::stdout().lock(), &response).is_err(),
    ))
}

fn allows_tool(input: &[u8]) -> bool {
    serde_json::from_slice::<Value>(input)
        .is_ok_and(|event| event["toolCall"]["name"] == "mcp_agena_tool")
}

#[cfg(test)]
mod tests {
    use super::allows_tool;

    #[test]
    fn only_the_agena_bridge_is_allowed() {
        assert!(allows_tool(br#"{"toolCall":{"name":"mcp_agena_tool"}}"#));
        for event in [
            br#"{"toolCall":{"name":"run_command"}}"#.as_slice(),
            br#"{"toolCall":{"name":"write_file"}}"#,
            br#"{"toolCall":{"name":"search_web"}}"#,
            br#"{"toolCall":{"name":"mcp_agena_tool_extended"}}"#,
            br#"{"toolCall":{"name":"mcp_other_tool"}}"#,
            br#"{"toolCall":{"args":{"name":"mcp_agena_tool"}}}"#,
            br#"{"toolCall":{"name":"mcp_agena_tool"}"#,
        ] {
            assert!(!allows_tool(event));
        }
    }
}
