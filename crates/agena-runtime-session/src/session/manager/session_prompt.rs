//! Compose shared rules, session responsibilities, and prompt layers.
//! Tool schemas and execution manuals remain in live tool help.

use std::collections::BTreeMap;

use agena_domain::{ModelRef, SessionRelationKind};
use agena_prompt_layers::{
    LayerSource, PromptContext, PromptLayer, PromptRoute, PromptScope, route_matches,
};

use crate::merge_system_prompts;
use crate::tool::ToolExecutor;

use super::{ExecutionState, SessionManager};

/// Everything needed to resolve one request's additional instructions.
pub(super) struct PromptInputs<'a> {
    pub session: &'a PromptLayer,
    pub request_layer: Option<&'a PromptLayer>,
    pub model: &'a ModelRef,
    pub session_id: i64,
    pub workspace: String,
    pub tools_enabled: bool,
}

/// Resolve the configured layers plus the session and request layers.
///
/// Assembly failures are reported to the model instead of aborting the turn:
/// a bad fragment id must not make a session unusable, but it must be visible.
pub(super) fn resolve_configured_prompts(
    state: &ExecutionState,
    inputs: &PromptInputs<'_>,
) -> String {
    match resolve_prompt_layers(state, inputs) {
        Ok(resolution) => {
            for warning in &resolution.warnings {
                tracing::warn!(
                    scope = warning.scope.as_str(),
                    fragment = %warning.id,
                    detail = %warning.detail,
                    "prompt fragment skipped"
                );
            }
            resolution.text
        }
        Err(error) => {
            tracing::error!(%error, "prompt layers could not be assembled");
            format!("[Additional instructions could not be assembled: {error}]")
        }
    }
}

/// The fragments that consume the most budget, largest first.
pub(super) fn prompt_layer_contributors(
    state: &ExecutionState,
    inputs: &PromptInputs<'_>,
) -> Vec<String> {
    let mut fragments = resolve_prompt_layers(state, inputs)
        .map(|resolution| resolution.fragments)
        .unwrap_or_default();
    fragments.sort_by_key(|fragment| std::cmp::Reverse(fragment.bytes));
    fragments
        .into_iter()
        .take(3)
        .map(|fragment| {
            format!(
                "{} ({}), {} bytes",
                fragment.id,
                fragment.scope.as_str(),
                fragment.bytes
            )
        })
        .collect()
}

/// Resolve the same layers assembly uses, for a preview request.
pub(super) fn prompt_preview(
    state: &ExecutionState,
    inputs: &PromptInputs<'_>,
) -> Result<agena_prompt_layers::PromptResolution, agena_prompt_layers::PromptError> {
    resolve_prompt_layers(state, inputs)
}

fn resolve_prompt_layers(
    state: &ExecutionState,
    inputs: &PromptInputs<'_>,
) -> Result<agena_prompt_layers::PromptResolution, agena_prompt_layers::PromptError> {
    let config = &state.config.prompts;
    let route = PromptRoute::new(
        inputs.model.provider_id.as_ref(),
        inputs.model.adapter_id.as_ref().map(ToString::to_string),
        inputs.model.model_id.as_ref(),
    );
    let context = PromptContext {
        route: Some(route.clone()),
        workspace: inputs.workspace.clone(),
        platform: std::env::consts::OS.to_owned(),
        session_id: inputs.session_id,
        tools_enabled: inputs.tools_enabled,
    };
    let mut global_library = config
        .library(PromptScope::Global)
        .unwrap_or_else(|_| BTreeMap::new());
    let mut workspace_library = config
        .library(PromptScope::Workspace)
        .unwrap_or_else(|_| BTreeMap::new());
    // Saved prompts (and legacy inline definitions) are shared by reference.
    // A narrower workspace definition shadows a global definition with the
    // same id, while an inline fragment in the current layer wins locally.
    for fragment in &config.global.inline {
        global_library.insert(fragment.id.clone(), fragment.clone());
    }
    for fragment in &config.workspace.inline {
        workspace_library.insert(fragment.id.clone(), fragment.clone());
    }
    let mut shared_library = global_library.clone();
    shared_library.extend(workspace_library.clone());
    let empty = BTreeMap::new();
    let global_layer = config.global.layer();
    let workspace_layer = config.workspace.layer();
    let empty_layer = PromptLayer::default();
    let mut sources = vec![
        LayerSource::new(PromptScope::Global, &global_layer, &global_library),
        LayerSource::new(PromptScope::Workspace, &workspace_layer, &shared_library),
    ];
    // A model layer keeps its own scope's library so it can reference
    // fragments declared beside it.
    let global_model_layers = matching_layers(&config.global.models, &route);
    let workspace_model_layers = matching_layers(&config.workspace.models, &route);
    for layer in &global_model_layers {
        sources.push(LayerSource::new(PromptScope::Model, layer, &global_library));
    }
    for layer in &workspace_model_layers {
        sources.push(LayerSource::new(PromptScope::Model, layer, &shared_library));
    }
    sources.push(LayerSource::new(
        PromptScope::Session,
        inputs.session,
        &shared_library,
    ));
    sources.push(LayerSource::new(
        PromptScope::Request,
        inputs.request_layer.unwrap_or(&empty_layer),
        &empty,
    ));
    agena_prompt_layers::resolve(&sources, &context)
}

