//! Compose shared rules, session responsibilities, and available workflows.
//! Tool schemas and execution manuals remain in live tool help.

use std::collections::BTreeMap;

use agena_domain::{ExecutionSelection, ModelRef, SessionRelationKind};

use crate::merge_system_prompts;
use crate::tool::ToolExecutor;

use super::{ExecutionState, SessionManager};

/// Resolve inheritance before trimming: an explicit empty value blocks fallback.
/// The project/global default has already been merged by configuration loading.
pub(super) fn resolve_additional_system_prompt<'a>(
    request: Option<&'a str>,
    session: Option<&'a str>,
    model: Option<&'a str>,
    default: Option<&'a str>,
) -> Option<&'a str> {
    request.or(session).or(model).or(default)
}

/// Use the same adapter resolution as execution, including implicit selections.
pub(super) fn model_additional_system_prompt<'a>(
    state: &'a ExecutionState,
    model: &ModelRef,
) -> Option<&'a str> {
    let selection = ExecutionSelection {
        provider: Some(model.provider_id.to_string()),
        adapter: model.adapter_id.as_ref().map(ToString::to_string),
        model: Some(model.model_id.to_string()),
        ..Default::default()
    };
    let adapter_id = state.providers.tool_execution_adapter(&selection).ok()?;
    let resolved = ModelRef {
        adapter_id,
        ..model.clone()
    };
    state.config.model_additional_system_prompt(&resolved)
}

/// Canonical tool identities mapped to the names advertised by this catalog.
type AvailableTools = BTreeMap<String, String>;

fn plugin_tools(tools: &AvailableTools, plugin: &str, names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|name| tools.get(&format!("{plugin}.{name}")))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn workflow_sections(role: SessionRelationKind, tools: &AvailableTools) -> Vec<String> {
    let subagent = role.is_subagent();
    let has = |name: &str| tools.contains_key(name);
    let mut paragraphs = Vec::new();

    let plan = plugin_tools(
        tools,
        "agena.plan",
        if subagent {
            &["get", "edit"]
        } else {
            &["get", "set", "edit", "review", "phase", "clear"]
        },
    );
    if !plan.is_empty() {
        let mut text = if subagent {
            format!(
                "Use the `agena.plan` plugin ({plan}) for available plan operations within your assignment. Report decisions and approval needs to the parent."
            )
        } else {
            format!(
                "For substantial work, use the `agena.plan` plugin ({plan}) to manage plans and progress under the current workflow. Match planning detail to the task."
            )
        };
        if !subagent {
            if has("agena.plan.review") || has("agena.plan.phase") {
                text.push_str(" Use its approval workflow when required; ordinary progress updates do not require repeated approval.");
            } else if has("agena.plan.set") {
                text.push_str(" Keep new plans in prose when no approval tool is available; do not create a planning state you cannot activate.");
            }
        }
        paragraphs.push(text);
    }

    if !subagent {
        let inline_ui = plugin_tools(tools, "agena.inline_ui", &["guide", "validate", "prepare"]);
        if has("agena.inline_ui.prepare") && !inline_ui.is_empty() {
            paragraphs.push(format!("Use Inline Generative UI ({inline_ui}) only when visual structure makes your answer clearer. Start with guide, then read only the needed content kind. Preparing content does not display it. Put the exact returned embed on a standalone line in the normal answer, with blank lines before and after. Web and TUI resolve it at that position. Do not print the document JSON. This is explanation, not an app or task-progress system."));
        }
    }

    if !subagent {
        let interaction = plugin_tools(tools, "agena.interaction", &["ask", "notify"]);
        if !interaction.is_empty() {
            let mut text = format!(
                "Use the `agena.interaction` plugin ({interaction}) for the available user interaction workflows."
            );
            if has("agena.interaction.ask") {
                text.push_str(" Ask specific, necessary questions for user decisions or missing authorization, with enough context to decide. Plan approval belongs to the planning workflow.");
            }
            if has("agena.interaction.notify") {
                text.push_str(" Send notifications when user attention is needed.");
            }
            paragraphs.push(text);
        }
    }

    let agent = plugin_tools(
        tools,
        "agena.agent",
        if subagent {
            &["message", "wait"]
        } else {
            &["spawn", "message", "followup", "wait", "cancel"]
        },
    );
    if !agent.is_empty() {
        let mut text = if subagent {
            format!(
                "Use the `agena.agent` plugin ({agent}) to coordinate with the parent and visible peers as the task requires."
            )
        } else {
            format!(
                "Use the `agena.agent` plugin ({agent}) for available delegation and coordination operations. Sequence work with dependencies, shared state, or conflicts; verify and integrate delegated results."
            )
        };
        if subagent && has("agena.agent.message") {
            text.push_str(" Send important findings and blockers to the parent.");
        }
        if !subagent {
            if has("agena.agent.spawn") {
                text.push_str(" Delegate bounded independent work when useful.");
            }
            if [
                "agena.agent.spawn",
                "agena.agent.followup",
                "agena.agent.message",
            ]
            .into_iter()
            .any(has)
            {
                text.push_str(" Include the necessary context, constraints, and output contracts in delegated tasks; child agents receive Markdown guidance by default and do not inherit this conversation. For rich-output assignments, direct the child to the available agena.inline_ui guide; do not forward legacy fenced-JSON UI instructions.");
            }
        }
        paragraphs.push(text);
    }

    let memory = plugin_tools(tools, "agena.memory", &["search", "get", "write"]);
    if !memory.is_empty() {
        let mut text = format!(
            "Use the `agena.memory` plugin ({memory}) for relevant historical context or durable information, choosing the available operations that fit the task."
        );
        if has("agena.memory.write") {
            text.push_str(" Save only suitable information with lasting value, never secrets or other sensitive content.");
        }
        paragraphs.push(text);
    }

    if paragraphs.is_empty() {
        Vec::new()
    } else {
        vec![format!("# Workflows\n\n{}", paragraphs.join("\n\n"))]
    }
}

