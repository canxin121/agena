//! Agena's built-in system prompt catalog.
//!
//! The catalog supplies shared instructions in their default order. Session
//! relationship instructions are assembled from the current session.

use agena_domain::SessionRelationKind;
use agena_prompt_layers::{
    BuiltinPrompt, BuiltinPromptWhen, FragmentId, PromptFragment, PromptResolution, PromptScope,
    ResolvedFragment,
};

pub const AGENA_AGENT_ID: &str = "agena";

/// Detailed Git reference available to callers that need it.
pub const AGENA_GIT_WORKFLOW_PROMPT: &str = include_str!("git_workflow.md");

const TOOL_PROTOCOL_ID: &str = "agena.system.tools";

struct PromptSpec {
    id: &'static str,
    title: &'static str,
    aliases: &'static [&'static str],
    sections: &'static [(BuiltinPromptWhen, &'static str)],
}

fn applies(when: BuiltinPromptWhen, relation: SessionRelationKind, tools_enabled: bool) -> bool {
    match when {
        BuiltinPromptWhen::Always => true,
        BuiltinPromptWhen::UserFacing => !relation.is_subagent(),
        BuiltinPromptWhen::Subagent => relation.is_subagent(),
        BuiltinPromptWhen::Tools => tools_enabled,
        BuiltinPromptWhen::UserFacingTools => tools_enabled && !relation.is_subagent(),
    }
}

use BuiltinPromptWhen::{Always, UserFacingTools};

const BUILTIN_PROMPTS: &[PromptSpec] = &[
    PromptSpec {
        id: "agena.system.identity",
        title: "Assistant identity and shared rules",
        aliases: &[
            "agena.system.evidence",
            "agena.system.authorization",
            "agena.system.context",
            "agena.system.conversation",
            "agena.system.delegation",
        ],
        sections: &[
            (Always, include_str!("identity.md")),
            (Always, include_str!("evidence.md")),
            (Always, include_str!("authorization.md")),
            (Always, include_str!("context.md")),
        ],
    },
    PromptSpec {
        id: TOOL_PROTOCOL_ID,
        title: "Tool protocol",
        aliases: &[],
        sections: &[(Always, include_str!("tools.md"))],
    },
    PromptSpec {
        id: "agena.system.communication",
        title: "Communication and delivery",
        aliases: &["agena.system.markdown", "agena.system.rich-output"],
        sections: &[
            (Always, include_str!("communication.md")),
            (Always, include_str!("markdown.md")),
            (UserFacingTools, include_str!("rich_output.md")),
        ],
    },
];

pub const INVESTIGATION_PROMPT_ID: &str = "agena.profile.investigation";
pub const IMPLEMENTATION_PROMPT_ID: &str = "agena.profile.implementation";
pub const READ_ONLY_PROMPT_ID: &str = "agena.profile.read-only";
pub const PLANNING_PROMPT_ID: &str = "agena.profile.planning";
pub const VERIFICATION_PROMPT_ID: &str = "agena.profile.verification";

const REUSABLE_PROMPTS: &[(&str, &str, &str)] = &[
    (
        INVESTIGATION_PROMPT_ID,
        "Investigation and evidence",
        include_str!("investigation.md"),
    ),
    (
        IMPLEMENTATION_PROMPT_ID,
        "Implementation and verification",
        include_str!("implementation.md"),
    ),
    (
        READ_ONLY_PROMPT_ID,
        "Read-only work",
        include_str!("read_only.md"),
    ),
    (
        PLANNING_PROMPT_ID,
        "Planning and acceptance criteria",
        include_str!("planning.md"),
    ),
    (
        VERIFICATION_PROMPT_ID,
        "Verification and evidence",
        include_str!("verification.md"),
    ),
];

fn fragment_id(id: &str) -> FragmentId {
    FragmentId::parse(id).expect("valid built-in prompt ID")
}

pub fn canonical_builtin_prompt_id(id: &FragmentId) -> Option<&'static str> {
    BUILTIN_PROMPTS
        .iter()
        .find(|spec| spec.id == id.as_str() || spec.aliases.contains(&id.as_str()))
        .map(|spec| spec.id)
}

pub fn is_builtin_prompt(id: &FragmentId) -> bool {
    canonical_builtin_prompt_id(id).is_some()
}

pub fn builtin_prompt_catalog() -> Vec<BuiltinPrompt> {
    BUILTIN_PROMPTS
        .iter()
        .map(|spec| BuiltinPrompt {
            id: fragment_id(spec.id),
            title: spec.title.to_owned(),
            body: spec
                .sections
                .iter()
                .map(|(when, body)| {
                    if *when == UserFacingTools {
                        format!(
                            "> Applies to user-facing sessions with tools.\n\n{}",
                            body.trim()
                        )
                    } else {
                        body.trim().to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            when: Always,
            aliases: spec.aliases.iter().map(|id| fragment_id(id)).collect(),
            reusable: false,
        })
        .collect()
}

pub fn reusable_prompt_fragments() -> impl Iterator<Item = PromptFragment> {
    REUSABLE_PROMPTS
        .iter()
        .map(|&(id, title, body)| PromptFragment {
            title: title.to_owned(),
            ..PromptFragment::new(fragment_id(id), body.trim())
        })
}

pub fn reusable_prompt_catalog() -> impl Iterator<Item = BuiltinPrompt> {
    reusable_prompt_fragments().map(|fragment| BuiltinPrompt {
        id: fragment.id,
        title: fragment.title,
        body: fragment.body,
        when: Always,
        aliases: Vec::new(),
        reusable: true,
    })
}

/// Build the required prompt fragments. Append live capability guidance to the
/// tool protocol when the tool API is enabled.
pub fn builtin_prompt_sections(
    relation: SessionRelationKind,
    tools_enabled: bool,
    sections: &[String],
) -> Vec<ResolvedFragment> {
    BUILTIN_PROMPTS
        .iter()
        .map(|spec| {
            let mut body: Vec<_> = spec
                .sections
                .iter()
                .filter(|(when, _)| applies(*when, relation, tools_enabled))
                .map(|(_, body)| body.trim())
                .collect();
            if spec.id == TOOL_PROTOCOL_ID && tools_enabled {
                body.extend(
                    sections
                        .iter()
                        .map(|text| text.trim())
                        .filter(|text| !text.is_empty()),
                );
            }
            ResolvedFragment::new(
                fragment_id(spec.id),
                spec.title,
                PromptScope::Builtin,
                body.join("\n\n"),
            )
        })
        .collect()
}

/// Choose the recipient and authorization workflow for the current session.
pub fn session_relation_prompt(relation: SessionRelationKind) -> ResolvedFragment {
    let body = if relation.is_subagent() {
        include_str!("delegation.md")
    } else {
        include_str!("conversation.md")
    };
    ResolvedFragment::new(
        fragment_id("agena.runtime.session-relation"),
        "Session relationship",
        PromptScope::Runtime,
        body.trim().to_owned(),
    )
}

/// Build the user-facing execution foundation without profile instructions.
pub fn system_prompt() -> String {
    system_prompt_with_sections(&[])
}

pub fn system_prompt_with_sections(sections: &[String]) -> String {
    system_prompt_for_session(SessionRelationKind::Root, true, sections)
}

pub fn system_prompt_for_session(
    relation: SessionRelationKind,
    tools_enabled: bool,
    sections: &[String],
) -> String {
    let mut fragments = builtin_prompt_sections(relation, tools_enabled, sections);
    fragments.push(session_relation_prompt(relation));
    PromptResolution::from_fragments(fragments, Vec::new()).text
}
