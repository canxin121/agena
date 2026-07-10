//! Custom subagent profile registry.
//!
//! User-defined subagents are configured in the shared `~/agena/agena.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::BTreeMap, fmt};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    Project,
    User,
    Default,
}

impl AsRef<str> for AgentScope {
    fn as_ref(&self) -> &str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Default => "default",
        }
    }
}

impl fmt::Display for AgentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSelectionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl AgentSelectionConfig {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontmatter {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "AgentSelectionConfig::is_empty")]
    pub defaults: AgentSelectionConfig,
}

#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub name: String,
    pub frontmatter: AgentFrontmatter,
    pub prompt: String,
    pub source_path: Option<PathBuf>,
    pub scope: AgentScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "AgentSelectionConfig::is_empty")]
    pub defaults: AgentSelectionConfig,
    pub scope: AgentScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed agent: {0}")]
    Malformed(String),
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("unknown subagent profile: {0}")]
    UnknownProfile(String),
}

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug, Clone, Default)]
pub struct SubagentRegistry {
    inner: Arc<RwLock<AgentRegistryInner>>,
}

#[derive(Debug, Default)]
struct AgentRegistryInner {
    by_name: BTreeMap<String, AgentProfile>,
}

impl SubagentRegistry {
    pub fn discover(_workspace_root: &Path, _user_root: Option<&Path>) -> Self {
        let registry = Self::default();
        registry.reload_disk();
        registry
    }

    pub fn reload_disk(&self) {
        let mut inner = self.inner.write();
        inner.by_name.clear();
        for profile in default_profiles() {
            agents_upsert(&mut inner.by_name, profile, AgentScope::Default);
        }
    }

    pub fn register_runtime(&self, profile: AgentProfile) {
        let mut inner = self.inner.write();
        let scope = profile.scope;
        agents_upsert(&mut inner.by_name, profile, scope);
    }

    pub fn remove_runtime(&self, name: &str) -> bool {
        let mut inner = self.inner.write();
        let removed = inner.by_name.remove(name).is_some();
        let to_drop: Vec<String> = inner
            .by_name
            .iter()
            .filter(|(_, profile)| profile.name == name)
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_drop {
            inner.by_name.remove(&key);
        }
        removed
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().by_name.keys().cloned().collect()
    }

    pub fn list(&self) -> Vec<AgentProfile> {
        let inner = self.inner.read();
        let mut seen: BTreeMap<String, AgentProfile> = BTreeMap::new();
        for profile in inner.by_name.values() {
            if !profile.is_exposed() {
                continue;
            }
            seen.entry(profile.name.clone())
                .or_insert_with(|| profile.clone());
        }
        seen.into_values().collect()
    }

    pub fn list_descriptors(&self) -> Vec<AgentDescriptor> {
        let mut descriptors = self
            .list()
            .into_iter()
            .map(AgentDescriptor::from)
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    pub fn get(&self, name: &str) -> Option<AgentProfile> {
        self.inner.read().by_name.get(name).cloned()
    }

    pub fn require(&self, name: &str) -> AgentResult<AgentProfile> {
        self.get(name)
            .ok_or_else(|| AgentError::UnknownProfile(name.to_string()))
    }
}

fn agents_upsert(
    by_name: &mut BTreeMap<String, AgentProfile>,
    profile: AgentProfile,
    scope: AgentScope,
) {
    let key = profile.name.clone();
    match by_name.get(&key) {
        Some(existing) => {
            if scope_priority(scope) >= scope_priority(existing.scope) {
                by_name.insert(key, profile);
            }
        }
        None => {
            by_name.insert(key, profile);
        }
    }
}

impl AgentProfile {
    pub fn is_exposed(&self) -> bool {
        self.name != "compaction"
    }