fn assemble_system_prompt(
    role: SessionRelationKind,
    tools_enabled: bool,
    tools: &AvailableTools,
    user_system: Option<&str>,
    conversation_instruction: Option<&str>,
    project_guidance: &str,
) -> String {
    let sections = if tools_enabled {
        workflow_sections(role, tools)
    } else {
        Vec::new()
    };
    let base = crate::identity::system_prompt_for_session(role, tools_enabled, &sections);
    let mut prompt = merge_system_prompts(Some(base.as_str()), user_system).unwrap_or(base);
    if let Some(instruction) = conversation_instruction {
        prompt.push_str("\n\n");
        prompt.push_str(instruction);
    }
    if !project_guidance.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(project_guidance);
    }
    prompt
}

impl SessionManager {
    pub(crate) fn assemble_session_system_prompt_with_executor(
        &self,
        scoped_executor: &ToolExecutor,
        user_system: Option<&str>,
        tools_enabled: bool,
    ) -> String {
        let tools = if tools_enabled {
            let available = scoped_executor.available_execution_tools();
            let names = agena_runtime_tools::tool::execution_tool_names(&available);
            available
                .iter()
                .zip(names)
                .map(|(tool, name)| (tool.canonical_name(), name))
                .collect()
        } else {
            AvailableTools::new()
        };
        let conversation_instruction = scoped_executor.conversation_instruction();
        let project_guidance = scoped_executor.project_instruction_section(&[]);
        assemble_system_prompt(
            scoped_executor.session_relation_kind(),
            tools_enabled,
            &tools,
            user_system,
            conversation_instruction.as_deref(),
            &project_guidance,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{ConversationMode, SessionConversation};

    const ALL_WORKFLOW_TOOLS: &[&str] = &[
        "plan.get",
        "plan.set",
        "plan.edit",
        "plan.review",
        "plan.phase",
        "plan.clear",
        "interaction.ask",
        "interaction.notify",
        "agent.spawn",
        "agent.message",
        "agent.followup",
        "agent.wait",
        "agent.cancel",
        "memory.search",
        "memory.get",
        "memory.write",
        "inline_ui.guide",
        "inline_ui.validate",
        "inline_ui.prepare",
    ];

    fn catalog(names: &[&str]) -> AvailableTools {
        names
            .iter()
            .map(|name| (format!("agena.{name}"), (*name).to_owned()))
            .collect()
    }

    fn prompt(role: SessionRelationKind, enabled: bool, names: &[&str]) -> String {
        assemble_system_prompt(role, enabled, &catalog(names), None, None, "")
    }

    #[test]
    fn main_session_has_available_workflows_and_full_presentation() {
        let prompt = prompt(SessionRelationKind::Root, true, ALL_WORKFLOW_TOOLS);
        assert!(prompt.contains("# Main session"));
        assert!(!prompt.contains("# Subagent role"));
        for name in ALL_WORKFLOW_TOOLS {
            assert!(prompt.contains(&format!("`{name}`")), "{name}");
        }
        assert!(prompt.contains("output contracts in delegated tasks"));
        assert_eq!(prompt.matches("- **").count(), 9);
        assert_eq!(prompt.matches("# Presentation\n").count(), 1);
        assert!(prompt.contains("## Inline Generative UI"));
        assert!(prompt.contains("Start with guide"));
        assert!(!prompt.contains("```agena-chart"));
        assert!(!prompt.contains("```agena-ui"));
        assert!(!prompt.contains("await window.agena.ready"));
        assert!(!prompt.contains("await agena.fetch"));
    }

    #[test]
    fn subagent_focuses_on_parent_and_keeps_markdown_without_rich_contracts() {
        // Role guidance remains correct even if a caller supplies an unfiltered
        // catalog; runtime capability filtering is tested in the executor.
        let prompt = prompt(SessionRelationKind::Subagent, true, ALL_WORKFLOW_TOOLS);
        assert!(prompt.contains("# Subagent role"));
        assert!(prompt.contains("Your direct recipient is the parent agent"));
        assert!(prompt.contains("missing context or output contracts from the parent"));
        assert!(!prompt.contains("# Main session"));
        for name in [
            "interaction.ask",
            "interaction.notify",
            "agent.spawn",
            "agent.followup",
            "agent.cancel",
            "plan.set",
            "plan.review",
            "plan.phase",
            "plan.clear",
        ] {
            assert!(!prompt.contains(&format!("`{name}`")), "{name}");
        }
        for name in ["agent.message", "agent.wait", "plan.get", "plan.edit"] {
            assert!(prompt.contains(&format!("`{name}`")), "{name}");
        }
        assert_eq!(prompt.matches("- **").count(), 9);
        assert!(prompt.contains("`mermaid`"));
        assert!(!prompt.contains("agena-chart"));
        assert!(!prompt.contains("agena-ui"));
        assert!(!prompt.contains("agena.fetch"));
    }

    #[test]
    fn partial_plugins_use_only_advertised_names_and_keep_planning_recoverable() {
        let mut tools = catalog(&[
            "plan.get",
            "plan.set",
            "agent.message",
            "interaction.notify",
            "memory.get",
        ]);
        tools.insert("agena.agent.message".into(), "agena.agent.message".into());
        let prompt =
            assemble_system_prompt(SessionRelationKind::Root, true, &tools, None, None, "");
        assert!(prompt.contains("Keep new plans in prose"));
        assert!(prompt.contains("`agena.agent.message`"));
        assert!(!prompt.contains("`agent.message`"));
        for name in [
            "plan.review",
            "plan.phase",
            "agent.spawn",
            "agent.wait",
            "interaction.ask",
            "memory.write",
        ] {
            assert!(!prompt.contains(&format!("`{name}`")), "{name}");
        }
        assert!(!prompt.contains("Save only suitable information"));
    }

    #[test]
    fn absent_plugins_do_not_add_workflows() {
        let prompt = prompt(SessionRelationKind::Root, true, &["fs.read"]);
        assert!(prompt.contains("`tools_help`"));
        assert!(!prompt.contains("# Workflows"));
        for plugin in [
            "agena.plan",
            "agena.interaction",
            "agena.agent",
            "agena.memory",
        ] {
            assert!(!prompt.contains(&format!("`{plugin}`")), "{plugin}");
        }
    }

    #[test]
    fn followup_preserves_contract_handoff_without_spawn() {
        let prompt = prompt(SessionRelationKind::Root, true, &["agent.followup"]);
        assert!(prompt.contains("output contracts in delegated tasks"));
        assert!(!prompt.contains("`agent.spawn`"));
        assert!(!prompt.contains("Delegate bounded independent work"));
    }

    #[test]
    fn waiting_without_messaging_uses_the_final_report_fallback() {
        let prompt = prompt(SessionRelationKind::Subagent, true, &["agent.wait"]);
        assert!(prompt.contains("`agent.wait`"));
        assert!(!prompt.contains("`agent.message`"));
        assert!(!prompt.contains("Send important findings and blockers"));
        assert!(prompt.contains("if messaging is unavailable"));
    }

    #[test]
    fn disabled_tool_api_omits_tool_instructions_even_with_a_full_catalog() {
        for role in [SessionRelationKind::Root, SessionRelationKind::Subagent] {
            let prompt = prompt(role, false, ALL_WORKFLOW_TOOLS);
            assert!(!prompt.contains("# Tool discovery"));
            assert!(!prompt.contains("# Workflows"));
            for name in ["plugins_list", "tools_list", "tools_help", "tools_call"] {
                assert!(!prompt.contains(&format!("`{name}`")), "{name}");
            }
            assert!(prompt.contains("# Quality and verification"));
            assert!(prompt.contains("# Authorization and protecting work"));
            assert_eq!(prompt.matches("- **").count(), 9);
        }
    }

    #[test]
    fn user_forks_and_children_keep_the_main_role() {
        for role in [
            SessionRelationKind::Root,
            SessionRelationKind::Child,
            SessionRelationKind::Fork,
            SessionRelationKind::Rewind,
        ] {
            let prompt = prompt(role, true, &[]);
            assert!(prompt.contains("# Main session"), "{role:?}");
            assert!(!prompt.contains("# Subagent role"), "{role:?}");
            assert!(prompt.contains("## Inline Generative UI"), "{role:?}");
            assert!(!prompt.contains("```agena-ui"), "{role:?}");
        }
    }

    #[test]
    fn inline_ui_discovery_is_progressive_and_uses_only_available_tools() {
        let prompt = prompt(
            SessionRelationKind::Root,
            true,
            &["inline_ui.guide", "inline_ui.prepare"],
        );
        assert!(prompt.contains("`inline_ui.guide`"));
        assert!(prompt.contains("`inline_ui.prepare`"));
        assert!(!prompt.contains("`inline_ui.validate`"));
        assert!(prompt.contains("read only the needed content kind"));
        assert!(!prompt.contains("\"blocks\":"));
        let missing_publisher = assemble_system_prompt(
            SessionRelationKind::Root,
            true,
            &catalog(&["inline_ui.guide"]),
            None,
            None,
            "",
        );
        assert!(!missing_publisher.contains("Use Inline Generative UI ("));
    }

    #[test]
    fn preserves_session_instruction_layers() {
        let tools = catalog(ALL_WORKFLOW_TOOLS);
        let base = assemble_system_prompt(SessionRelationKind::Root, true, &tools, None, None, "");
        let prompt = assemble_system_prompt(
            SessionRelationKind::Root,
            true,
            &tools,
            Some("User-specific instructions."),
            Some("Conversation-specific instructions."),
            "Root project instructions.",
        );
        assert_eq!(
            prompt,
            format!(
                "{base}\n\nUser-specific instructions.\n\nConversation-specific instructions.\n\nRoot project instructions."
            )
        );
    }

    #[test]
    fn additional_prompt_precedence_and_explicit_empty_values() {
        let cases = [
            ([None, None, None, None], None),
            (
                [None, None, None, Some("global/project")],
                Some("global/project"),
            ),
            (
                [None, None, Some("model"), Some("global/project")],
                Some("model"),
            ),
            (
                [None, Some("session"), Some("model"), Some("global/project")],
                Some("session"),
            ),
            (
                [
                    Some("request"),
                    Some("session"),
                    Some("model"),
                    Some("global/project"),
                ],
                Some("request"),
            ),
            ([None, None, None, Some("")], Some("")),
            ([None, None, Some(""), Some("global/project")], Some("")),
            (
                [None, Some(""), Some("model"), Some("global/project")],
                Some(""),
            ),
            (
                [
                    Some(""),
                    Some("session"),
                    Some("model"),
                    Some("global/project"),
                ],
                Some(""),
            ),
            (
                [None, Some(" \n\t"), Some("model"), Some("global/project")],
                Some(" \n\t"),
            ),
        ];
        for ([request, session, model, default], expected) in cases {
            assert_eq!(
                resolve_additional_system_prompt(request, session, model, default),
                expected
            );
        }
    }

    #[test]
    fn disabling_additional_instructions_preserves_each_role_and_boundary() {
        for role in [SessionRelationKind::Root, SessionRelationKind::Subagent] {
            for empty in ["", " \n\t"] {
                let base = assemble_system_prompt(
                    role,
                    false,
                    &AvailableTools::new(),
                    None,
                    Some("Conversation boundary."),
                    "Project guidance.",
                );
                let selected = resolve_additional_system_prompt(
                    None,
                    None,
                    Some(empty),
                    Some("Inherited global instructions."),
                );
                let actual = assemble_system_prompt(
                    role,
                    false,
                    &AvailableTools::new(),
                    selected,
                    Some("Conversation boundary."),
                    "Project guidance.",
                );
                assert_eq!(actual, base);
                assert!(!actual.contains("Inherited global instructions."));
            }
        }
    }

    #[test]
    fn does_not_repeat_the_selected_base_from_user_system() {
        let tools = catalog(ALL_WORKFLOW_TOOLS);
        for role in [SessionRelationKind::Root, SessionRelationKind::Subagent] {
            for enabled in [false, true] {
                let base = assemble_system_prompt(role, enabled, &tools, None, None, "");
                let user_system = format!("{base}\n\nUser-specific instructions.");
                let prompt =
                    assemble_system_prompt(role, enabled, &tools, Some(&user_system), None, "");
                assert_eq!(prompt, user_system);
            }
        }
    }

    #[test]
    fn keeps_btw_boundary_without_mutating_workflows() {
        let boundary = SessionConversation {
            mode: ConversationMode::Btw,
            parent_session_id: 42,
        }
        .instruction();
        let prompt = assemble_system_prompt(
            SessionRelationKind::Fork,
            true,
            &catalog(&["fs.read"]),
            None,
            Some(&boundary),
            "",
        );
        assert!(prompt.contains("# Main session"));
        assert!(prompt.ends_with(&boundary));
        assert!(!prompt.contains("# Workflows"));
        assert!(!prompt.contains("`agent.spawn`"));
        assert!(!prompt.contains("`plan.set`"));
    }
}