/// Model-scoped layers whose `match` pattern selects this request route.
fn matching_layers(
    models: &[crate::config::ModelPromptConfig],
    route: &PromptRoute,
) -> Vec<PromptLayer> {
    models
        .iter()
        .filter(|model| route_matches(&model.match_pattern, route))
        .map(crate::config::ModelPromptConfig::layer)
        .collect()
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

    // Inspect-then-answer guidance for the model's own identity and runtime
    // state. Only tools advertised to this session are named.
    let session_roles: &[(&str, &str)] = &[
        ("get", "session identity, relation, and title"),
        (
            "model",
            "the serving model, provider, adapter, runtime modes, and model token limits",
        ),
        (
            "tokens",
            "measured and projected token use and the remaining budget",
        ),
        (
            "environment",
            "the workspace path, Git state, shell, platform, and host toolchain",
        ),
        (
            "executables",
            "installed host CLIs and their task-specific usage",
        ),
    ];

    let mut sources = session_roles
        .iter()
        .filter_map(|(name, purpose)| {
            tools
                .get(&format!("agena.session.{name}"))
                .map(|advertised| format!("`{advertised}` for {purpose}"))
        })
        .collect::<Vec<_>>();
    if let Some(models) = tools.get("agena.provider.models") {
        sources.push(format!(
            "`{models}` for the other model routes configured on the current provider"
        ));
    }
    if !sources.is_empty() {
        paragraphs.push(format!(
            "Questions about your own identity, the current session, or its runtime state are facts to inspect rather than recall. Call the relevant tool before answering and report what it returns, including provider and adapter identifiers: {}. Never substitute a model name, version, context window, or token count from memory, and name any fact you could not verify.",
            sources.join("; ")
        ));
    }

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
        let inline_ui = plugin_tools(
            tools,
            "agena.inline_ui",
            &["guide", "prepare", "read", "update"],
        );
        if has("agena.inline_ui.prepare") {
            let mut guidance = format!(
                "Use Inline Generative UI ({inline_ui}) when visual structure or local interaction materially improves the answer over Markdown, or the user explicitly requests a supported rich view. Do not add decorative UI to otherwise simple answers. Follow the selection criteria under Presentation. "
            );
            if has("agena.inline_ui.guide") {
                guidance.push_str("A single guide() explains the normal contract; request advanced sections only when needed, and prepare directly once learned. ");
            }
            guidance.push_str("Prepare validates content but does not display it. Paste the exact returned inline-ui embed fence between Markdown paragraphs, not the document JSON or an outer code fence. Web and TUI render it there. ");
            if has("agena.inline_ui.update") {
                guidance.push_str("For genuine ongoing work, prepare with phase working, display the fence early in commentary, and update the same reference using returned revisions. ");
            }
            if has("agena.inline_ui.read") {
                guidance.push_str("Use read only to inspect or resolve conflicts, not as a mandatory update step. ");
            }
            guidance.push_str("Show only observed progress; preserve user-local selections, and never treat UI actions as external tool execution.");
            paragraphs.push(guidance);
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
        "session.get",
        "session.model",
        "session.tokens",
        "session.environment",
        "session.executables",
        "provider.models",
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
        "inline_ui.prepare",
        "inline_ui.read",
        "inline_ui.update",
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
        assert!(prompt.contains("single `guide()` call, when offered"));
        assert!(prompt.contains("initial phase complete by default"));
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
        assert!(!prompt.contains("## Inline Generative UI"));
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
        assert!(prompt.contains("If the relevant tools"));
        assert!(!prompt.contains("Use Inline Generative UI ("));
        assert!(!prompt.contains("facts to inspect rather than recall"));
        for plugin in [
            "agena.plan",
            "agena.interaction",
            "agena.agent",
            "agena.memory",
            "agena.session",
            "agena.provider",
        ] {
            assert!(!prompt.contains(&format!("`{plugin}`")), "{plugin}");
        }
    }

    #[test]
    fn self_knowledge_routes_identity_questions_to_session_tools() {
        // The failure this guards against: a model answering "which model are
        // you?" from memory because nothing in the prompt points at the
        // inspection tools that are actually available.
        let full = prompt(SessionRelationKind::Root, true, ALL_WORKFLOW_TOOLS);
        let section = full
            .split("\n\n")
            .find(|paragraph| paragraph.contains("facts to inspect rather than recall"))
            .expect("self-knowledge paragraph");
        for name in [
            "session.get",
            "session.model",
            "session.tokens",
            "session.environment",
            "session.executables",
            "provider.models",
        ] {
            assert!(section.contains(&format!("`{name}`")), "{name}");
        }
        assert!(section.contains("Never substitute a model name"));

        // Partial catalogues advertise only what this session can actually call.
        let limited = prompt(
            SessionRelationKind::Root,
            true,
            &["session.model", "session.tokens"],
        );
        let section = limited
            .split("\n\n")
            .find(|paragraph| paragraph.contains("facts to inspect rather than recall"))
            .expect("self-knowledge paragraph");
        assert!(section.contains("`session.model`"));
        assert!(section.contains("`session.tokens`"));
        for name in [
            "session.get",
            "session.environment",
            "session.executables",
            "provider.models",
        ] {
            assert!(!section.contains(&format!("`{name}`")), "{name}");
        }

        // A provider-only catalogue still routes "which models are configured?".
        let provider_only = prompt(SessionRelationKind::Root, true, &["provider.models"]);
        assert!(provider_only.contains("facts to inspect rather than recall"));
        assert!(provider_only.contains("`provider.models`"));
        assert!(!provider_only.contains("`session.model`"));
    }

    #[test]
    fn subagents_get_the_same_self_knowledge_guidance() {
        let full = prompt(SessionRelationKind::Subagent, true, ALL_WORKFLOW_TOOLS);
        let section = full
            .split("\n\n")
            .find(|paragraph| paragraph.contains("facts to inspect rather than recall"))
            .expect("self-knowledge paragraph");
        assert!(section.contains("`session.model`"));
        assert!(section.contains("`session.tokens`"));
        assert!(!section.contains("`session.rename`"));
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
            assert!(!prompt.contains("## Inline Generative UI"));
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
    fn inline_ui_discovery_teaches_core_once_and_uses_only_available_tools() {
        let limited = prompt(
            SessionRelationKind::Root,
            true,
            &["inline_ui.guide", "inline_ui.prepare"],
        );
        assert!(limited.contains("`inline_ui.guide`"));
        assert!(limited.contains("`inline_ui.prepare`"));
        assert!(!limited.contains("`inline_ui.validate`"));
        assert!(limited.contains("A single guide() explains the normal contract"));
        assert!(limited.contains("prepare directly once learned"));
        assert!(!limited.contains("For genuine ongoing work, prepare with phase working"));
        assert!(!limited.contains("Use read only to inspect or resolve conflicts"));
        assert!(!limited.contains("read only the needed content kind"));
        assert!(!limited.contains("\"blocks\":"));

        let full = prompt(SessionRelationKind::Root, true, ALL_WORKFLOW_TOOLS);
        assert!(full.contains("`inline_ui.read`"));
        assert!(full.contains("`inline_ui.update`"));
        assert!(full.contains("For genuine ongoing work, prepare with phase working"));
        assert!(full.contains("Use read only to inspect or resolve conflicts"));

        let no_guide = prompt(SessionRelationKind::Root, true, &["inline_ui.prepare"]);
        assert!(no_guide.contains("Use Inline Generative UI (`inline_ui.prepare`)"));
        assert!(!no_guide.contains("A single guide() explains the normal contract"));

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
    fn rich_output_selection_promotes_useful_ui_without_forcing_it() {
        let prompt = prompt(SessionRelationKind::Root, true, ALL_WORKFLOW_TOOLS);
        let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        // Both a positive threshold and concrete negative cases are needed:
        // otherwise the model either never discovers UI or decorates every answer.
        for required in [
            "Proactively use",
            "materially improve understanding or usability over clear Markdown",
            "the user need not explicitly request a UI",
            "Good fits include several related metrics",
            "multidimensional comparisons",
            "filters or other local controls",
            "*observed* status",
            "Choose the smallest useful UI",
            "skip inline UI for greetings, short factual answers",
            "Honor an explicit request for a supported rich view",
            "simple Markdown tables",
            "code or diffs",
            "plain text, raw JSON",
            "standalone application",
            "If the relevant tools",
            "Never invent measurements or progress",
            "Prepare validates and stores content",
            "Read only to inspect state or reconcile a conflict",
        ] {
            assert!(
                normalized.contains(required),
                "missing UI-selection rule: {required}"
            );
        }
        for stale in [
            "optionally `validate`",
            "read only the needed topic",
            "start with `guide`,",
            "```agena-ui",
        ] {
            assert!(!prompt.contains(stale), "stale UI contract: {stale}");
        }
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
    fn retired_string_prompt_field_is_gone() {
        let error = serde_json::from_str::<crate::config::SessionConfig>(
            r#"{"additional_system_prompt":"legacy"}"#,
        );
        assert!(error.is_err());
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
