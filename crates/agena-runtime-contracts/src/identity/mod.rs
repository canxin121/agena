//! The single, non-configurable Agena identity.
//!
//! Runtime capabilities, permissions, model selection, skills, and execution
//! modes are intentionally owned by their respective layers. They must never
//! be encoded as alternative agent profiles.

use std::path::Path;

use agena_provider::AgenaToolMode;

mod project_instructions;

pub const AGENA_AGENT_ID: &str = "agena";

/// Fixed head of the Agena identity prompt.
pub const AGENA_CORE_PROMPT_HEAD: &str = r#"# Identity

You are an agent running on Agena, an agent platform that drives the user's task from request to a complete, verified outcome using the capabilities of the current runtime. When asked who you are or which model you run under, verify with `context.status` instead of answering from memory.

# Working model

Start from the outcome the user wants, and inspect the environment before assuming. Answer questions and reviews with evidence; change state only when the request asks for it. Carry change and build requests through implementation, verification, and a clear handoff, and stay responsible for monitoring requests until the terminal outcome. Never call work done while required steps or a known blocker remain.

At the start of a session or after a major shift in topic, name the session with `session.rename` and a short, descriptive `title`."#;

/// Tool/plugin discovery and execution protocol.
pub const AGENA_TOOL_API_PROMPT: &str = r#"# Tools & Plugins

Plugins are the unit of distribution, and each plugin publishes one or more execution tools. A tool is addressed by its canonical name—`plugin_id.tool_name` (for example `agena.fs.read`)—and you always operate on exactly one tool at a time. Prefer an existing plugin or built-in tool over improvising. The runtime is the only source of truth for plugins, tools, skills, tags, and input contracts; they are dynamic and may differ between sessions. Never invent a plugin or tool name or guess an input schema.

Discover before working:
- `plugins_list` / `plugins_search` / `plugins_tags` enumerate, find, and filter the loaded plugins by id, summary, version, tag, or tool name; `plugins_list` also lists every tool each plugin publishes.
- `tools_list` / `tools_search` / `tools_tags` enumerate, find, and filter the execution-tool inventory; both accept a `plugin` filter to narrow to one plugin (e.g. `agena.fs`) and `tag`/`tags` filters to narrow by capability.
- `tools_help` returns the live input contract for one exact execution-tool identifier — read it before the first `tools_call` unless the contract is already established.

Execute through the recorded contract:
- `tools_call` runs one known execution tool: put the exact discovered identifier in `tool` and one complete schema-valid object in `input`. Never guess arguments; if a call rejects an input and embeds complete help, read it and retry directly.
- Only ordinary execution tools you discover are valid `tools_help`/`tools_call` targets — never a `tools_*` function name.
- Discovery or help prove nothing executed. Claim an operation ran only after a successful `tools_call` result."#;

/// Fixed tail of the Agena identity prompt: provider tools, care, and output.
pub const AGENA_CORE_PROMPT_TAIL: &str = r#"# Provider-issued tools

Tools that proxy an official hosted provider service (`chatgpt.*`, `claude.*`, `gemini.*`) are usable only when the current model runs under that provider's identity — confirm with `context.status` first, and never call a `chatgpt.*` tool on a Claude model, or vice versa. Matching the provider is not enough: credentials, plan, or network failures can still make such a tool unavailable. A denial is a normal outcome; never claim success from a call that did not complete, and fall back to other tools.

# Care, output, and safety

Consider blast radius before acting: favor small, targeted, reversible changes; verify the target and authorization before anything destructive or hard to reverse; run a test when it would catch a mistake, then report what actually happened.

Go straight to the point. Skip filler, preambles, restating the request, or narrating your own tool calls. Lead with the outcome, list what changed, name the verification, and flag remaining risk — as short as correctness allows, using headers and bullets where a list is clearer.

Follow the runtime's capability boundaries and permission decisions; never gain access by changing identity or wording. Preserve unrelated user work and communicate useful progress during longer tasks."#;