    pub fn from_raw(raw: &str, default_name: &str, scope: AgentScope) -> AgentResult<Self> {
        if default_name.trim().is_empty() {
            return Err(AgentError::Malformed(
                "agent file name (stem) must not be empty".into(),
            ));
        }
        let (frontmatter, prompt) = parse_frontmatter(raw)?;
        Ok(Self {
            name: default_name.to_string(),
            frontmatter,
            prompt,
            source_path: None,
            scope,
        })
    }
}

impl From<AgentProfile> for AgentDescriptor {
    fn from(profile: AgentProfile) -> Self {
        Self {
            name: profile.name,
            description: profile.frontmatter.description,
            permission: profile.frontmatter.permission,
            defaults: profile.frontmatter.defaults,
            scope: profile.scope,
            source_path: profile.source_path,
        }
    }
}

pub(crate) fn internal_allowed_tools(profile_name: &str) -> Vec<String> {
    match profile_name {
        "compaction" => vec!["__agena_compaction_no_tools__".to_string()],
        _ => Vec::new(),
    }
}

fn parse_frontmatter(raw: &str) -> AgentResult<(AgentFrontmatter, String)> {
    let normalized = raw.replace("\r\n", "\n");
    let Some(stripped) = normalized.strip_prefix("---\n") else {
        return Ok((AgentFrontmatter::default(), normalized.trim().to_string()));
    };
    let Some(end) = stripped.find("\n---") else {
        return Err(AgentError::Malformed(
            "frontmatter missing closing '---'".into(),
        ));
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter: AgentFrontmatter = if yaml.trim().is_empty() {
        AgentFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml)?
    };
    Ok((frontmatter, body))
}

fn scope_priority(scope: AgentScope) -> u8 {
    match scope {
        AgentScope::Default => 0,
        AgentScope::User => 1,
        AgentScope::Project => 2,
    }
}

fn default_permission(
    workspace_write: crate::permission::PermissionMode,
    external: crate::permission::PermissionMode,
    names: &[(&str, crate::permission::PermissionMode)],
) -> crate::agent::AgentPermissionConfig {
    crate::agent::PermissionConfig {
        path: Some(crate::agent::PathPermissionConfig {
            workspace: Some(crate::agent::PathAccessModes {
                read: Some(crate::permission::PermissionMode::Allow),
                write: Some(workspace_write),
            }),
            external: Some(crate::agent::PathAccessModes {
                read: Some(external),
                write: Some(external),
            }),
            ..Default::default()
        }),
        tools: Some(crate::agent::ToolPermissionConfig {
            names: names
                .iter()
                .map(|(name, mode)| ((*name).to_string(), *mode))
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn default_profiles() -> Vec<AgentProfile> {
    vec![
        compaction_profile(),
        default_profile(
            "build",
            "Primary coding agent for normal end-to-end implementation work.",
            default_permission(
                crate::permission::PermissionMode::Ask,
                crate::permission::PermissionMode::Ask,
                &[
                    ("user", crate::permission::PermissionMode::Allow),
                    ("plan", crate::permission::PermissionMode::Allow),
                    ("agent", crate::permission::PermissionMode::Allow),
                    ("session", crate::permission::PermissionMode::Allow),
                    ("task", crate::permission::PermissionMode::Allow),
                    ("tools", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are the primary engineering agent. Own the task end to end, choose tools pragmatically, delegate when it helps, preserve surrounding behavior, and avoid reverting unrelated work.",
        ),
        default_profile(
            "general",
            "General-purpose delegated agent for broad research and mixed tasks.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("fs", crate::permission::PermissionMode::Allow),
                    ("web", crate::permission::PermissionMode::Allow),
                    ("tools", crate::permission::PermissionMode::Allow),
                    ("user", crate::permission::PermissionMode::Allow),
                    ("agent", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a general-purpose delegated agent. Investigate broadly, combine code reading with focused web research when useful, and return evidence-backed conclusions without making workspace edits unless explicitly allowed.",
        ),
        default_profile(
            "explore",
            "Read-only codebase explorer for fast repo analysis.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("fs", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                    ("web", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a focused read-only engineering explorer. Gather evidence quickly, inspect code paths, summarize findings concisely, and do not make edits.",
        ),
        default_profile(
            "scout",
            "Read-only external research agent for docs, APIs, and dependency behavior.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("fs", crate::permission::PermissionMode::Allow),
                    ("web", crate::permission::PermissionMode::Allow),
                    ("shell", crate::permission::PermissionMode::Ask),
                ],
            ),
            "You are a read-only research agent for external documentation, APIs, and dependency behavior. Prefer direct evidence from docs, source, or fetched pages, separate verified facts from inference, and do not modify the user's workspace.",
        ),
        default_profile(
            "implement",
            "Editing agent for making targeted code changes.",
            default_permission(
                crate::permission::PermissionMode::Ask,
                crate::permission::PermissionMode::Ask,
                &[],
            ),
            "You are a pragmatic implementation agent. Make the requested code changes, preserve surrounding behavior, adapt to concurrent edits, and avoid reverting unrelated work.",
        ),
        default_profile(
            "verify",
            "Validation agent for targeted testing and regression checks.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("shell", crate::permission::PermissionMode::Ask),
                    ("agent", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a verification agent. Run focused checks, inspect outputs critically, look for regressions, and report the remaining risks plainly.",
        ),
        default_profile(
            "planner",
            "Planning agent for read-only decomposition and execution strategy.",
            default_permission(
                crate::permission::PermissionMode::Allow,
                crate::permission::PermissionMode::Ask,
                &[
                    ("plan", crate::permission::PermissionMode::Allow),
                    ("agent", crate::permission::PermissionMode::Allow),
                    ("session", crate::permission::PermissionMode::Allow),
                    ("tools", crate::permission::PermissionMode::Allow),
                    ("user", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a planning agent. Break work into concrete steps, surface assumptions and blockers, and keep the output actionable. Prefer read-only investigation unless the user explicitly asks to execute.",
        ),
        default_profile(
            "reviewer",
            "Code review agent focused on bugs, risks, and missing tests.",
            default_permission(
                crate::permission::PermissionMode::Deny,
                crate::permission::PermissionMode::Ask,
                &[
                    ("shell", crate::permission::PermissionMode::Ask),
                    ("agent", crate::permission::PermissionMode::Allow),
                ],
            ),
            "You are a strict code review agent. Prioritize correctness issues, behavioral regressions, and test gaps. Findings come first; summaries are secondary.",
        ),
    ]
}

fn compaction_profile() -> AgentProfile {
    let deny = crate::permission::PermissionMode::Deny;
    AgentProfile {
        name: "compaction".to_string(),
        frontmatter: AgentFrontmatter {
            description: "Agent used only for conversation compaction.".to_string(),
            permission: crate::agent::PermissionConfig {
                path: Some(crate::agent::PathPermissionConfig {
                    workspace: Some(crate::agent::PathAccessModes {
                        read: Some(deny),
                        write: Some(deny),
                    }),
                    external: Some(crate::agent::PathAccessModes {
                        read: Some(deny),
                        write: Some(deny),
                    }),
                    ..Default::default()
                }),
                network: Some(crate::agent::NetworkPermissionConfig {
                    internet: Some(deny),
                    private: Some(deny),
                    loopback: Some(deny),
                    ..Default::default()
                }),
                tools: Some(crate::agent::ToolPermissionConfig::default()),
            },
            defaults: AgentSelectionConfig::default(),
        },
        prompt: "You are Agena's conversation compaction agent. Summarize only the transcript and context provided by the user message. Preserve the user's current objective, explicit constraints, decisions already made, important files or commands, tool results, pending work, blockers, and open questions. Do not call tools, do not invent facts, and do not mention the act of compaction. Return a concise Markdown summary with stable section headings.".to_string(),
        source_path: None,
        scope: AgentScope::Default,
    }
}

fn default_profile(
    name: &str,
    description: &str,
    permission: crate::agent::AgentPermissionConfig,
    prompt: &str,
) -> AgentProfile {
    AgentProfile {
        name: name.to_string(),
        frontmatter: AgentFrontmatter {
            description: description.to_string(),
            permission,
            defaults: AgentSelectionConfig::default(),
        },
        prompt: format!(
            "{prompt} When the user asks what tools are available, whether a tool exists, or how to inspect tool usage, do not answer from memory. Inspect the live tools gateway first. Use `tools_help` for exact tool schemas and `tools_call` to execute tools through the tools gateway. The names `tools_help` and `tools_call` are top-level gateway functions, not values for the `tool` argument of `tools_call`; pass the dotted catalog target name returned by the catalog, such as `web.search`."
        ),
        source_path: None,
        scope: AgentScope::Default,
    }
}