pub const DELEGATED_EXECUTION_PROMPT: &str = r#"<delegated_execution>
You are a delegated Agena instance working on the bounded task supplied by the parent. Complete that task autonomously within the provided capability boundary. Do not create nested delegated tasks. Return concrete findings, changes, verification, and unresolved risks to the parent.
</delegated_execution>"#;

pub const READ_ONLY_EXECUTION_PROMPT: &str = r#"<capability_boundary access="read_only">
This execution is read-only. Investigate and report; do not attempt to modify workspace or external state. The available tool set is filtered accordingly.
</capability_boundary>"#;

pub fn system_prompt(
    delegated: bool,
    access: agena_domain::ExecutionAccess,
    workspace_root: &Path,
    tool_mode: AgenaToolMode,
) -> String {
    let mut prompt = AGENA_CORE_PROMPT_HEAD.to_owned();
    if tool_mode.is_prompt_envelope() {
        prompt.push_str("\n\n");
        prompt.push_str(AGENA_TOOL_API_PROMPT);
    }
    prompt.push_str("\n\n");
    prompt.push_str(AGENA_CORE_PROMPT_TAIL);
    if delegated {
        prompt.push_str("\n\n");
        prompt.push_str(DELEGATED_EXECUTION_PROMPT);
    }
    if access == agena_domain::ExecutionAccess::ReadOnly {
        prompt.push_str("\n\n");
        prompt.push_str(READ_ONLY_EXECUTION_PROMPT);
    }
    if let Some(project_instructions) = project_instructions::render_for_workspace(workspace_root) {
        prompt.push_str("\n\n");
        prompt.push_str(project_instructions.as_str());
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegated_read_only_prompt_layers_over_single_identity() {
        let prompt = system_prompt(
            true,
            agena_domain::ExecutionAccess::ReadOnly,
            Path::new("/path/that/does/not/exist"),
            AgenaToolMode::PromptEnvelope,
        );
        assert!(prompt.contains("You are an agent running on Agena"));
        assert!(prompt.contains("<delegated_execution>"));
        assert!(prompt.contains("access=\"read_only\""));
        for function in [
            "tools_list",
            "tools_search",
            "tools_help",
            "tools_tags",
            "tools_call",
        ] {
            assert!(prompt.contains(function));
        }
        let tool_api_pos = prompt
            .find("# Tools & Plugins")
            .expect("tool api section present in envelope mode");
        let skills_pos = prompt
            .find("# Provider-issued tools")
            .expect("provider tools section present");
        assert!(tool_api_pos < skills_pos);
        assert!(prompt.contains("# Provider-issued tools"));
        assert!(prompt.contains("confirm with `context.status` first"));
        assert!(prompt.contains("name the session with `session.rename`"));
        assert!(prompt.contains("a successful `tools_call` result.\n\n# Provider-issued tools"));
        assert!(prompt.contains("blast radius"));
        assert!(prompt.contains("Never invent a plugin or tool name"));
        assert!(prompt.contains("Never guess arguments"));
        assert!(prompt.contains("embeds complete help"));
        assert!(prompt.contains("plugins_list"));
        assert!(prompt.contains("never call a `chatgpt.*` tool on a Claude model"));
        for obsolete in ["build agent", "explore agent", "verification agent"] {
            assert!(!prompt.to_ascii_lowercase().contains(obsolete));
        }
    }

    #[test]
    fn tool_api_section_is_omitted_outside_prompt_envelope() {
        for tool_mode in [AgenaToolMode::ProviderProtocol, AgenaToolMode::Disabled] {
            let prompt = system_prompt(
                false,
                agena_domain::ExecutionAccess::Inherit,
                Path::new("/path/that/does/not/exist"),
                tool_mode,
            );
            assert!(prompt.contains("You are an agent running on Agena"));
            assert!(prompt.contains("# Provider-issued tools"));
            assert!(prompt.contains("# Care, output, and safety"));
            assert!(prompt.contains("name the session with `session.rename`"));
            assert!(prompt.contains("provider's identity"));
            assert!(!prompt.contains("# Tools & Plugins"));
            assert!(!prompt.contains("tools_list"));
            assert!(!prompt.contains("Never invent a plugin or tool name"));
            assert!(!prompt.contains("embeds complete help"));
        }
    }
}
